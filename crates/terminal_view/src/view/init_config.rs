use super::*;

pub(super) struct TerminalViewInit {
    pub(super) terminal: Entity<Terminal>,
    pub(super) connection_id: Option<i64>,
    pub(super) stored_connection: Option<StoredConnection>,
    pub(super) sync_path_enabled: bool,
    pub(super) local_working_dir: Option<PathBuf>,
    /// 本地终端文件树的根目录；WSL 会话指向发行版文件系统（`\\wsl$\<发行版>`）。
    pub(super) workspace_root: Option<PathBuf>,
    pub(super) tab_index: Option<usize>,
    pub(super) duplicate_source: Option<TerminalDuplicateSource>,
    pub(super) recording_playback_name: Option<SharedString>,
    pub(super) session_log_name: Option<SharedString>,
}
