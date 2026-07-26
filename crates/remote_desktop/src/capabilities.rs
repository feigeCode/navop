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
            clipboard_text: true,
            cursor_shape: false,
            audio: true,
            file_transfer: false,
        }
    }

    pub fn vnc_mvp() -> Self {
        Self {
            resize: ResizeSupport::LocalScaleOnly,
            clipboard_text: true,
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

#[cfg(test)]
mod tests {
    use super::RemoteDesktopCapabilities;

    #[test]
    fn rdp_mvp_declares_audio_and_file_clipboard_support() {
        let capabilities = RemoteDesktopCapabilities::rdp_mvp();

        assert!(capabilities.audio);
        assert!(capabilities.clipboard_text);
    }

    #[test]
    fn vnc_mvp_declares_text_only_clipboard_support() {
        let capabilities = RemoteDesktopCapabilities::vnc_mvp();

        assert!(capabilities.clipboard_text);
        assert!(!capabilities.file_transfer);
        assert!(!capabilities.audio);
    }
}
