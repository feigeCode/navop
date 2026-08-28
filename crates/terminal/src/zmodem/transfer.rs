use super::{
    DetectedZmodem, ZmodemDirection, ZmodemPickerKind, ZmodemPickerResponse, ZmodemResponder,
    ZmodemTransferDirection, ZmodemTransferOutcome,
};
use anyhow::{Context as _, Result, bail};
use ssh::{ChannelEvent, SshChannel};
use std::{error::Error as StdError, fmt, time::Duration};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

pub(crate) const ZCAN: &[u8] = b"\x18\x18\x18\x18\x18\x18\x18\x18\x08\x08\x08\x08\x08\x08\x08\x08";
const SEND_TIMEOUT: Duration = Duration::from_secs(30);
const CANCEL_SEND_TIMEOUT: Duration = Duration::from_secs(1);
const RECEIVE_TIMEOUT: Duration = Duration::from_secs(10);
const FINISH_RECEIVE_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_PICKER_WIRE_BYTES: usize = 1024 * 1024;
pub(super) const MAX_PROTOCOL_TIMEOUTS: usize = 6;

#[derive(Debug)]
struct ChannelClosed;

#[derive(Debug)]
struct TransferCancelled;

impl fmt::Display for ChannelClosed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SSH channel closed before ZMODEM transfer completed")
    }
}

impl StdError for ChannelClosed {}

impl fmt::Display for TransferCancelled {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ZMODEM transfer was cancelled")
    }
}

impl StdError for TransferCancelled {}

pub(crate) async fn run_transfer(
    channel: &mut dyn SshChannel,
    detected: DetectedZmodem,
    responder: &ZmodemResponder,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>> {
    let direction = match detected.direction {
        ZmodemDirection::Upload => ZmodemTransferDirection::Upload,
        ZmodemDirection::Download => ZmodemTransferDirection::Download,
    };
    let transfer_id = responder.begin_transfer(direction);
    let result =
        run_selected_transfer(channel, detected, responder, transfer_id, cancellation).await;
    let was_cancelled = cancellation.is_cancelled();
    if result.is_err() {
        send_cancel(channel).await;
    }
    responder.finish_transfer(
        transfer_id,
        match result {
            Ok(_) => ZmodemTransferOutcome::Succeeded,
            Err(ref error) => {
                if was_cancelled || error.downcast_ref::<TransferCancelled>().is_some() {
                    ZmodemTransferOutcome::Cancelled
                } else {
                    ZmodemTransferOutcome::Failed(format!("{error:#}"))
                }
            }
        },
    );
    result
}

async fn run_selected_transfer(
    channel: &mut dyn SshChannel,
    detected: DetectedZmodem,
    responder: &ZmodemResponder,
    transfer_id: super::ZmodemTransferId,
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
                return Err(TransferCancelled.into());
            };
            let request = super::upload::UploadRequest {
                initial_wire: wire,
                paths,
                responder: responder.clone(),
                transfer_id,
            };
            super::upload::run_upload(channel, request, cancellation).await
        }
        ZmodemDirection::Download => {
            let ZmodemPickerResponse::DownloadDirectory(directory) = response else {
                return Err(TransferCancelled.into());
            };
            super::download::run_download(
                channel,
                wire,
                directory,
                responder.clone(),
                transfer_id,
                cancellation,
            )
            .await
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

pub(super) fn strip_hex_header_terminator(pending: &mut Vec<u8>) -> bool {
    if pending.len() >= 2 && pending[0] == b'\r' && pending[1] & 0x7f == b'\n' {
        pending.drain(..2);
        true
    } else {
        false
    }
}

pub(super) async fn consume_hex_header_terminator(
    channel: &mut dyn SshChannel,
    pending: &mut Vec<u8>,
    cancellation: &CancellationToken,
) -> Result<()> {
    if pending.first().is_some_and(|byte| *byte != b'\r') {
        return Ok(());
    }
    while pending.len() < 2 {
        let Some(data) = receive_finish_wire(channel, cancellation).await? else {
            return Ok(());
        };
        pending.extend(data);
        if pending.first() != Some(&b'\r') {
            return Ok(());
        }
    }
    strip_hex_header_terminator(pending);
    Ok(())
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
    receive_wire_with_timeout(channel, cancellation, RECEIVE_TIMEOUT).await
}

pub(super) async fn receive_finish_wire(
    channel: &mut dyn SshChannel,
    cancellation: &CancellationToken,
) -> Result<Option<Vec<u8>>> {
    receive_wire_with_timeout(channel, cancellation, FINISH_RECEIVE_TIMEOUT).await
}

async fn receive_wire_with_timeout(
    channel: &mut dyn SshChannel,
    cancellation: &CancellationToken,
    receive_timeout: Duration,
) -> Result<Option<Vec<u8>>> {
    loop {
        let event = tokio::select! {
            _ = cancellation.cancelled() => bail!("ZMODEM transfer was cancelled"),
            result = timeout(receive_timeout, channel.recv()) => {
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
    let _ = timeout(CANCEL_SEND_TIMEOUT, channel.send_data(ZCAN)).await;
}
