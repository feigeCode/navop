//! X11 设置的状态映射与下载状态契约。

use one_core::settings::{X11ComponentError, X11ForwardingStatus, X11RestartNotification};
use rust_i18n::t;

#[derive(Default)]
pub(crate) struct X11DownloadState {
    downloading: bool,
    message: Option<String>,
    restart_notification: Option<X11RestartNotification>,
}

impl X11DownloadState {
    pub(crate) fn begin_download(&mut self) -> bool {
        if self.downloading {
            return false;
        }
        self.downloading = true;
        self.message = None;
        true
    }

    pub(crate) fn finish_download(&mut self, result: Result<String, String>) {
        self.downloading = false;
        self.message = Some(match result {
            Ok(version) => {
                t!("Settings.General.X11.download_success", version = version).to_string()
            }
            Err(error) => error,
        });
    }

    pub(crate) fn is_downloading(&self) -> bool {
        self.downloading
    }

    pub(crate) fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    pub(crate) fn record_restart(&mut self, notification: Option<X11RestartNotification>) {
        if let Some(notification) = notification {
            self.restart_notification = Some(notification);
        }
    }

    pub(crate) fn restart_notification(&self) -> Option<&X11RestartNotification> {
        self.restart_notification.as_ref()
    }
}

pub(crate) fn x11_download_error_key(error: &X11ComponentError) -> &'static str {
    match error {
        X11ComponentError::Sha256Mismatch { .. } | X11ComponentError::SignatureInvalid(_) => {
            "Settings.General.X11.signature_failed_support"
        }
        X11ComponentError::VulkanUnavailable { .. } => {
            "Settings.General.X11.vulkan_unavailable_use_external"
        }
        X11ComponentError::Handshake(_) | X11ComponentError::EmbeddedDisabled(_) => {
            "Settings.General.X11.handshake_failed_fallback"
        }
        _ => "Settings.General.X11.download_failed_retry",
    }
}

pub(crate) fn x11_disabled_guidance_key(status: &X11ForwardingStatus) -> Option<&'static str> {
    let X11ForwardingStatus::Disabled { reason } = status else {
        return None;
    };
    if reason.contains("Vulkan") {
        return Some("Settings.General.X11.vulkan_unavailable_use_external");
    }
    if reason.contains("验签") || reason.contains("SHA256") {
        return Some("Settings.General.X11.signature_failed_support");
    }
    if reason.contains("握手") || reason.contains("内置 X11") || reason.contains("内置服务")
    {
        return Some("Settings.General.X11.handshake_failed_fallback");
    }
    if reason.contains("下载 X11 组件失败") {
        return Some("Settings.General.X11.download_failed_retry");
    }
    None
}

pub(crate) fn x11_status_label(status: &X11ForwardingStatus) -> String {
    match status {
        X11ForwardingStatus::External { server } => {
            t!("Settings.General.X11.status_external", server = server).to_string()
        }
        X11ForwardingStatus::Embedded { version } => {
            t!("Settings.General.X11.status_embedded", version = version).to_string()
        }
        X11ForwardingStatus::Disabled { reason } => {
            t!("Settings.General.X11.status_disabled", reason = reason).to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use one_core::settings::{X11ComponentError, X11ForwardingStatus, X11RestartNotification};

    use super::{
        X11DownloadState, x11_disabled_guidance_key, x11_download_error_key, x11_status_label,
    };

    #[test]
    fn restart_notice_requires_a_structured_event() {
        let mut state = X11DownloadState::default();
        state.record_restart(None);
        assert!(state.restart_notification().is_none());

        let notification = X11RestartNotification {
            generation: 1,
            reason: "yserver exited".into(),
        };
        state.record_restart(Some(notification.clone()));
        assert_eq!(Some(&notification), state.restart_notification());
    }

    #[test]
    fn status_poll_refreshes_even_without_a_restart() {
        let source = include_str!("x11_settings.rs");
        assert!(source.contains(
            "this.download.record_restart(notification);\n                        cx.notify();"
        ));
    }

    #[test]
    fn download_rejects_duplicate_clicks_and_finishes_visibly() {
        let mut state = X11DownloadState::default();
        assert!(state.begin_download());
        assert!(!state.begin_download(), "下载中不得重复发起");
        assert!(state.is_downloading());

        state.finish_download(Ok("1.2.3".to_string()));
        assert!(!state.is_downloading());
        assert!(
            state
                .message()
                .is_some_and(|message| message.contains("1.2.3"))
        );

        assert!(state.begin_download());
        state.finish_download(Err("网络不可用".to_string()));
        assert!(!state.is_downloading());
        assert_eq!(Some("网络不可用"), state.message());
    }

    #[test]
    fn typed_download_errors_map_to_actionable_messages() {
        let cases = [
            (
                X11ComponentError::Download("timeout".to_string()),
                "Settings.General.X11.download_failed_retry",
            ),
            (
                X11ComponentError::SignatureInvalid("bad signature".to_string()),
                "Settings.General.X11.signature_failed_support",
            ),
            (
                X11ComponentError::Sha256Mismatch {
                    expected: "a".to_string(),
                    actual: "b".to_string(),
                },
                "Settings.General.X11.signature_failed_support",
            ),
            (
                X11ComponentError::VulkanUnavailable {
                    required: "1.2".to_string(),
                    actual: None,
                },
                "Settings.General.X11.vulkan_unavailable_use_external",
            ),
            (
                X11ComponentError::Handshake("protocol".to_string()),
                "Settings.General.X11.handshake_failed_fallback",
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(expected, x11_download_error_key(&error));
        }
    }

    #[test]
    fn status_label_uses_structured_server_version_and_reason() {
        let external = x11_status_label(&X11ForwardingStatus::External {
            server: ":0".to_string(),
        });
        let embedded = x11_status_label(&X11ForwardingStatus::Embedded {
            version: "1.2.3".to_string(),
        });
        let disabled = x11_status_label(&X11ForwardingStatus::Disabled {
            reason: "display unavailable".to_string(),
        });

        assert!(external.contains(":0"));
        assert!(embedded.contains("1.2.3"));
        assert!(embedded.contains("Vulkan"));
        assert!(disabled.contains("display unavailable"));
    }

    #[test]
    fn disabled_guidance_matches_local_error_without_remote_restart_guesses() {
        let cases = [
            (
                "Vulkan 能力不满足要求",
                Some("Settings.General.X11.vulkan_unavailable_use_external"),
            ),
            (
                "X11 组件 Ed25519 验签失败",
                Some("Settings.General.X11.signature_failed_support"),
            ),
            (
                "X11 组件 SHA256 不匹配",
                Some("Settings.General.X11.signature_failed_support"),
            ),
            (
                "X11 握手失败",
                Some("Settings.General.X11.handshake_failed_fallback"),
            ),
            (
                "下载 X11 组件失败",
                Some("Settings.General.X11.download_failed_retry"),
            ),
            ("尚未解析 X11 服务", None),
            ("远程显示已重启", None),
            ("未找到外部 X11 服务", None),
        ];

        for (reason, expected) in cases {
            let status = X11ForwardingStatus::Disabled {
                reason: reason.to_string(),
            };
            assert_eq!(
                expected,
                x11_disabled_guidance_key(&status),
                "原因: {reason}"
            );
        }
    }

    #[test]
    fn x11_failure_copy_exists_in_all_three_languages() {
        for language in ["en", "zh-CN", "zh-HK"] {
            rust_i18n::set_locale(language);
            for key in [
                "Settings.General.X11.download_failed_retry",
                "Settings.General.X11.signature_failed_support",
                "Settings.General.X11.vulkan_unavailable_use_external",
                "Settings.General.X11.handshake_failed_fallback",
                "Settings.General.X11.remote_display_restarted",
                "Settings.General.X11.remote_forwarding_disabled_checklist",
                "Settings.General.X11.untrusted_ssh_x_security",
                "Settings.General.X11.macos_xquartz_guidance",
            ] {
                assert_ne!(key, rust_i18n::t!(key), "{language} 缺少文案: {key}");
            }
        }
        rust_i18n::set_locale("en");
    }
}
