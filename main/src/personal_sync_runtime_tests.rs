use gpui::TestAppContext;
use one_core::settings::{AppSettings, PersonalSyncBackendKind};

use crate::personal_sync_runtime::{actions_enabled, runtime_status};
use crate::personal_sync_status::PersonalSyncRuntimeStatus;

#[gpui::test]
fn personal_sync_actions_disabled_without_path(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_global(AppSettings::default());
    });

    cx.update(|cx| {
        assert!(!actions_enabled(cx));
        assert_eq!(PersonalSyncRuntimeStatus::Disabled, runtime_status(cx));
    });
}

#[gpui::test]
fn personal_sync_actions_enabled_with_configured_path(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().expect("tempdir");
    cx.update(|cx| {
        let mut settings = AppSettings::default();
        settings.personal_sync.enabled = true;
        settings.personal_sync.backend = PersonalSyncBackendKind::Folder;
        settings.personal_sync.path = temp.path().to_string_lossy().to_string();
        cx.set_global(settings);
    });

    cx.update(|cx| {
        assert!(actions_enabled(cx));
    });
}
