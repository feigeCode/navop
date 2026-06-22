use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteDesktopCapabilities {
    pub resize: ResizeSupport,
    pub clipboard_text: bool,
    pub cursor_shape: bool,
    pub audio: bool,
    pub file_transfer: bool,
}

impl RemoteDesktopCapabilities {
    pub fn rdp_mvp() -> Self {
        Self {
            resize: ResizeSupport::RemoteResize,
            clipboard_text: false,
            cursor_shape: false,
            audio: false,
            file_transfer: false,
        }
    }

    pub fn vnc_mvp() -> Self {
        Self {
            resize: ResizeSupport::LocalScaleOnly,
            clipboard_text: false,
            cursor_shape: false,
            audio: false,
            file_transfer: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResizeSupport {
    Unsupported,
    LocalScaleOnly,
    RemoteResize,
}
