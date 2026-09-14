use ftp::FtpConnectConfig;
use one_core::storage::StoredConnection;
use sftp::SharedRemoteFileClient;
use ssh::SshConnectConfig;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LeftRemoteConnectionState {
    Connecting,
    Connected,
    Disconnected(String),
}

pub(crate) struct LeftRemoteEndpoint {
    pub connection: StoredConnection,
    pub config: SshConnectConfig,
    /// 远程文件协议为 FTP 时的独立 FTP 连接配置；`None` 表示走 SFTP。
    pub remote_file_ftp: Option<FtpConnectConfig>,
    /// 连接成功后进入的 SFTP 初始目录；`None` 时回退到服务器登录目录。
    pub sftp_initial_directory: Option<String>,
    pub client: Option<SharedRemoteFileClient>,
    pub state: LeftRemoteConnectionState,
    pub current_path: String,
    pub history: Vec<String>,
    pub history_index: usize,
    pub loading: bool,
}

impl LeftRemoteEndpoint {
    pub fn connecting(
        connection: StoredConnection,
        config: SshConnectConfig,
        remote_file_ftp: Option<FtpConnectConfig>,
        sftp_initial_directory: Option<String>,
    ) -> Self {
        Self {
            connection,
            config,
            remote_file_ftp,
            sftp_initial_directory,
            client: None,
            state: LeftRemoteConnectionState::Connecting,
            current_path: ".".to_string(),
            history: vec![".".to_string()],
            history_index: 0,
            loading: false,
        }
    }
}
