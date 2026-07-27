use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::{RemoteDesktopConnectionOptions, RemoteDesktopSharedFolder, RemoteDesktopSize};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum HelperRequest {
    Connect {
        destination: String,
        username: Option<String>,
        password: Option<String>,
        domain: Option<String>,
        width: u16,
        height: u16,
        scale_factor: u32,
        #[serde(default)]
        audio_playback: bool,
        #[serde(default)]
        audio_capture: bool,
        #[serde(default)]
        shared_folders: Vec<RemoteDesktopSharedFolder>,
    },
    Resize {
        width: u16,
        height: u16,
        scale_factor: u32,
    },
    MouseMove {
        x: u16,
        y: u16,
    },
    MouseButton {
        button: HelperMouseButton,
        pressed: bool,
    },
    Wheel {
        vertical: bool,
        units: i16,
    },
    Key {
        code: u16,
        extended: bool,
        pressed: bool,
    },
    KeySym {
        keysym: u32,
        pressed: bool,
    },
    Text {
        text: String,
    },
    ClipboardText {
        text: String,
    },
    ClipboardFiles {
        #[serde(default)]
        transfer_id: u64,
        paths: Vec<String>,
    },
    CancelClipboardTransfer {
        transfer_id: u64,
    },
    Close,
}

impl HelperRequest {
    pub fn connect_from_options(
        options: &RemoteDesktopConnectionOptions,
        size: RemoteDesktopSize,
    ) -> Self {
        Self::Connect {
            destination: options.destination.clone(),
            username: options.username.clone(),
            password: options.password.clone(),
            domain: options.domain.clone(),
            width: size.width,
            height: size.height,
            scale_factor: size.scale_factor,
            audio_playback: options.audio_playback,
            audio_capture: options.audio_capture,
            shared_folders: options.shared_folders.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HelperMouseButton {
    Left,
    Middle,
    Right,
    X1,
    X2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HelperReconnectReason {
    DisplayUpdate,
    SessionError,
    ConnectionLost,
    Manual,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum HelperEvent {
    Status {
        message: String,
    },
    Connected {
        width: u16,
        height: u16,
    },
    Frame {
        width: u16,
        height: u16,
        rgba_base64: String,
    },
    FrameBytes {
        width: u16,
        height: u16,
        rgba_len: usize,
    },
    FrameBgraBytes {
        width: u16,
        height: u16,
        bgra_len: usize,
    },
    FrameBgraRects {
        width: u16,
        height: u16,
        rects: Vec<HelperFrameRect>,
        bgra_len: usize,
    },
    CursorDefault,
    CursorHidden,
    CursorPosition {
        x: u16,
        y: u16,
    },
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
    Reconnecting {
        reason: HelperReconnectReason,
        delay_secs: Option<u64>,
    },
    ConnectionFailure {
        message: String,
    },
    Terminated {
        message: String,
    },
}

impl HelperEvent {
    pub fn frame(width: u16, height: u16, rgba: Vec<u8>) -> Self {
        Self::Frame {
            width,
            height,
            rgba_base64: base64::engine::general_purpose::STANDARD.encode(rgba),
        }
    }

    pub fn into_rgba(self) -> anyhow::Result<Vec<u8>> {
        match self {
            Self::Frame { rgba_base64, .. } => {
                Ok(base64::engine::general_purpose::STANDARD.decode(rgba_base64.as_bytes())?)
            }
            Self::FrameBytes { .. } | Self::FrameBgraBytes { .. } | Self::FrameBgraRects { .. } => {
                anyhow::bail!("binary frame payload is not in JSON event")
            }
            _ => anyhow::bail!("helper event is not a frame"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelperFrameRect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub byte_len: usize,
}

pub fn encode_request_line(request: &HelperRequest) -> anyhow::Result<String> {
    encode_line(request)
}

pub fn decode_request_line(line: &str) -> anyhow::Result<HelperRequest> {
    Ok(serde_json::from_str(line.trim_end())?)
}

pub fn encode_event_line(event: &HelperEvent) -> anyhow::Result<String> {
    encode_line(event)
}

pub fn decode_event_line(line: &str) -> anyhow::Result<HelperEvent> {
    Ok(serde_json::from_str(line.trim_end())?)
}

fn encode_line<T>(value: &T) -> anyhow::Result<String>
where
    T: Serialize,
{
    let mut line = serde_json::to_string(value)?;
    line.push('\n');
    Ok(line)
}

#[cfg(test)]
#[path = "helper_protocol_tests.rs"]
mod tests;
