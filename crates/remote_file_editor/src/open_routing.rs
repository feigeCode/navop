use std::sync::Arc;

use gpui::{Context, Window};
use gpui_component::{WindowExt as _, notification::Notification};
use one_core::settings::{AppSettings, RemoteFileEditorUserSettings, RemoteFileOpenMode};
use rust_i18n::t;
use sftp::RusshSftpClient;
use tokio::sync::Mutex;

use crate::{
    ExternalEditorOpenRequest, RemoteMutationCallback, open_remote_file_editor,
    open_remote_file_external_editor,
};

/// 默认打开（双击等）一个远程文件所需的最小信息。
#[derive(Clone)]
pub struct OpenRemoteFileRequest {
    pub remote_path: String,
    pub client: Arc<Mutex<RusshSftpClient>>,
    pub on_remote_changed: RemoteMutationCallback,
}

/// 根据用户“默认打开方式”设置得到的分派目标。
#[derive(Debug, Clone, PartialEq, Eq)]
enum DefaultOpenTarget {
    BuiltIn,
    External { editor_key: String },
    ExternalNotConfigured,
}

/// 按用户默认打开方式决定普通远程文件应交给内置还是外部编辑器打开。
fn decide_default_open(settings: &RemoteFileEditorUserSettings) -> DefaultOpenTarget {
    match settings.open_mode {
        RemoteFileOpenMode::BuiltIn => DefaultOpenTarget::BuiltIn,
        RemoteFileOpenMode::External => match configured_default_external_editor(settings) {
            Some(editor_key) => DefaultOpenTarget::External { editor_key },
            None => DefaultOpenTarget::ExternalNotConfigured,
        },
    }
}

fn configured_default_external_editor(settings: &RemoteFileEditorUserSettings) -> Option<String> {
    settings
        .default_external_editor
        .as_deref()
        .map(str::trim)
        .filter(|editor_key| !editor_key.is_empty())
        .map(str::to_string)
}

/// 默认打开入口：复用既有内置编辑器与外部编辑器打开 API。
pub fn open_remote_file_with_default<T: 'static>(
    request: OpenRemoteFileRequest,
    window: &mut Window,
    cx: &mut Context<T>,
) {
    let settings = AppSettings::current(cx);
    match decide_default_open(&settings.remote_file_editor) {
        DefaultOpenTarget::BuiltIn => open_remote_file_editor(
            request.remote_path,
            request.client,
            request.on_remote_changed,
            cx,
        ),
        DefaultOpenTarget::External { editor_key } => open_remote_file_external_editor(
            ExternalEditorOpenRequest {
                remote_path: request.remote_path,
                editor_key,
                client: request.client,
                on_remote_changed: request.on_remote_changed,
            },
            window,
            cx,
        ),
        DefaultOpenTarget::ExternalNotConfigured => {
            window.push_notification(
                Notification::error(t!(
                    "RemoteFileEditor.notification.default_external_editor_required"
                )),
                cx,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_core::settings::RemoteFileEditorUserSettings;

    fn settings_with(
        open_mode: RemoteFileOpenMode,
        default_external_editor: Option<&str>,
    ) -> RemoteFileEditorUserSettings {
        RemoteFileEditorUserSettings {
            open_mode,
            default_external_editor: default_external_editor.map(str::to_string),
            ..RemoteFileEditorUserSettings::default()
        }
    }

    #[test]
    fn built_in_mode_always_opens_with_the_built_in_editor() {
        assert_eq!(
            DefaultOpenTarget::BuiltIn,
            decide_default_open(&settings_with(RemoteFileOpenMode::BuiltIn, None))
        );
        assert_eq!(
            DefaultOpenTarget::BuiltIn,
            decide_default_open(&settings_with(RemoteFileOpenMode::BuiltIn, Some("zed")))
        );
    }

    #[test]
    fn external_mode_uses_the_configured_default_editor() {
        assert_eq!(
            DefaultOpenTarget::External {
                editor_key: "zed".to_string()
            },
            decide_default_open(&settings_with(RemoteFileOpenMode::External, Some("zed")))
        );
    }

    #[test]
    fn external_mode_without_a_default_editor_reports_configuration_required() {
        assert_eq!(
            DefaultOpenTarget::ExternalNotConfigured,
            decide_default_open(&settings_with(RemoteFileOpenMode::External, None))
        );
        assert_eq!(
            DefaultOpenTarget::ExternalNotConfigured,
            decide_default_open(&settings_with(RemoteFileOpenMode::External, Some("")))
        );
        assert_eq!(
            DefaultOpenTarget::ExternalNotConfigured,
            decide_default_open(&settings_with(RemoteFileOpenMode::External, Some("   ")))
        );
    }

    #[test]
    fn entry_reuses_built_in_and_external_launch_apis_and_reports_missing_default() {
        let source = include_str!("open_routing.rs");
        let entry_start = source
            .find("pub fn open_remote_file_with_default")
            .expect("routing entry");
        let entry = &source[entry_start..];

        assert!(
            entry.contains("open_remote_file_editor("),
            "default built-in open reuses the built-in editor API"
        );
        assert!(
            entry.contains("open_remote_file_external_editor("),
            "default external open reuses the external editor API"
        );
        assert!(
            entry.contains("default_external_editor_required"),
            "external mode without a default shows a localized configure-default error"
        );
    }
}
