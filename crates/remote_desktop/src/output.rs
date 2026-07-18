use crate::capabilities::RemoteDesktopCapabilities;

#[derive(Clone, Debug, PartialEq, Eq)]
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
    ClipboardText {
        text: String,
    },
    Status(String),
    ConnectionFailure(String),
    Terminated(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteDesktopFrameRect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub byte_len: usize,
}
