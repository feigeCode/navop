use crate::helper_protocol::{HelperEvent, HelperReconnectReason};

use super::*;

pub(super) fn helper_event_to_output(
    event: HelperEvent,
    protocol: RemoteDesktopProtocol,
) -> anyhow::Result<Option<RemoteDesktopOutput>> {
    Ok(match event {
        // Helper status text is an untrusted diagnostic boundary. Keep the
        // backend-owned status messages user-visible, but never forward
        // arbitrary helper text that may contain source paths or protocol
        // implementation details.
        HelperEvent::Status { .. } => None,
        HelperEvent::Connected { width, height } => Some(RemoteDesktopOutput::Connected {
            width,
            height,
            capabilities: capabilities_for_protocol(protocol),
        }),
        HelperEvent::Frame { width, height, .. } => Some(RemoteDesktopOutput::Frame {
            width,
            height,
            rgba: event.into_rgba()?,
        }),
        HelperEvent::FrameBytes { .. }
        | HelperEvent::FrameBgraBytes { .. }
        | HelperEvent::FrameBgraRects { .. } => {
            anyhow::bail!("binary frame payload is missing")
        }
        HelperEvent::CursorRgbaBytes { .. } => {
            anyhow::bail!("binary cursor payload is missing")
        }
        HelperEvent::CursorDefault => Some(RemoteDesktopOutput::CursorDefault),
        HelperEvent::CursorHidden => Some(RemoteDesktopOutput::CursorHidden),
        HelperEvent::CursorPosition { x, y } => Some(RemoteDesktopOutput::CursorPosition { x, y }),
        HelperEvent::ClipboardText { text } => Some(RemoteDesktopOutput::ClipboardText { text }),
        HelperEvent::ClipboardFilesReady { transfer_id, paths } => {
            Some(RemoteDesktopOutput::ClipboardFilesReady { transfer_id, paths })
        }
        HelperEvent::ClipboardTransferFailed {
            transfer_id,
            message,
        } => Some(RemoteDesktopOutput::ClipboardTransferFailed {
            transfer_id,
            message,
        }),
        HelperEvent::Reconnecting { reason, delay_secs } => {
            Some(RemoteDesktopOutput::Reconnecting(RemoteDesktopReconnect {
                reason: reconnect_reason(reason),
                delay_secs,
            }))
        }
        HelperEvent::ConnectionFailure { .. } | HelperEvent::Terminated { .. } => None,
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

pub(super) fn helper_disconnect(event: &HelperEvent) -> Option<HelperDisconnect> {
    match event {
        HelperEvent::ConnectionFailure { message } => Some(HelperDisconnect {
            kind: Some(HelperDisconnectKind::ConnectionFailure),
            reason: message.clone(),
        }),
        HelperEvent::Terminated { message } => Some(HelperDisconnect {
            kind: Some(HelperDisconnectKind::Terminated),
            reason: message.clone(),
        }),
        _ => None,
    }
}

#[cfg(test)]
#[path = "helper_events_tests.rs"]
mod tests;
