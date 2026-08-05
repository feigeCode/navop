use super::{
    download_path,
    transfer::{MAX_PROTOCOL_TIMEOUTS, receive_wire, send_wire},
};
use anyhow::{Context as _, Result, bail};
use ssh::SshChannel;
use std::path::PathBuf;
use tokio::{
    fs::{File, OpenOptions},
    io::AsyncWriteExt as _,
};
use tokio_util::sync::CancellationToken;
use zmodem2::{Action, Event, Receiver};

struct DownloadFile {
    path: PathBuf,
    file: File,
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
    cancellation: &CancellationToken,
) -> Result<Vec<u8>> {
    validate_directory(&directory).await?;
    let mut receiver = Receiver::new().context("create ZMODEM receiver")?;
    let mut current = None;
    let result = drive_download(
        channel,
        &mut receiver,
        &mut current,
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
    mut pending: Vec<u8>,
    directory: &std::path::Path,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>> {
    let mut timeouts = 0;
    let mut session_complete = false;
    loop {
        match drive_receiver(channel, receiver, current, directory, cancellation).await? {
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
    strip_zfin_terminator(&mut pending);
    loop {
        if pending.starts_with(b"OO") {
            pending.drain(..2);
            return Ok(pending);
        }
        if pending.len() >= 2 || pending.first().is_some_and(|byte| *byte != b'O') {
            return Ok(pending);
        }
        let Some(data) = receive_wire(channel, cancellation).await? else {
            return Ok(pending);
        };
        pending.extend(data);
    }
}

fn strip_zfin_terminator(pending: &mut Vec<u8>) {
    if pending.starts_with(b"\r\n") {
        pending.drain(..2);
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
            receiver
                .file_written(bytes.len())
                .context("acknowledge ZMODEM download data")?;
            Ok(ReceiverStep::Progress)
        }
        Action::Event(Event::FileStarted(info)) => {
            *current = Some(open_download_file(directory, info.name).await?);
            Ok(ReceiverStep::Progress)
        }
        Action::Event(Event::FileCompleted) => {
            finish_download_file(current).await?;
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
) -> Result<DownloadFile> {
    let path = download_path(directory, remote_name)?;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .await
        .with_context(|| format!("create ZMODEM download file {}", path.display()))?;
    Ok(DownloadFile { path, file })
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

async fn finish_download_file(current: &mut Option<DownloadFile>) -> Result<()> {
    let mut file = current
        .take()
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
