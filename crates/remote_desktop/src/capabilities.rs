use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteDesktopCapabilities {
    pub resize: ResizeSupport,
    pub clipboard_text: bool,
    pub cursor_shape: bool,
    /// Legacy aggregate capability kept for provider-manifest compatibility.
    pub audio: bool,
    /// Legacy aggregate capability kept for provider-manifest compatibility.
    pub file_transfer: bool,
    #[serde(default)]
    pub audio_playback: bool,
    #[serde(default)]
    pub audio_capture: bool,
    #[serde(default)]
    pub clipboard_files_to_remote: bool,
    #[serde(default)]
    pub clipboard_files_from_remote: bool,
    #[serde(default)]
    pub clipboard_directories_to_remote: bool,
    #[serde(default)]
    pub clipboard_directories_from_remote: bool,
    #[serde(default)]
    pub shared_drives: bool,
}

impl RemoteDesktopCapabilities {
    pub fn rdp_mvp() -> Self {
        Self {
            resize: ResizeSupport::RemoteResize,
            clipboard_text: true,
            cursor_shape: false,
            audio: true,
            file_transfer: true,
            audio_playback: true,
            audio_capture: false,
            clipboard_files_to_remote: true,
            clipboard_files_from_remote: true,
            clipboard_directories_to_remote: true,
            clipboard_directories_from_remote: true,
            shared_drives: false,
        }
    }

    pub fn vnc_mvp() -> Self {
        Self {
            resize: ResizeSupport::LocalScaleOnly,
            clipboard_text: true,
            cursor_shape: false,
            audio: false,
            file_transfer: false,
            audio_playback: false,
            audio_capture: false,
            clipboard_files_to_remote: false,
            clipboard_files_from_remote: false,
            clipboard_directories_to_remote: false,
            clipboard_directories_from_remote: false,
            shared_drives: false,
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
    fn legacy_capabilities_default_detailed_features_to_disabled() {
        let capabilities: RemoteDesktopCapabilities = serde_json::from_str(
            r#"{
                "resize":"remote_resize",
                "clipboard_text":true,
                "cursor_shape":false,
                "audio":true,
                "file_transfer":true
            }"#,
        )
        .expect("legacy capabilities decode");

        assert!(!capabilities.audio_playback);
        assert!(!capabilities.audio_capture);
        assert!(!capabilities.clipboard_files_to_remote);
        assert!(!capabilities.clipboard_files_from_remote);
        assert!(!capabilities.clipboard_directories_to_remote);
        assert!(!capabilities.clipboard_directories_from_remote);
        assert!(!capabilities.shared_drives);
    }

    #[test]
    fn rdp_mvp_declares_detailed_audio_and_clipboard_support() {
        let capabilities = RemoteDesktopCapabilities::rdp_mvp();

        assert!(capabilities.audio);
        assert!(capabilities.audio_playback);
        assert!(!capabilities.audio_capture);
        assert!(capabilities.clipboard_text);
        assert!(capabilities.file_transfer);
        assert!(capabilities.clipboard_files_to_remote);
        assert!(capabilities.clipboard_files_from_remote);
        assert!(capabilities.clipboard_directories_to_remote);
        assert!(capabilities.clipboard_directories_from_remote);
        assert!(!capabilities.shared_drives);
    }

    #[test]
    fn vnc_mvp_declares_text_only_clipboard_support() {
        let capabilities = RemoteDesktopCapabilities::vnc_mvp();

        assert!(capabilities.clipboard_text);
        assert!(!capabilities.file_transfer);
        assert!(!capabilities.audio);
        assert!(!capabilities.audio_playback);
        assert!(!capabilities.audio_capture);
        assert!(!capabilities.clipboard_files_to_remote);
        assert!(!capabilities.clipboard_files_from_remote);
        assert!(!capabilities.clipboard_directories_to_remote);
        assert!(!capabilities.clipboard_directories_from_remote);
        assert!(!capabilities.shared_drives);
    }
}
