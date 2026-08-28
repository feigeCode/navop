use super::{
    ZmodemResponder, ZmodemTransferDirection, ZmodemTransferId, ZmodemTransferProgress,
    download_path,
    transfer::{
        MAX_PROTOCOL_TIMEOUTS, consume_hex_header_terminator, receive_finish_wire, receive_wire,
        send_wire,
    },
};
use anyhow::{Context as _, Result, bail};
use ssh::SshChannel;
use std::{io::ErrorKind, path::PathBuf, string::String};
use tokio::{
    fs::{File, OpenOptions},
    io::AsyncWriteExt as _,
};
use tokio_util::sync::CancellationToken;
use zmodem2::{Action, Event, Position, Receiver};

struct DownloadFile {
    path: PathBuf,
    remote_name: String,
    advertised_size: Option<u64>,
    written: u64,
    file: File,
}

#[derive(Default)]
struct DownloadProgressTracker {
    responder: ZmodemResponder,
    transfer_id: ZmodemTransferId,
    file_index: usize,
    completed: u64,
}

enum ReceiverStep {
    Progress,
    Idle,
    SessionComplete,
}

pub(super) async fn run_download(
    channel: &mut dyn SshChannel,
    initial_wire: Vec<u8>,
    directory: PathBuf,
    responder: ZmodemResponder,
    transfer_id: ZmodemTransferId,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>> {
    validate_directory(&directory).await?;
    let mut receiver = Receiver::new().context("create ZMODEM receiver")?;
    if !initial_wire.is_empty() {
        // The receiver pre-queues a ZRINIT before it has seen the remote
        // ZRQINIT. In the transfer flow the initial wire already contains
        // that request, so drop the premature handshake and let the
        // ZRQINIT handler emit the response. This prevents lrzsz `sz -e`
        // from restarting its handshake when it receives a duplicate ZRINIT.
        let initial_wire_len = match receiver.poll() {
            zmodem2::Action::WriteWire(bytes) => Some(bytes.len()),
            _ => None,
        };
        if let Some(initial_wire_len) = initial_wire_len {
            receiver.wire_written(initial_wire_len);
        }
    }
    let mut current = None;
    let mut progress = DownloadProgressTracker {
        responder,
        transfer_id,
        ..Default::default()
    };
    let result = drive_download(
        channel,
        &mut receiver,
        &mut current,
        &mut progress,
        initial_wire,
        &directory,
        cancellation,
    )
    .await;
    if result.is_err() {
        if let Some(file) = current {
            let _ = tokio::fs::remove_file(file.path).await;
        }
    }
    result
}

async fn drive_download(
    channel: &mut dyn SshChannel,
    receiver: &mut Receiver,
    current: &mut Option<DownloadFile>,
    progress: &mut DownloadProgressTracker,
    mut pending: Vec<u8>,
    directory: &std::path::Path,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>> {
    let mut timeouts = 0;
    let mut session_complete = false;
    loop {
        match drive_receiver(
            channel,
            receiver,
            current,
            progress,
            directory,
            cancellation,
        )
        .await?
        {
            ReceiverStep::Progress => continue,
            ReceiverStep::SessionComplete => {
                session_complete = true;
                continue;
            }
            ReceiverStep::Idle => {}
        }
        if session_complete {
            return finish_download(channel, pending, cancellation).await;
        }
        if feed_receiver(receiver, &mut pending)? {
            timeouts = 0;
            continue;
        }
        match receive_wire(channel, cancellation).await? {
            Some(data) => pending.extend(data),
            None => {
                timeouts += 1;
                if timeouts >= MAX_PROTOCOL_TIMEOUTS {
                    bail!("ZMODEM download timed out");
                }
                receiver.timeout().context("retry ZMODEM download")?;
            }
        }
    }
}

async fn finish_download(
    channel: &mut dyn SshChannel,
    mut pending: Vec<u8>,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>> {
    consume_hex_header_terminator(channel, &mut pending, cancellation).await?;
    loop {
        if pending.starts_with(b"OO") {
            pending.drain(..2);
            return Ok(pending);
        }
        if pending.len() >= 2 || pending.first().is_some_and(|byte| *byte != b'O') {
            return Ok(pending);
        }
        let Some(data) = receive_finish_wire(channel, cancellation).await? else {
            return Ok(pending);
        };
        pending.extend(data);
    }
}

async fn validate_directory(directory: &std::path::Path) -> Result<()> {
    let metadata = tokio::fs::metadata(directory)
        .await
        .with_context(|| format!("read download directory {}", directory.display()))?;
    if !metadata.is_dir() {
        bail!(
            "ZMODEM download destination is not a directory: {}",
            directory.display()
        );
    }
    Ok(())
}

async fn drive_receiver(
    channel: &mut dyn SshChannel,
    receiver: &mut Receiver,
    current: &mut Option<DownloadFile>,
    progress: &mut DownloadProgressTracker,
    directory: &std::path::Path,
    cancellation: &CancellationToken,
) -> Result<ReceiverStep> {
    match receiver.poll() {
        Action::WriteWire(bytes) => {
            let bytes = bytes.to_vec();
            send_wire(channel, &bytes, cancellation).await?;
            receiver.wire_written(bytes.len());
            Ok(ReceiverStep::Progress)
        }
        Action::WriteFile(bytes) => {
            let bytes = bytes.to_vec();
            write_file_chunk(current, &bytes).await?;
            let written = bytes.len() as u64;
            if let Some(file) = current.as_mut() {
                file.written = file.written.saturating_add(written);
            }
            progress.advance(current);
            receiver
                .file_written(bytes.len())
                .context("acknowledge ZMODEM download data")?;
            Ok(ReceiverStep::Progress)
        }
        Action::Event(Event::FileStarted(info)) => {
            let file = open_download_file(directory, info.name, info.size).await?;
            progress.start_file(&file);
            *current = Some(file);
            Ok(ReceiverStep::Progress)
        }
        Action::Event(Event::FileCompleted) => {
            finish_download_file(current).await?;
            progress.complete(current);
            current.take();
            Ok(ReceiverStep::Progress)
        }
        Action::Event(Event::SessionCompleted) => Ok(ReceiverStep::SessionComplete),
        Action::Event(Event::Aborted) => bail!("remote aborted ZMODEM download"),
        Action::Event(_) | Action::ReadFile { .. } => {
            bail!("unexpected ZMODEM receiver action")
        }
        Action::Idle => Ok(ReceiverStep::Idle),
        _ => Ok(ReceiverStep::Progress),
    }
}

async fn open_download_file(
    directory: &std::path::Path,
    remote_name: &[u8],
    advertised_size: Option<Position>,
) -> Result<DownloadFile> {
    let requested_path = download_path(directory, remote_name)?;
    let (path, file) = create_download_target(&requested_path).await?;
    Ok(DownloadFile {
        path,
        remote_name: String::from_utf8_lossy(remote_name).into_owned(),
        advertised_size: advertised_size.map(|size| u64::from(size.get())),
        written: 0,
        file,
    })
}

pub(super) async fn create_download_target(
    requested_path: &std::path::Path,
) -> Result<(PathBuf, File)> {
    const MAX_DUPLICATE_SUFFIX: usize = 10_000;

    for suffix in 0..=MAX_DUPLICATE_SUFFIX {
        let path = if suffix == 0 {
            requested_path.to_path_buf()
        } else {
            suffixed_download_path(requested_path, suffix)
        };
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("create ZMODEM download file {}", path.display()));
            }
        }
    }

    bail!(
        "could not allocate a unique ZMODEM download path for {}",
        requested_path.display()
    )
}

fn suffixed_download_path(path: &std::path::Path, suffix: usize) -> PathBuf {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let name = match path.extension() {
        Some(extension) => format!("{stem} ({suffix}).{}", extension.to_string_lossy()),
        None => format!("{stem} ({suffix})"),
    };
    path.with_file_name(name)
}

async fn write_file_chunk(current: &mut Option<DownloadFile>, bytes: &[u8]) -> Result<()> {
    current
        .as_mut()
        .context("ZMODEM receiver produced data without an open file")?
        .file
        .write_all(bytes)
        .await
        .context("write ZMODEM download file")
}

impl DownloadProgressTracker {
    fn start_file(&mut self, file: &DownloadFile) {
        self.responder
            .begin_download(self.transfer_id, self.snapshot(file, 0));
    }

    fn advance(&mut self, current: &Option<DownloadFile>) {
        let Some(file) = current.as_ref() else {
            return;
        };
        let current_file_transferred = file.written;
        let transferred = self.completed.saturating_add(file.written);
        self.responder.update_download(
            self.transfer_id,
            self.snapshot_with_current(file, current_file_transferred, transferred),
        );
    }

    fn complete(&mut self, current: &Option<DownloadFile>) {
        let Some(file) = current.as_ref() else {
            return;
        };
        self.completed = self.completed.saturating_add(file.written);
        self.file_index = self.file_index.saturating_add(1);
        self.responder
            .update_download(self.transfer_id, self.snapshot(file, self.completed));
    }

    fn snapshot(&self, file: &DownloadFile, transferred: u64) -> ZmodemTransferProgress {
        self.snapshot_with_current(file, file.written, transferred)
    }

    fn snapshot_with_current(
        &self,
        file: &DownloadFile,
        current_file_transferred: u64,
        transferred: u64,
    ) -> ZmodemTransferProgress {
        let current_file_transferred = file
            .advertised_size
            .map(|size| current_file_transferred.min(size))
            .unwrap_or(0);
        ZmodemTransferProgress {
            transfer_id: self.transfer_id,
            direction: ZmodemTransferDirection::Download,
            file_name: file.remote_name.clone(),
            file_index: self.file_index,
            file_count: 0,
            current_file_transferred,
            current_file_total: file.advertised_size.unwrap_or(0),
            transferred,
            total: 0,
        }
    }
}

async fn finish_download_file(current: &mut Option<DownloadFile>) -> Result<()> {
    let file = current
        .as_mut()
        .context("ZMODEM receiver completed a file that was not open")?;
    file.file
        .flush()
        .await
        .context("flush ZMODEM download file")
}

fn feed_receiver(receiver: &mut Receiver, pending: &mut Vec<u8>) -> Result<bool> {
    if pending.is_empty() {
        return Ok(false);
    }
    let consumed = receiver
        .submit_wire(pending)
        .context("process ZMODEM download wire data")?;
    if consumed == 0 {
        return Ok(false);
    }
    pending.drain(..consumed);
    Ok(true)
}
