use super::{
    ZmodemResponder, ZmodemTransferDirection, ZmodemTransferId, ZmodemTransferProgress,
    checked_file_size,
    transfer::{MAX_PROTOCOL_TIMEOUTS, consume_hex_header_terminator, receive_wire, send_wire},
    upload_file_name,
};
use anyhow::{Context as _, Result, bail};
use ssh::SshChannel;
use std::{collections::VecDeque, path::PathBuf};
use tokio::{
    fs::File,
    io::{AsyncReadExt as _, AsyncSeekExt as _, SeekFrom},
};
use tokio_util::sync::CancellationToken;
use zmodem2::{Action, Event, FileInfo, Position, Sender};

struct UploadEntry {
    path: PathBuf,
    name: Vec<u8>,
    size: u32,
}

struct UploadFile {
    file: File,
    name: String,
    index: usize,
    size: u64,
}

pub(super) struct UploadRequest {
    pub(super) initial_wire: Vec<u8>,
    pub(super) paths: Vec<PathBuf>,
    pub(super) responder: ZmodemResponder,
    pub(super) transfer_id: ZmodemTransferId,
}

struct UploadProgressTracker {
    responder: ZmodemResponder,
    transfer_id: ZmodemTransferId,
    file_count: usize,
    total: u64,
    completed: u64,
    current_high_water: u64,
}

enum SenderStep {
    Progress,
    Idle,
    Complete,
}

pub(super) async fn run_upload(
    channel: &mut dyn SshChannel,
    request: UploadRequest,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>> {
    let mut queue = prepare_entries(request.paths).await?;
    let mut progress = UploadProgressTracker::new(&queue, request.responder, request.transfer_id);
    let mut sender = Sender::new().context("create ZMODEM sender")?;
    sender.set_streaming_window(usize::MAX);
    let mut current = start_next(&mut sender, &mut queue, progress.file_count).await?;
    let _progress_guard = progress.begin(current.as_ref())?;
    let mut pending = request.initial_wire;
    let mut timeouts = 0;

    loop {
        match drive_sender(
            channel,
            &mut sender,
            &mut queue,
            &mut current,
            cancellation,
            &mut progress,
        )
        .await?
        {
            SenderStep::Progress => continue,
            SenderStep::Complete => {
                drain_sender(channel, &mut sender, cancellation).await?;
                consume_hex_header_terminator(channel, &mut pending, cancellation).await?;
                return Ok(pending);
            }
            SenderStep::Idle => {}
        }
        if feed_sender(&mut sender, &mut pending)? {
            timeouts = 0;
            continue;
        }
        match receive_wire(channel, cancellation).await? {
            Some(data) => pending.extend(data),
            None => {
                timeouts += 1;
                if timeouts >= MAX_PROTOCOL_TIMEOUTS {
                    bail!("ZMODEM upload timed out");
                }
                sender.timeout().context("retry ZMODEM upload")?;
            }
        }
    }
}

/// Drains any wire bytes the sender still has queued after the session
/// completes. The ZFIN acknowledgement (`OO`) lives in the sender's outgoing
/// buffer at this point, and the remote `rz` blocks on it before exiting, so
/// returning early without flushing it would leave the transfer hung.
async fn drain_sender(
    channel: &mut dyn SshChannel,
    sender: &mut Sender,
    cancellation: &CancellationToken,
) -> Result<()> {
    while let Action::WriteWire(bytes) = sender.poll() {
        let bytes = bytes.to_vec();
        send_wire(channel, &bytes, cancellation).await?;
        sender.wire_written(bytes.len());
    }
    Ok(())
}

async fn prepare_entries(paths: Vec<PathBuf>) -> Result<VecDeque<UploadEntry>> {
    if paths.is_empty() {
        bail!("no files selected for ZMODEM upload");
    }
    let mut entries = VecDeque::with_capacity(paths.len());
    for path in paths {
        let metadata = tokio::fs::metadata(&path)
            .await
            .with_context(|| format!("read upload metadata for {}", path.display()))?;
        if !metadata.is_file() {
            bail!("ZMODEM upload path is not a file: {}", path.display());
        }
        entries.push_back(UploadEntry {
            name: upload_file_name(&path)?,
            size: checked_file_size(metadata.len())?,
            path,
        });
    }
    Ok(entries)
}

async fn start_next(
    sender: &mut Sender,
    queue: &mut VecDeque<UploadEntry>,
    file_count: usize,
) -> Result<Option<UploadFile>> {
    let index = file_count.saturating_sub(queue.len());
    let Some(entry) = queue.pop_front() else {
        sender.finish().context("finish ZMODEM upload session")?;
        return Ok(None);
    };
    let file = File::open(&entry.path)
        .await
        .with_context(|| format!("open upload file {}", entry.path.display()))?;
    sender
        .start_file(FileInfo::new(&entry.name, Some(Position::new(entry.size))))
        .context("start ZMODEM upload file")?;
    Ok(Some(UploadFile {
        file,
        name: String::from_utf8_lossy(&entry.name).into_owned(),
        index,
        size: u64::from(entry.size),
    }))
}

async fn drive_sender(
    channel: &mut dyn SshChannel,
    sender: &mut Sender,
    queue: &mut VecDeque<UploadEntry>,
    current: &mut Option<UploadFile>,
    cancellation: &CancellationToken,
    progress: &mut UploadProgressTracker,
) -> Result<SenderStep> {
    match sender.poll() {
        Action::WriteWire(bytes) => {
            let bytes = bytes.to_vec();
            send_wire(channel, &bytes, cancellation).await?;
            sender.wire_written(bytes.len());
            Ok(SenderStep::Progress)
        }
        Action::ReadFile { offset, max_len } => {
            let read = submit_file_chunk(sender, current, offset, max_len).await?;
            progress.advance(current.as_ref(), offset, read)?;
            Ok(SenderStep::Progress)
        }
        Action::Event(Event::FileCompleted) => {
            progress.complete(current.as_ref())?;
            *current = start_next(sender, queue, progress.file_count).await?;
            progress.start_file(current.as_ref());
            Ok(SenderStep::Progress)
        }
        Action::Event(Event::FileSkipped) => {
            *current = start_next(sender, queue, progress.file_count).await?;
            progress.start_file(current.as_ref());
            Ok(SenderStep::Progress)
        }
        Action::Event(Event::SessionCompleted) => Ok(SenderStep::Complete),
        Action::Event(Event::Aborted) => bail!("remote aborted ZMODEM upload"),
        Action::Event(Event::FileStarted(_)) | Action::WriteFile(_) => {
            bail!("unexpected ZMODEM sender action")
        }
        Action::Idle => Ok(SenderStep::Idle),
        _ => Ok(SenderStep::Progress),
    }
}

async fn submit_file_chunk(
    sender: &mut Sender,
    current: &mut Option<UploadFile>,
    offset: Position,
    max_len: usize,
) -> Result<usize> {
    let current = current
        .as_mut()
        .context("ZMODEM sender requested data without an open file")?;
    current
        .file
        .seek(SeekFrom::Start(u64::from(offset.get())))
        .await
        .context("seek ZMODEM upload file")?;
    let mut buffer = vec![0; max_len];
    let read = current
        .file
        .read(&mut buffer)
        .await
        .context("read ZMODEM upload file")?;
    sender
        .submit_file(&buffer[..read])
        .context("submit ZMODEM upload data")?;
    Ok(read)
}

fn feed_sender(sender: &mut Sender, pending: &mut Vec<u8>) -> Result<bool> {
    if pending.is_empty() {
        return Ok(false);
    }
    let consumed = sender
        .submit_wire(pending)
        .context("process ZMODEM upload wire data")?;
    if consumed == 0 {
        return Ok(false);
    }
    pending.drain(..consumed);
    Ok(true)
}

impl UploadProgressTracker {
    fn new(
        queue: &VecDeque<UploadEntry>,
        responder: ZmodemResponder,
        transfer_id: ZmodemTransferId,
    ) -> Self {
        Self {
            responder,
            transfer_id,
            file_count: queue.len(),
            total: queue.iter().map(|entry| u64::from(entry.size)).sum(),
            completed: 0,
            current_high_water: 0,
        }
    }

    fn begin(&self, current: Option<&UploadFile>) -> Result<super::TransferProgressGuard> {
        let current = current.context("ZMODEM upload has no file to start")?;
        Ok(self
            .responder
            .begin_upload(self.transfer_id, self.snapshot(current)))
    }

    fn start_file(&mut self, current: Option<&UploadFile>) {
        self.current_high_water = 0;
        if let Some(current) = current {
            self.responder
                .update_upload(self.transfer_id, self.snapshot(current));
        }
    }

    fn advance(
        &mut self,
        current: Option<&UploadFile>,
        offset: Position,
        read: usize,
    ) -> Result<()> {
        let current = current.context("ZMODEM upload progress has no current file")?;
        let position = u64::from(offset.get())
            .saturating_add(read as u64)
            .min(current.size);
        self.current_high_water = self.current_high_water.max(position);
        self.responder
            .update_upload(self.transfer_id, self.snapshot(current));
        Ok(())
    }

    fn complete(&mut self, current: Option<&UploadFile>) -> Result<()> {
        let current = current.context("ZMODEM completed without a current upload file")?;
        self.current_high_water = current.size;
        self.responder
            .update_upload(self.transfer_id, self.snapshot(current));
        self.completed = self.completed.saturating_add(current.size).min(self.total);
        Ok(())
    }

    fn snapshot(&self, current: &UploadFile) -> ZmodemTransferProgress {
        ZmodemTransferProgress {
            transfer_id: self.transfer_id,
            direction: ZmodemTransferDirection::Upload,
            file_name: current.name.clone(),
            file_index: current.index,
            file_count: self.file_count,
            current_file_transferred: self.current_high_water.min(current.size),
            current_file_total: current.size,
            transferred: self
                .completed
                .saturating_add(self.current_high_water)
                .min(self.total),
            total: self.total,
        }
    }
}
