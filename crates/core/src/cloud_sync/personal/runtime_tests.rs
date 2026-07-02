use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::cloud_sync::personal::{
    PersonalSyncRuntimeError, SelfWriteGuard, build_personal_sync_runtime_config,
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
        build_personal_sync_runtime_config(&settings)
    );
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
