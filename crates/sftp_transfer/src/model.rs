use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

use gpui::SharedString;
use one_core::storage::models::StoredConnection;
use sftp::DirectoryConflictPolicy;
use ssh::{SshConnectConfig, SshSessionManager};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SftpConnectionIdentity {
    Local(i64),
    Cloud(String),
    Runtime(u64),
}

impl SftpConnectionIdentity {
    pub fn from_stored(connection: &StoredConnection) -> Option<Self> {
        connection
            .id
            .map(Self::Local)
            .or_else(|| connection.cloud_id.clone().map(Self::Cloud))
    }
}

pub fn upload_task_key(
    connection: &SftpConnectionIdentity,
    local_path: &Path,
    remote_path: &str,
) -> SharedString {
    let connection = connection_label(connection);
    let local_path = local_path.to_string_lossy();
    format!(
        "sftp-upload:{connection}:{}:{local_path}:{}:{remote_path}",
        local_path.len(),
        remote_path.len()
    )
    .into()
}

pub fn download_task_key(
    connection: &SftpConnectionIdentity,
    remote_path: &str,
    local_path: &Path,
) -> SharedString {
    let connection = connection_label(connection);
    let local_path = local_path.to_string_lossy();
    format!(
        "sftp-download:{connection}:{}:{remote_path}:{}:{local_path}",
        remote_path.len(),
        local_path.len()
    )
    .into()
}

pub fn delete_remote_task_key(
    connection: &SftpConnectionIdentity,
    remote_dir: &str,
    entries: &[SftpRemoteDeleteEntry],
) -> SharedString {
    let connection = connection_label(connection);
    let mut key = format!(
        "sftp-delete-remote:{connection}:{}:{remote_dir}:{}",
        remote_dir.len(),
        entries.len()
    );
    for entry in entries {
        let kind = if entry.is_dir { 'd' } else { 'f' };
        key.push_str(&format!(
            ":{}:{}:{kind}",
            entry.remote_path.len(),
            entry.remote_path
        ));
    }
    key.into()
}

fn connection_label(connection: &SftpConnectionIdentity) -> String {
    match connection {
        SftpConnectionIdentity::Local(id) => format!("local:{id}"),
        SftpConnectionIdentity::Cloud(id) => format!("cloud:{id}"),
        SftpConnectionIdentity::Runtime(id) => format!("runtime:{id}"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SftpTransferId(u64);

impl SftpTransferId {
    pub fn new(value: u64) -> Self {
        Self(value)
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Clone)]
pub enum SftpUploadConnection {
    SessionManager(Arc<SshSessionManager>),
    Config(SshConnectConfig),
}

#[derive(Clone)]
pub struct SftpUploadRequest {
    pub connection: SftpConnectionIdentity,
    pub connection_source: SftpUploadConnection,
    pub local_path: PathBuf,
    pub remote_path: String,
    pub is_dir: bool,
    pub directory_conflict_policy: DirectoryConflictPolicy,
    pub display_name: String,
    pub title: SharedString,
    pub task_group: Option<SharedString>,
    pub task_key: Option<SharedString>,
}

#[derive(Clone)]
pub struct SftpUploadExecution {
    pub id: SftpTransferId,
    pub connection_source: SftpUploadConnection,
    pub local_path: PathBuf,
    pub remote_path: String,
    pub is_dir: bool,
    pub directory_conflict_policy: DirectoryConflictPolicy,
    pub cancelled: Arc<AtomicBool>,
}

#[derive(Clone)]
pub struct SftpDownloadRequest {
    pub connection: SftpConnectionIdentity,
    pub connection_source: SftpUploadConnection,
    pub remote_path: String,
    pub local_path: PathBuf,
    pub is_dir: bool,
    pub display_name: String,
    pub title: SharedString,
    pub task_group: Option<SharedString>,
    pub task_key: Option<SharedString>,
}

#[derive(Clone)]
pub struct SftpDownloadExecution {
    pub id: SftpTransferId,
    pub connection_source: SftpUploadConnection,
    pub remote_path: String,
    pub local_path: PathBuf,
    pub is_dir: bool,
    pub cancelled: Arc<AtomicBool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SftpRemoteDeleteEntry {
    pub remote_path: String,
    pub is_dir: bool,
}

#[derive(Clone)]
pub struct SftpDeleteRemoteRequest {
    pub connection: SftpConnectionIdentity,
    pub connection_source: SftpUploadConnection,
    pub entries: Vec<SftpRemoteDeleteEntry>,
    pub remote_dir: String,
    pub display_name: String,
    pub title: SharedString,
    pub task_group: Option<SharedString>,
    pub task_key: Option<SharedString>,
}

#[derive(Clone)]
pub struct SftpDeleteRemoteExecution {
    pub id: SftpTransferId,
    pub connection_source: SftpUploadConnection,
    pub entries: Vec<SftpRemoteDeleteEntry>,
    pub remote_dir: String,
    pub cancelled: Arc<AtomicBool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SftpTransferOperation {
    Upload,
    Download,
    DeleteRemote,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SftpTransferState {
    Queued,
    Running,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
}

impl SftpTransferState {
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Queued | Self::Running | Self::Cancelling)
    }
}

#[derive(Clone, Debug)]
pub struct SftpTransferSnapshot {
    pub id: SftpTransferId,
    pub operation: SftpTransferOperation,
    pub connection: SftpConnectionIdentity,
    pub local_path: PathBuf,
    pub remote_path: String,
    pub display_name: String,
    pub state: SftpTransferState,
    pub transferred: u64,
    pub total: Option<u64>,
    pub speed: f64,
    pub current_file: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub enum SftpTransferEvent {
    Added(SftpTransferId),
    Updated(SftpTransferId),
    Finished(SftpTransferId),
}

#[cfg(test)]
mod tests {
    use super::{SftpConnectionIdentity, upload_task_key};
    use std::path::PathBuf;

    #[test]
    fn upload_task_key_is_stable_connection_scoped_and_format_compatible() {
        let local_path = PathBuf::from("/tmp/archive.tar");
        let first = upload_task_key(
            &SftpConnectionIdentity::Local(7),
            &local_path,
            "/remote/archive.tar",
        );
        let repeated = upload_task_key(
            &SftpConnectionIdentity::Local(7),
            &local_path,
            "/remote/archive.tar",
        );
        let other_connection = upload_task_key(
            &SftpConnectionIdentity::Cloud("cloud-7".to_string()),
            &local_path,
            "/remote/archive.tar",
        );

        assert_eq!(first, repeated);
        assert_ne!(first, other_connection);
        assert_eq!(
            first.as_ref(),
            "sftp-upload:local:7:16:/tmp/archive.tar:19:/remote/archive.tar"
        );
    }
}
