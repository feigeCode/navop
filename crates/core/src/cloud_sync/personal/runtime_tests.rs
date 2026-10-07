use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::cloud_sync::personal::{
    PersonalSyncRuntimeError, PersonalSyncWatcher, SelfWriteGuard,
    build_personal_sync_runtime_config,
};
use crate::settings::PersonalSyncSettings;

#[test]
fn personal_sync_runtime_is_disabled_without_path() {
    let settings = PersonalSyncSettings {
        path: String::new(),
        ..PersonalSyncSettings::default()
    };

    assert_eq!(
        Err(PersonalSyncRuntimeError::NotConfigured),
        build_personal_sync_runtime_config(&settings, None)
    );
}

#[test]
fn personal_sync_runtime_requires_webdav_credentials() {
    let settings = PersonalSyncSettings {
        backend: crate::settings::PersonalSyncBackendKind::Webdav,
        webdav: crate::settings::PersonalWebdavSyncSettings {
            url: "https://dav.example.com/dav/navop/".to_string(),
            username: "user".to_string(),
            password: "ENC:sealed".to_string(),
        },
        ..PersonalSyncSettings::default()
    };

    // 配置填了但解不出密码，等同于未配置。
    assert_eq!(
        Err(PersonalSyncRuntimeError::NotConfigured),
        build_personal_sync_runtime_config(&settings, None)
    );

    let config =
        build_personal_sync_runtime_config(&settings, Some("plain-password")).expect("config");
    // 运行时日志里不应该出现明文：Debug 实现会把密码打码。
    assert!(!format!("{config:?}").contains("plain-password"));
    assert!(format!("{config:?}").contains("<redacted>"));
}

#[test]
fn watcher_ignores_self_written_path_within_window() {
    let mut guard = SelfWriteGuard::new(Duration::from_secs(2));
    let now = Instant::now();
    let path = PathBuf::from("/sync/.onetcli-sync/records/connection/a.json");

    guard.mark_written(path.clone(), now);

    assert!(guard.should_ignore(&path, now + Duration::from_millis(500)));
    assert!(!guard.should_ignore(&path, now + Duration::from_secs(3)));
}

#[test]
fn watcher_start_creates_missing_record_directories() {
    let temp = tempfile::tempdir().expect("tempdir");
    let watcher =
        PersonalSyncWatcher::start(temp.path().to_path_buf(), Duration::from_secs(2), |_| {})
            .expect("watcher starts");

    assert!(temp.path().join(".onetcli-sync/records").is_dir());
    assert!(temp.path().join(".onetcli-sync/tombstones").is_dir());
    drop(watcher);
}
