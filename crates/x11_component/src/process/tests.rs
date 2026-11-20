use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn test_root(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("x11-{name}-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    root
}

use crate::X11ComponentError;

use super::{
    OrphanState, PidFile, cleanup_orphan, instance_path, orphan_state, remove_pidfile_if_owned,
    same_process,
};

fn record() -> PidFile {
    PidFile {
        pid: 42,
        started_at: 100,
        executable: PathBuf::from("/opt/navop/yserver"),
        version: "1.9.0".into(),
        owner_pid: 7,
        owner_started_at: 50,
    }
}

#[test]
fn pid_reuse_and_executable_change_prevent_cleanup() {
    let record = record();
    assert!(same_process(
        &record,
        Some((100, record.executable.clone()))
    ));
    assert!(!same_process(
        &record,
        Some((101, record.executable.clone()))
    ));
    assert!(!same_process(
        &record,
        Some((100, PathBuf::from("/tmp/other")))
    ));
    assert!(!same_process(&record, None));
}

#[test]
fn pidfile_roundtrip_and_corrupt_input() {
    let root = test_root("pid");
    let path = root.join("yserver.pid");
    let record = record();
    record.write(&path).unwrap();
    assert_eq!(PidFile::read(&path).unwrap(), record);
    std::fs::write(&path, "{broken").unwrap();
    assert!(PidFile::read(&path).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn pidfile_is_removed_only_when_still_owned_by_this_server() {
    let root = test_root("owned-pid");
    let path = root.join("yserver.pid");
    let owned = record();
    owned.write(&path).unwrap();
    remove_pidfile_if_owned(&root, &owned).unwrap();
    assert!(!path.exists(), "仍属于自己的记录应被清理");

    let mut replacement = record();
    replacement.pid += 1;
    replacement.write(&path).unwrap();
    remove_pidfile_if_owned(&root, &owned).unwrap();
    assert_eq!(PidFile::read(&path).unwrap(), replacement);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn active_server_has_a_distinct_error() {
    let root = test_root("active-pid");
    let record = PidFile::capture(
        std::process::id(),
        &std::env::current_exe().unwrap(),
        "1.9.0",
    )
    .unwrap();
    record.write(&root.join("yserver.pid")).unwrap();
    let result = cleanup_orphan(&root.join("yserver.pid"));
    assert!(
        matches!(result, Err(X11ComponentError::ActiveOtherInstance(_))),
        "实际结果：{result:?}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn instance_record_without_shared_pidfile_still_blocks_another_launch() {
    let root = test_root("instance-without-shared-pid");
    let record = PidFile::capture(
        std::process::id(),
        &std::env::current_exe().unwrap(),
        "1.9.0",
    )
    .unwrap();
    record.write(&instance_path(&root, &record)).unwrap();
    let result = cleanup_orphan(&root.join("yserver.pid"));
    assert!(
        matches!(result, Err(X11ComponentError::ActiveOtherInstance(_))),
        "实际结果：{result:?}"
    );
    assert!(instance_path(&root, &record).is_file());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn orphan_process_worker() {
    if std::env::var_os("NAVOP_X11_ORPHAN_TEST_WORKER").is_some() {
        std::thread::sleep(std::time::Duration::from_secs(30));
    }
}

#[test]
fn instance_scan_removes_stale_records_and_cleans_live_orphans() {
    let root = test_root("instance-stale");
    let stale = record();
    stale.write(&instance_path(&root, &stale)).unwrap();
    cleanup_orphan(&root.join("yserver.pid")).unwrap();
    assert!(!instance_path(&root, &stale).exists());

    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "process::tests::orphan_process_worker",
            "--nocapture",
        ])
        .env("NAVOP_X11_ORPHAN_TEST_WORKER", "1")
        .spawn()
        .unwrap();
    let mut orphan =
        PidFile::capture(child.id(), &std::env::current_exe().unwrap(), "1.9.0").unwrap();
    orphan.owner_pid = u32::MAX;
    orphan.owner_started_at = 0;
    orphan.write(&instance_path(&root, &orphan)).unwrap();
    cleanup_orphan(&root.join("yserver.pid")).unwrap();
    assert!(!instance_path(&root, &orphan).exists());
    let status = child.wait().unwrap();
    assert!(!status.success(), "活孤儿子进程应被终止");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn active_other_instance_is_preserved_and_dead_owner_allows_cleanup() {
    let record = record();
    let child = Some((100, record.executable.clone()));
    assert_eq!(
        orphan_state(&record, child.clone(), Some((50, ()))),
        OrphanState::Active
    );
    assert_eq!(
        orphan_state(&record, child.clone(), None),
        OrphanState::Orphan
    );
    assert_eq!(
        orphan_state(&record, child, Some((51, ()))),
        OrphanState::Orphan
    );
    assert_eq!(orphan_state(&record, None, None), OrphanState::Stale);
}
