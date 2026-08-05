use super::{
    checked_file_size,
    transfer::{MAX_PROTOCOL_TIMEOUTS, receive_wire, send_wire},
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
}

enum SenderStep {
    Progress,
    Idle,
    Complete,
}

pub(super) async fn run_upload(
    channel: &mut dyn SshChannel,
    initial_wire: Vec<u8>,
    paths: Vec<PathBuf>,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>> {
    let mut queue = prepare_entries(paths).await?;
    let mut sender = Sender::new().context("create ZMODEM sender")?;
    let mut current = start_next(&mut sender, &mut queue).await?;
    let mut pending = initial_wire;
    let mut timeouts = 0;

    loop {
        match drive_sender(channel, &mut sender, &mut queue, &mut current, cancellation).await? {
            SenderStep::Progress => continue,
            SenderStep::Complete => return Ok(strip_zfin_terminator(pending)),
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

fn strip_zfin_terminator(mut pending: Vec<u8>) -> Vec<u8> {
    if pending.starts_with(b"\r\n") {
        pending.drain(..2);
    }
    pending
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
) -> Result<Option<UploadFile>> {
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
    Ok(Some(UploadFile { file }))
}

async fn drive_sender(
    channel: &mut dyn SshChannel,
    sender: &mut Sender,
    queue: &mut VecDeque<UploadEntry>,
    current: &mut Option<UploadFile>,
    cancellation: &CancellationToken,
) -> Result<SenderStep> {
    match sender.poll() {
        Action::WriteWire(bytes) => {
            let bytes = bytes.to_vec();
            send_wire(channel, &bytes, cancellation).await?;
            sender.wire_written(bytes.len());
            Ok(SenderStep::Progress)
        }
        Action::ReadFile { offset, max_len } => {
            submit_file_chunk(sender, current, offset, max_len).await?;
            Ok(SenderStep::Progress)
        }
        Action::Event(Event::FileCompleted) => {
            *current = start_next(sender, queue).await?;
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
) -> Result<()> {
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
        .context("submit ZMODEM upload data")
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
