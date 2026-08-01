use super::*;

pub(super) struct TerminalViewInit {
    pub(super) terminal: Entity<Terminal>,
    pub(super) connection_id: Option<i64>,
    pub(super) stored_connection: Option<StoredConnection>,
    pub(super) sync_path_enabled: bool,
    pub(super) local_working_dir: Option<PathBuf>,
    pub(super) tab_index: Option<usize>,
    pub(super) duplicate_source: Option<TerminalDuplicateSource>,
    pub(super) recording_playback_name: Option<SharedString>,
}
