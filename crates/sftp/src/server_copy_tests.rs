use super::{
    DirectCopyDecision, DirectCopyPreview, DirectCopyStrategy, ServerCopyItem, build_copy_plan,
    build_item_copy_plan, direct_copy_is_selected, join_copy_path, request_direct_copy_approval,
    should_prepare_direct_copy,
};
use crate::{DirectoryConflictPolicy, FileEntry, TransferCancelled};
use ssh::{HostKeyVerifier, SshAuth, SshConnectConfig};
use std::sync::Arc;
use std::time::SystemTime;

fn file(path: &str, size: u64) -> FileEntry {
    FileEntry {
        name: path.rsplit('/').next().unwrap_or(path).to_string(),
        path: path.to_string(),
        size,
        modified: SystemTime::UNIX_EPOCH,
        is_dir: false,
        permissions: 0,
        uid: None,
        gid: None,
        user: None,
        group: None,
    }
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
        directory_conflict_policy: DirectoryConflictPolicy::Merge,
    }];
    let plan = build_copy_plan(&items, &[vec![file("/src/app/bin", 42)]]);
    assert_eq!(plan[1].target_path, "/dst/app/bin");
    assert_eq!(plan[1].size, 42);
}

#[test]
fn directory_replace_plan_targets_only_the_staging_tree() {
    let item = ServerCopyItem {
        source_path: "/src/app".to_string(),
        target_path: "/dst/app".to_string(),
        is_dir: true,
        size: 0,
        directory_conflict_policy: DirectoryConflictPolicy::Replace,
    };
    let descendants = vec![
        FileEntry {
            name: "config".to_string(),
            path: "/src/app/config".to_string(),
            size: 0,
            modified: SystemTime::UNIX_EPOCH,
            is_dir: true,
            permissions: 0,
            uid: None,
            gid: None,
            user: None,
            group: None,
        },
        file("/src/app/config/app.toml", 42),
    ];
    let staging_root = "/dst/.app.navop-part-dir-test";

    let plan = build_item_copy_plan(&item, &descendants, staging_root);

    assert_eq!(plan[0].target_path, staging_root);
    assert_eq!(plan[1].target_path, format!("{staging_root}/config"));
    assert_eq!(
        plan[2].target_path,
        format!("{staging_root}/config/app.toml")
    );
    assert!(
        plan.iter()
            .all(|entry| !entry.target_path.starts_with("/dst/app")),
        "directory replacement must not write into the live target before commit"
    );
}

fn preview() -> DirectCopyPreview {
    DirectCopyPreview {
        strategy: DirectCopyStrategy::Rsync,
        source_host: "source.example".to_string(),
        source_port: 22,
        source_username: "source".to_string(),
        target_host: "target.example".to_string(),
        target_port: 2222,
        target_username: "target".to_string(),
        item_count: 2,
    }
}

fn direct_config(host: &str) -> SshConnectConfig {
    SshConnectConfig {
        host: host.to_string(),
        port: 22,
        username: "root".to_string(),
        auth: SshAuth::Agent,
        timeout: None,
        keepalive_interval: None,
        keepalive_max: None,
        jump_server: None,
        proxy: None,
        keyboard_interactive_responder: None,
        host_key_verifier: HostKeyVerifier::default(),
        x11_forwarding: false,
        allow_legacy_algorithms: false,
    }
}

#[test]
fn disabled_direct_copy_skips_direct_copy_preparation() {
    let source = direct_config("source.example");
    let target = direct_config("target.example");

    assert!(!should_prepare_direct_copy(false, &source, &target));
    assert!(should_prepare_direct_copy(true, &source, &target));
}

#[tokio::test]
async fn missing_direct_approval_defaults_to_relay() {
    assert_eq!(
        DirectCopyDecision::UseRelay,
        request_direct_copy_approval(None, preview()).await
    );
}

#[tokio::test]
async fn direct_approval_result_is_respected() {
    for decision in [
        DirectCopyDecision::UseDirect,
        DirectCopyDecision::UseRelay,
        DirectCopyDecision::Cancel,
    ] {
        let approval = Arc::new(move |_| Box::pin(async move { decision }) as _);
        assert_eq!(
            decision,
            request_direct_copy_approval(Some(approval), preview()).await
        );
    }
}

#[test]
fn direct_copy_decision_routes_cancel_as_transfer_cancellation() {
    assert!(direct_copy_is_selected(DirectCopyDecision::UseDirect).unwrap());
    assert!(!direct_copy_is_selected(DirectCopyDecision::UseRelay).unwrap());

    let error = direct_copy_is_selected(DirectCopyDecision::Cancel).unwrap_err();
    assert!(error.is::<TransferCancelled>());
}
