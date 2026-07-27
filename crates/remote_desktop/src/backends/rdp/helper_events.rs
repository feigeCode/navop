use crate::helper_protocol::{HelperEvent, HelperReconnectReason};

use super::*;

pub(super) fn helper_event_to_output(
    event: HelperEvent,
    protocol: RemoteDesktopProtocol,
) -> anyhow::Result<RemoteDesktopOutput> {
    Ok(match event {
        HelperEvent::Status { message } => RemoteDesktopOutput::Status(message),
        HelperEvent::Connected { width, height } => RemoteDesktopOutput::Connected {
            width,
            height,
            capabilities: capabilities_for_protocol(protocol),
        },
        HelperEvent::Frame { width, height, .. } => RemoteDesktopOutput::Frame {
            width,
            height,
            rgba: event.into_rgba()?,
        },
        HelperEvent::FrameBytes { .. }
        | HelperEvent::FrameBgraBytes { .. }
        | HelperEvent::FrameBgraRects { .. } => {
            anyhow::bail!("binary frame payload is missing")
        }
        HelperEvent::CursorRgbaBytes { .. } => {
            anyhow::bail!("binary cursor payload is missing")
        }
        HelperEvent::CursorDefault => RemoteDesktopOutput::CursorDefault,
        HelperEvent::CursorHidden => RemoteDesktopOutput::CursorHidden,
        HelperEvent::CursorPosition { x, y } => RemoteDesktopOutput::CursorPosition { x, y },
        HelperEvent::ClipboardText { text } => RemoteDesktopOutput::ClipboardText { text },
        HelperEvent::ClipboardFilesReady { transfer_id, paths } => {
            RemoteDesktopOutput::ClipboardFilesReady { transfer_id, paths }
        }
        HelperEvent::ClipboardTransferFailed {
            transfer_id,
            message,
        } => RemoteDesktopOutput::ClipboardTransferFailed {
            transfer_id,
            message,
        },
        HelperEvent::Reconnecting { reason, delay_secs } => {
            RemoteDesktopOutput::Reconnecting(RemoteDesktopReconnect {
                reason: reconnect_reason(reason),
                delay_secs,
            })
        }
        HelperEvent::ConnectionFailure { message } => {
            RemoteDesktopOutput::ConnectionFailure(message)
        }
        HelperEvent::Terminated { message } => RemoteDesktopOutput::Terminated(message),
    })
}

fn reconnect_reason(reason: HelperReconnectReason) -> RemoteDesktopReconnectReason {
    match reason {
        HelperReconnectReason::DisplayUpdate => RemoteDesktopReconnectReason::DisplayUpdate,
        HelperReconnectReason::SessionError => RemoteDesktopReconnectReason::SessionError,
        HelperReconnectReason::ConnectionLost => RemoteDesktopReconnectReason::ConnectionLost,
        HelperReconnectReason::Manual => RemoteDesktopReconnectReason::Manual,
    }
}

fn capabilities_for_protocol(protocol: RemoteDesktopProtocol) -> RemoteDesktopCapabilities {
    match protocol {
        RemoteDesktopProtocol::Rdp => RemoteDesktopCapabilities::rdp_mvp(),
        RemoteDesktopProtocol::Vnc => RemoteDesktopCapabilities::vnc_mvp(),
    }
}

pub(super) fn helper_disconnect_message(event: &HelperEvent) -> Option<String> {
    match event {
        HelperEvent::ConnectionFailure { message } | HelperEvent::Terminated { message } => {
            Some(message.clone())
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "helper_events_tests.rs"]
mod tests;
