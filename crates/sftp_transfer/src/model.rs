use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

use ftp::FtpConnectConfig;
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
    Ftp(FtpConnectConfig),
}

impl SftpUploadConnection {
    /// 面板端点选择：远程文件协议为 FTP 时走独立 FTP 连接，
    /// 否则借共享 SSH 会话。FTP 不复用 SSH socket，SSH 会话失败不影响 FTP。
    pub fn for_endpoint(
        ftp_config: Option<&FtpConnectConfig>,
        session_manager: Arc<SshSessionManager>,
    ) -> Self {
        match ftp_config {
            Some(config) => Self::Ftp(config.clone()),
            None => Self::SessionManager(session_manager),
        }
    }
}

/// 连接记录的远程文件协议为 FTP 时，构造独立 FTP 连接配置。
///
/// 覆盖两种形态：独立 `ConnectionType::Ftp` 连接（顶层即 FTP 参数），
/// 以及 SSH 聚合连接（`remote_file.protocol == Ftp`）。SFTP 协议返回
/// `None`；协议声明为 FTP 但缺少 FTP 参数时同样返回 `None`（连接
/// 表单校验应阻止保存该状态）。
pub fn ftp_connect_config_from_stored(
    connection: &StoredConnection,
) -> Option<FtpConnectConfig> {
    let ftp = match connection.connection_type {
        one_core::storage::ConnectionType::Ftp => connection.to_ftp_params().ok()?,
        _ => {
            let params = connection.to_ssh_params().ok()?;
            let remote_file = params.remote_file.as_ref()?;
            if remote_file.protocol != one_core::storage::models::RemoteFileProtocol::Ftp {
                return None;
            }
            params.ftp_params()?.clone()
        }
    };
    Some(FtpConnectConfig {
        host: ftp.host.clone(),
        port: ftp.port,
        username: ftp.username.clone(),
        password: ftp.password.clone(),
        passive_mode: ftp.passive_mode,
        use_tls: ftp.use_tls,
        connect_timeout: ftp.connect_timeout,
    })
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
    use super::{
        SftpConnectionIdentity, SftpUploadConnection, ftp_connect_config_from_stored,
        upload_task_key,
    };
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

    fn ftp_stored_connection() -> one_core::storage::models::StoredConnection {
        let params: one_core::storage::models::SshParams = serde_json::from_value(serde_json::json!({
            "host": "127.0.0.1",
            "port": 22,
            "username": "ssh-user",
            "auth_method": { "Password": { "password": "ssh-pass" } },
            "remote_file": {
                "protocol": "Ftp",
                "ftp": {
                    "host": "localhost",
                    "port": 2121,
                    "username": "testuser",
                    "password": "testpass",
                    "passive_mode": true,
                    "use_tls": true,
                    "connect_timeout": 10
                }
            }
        }))
        .expect("valid ssh params");
        one_core::storage::models::StoredConnection::new_ssh(
            "站点".to_string(),
            params,
            None,
        )
    }

    #[test]
    fn ftp_connect_config_is_extracted_from_ftp_protocol_connection() {
        let config = ftp_connect_config_from_stored(&ftp_stored_connection())
            .expect("ftp config present");
        assert_eq!(config.host, "localhost");
        assert_eq!(config.port, 2121);
        assert_eq!(config.username, "testuser");
        assert_eq!(config.password, "testpass");
        assert!(config.passive_mode);
        assert!(config.use_tls);
        assert_eq!(config.connect_timeout, Some(10));
    }

    #[test]
    fn ftp_connect_config_is_extracted_from_ftp_type_connection() {
        let params = one_core::storage::models::FtpParams {
            host: "127.0.0.1".to_string(),
            port: 2121,
            username: "testuser".to_string(),
            password: "testpass".to_string(),
            credential_reference: None,
            prompt_username: None,
            prompt_password: None,
            passive_mode: true,
            use_tls: false,
            connect_timeout: Some(10),
        };
        let connection = one_core::storage::models::StoredConnection::new_ftp(
            "FTP测试".to_string(),
            params,
            None,
        );

        let config =
            ftp_connect_config_from_stored(&connection).expect("ftp config present");
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 2121);
        assert_eq!(config.username, "testuser");
        assert_eq!(config.password, "testpass");
        assert!(config.passive_mode);
        assert!(!config.use_tls);
        assert_eq!(config.connect_timeout, Some(10));
    }

    #[test]
    fn ftp_connect_config_is_none_for_sftp_protocol() {
        let params: one_core::storage::models::SshParams = serde_json::from_value(serde_json::json!({
            "host": "127.0.0.1",
            "port": 22,
            "username": "ssh-user",
            "auth_method": { "Password": { "password": "ssh-pass" } }
        }))
        .expect("valid ssh params");
        let connection = one_core::storage::models::StoredConnection::new_ssh(
            "纯SSH".to_string(),
            params,
            None,
        );
        assert!(ftp_connect_config_from_stored(&connection).is_none());
    }

    #[test]
    fn ftp_connect_config_is_none_when_ftp_params_missing() {
        let params: one_core::storage::models::SshParams = serde_json::from_value(serde_json::json!({
            "host": "127.0.0.1",
            "port": 22,
            "username": "ssh-user",
            "auth_method": { "Password": { "password": "ssh-pass" } },
            "remote_file": { "protocol": "Ftp" }
        }))
        .expect("valid ssh params");
        let connection = one_core::storage::models::StoredConnection::new_ssh(
            "缺FTP参数".to_string(),
            params,
            None,
        );
        assert!(ftp_connect_config_from_stored(&connection).is_none());
    }

    #[test]
    fn upload_connection_for_endpoint_prefers_ftp_config() {
        let ftp_config = ftp_connect_config_from_stored(&ftp_stored_connection())
            .expect("ftp config present");
        let source = SftpUploadConnection::for_endpoint(Some(&ftp_config), test_session_manager());
        assert!(matches!(source, SftpUploadConnection::Ftp(_)));

        let fallback = SftpUploadConnection::for_endpoint(None, test_session_manager());
        assert!(matches!(
            fallback,
            SftpUploadConnection::SessionManager(_)
        ));
    }

    fn test_session_manager() -> std::sync::Arc<ssh::SshSessionManager> {
        std::sync::Arc::new(ssh::SshSessionManager::new(test_ssh_connect_config()))
    }

    fn test_ssh_connect_config() -> ssh::SshConnectConfig {
        ssh::SshConnectConfig {
            host: "example.com".to_string(),
            port: 22,
            username: "tester".to_string(),
            auth: ssh::SshAuth::Agent,
            timeout: None,
            keepalive_interval: None,
            keepalive_max: None,
            jump_server: None,
            proxy: None,
            keyboard_interactive_responder: None,
            host_key_verifier: ssh::HostKeyVerifier::default(),
            x11_forwarding: false,
            allow_legacy_algorithms: false,
        }
    }
}
