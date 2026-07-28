use crate::{RemoteDesktopProtocol, capabilities::RemoteDesktopCapabilities};

#[derive(Clone, PartialEq, Eq)]
pub enum RemoteDesktopOutput {
    Connected {
        width: u16,
        height: u16,
        capabilities: RemoteDesktopCapabilities,
    },
    Frame {
        width: u16,
        height: u16,
        rgba: Vec<u8>,
    },
    FrameBgra {
        width: u16,
        height: u16,
        bgra: Vec<u8>,
    },
    FrameBgraRects {
        width: u16,
        height: u16,
        rects: Vec<RemoteDesktopFrameRect>,
        bgra: Vec<u8>,
    },
    CursorDefault,
    CursorHidden,
    CursorPosition {
        x: u16,
        y: u16,
    },
    CursorBitmap(RemoteDesktopCursor),
    ClipboardText {
        text: String,
    },
    ClipboardFilesReady {
        transfer_id: u64,
        paths: Vec<String>,
    },
    ClipboardTransferFailed {
        transfer_id: u64,
        message: String,
    },
    /// The backend is tearing down the current helper session and will start
    /// another one. The event is deliberately structured so the view can
    /// localize it without parsing or exposing backend error text.
    Reconnecting(RemoteDesktopReconnect),
    Status(String),
    ConnectionFailure(RemoteDesktopFailure),
    Terminated(RemoteDesktopFailure),
}

#[derive(Clone, PartialEq, Eq)]
pub enum RemoteDesktopFailure {
    AuthenticationFailed,
    SessionTakenOver,
    HostUnreachable,
    ServerEndedSession,
    ConnectionFailed,
    ProviderVersion {
        protocol: RemoteDesktopProtocol,
        installed: String,
        required: String,
        invalid: bool,
    },
}

#[derive(Clone, PartialEq, Eq)]
pub struct RemoteDesktopCursor {
    pub width: u16,
    pub height: u16,
    pub hotspot_x: u16,
    pub hotspot_y: u16,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemoteDesktopReconnect {
    pub reason: RemoteDesktopReconnectReason,
    pub delay_secs: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteDesktopReconnectReason {
    DisplayUpdate,
    SessionError,
    ConnectionLost,
    Manual,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteDesktopFrameRect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub byte_len: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_debug_reports_metadata_without_frames_text_paths_or_messages() {
        let outputs = [
            RemoteDesktopOutput::Frame {
                width: 1,
                height: 1,
                rgba: b"private-frame".to_vec(),
            },
            RemoteDesktopOutput::FrameBgra {
                width: 1,
                height: 1,
                bgra: b"private-bgra".to_vec(),
            },
            RemoteDesktopOutput::CursorBitmap(RemoteDesktopCursor {
                width: 1,
                height: 1,
                hotspot_x: 0,
                hotspot_y: 0,
                rgba: b"private-cursor".to_vec(),
            }),
            RemoteDesktopOutput::ClipboardText {
                text: "private-clipboard".to_string(),
            },
            RemoteDesktopOutput::ClipboardFilesReady {
                transfer_id: 17,
                paths: vec!["/Users/rachel/private-file".to_string()],
            },
            RemoteDesktopOutput::ClipboardTransferFailed {
                transfer_id: 18,
                message: "private-transfer-error".to_string(),
            },
            RemoteDesktopOutput::Status("private-status".to_string()),
            RemoteDesktopOutput::ConnectionFailure(RemoteDesktopFailure::AuthenticationFailed),
            RemoteDesktopOutput::Terminated(RemoteDesktopFailure::SessionTakenOver),
        ];

        for output in outputs {
            let debug = format!("{output:?}");

            assert!(!debug.contains("private-"));
        }
    }
}
