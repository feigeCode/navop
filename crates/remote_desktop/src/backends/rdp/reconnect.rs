use std::time::{Duration, Instant};

use super::*;

const RECONNECT_POLL_INTERVAL: Duration = Duration::from_millis(25);
const RECONNECT_DELAYS: [Duration; 4] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(10),
];

pub(super) fn wait_before_reconnect(
    connect: &mut HelperRequest,
    latest_clipboard_text: &mut Option<String>,
    latest_clipboard_files: &mut Option<ClipboardFilesSnapshot>,
    input_rx: &mut tokio::sync::mpsc::UnboundedReceiver<RemoteDesktopInput>,
    delay: Duration,
    protocol: RemoteDesktopProtocol,
) -> bool {
    let deadline = Instant::now() + delay;
    loop {
        match input_rx.try_recv() {
            Ok(RemoteDesktopInput::Close) => return false,
            Ok(RemoteDesktopInput::Reconnect) => return true,
            Ok(input) => remember_reconnect_state(
                &input,
                connect,
                latest_clipboard_text,
                latest_clipboard_files,
                protocol,
            ),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => return false,
        }
        if Instant::now() >= deadline {
            return true;
        }
        std::thread::sleep(RECONNECT_POLL_INTERVAL);
    }
}

pub(super) fn remember_reconnect_state(
    input: &RemoteDesktopInput,
    connect: &mut HelperRequest,
    latest_clipboard_text: &mut Option<String>,
    latest_clipboard_files: &mut Option<ClipboardFilesSnapshot>,
    protocol: RemoteDesktopProtocol,
) {
    match input {
        RemoteDesktopInput::Resize {
            width,
            height,
            scale_factor,
        } => remember_size(connect, *width, *height, *scale_factor),
        RemoteDesktopInput::ClipboardText { text } => {
            *latest_clipboard_text = Some(text.clone());
            *latest_clipboard_files = None;
        }
        RemoteDesktopInput::ClipboardFiles { transfer_id, paths }
            if protocol == RemoteDesktopProtocol::Rdp =>
        {
            tracing::debug!(
                transfer_id,
                file_count = paths.len(),
                "remembering RDP clipboard files for helper reconnect"
            );
            *latest_clipboard_text = None;
            *latest_clipboard_files = Some(ClipboardFilesSnapshot {
                transfer_id: *transfer_id,
                paths: paths.clone(),
            });
        }
        RemoteDesktopInput::CancelClipboardTransfer { transfer_id } => {
            forget_cancelled_files(latest_clipboard_files, *transfer_id);
        }
        _ => {}
    }
}

fn remember_size(connect: &mut HelperRequest, width: u16, height: u16, scale_factor: u32) {
    if let HelperRequest::Connect {
        width: connect_width,
        height: connect_height,
        scale_factor: connect_scale_factor,
        ..
    } = connect
    {
        *connect_width = width;
        *connect_height = height;
        *connect_scale_factor = scale_factor;
    }
}

fn forget_cancelled_files(
    latest_clipboard_files: &mut Option<ClipboardFilesSnapshot>,
    transfer_id: u64,
) {
    if latest_clipboard_files
        .as_ref()
        .is_some_and(|snapshot| snapshot.transfer_id == transfer_id)
    {
        *latest_clipboard_files = None;
    }
}

pub(super) fn reconnect_delay(attempt: usize) -> Duration {
    RECONNECT_DELAYS[attempt.min(RECONNECT_DELAYS.len() - 1)]
}

pub(super) fn reconnect_event(reason: &str, delay: Duration) -> RemoteDesktopReconnect {
    RemoteDesktopReconnect {
        reason: classify_disconnect_reason(reason),
        delay_secs: Some(delay.as_secs()),
    }
}

fn classify_disconnect_reason(reason: &str) -> RemoteDesktopReconnectReason {
    if reason.contains("Fast-Path") {
        RemoteDesktopReconnectReason::DisplayUpdate
    } else if reason.contains("/Users/") || reason.contains(".cargo/git/checkouts") {
        RemoteDesktopReconnectReason::SessionError
    } else {
        RemoteDesktopReconnectReason::ConnectionLost
    }
}

#[cfg(test)]
#[path = "reconnect_tests.rs"]
mod tests;
