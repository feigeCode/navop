use base64::Engine as _;
use one_core::storage::{RdpEgfxMode, RdpSettings};
use serde::{Deserialize, Serialize};

use crate::{RemoteDesktopConnectionOptions, RemoteDesktopSharedFolder, RemoteDesktopSize};

pub const HELPER_PROTOCOL_VERSION: u32 = 2;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum HelperRequest {
    ClientHello {
        protocol_version: u32,
        app_version: String,
    },
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
        /// How the helper advertises the RDP Graphics Pipeline Extension; see `RdpEgfxMode`.
        #[serde(default)]
        egfx: RdpEgfxMode,
        #[serde(default)]
        shared_folders: Vec<RemoteDesktopSharedFolder>,
        /// Loopback listener for a host-side SOCKS/HTTP tunnel. The original destination remains
        /// the TLS/CredSSP identity used by the RDP client.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        relay: Option<HelperRelay>,
        #[serde(default)]
        rdp: Box<RdpSettings>,
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
    RequestFullFrame,
    Ping {
        sequence: u64,
    },
    Close,
}

impl HelperRequest {
    pub fn connect_from_options(
        options: &RemoteDesktopConnectionOptions,
        size: RemoteDesktopSize,
    ) -> Self {
        Self::connect_from_options_with_relay(options, size, None)
    }

    pub fn connect_from_options_with_relay(
        options: &RemoteDesktopConnectionOptions,
        size: RemoteDesktopSize,
        relay: Option<HelperRelay>,
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
            egfx: options.rdp.graphics.egfx,
            shared_folders: options.shared_folders.clone(),
            relay,
            rdp: Box::new(options.rdp.clone()),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelperRelay {
    pub address: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authentication: Vec<u8>,
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
    ServerHello {
        protocol_version: u32,
        provider_version: String,
        capabilities: HelperServerCapabilities,
    },
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
    CursorRgbaBytes {
        width: u16,
        height: u16,
        hotspot_x: u16,
        hotspot_y: u16,
        rgba_len: usize,
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
        #[serde(default)]
        kind: Option<RdpFailureKind>,
    },
    Terminated {
        message: String,
        #[serde(default)]
        kind: Option<RdpFailureKind>,
    },
    Pong {
        sequence: u64,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HelperServerCapabilities {
    pub heartbeat: bool,
    pub typed_errors: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelperServerHello {
    pub protocol_version: u32,
    pub provider_version: String,
    pub capabilities: HelperServerCapabilities,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RdpFailureKind {
    Authentication,
    Certificate,
    Timeout,
    ConnectionRefused,
    Tls,
    Negotiation,
    SessionTakenOver,
    ServerEndedSession,
    Transport,
    Protocol,
    Internal,
    Other,
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
            Self::CursorRgbaBytes { .. } => {
                anyhow::bail!("binary cursor payload is not in JSON event")
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
