use one_core::storage::StoredConnection;
use sftp::RusshSftpClient;
use ssh::SshConnectConfig;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LeftRemoteConnectionState {
    Connecting,
    Connected,
    Disconnected(String),
}

pub(crate) struct LeftRemoteEndpoint {
    pub connection: StoredConnection,
    pub config: SshConnectConfig,
    pub client: Option<Arc<Mutex<RusshSftpClient>>>,
    pub state: LeftRemoteConnectionState,
    pub current_path: String,
    pub history: Vec<String>,
    pub history_index: usize,
    pub loading: bool,
}

impl LeftRemoteEndpoint {
    pub fn connecting(connection: StoredConnection, config: SshConnectConfig) -> Self {
        Self {
            connection,
            config,
            client: None,
            state: LeftRemoteConnectionState::Connecting,
            current_path: ".".to_string(),
            history: vec![".".to_string()],
            history_index: 0,
            loading: false,
        }
    }
}
