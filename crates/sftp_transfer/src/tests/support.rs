use std::{path::PathBuf, sync::Arc, thread, time::Duration};

use gpui::{AppContext, TestAppContext};
use sftp::DirectoryConflictPolicy;
use ssh::{HostKeyVerifier, SshAuth, SshConnectConfig, SshSessionManager};

use super::super::{
    SftpConnectionIdentity, SftpDeleteRemoteRequest, SftpDownloadRequest, SftpRemoteDeleteEntry,
    SftpTransferExecutor, SftpTransferId, SftpUploadConnection, SftpUploadRequest,
};

mod provider;

pub(super) use provider::{TestConnectionSourceKind, TestProvider, TestTransferOperation};

pub(super) fn transfer_id(value: u64) -> SftpTransferId {
    SftpTransferId::new(value)
}

pub(super) fn upload_request(connection: SftpConnectionIdentity, name: &str) -> SftpUploadRequest {
    upload_request_with_source(
        connection,
        name,
        SftpUploadConnection::SessionManager(test_session_manager()),
    )
}

pub(super) fn upload_request_with_config(
    connection: SftpConnectionIdentity,
    name: &str,
) -> SftpUploadRequest {
    upload_request_with_source(
        connection,
        name,
        SftpUploadConnection::Config(test_config()),
    )
}

pub(super) fn download_request(
    connection: SftpConnectionIdentity,
    name: &str,
) -> SftpDownloadRequest {
    SftpDownloadRequest {
        connection,
        connection_source: SftpUploadConnection::Config(test_config()),
        remote_path: format!("/remote/{name}"),
        local_path: PathBuf::from(format!("/tmp/{name}")),
        is_dir: false,
        display_name: name.to_string(),
        title: format!("Download {name}").into(),
        task_group: Some("Test SFTP".into()),
        task_key: None,
    }
}

pub(super) fn delete_remote_request(
    connection: SftpConnectionIdentity,
    name: &str,
) -> SftpDeleteRemoteRequest {
    SftpDeleteRemoteRequest {
        connection,
        connection_source: SftpUploadConnection::Config(test_config()),
        entries: vec![
            SftpRemoteDeleteEntry {
                remote_path: format!("/remote/{name}.txt"),
                is_dir: false,
            },
            SftpRemoteDeleteEntry {
                remote_path: format!("/remote/{name}-dir"),
                is_dir: true,
            },
        ],
        remote_dir: "/remote".to_string(),
        display_name: name.to_string(),
        title: format!("Delete {name}").into(),
        task_group: Some("Test SFTP".into()),
        task_key: None,
    }
}

fn upload_request_with_source(
    connection: SftpConnectionIdentity,
    name: &str,
    connection_source: SftpUploadConnection,
) -> SftpUploadRequest {
    SftpUploadRequest {
        connection,
        connection_source,
        local_path: PathBuf::from(format!("/tmp/{name}")),
        remote_path: format!("/remote/{name}"),
        is_dir: false,
        directory_conflict_policy: DirectoryConflictPolicy::Merge,
        display_name: name.to_string(),
        title: format!("Upload {name}").into(),
        task_group: Some("Test SFTP".into()),
        task_key: None,
    }
}

pub(super) fn wait_until(
    cx: &mut TestAppContext,
    mut predicate: impl FnMut(&TestAppContext) -> bool,
) {
    for _ in 0..100 {
        cx.run_until_parked();
        if predicate(cx) {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("condition was not reached before timeout");
}

pub(super) fn new_executor(
    provider: TestProvider,
    cx: &mut TestAppContext,
) -> gpui::Entity<SftpTransferExecutor> {
    new_executor_with_history_limit(provider, 200, cx)
}

pub(super) fn new_executor_with_history_limit(
    provider: TestProvider,
    completed_history_limit: usize,
    cx: &mut TestAppContext,
) -> gpui::Entity<SftpTransferExecutor> {
    cx.update(one_core::gpui_tokio::init);
    cx.update(one_core::background_tasks::init);
    cx.executor().allow_parking();
    cx.update(|cx| {
        cx.new(|_| {
            SftpTransferExecutor::new_with_completed_history_limit(
                Arc::new(provider),
                completed_history_limit,
            )
        })
    })
}

fn test_session_manager() -> Arc<SshSessionManager> {
    Arc::new(SshSessionManager::new(test_config()))
}

fn test_config() -> SshConnectConfig {
    SshConnectConfig {
        host: "example.com".to_string(),
        port: 22,
        username: "tester".to_string(),
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
