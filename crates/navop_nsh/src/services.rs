use std::sync::Arc;

use notes::NotesStorage;
use nsh_core::prelude::*;
use one_core::cloud_sync::TeamOption;
use one_core::settings::AppSettings;
use one_core::storage::{
    ConnectionRepository, CredentialRepository, SftpFavoritePathRepository, WorkspaceRepository,
};
use ssh::HostKeyVerifier;

/// A serializable snapshot of an installed nsh extension app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavopExtensionInfo {
    pub id: String,
    pub title: String,
    pub version: String,
    pub description: Option<String>,
    pub category: Option<String>,
    pub keywords: Vec<String>,
    pub permissions: Vec<String>,
    pub modules: Vec<String>,
    pub entry_path: std::path::PathBuf,
}

/// Process services the nsh engine may use synchronously.
///
/// The values are `Send + Sync` handles. Anything that needs the GPUI main
/// thread stays on the host side and is exposed later through nsh native
/// modules or native components, rather than leaking `Entity`/`Window` into
/// the nsh page model.
#[derive(Clone)]
pub struct NavopNshServices {
    pub connections: ConnectionRepository,
    pub workspaces: Option<WorkspaceRepository>,
    pub resolved_connections: Vec<one_core::storage::StoredConnection>,
    pub db: Option<db::GlobalDbState>,
    pub mongo: Option<mongodb_view::GlobalMongoState>,
    pub acp_agents: Vec<ai_chat_view::AcpAgentConfig>,
    pub tokio: tokio::runtime::Handle,
    pub workspace_root: std::path::PathBuf,
    pub notes: Option<Arc<NotesStorage>>,
    pub credentials: Option<CredentialRepository>,
    pub sftp_favorites: Option<SftpFavoritePathRepository>,
    pub session_logs_dir: Option<std::path::PathBuf>,
    pub extensions: Vec<NavopExtensionInfo>,
    pub teams: Vec<TeamOption>,
    pub settings: AppSettings,
    pub local_terminal: Option<terminal::LocalConfig>,
    pub known_hosts: HostKeyVerifier,
    pub version: Arc<str>,
    pub data_dir: Option<Arc<str>>,
}

impl NavopNshServices {
    pub fn current() -> Option<Arc<Self>> {
        engine_context::<Self>()
    }
}
