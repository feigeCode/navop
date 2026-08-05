use super::{
    DetectedZmodem, ZmodemDirection, ZmodemPickerKind, ZmodemPickerResponse, ZmodemResponder,
};
use anyhow::{Context as _, Result, bail};
use ssh::{ChannelEvent, SshChannel};
use std::{error::Error as StdError, fmt, time::Duration};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

pub(crate) const ZCAN: &[u8] = b"\x18\x18\x18\x18\x18\x18\x18\x18\x08\x08\x08\x08\x08\x08\x08\x08";
const SEND_TIMEOUT: Duration = Duration::from_secs(30);
const RECEIVE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_PICKER_WIRE_BYTES: usize = 1024 * 1024;
pub(super) const MAX_PROTOCOL_TIMEOUTS: usize = 6;

#[derive(Debug)]
struct ChannelClosed;

impl fmt::Display for ChannelClosed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SSH channel closed before ZMODEM transfer completed")
    }
}

impl StdError for ChannelClosed {}

pub(crate) async fn run_transfer(
    channel: &mut dyn SshChannel,
    detected: DetectedZmodem,
    responder: &ZmodemResponder,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>> {
    let result = run_selected_transfer(channel, detected, responder, cancellation).await;
    if result.is_err() {
        send_cancel(channel).await;
    }
    result
}

async fn run_selected_transfer(
    channel: &mut dyn SshChannel,
    detected: DetectedZmodem,
    responder: &ZmodemResponder,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>> {
    let direction = detected.direction;
    let (response, wire) = request_picker(
        channel,
        responder,
        picker_kind(direction),
        detected.wire,
        cancellation,
    )
    .await?;
    match direction {
        ZmodemDirection::Upload => {
            let ZmodemPickerResponse::UploadFiles(paths) = response else {
                bail!("ZMODEM upload was cancelled");
            };
            super::upload::run_upload(channel, wire, paths, cancellation).await
        }
        ZmodemDirection::Download => {
            let ZmodemPickerResponse::DownloadDirectory(directory) = response else {
                bail!("ZMODEM download was cancelled");
            };
            super::download::run_download(channel, wire, directory, cancellation).await
        }
    }
}

fn picker_kind(direction: ZmodemDirection) -> ZmodemPickerKind {
    match direction {
        ZmodemDirection::Upload => ZmodemPickerKind::UploadFiles,
        ZmodemDirection::Download => ZmodemPickerKind::DownloadDirectory,
    }
}

pub(crate) fn checked_file_size(size: u64) -> Result<u32> {
    u32::try_from(size).context("ZMODEM cannot upload files of 4 GiB or larger")
}

async fn request_picker(
    channel: &mut dyn SshChannel,
    responder: &ZmodemResponder,
    kind: ZmodemPickerKind,
    mut wire: Vec<u8>,
    cancellation: &CancellationToken,
) -> Result<(ZmodemPickerResponse, Vec<u8>)> {
    let request = responder.request(kind);
    tokio::pin!(request);
    loop {
        tokio::select! {
            _ = cancellation.cancelled() => bail!("ZMODEM transfer was cancelled"),
            response = &mut request => return Ok((response?, wire)),
            event = channel.recv() => append_picker_wire(&mut wire, event)?,
        }
    }
}

fn append_picker_wire(wire: &mut Vec<u8>, event: Option<ChannelEvent>) -> Result<()> {
    match event {
        Some(ChannelEvent::Data(data)) | Some(ChannelEvent::ExtendedData { data, .. }) => {
            if wire.len().saturating_add(data.len()) > MAX_PICKER_WIRE_BYTES {
                bail!("too much ZMODEM data arrived while waiting for file selection");
            }
            wire.extend(data);
            Ok(())
        }
        Some(ChannelEvent::Eof) | Some(ChannelEvent::Close) | None => Err(ChannelClosed.into()),
        Some(_) => Ok(()),
    }
}

pub(crate) fn is_channel_closed(error: &anyhow::Error) -> bool {
    error.downcast_ref::<ChannelClosed>().is_some()
}

pub(super) async fn send_wire(
    channel: &mut dyn SshChannel,
    bytes: &[u8],
    cancellation: &CancellationToken,
) -> Result<()> {
    tokio::select! {
        _ = cancellation.cancelled() => bail!("ZMODEM transfer was cancelled"),
        result = timeout(SEND_TIMEOUT, channel.send_data(bytes)) => {
            result.context("timed out writing ZMODEM data")?
                .context("write ZMODEM data")
        }
    }
}

pub(super) async fn receive_wire(
    channel: &mut dyn SshChannel,
    cancellation: &CancellationToken,
) -> Result<Option<Vec<u8>>> {
    loop {
        let event = tokio::select! {
            _ = cancellation.cancelled() => bail!("ZMODEM transfer was cancelled"),
            result = timeout(RECEIVE_TIMEOUT, channel.recv()) => {
                match result {
                    Ok(event) => event,
                    Err(_) => return Ok(None),
                }
            }
        };
        match event {
            Some(ChannelEvent::Data(data)) | Some(ChannelEvent::ExtendedData { data, .. }) => {
                return Ok(Some(data));
            }
            Some(ChannelEvent::Eof) | Some(ChannelEvent::Close) | None => {
                return Err(ChannelClosed.into());
            }
            Some(_) => {}
        }
    }
}

async fn send_cancel(channel: &mut dyn SshChannel) {
    let _ = timeout(SEND_TIMEOUT, channel.send_data(ZCAN)).await;
}
