use super::{
    CopyStrategy, ServerCopyItem, build_copy_plan, choose_copy_strategy, join_copy_path,
    run_with_fallback,
};
use crate::FileEntry;
use std::time::SystemTime;

fn file(path: &str, size: u64) -> FileEntry {
    FileEntry {
        name: path.rsplit('/').next().unwrap_or(path).to_string(),
        path: path.to_string(),
        size,
        modified: SystemTime::UNIX_EPOCH,
        is_dir: false,
        permissions: 0,
    }
}

#[test]
fn direct_is_preferred_when_probe_succeeds() {
    assert_eq!(CopyStrategy::Direct, choose_copy_strategy(true));
    assert_eq!(CopyStrategy::Relay, choose_copy_strategy(false));
}

#[test]
fn joining_copy_paths_does_not_duplicate_slashes() {
    assert_eq!("/srv/app/a.txt", join_copy_path("/srv/app/", "a.txt"));
    assert_eq!("/a.txt", join_copy_path("/", "a.txt"));
}

#[test]
fn directory_plan_keeps_relative_layout_and_sizes() {
    let items = vec![ServerCopyItem {
        source_path: "/src/app".to_string(),
        target_path: "/dst/app".to_string(),
        is_dir: true,
        size: 0,
    }];
    let plan = build_copy_plan(&items, &[vec![file("/src/app/bin", 42)]]);
    assert_eq!(plan[1].target_path, "/dst/app/bin");
    assert_eq!(plan[1].size, 42);
}

#[tokio::test]
async fn direct_failure_runs_relay_and_reports_relay_strategy() {
    let relay_called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let relay_flag = relay_called.clone();
    let strategy = run_with_fallback(
        || async { anyhow::bail!("unreachable") },
        move || async move {
            relay_flag.store(true, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        },
    )
    .await
    .expect("relay succeeds");
    assert_eq!(CopyStrategy::Relay, strategy);
    assert!(relay_called.load(std::sync::atomic::Ordering::Relaxed));
}

#[tokio::test]
async fn cancellation_does_not_start_relay() {
    let relay_called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let relay_flag = relay_called.clone();
    let result = run_with_fallback(
        || async { Err(anyhow::Error::from(crate::TransferCancelled)) },
        move || async move {
            relay_flag.store(true, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        },
    )
    .await;
    assert!(result.is_err());
    assert!(!relay_called.load(std::sync::atomic::Ordering::Relaxed));
}
