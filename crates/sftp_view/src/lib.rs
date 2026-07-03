rust_i18n::i18n!("locales", fallback = "en");

mod context_menu_handler;
mod file_list_panel;

use context_menu_handler::ContextMenuHandler;
pub use file_list_panel::{
    DraggedFileItem, DraggedFileItems, FileItem, FileListPanel, FileListPanelEvent,
};

use gpui::{
    AnyElement, App, AsyncApp, Context, Entity, EventEmitter, ExternalPaths, FocusHandle,
    Focusable, FontWeight, Hsla, IntoElement, MouseButton, ParentElement, Render, SharedString,
    Styled, WeakEntity, Window, actions, div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, Size, WindowExt,
    breadcrumb::{Breadcrumb, BreadcrumbItem},
    button::{Button, ButtonVariants},
    dialog::DialogButtonProps,
    h_flex,
    input::{Input, InputEvent, InputState},
    notification::Notification,
    popover::{Popover, PopoverState},
    progress::Progress,
    scroll::ScrollableElement,
    spinner::Spinner,
    tooltip::Tooltip,
    v_flex,
};
use one_core::gpui_tokio::Tokio;
use one_core::storage::models::{
    ActiveConnections, ProxyType as StorageProxyType, SshAuthMethod, StoredConnection,
};
use one_core::storage::{
    GlobalStorageState, SftpFavoritePathRepository, normalize_sftp_favorite_path,
    sftp_favorite_connection_key,
};
use one_core::tab_container::{TabContent, TabContentEvent};
use remote_file_editor::open_remote_file_editor;
use rust_i18n::t;
use sftp::{RusshSftpClient, SftpClient, TransferCancelled, TransferProgress};
use ssh::{
    ChannelEvent, JumpServerConnectConfig, ProxyConnectConfig, ProxyType, SshAuth, SshChannel,
    SshConnectConfig, SshSessionManager,
};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime};
use tokio::sync::Mutex;

actions!(
    sftp_view,
    [
        ToggleFocus,
        RefreshFiles,
        Upload,
        Download,
        Delete,
        NewFolder,
        Rename
    ]
);

/// SftpView 发出的事件
#[derive(Clone, Debug)]
pub enum SftpViewEvent {
    /// 请求打开本地终端，带工作目录
    OpenLocalTerminal { working_dir: String },
    /// 请求打开 SSH 终端，携带连接信息
    OpenSshTerminal {
        connection: StoredConnection,
        working_dir: String,
    },
}

#[derive(Clone, PartialEq)]
enum ConnectionState {
    Connecting,
    Connected,
    Disconnected { error: Option<String> },
}

#[derive(Clone, Copy, PartialEq)]
enum PanelSide {
    Local,
    Remote,
}

#[derive(Clone, PartialEq)]
struct FavoritePathEdit {
    side: PanelSide,
    original_path: String,
}

const MAX_CONCURRENT_TRANSFERS: usize = 2;
const BREADCRUMB_ITEM_MAX_WIDTH: f32 = 180.0;
const LOCAL_FAVORITE_CONNECTION_KEY: &str = "local-file-list:global";

struct TransferClientPool {
    config: SshConnectConfig,
    max_size: usize,
    total_created: usize,
    available: Vec<Arc<Mutex<RusshSftpClient>>>,
}

struct TransferQueue {
    tasks: Vec<TransferTask>,
    pending: VecDeque<usize>,
    max_concurrent: usize,
}

struct SharedProgress {
    transferred: AtomicU64,
    total: AtomicU64,
    speed: AtomicU64,
    cancelled: Arc<AtomicBool>,
    scanning: AtomicBool,
    current_file: std::sync::RwLock<Option<String>>,
    current_file_transferred: AtomicU64,
    current_file_total: AtomicU64,
}

#[derive(Clone)]
struct TransferTask {
    id: usize,
    operation: TransferOperation,
    state: TransferTaskState,
    shared_progress: Arc<SharedProgress>,
    error: Option<String>,
}

#[derive(Clone, PartialEq)]
enum TransferTaskState {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone)]
enum TransferOperation {
    Upload {
        local_path: PathBuf,
        remote_path: String,
        is_dir: bool,
        remote_dir: String,
    },
    Download {
        remote_path: String,
        local_path: PathBuf,
        is_dir: bool,
        local_dir: PathBuf,
    },
    DeleteRemote {
        entries: Vec<FileItem>,
        remote_dir: String,
    },
    DeleteLocal {
        entries: Vec<FileItem>,
        local_dir: PathBuf,
    },
}

#[derive(Clone)]
struct PendingTransfer {
    name: String,
    local_path: PathBuf,
    remote_path: String,
    is_dir: bool,
    has_conflict: bool,
}

#[derive(Clone)]
struct LocalFileEntry {
    name: String,
    size: u64,
    modified: SystemTime,
    is_dir: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ActiveExtract {
    name: String,
    path: String,
}

struct RemoteCommandOutput {
    stdout: String,
    stderr: String,
    exit_status: u32,
}

impl TransferClientPool {
    fn new(config: SshConnectConfig, max_size: usize) -> Self {
        Self {
            config,
            max_size,
            total_created: 0,
            available: Vec::new(),
        }
    }
}

impl TransferQueue {
    fn new(max_concurrent: usize) -> Self {
        Self {
            tasks: Vec::new(),
            pending: VecDeque::new(),
            max_concurrent,
        }
    }

    fn running_count(&self) -> usize {
        self.tasks
            .iter()
            .filter(|task| task.state == TransferTaskState::Running)
            .count()
    }

    fn has_active(&self) -> bool {
        self.tasks.iter().any(|task| {
            task.state == TransferTaskState::Running || task.state == TransferTaskState::Pending
        })
    }

    fn enqueue(&mut self, task: TransferTask) {
        self.pending.push_back(task.id);
        self.tasks.push(task);
    }

    fn next_startable(&mut self) -> Vec<TransferTask> {
        let mut startable = Vec::new();
        let mut available_slots = self.max_concurrent.saturating_sub(self.running_count());

        while available_slots > 0 {
            let Some(task_id) = self.pending.pop_front() else {
                break;
            };

            let Some(task) = self.tasks.iter_mut().find(|task| task.id == task_id) else {
                continue;
            };

            if task.state != TransferTaskState::Pending {
                continue;
            }

            task.state = TransferTaskState::Running;
            startable.push(task.clone());
            available_slots = available_slots.saturating_sub(1);
        }

        startable
    }

    fn active_tasks(&self) -> Vec<TransferTask> {
        self.tasks
            .iter()
            .filter(|task| {
                task.state == TransferTaskState::Running || task.state == TransferTaskState::Pending
            })
            .cloned()
            .collect()
    }
}

async fn acquire_transfer_client(
    pool: Arc<Mutex<TransferClientPool>>,
) -> anyhow::Result<Arc<Mutex<RusshSftpClient>>> {
    let config = {
        let mut pool_guard = pool.lock().await;
        if let Some(client) = pool_guard.available.pop() {
            return Ok(client);
        }

        if pool_guard.total_created >= pool_guard.max_size {
            return Err(anyhow::anyhow!("Transfer client pool exhausted"));
        }

        pool_guard.total_created += 1;
        pool_guard.config.clone()
    };

    match RusshSftpClient::connect(config).await {
        Ok(client) => Ok(Arc::new(Mutex::new(client))),
        Err(error) => {
            let mut pool_guard = pool.lock().await;
            pool_guard.total_created = pool_guard.total_created.saturating_sub(1);
            Err(error)
        }
    }
}

async fn release_transfer_client(
    pool: Arc<Mutex<TransferClientPool>>,
    client: Arc<Mutex<RusshSftpClient>>,
) {
    let mut pool_guard = pool.lock().await;
    pool_guard.available.push(client);
}

fn format_permissions(mode: u32, is_dir: bool) -> String {
    let mut result = String::with_capacity(10);

    result.push(if is_dir { 'd' } else { '-' });

    result.push(if mode & 0o400 != 0 { 'r' } else { '-' });
    result.push(if mode & 0o200 != 0 { 'w' } else { '-' });
    result.push(if mode & 0o100 != 0 { 'x' } else { '-' });

    result.push(if mode & 0o040 != 0 { 'r' } else { '-' });
    result.push(if mode & 0o020 != 0 { 'w' } else { '-' });
    result.push(if mode & 0o010 != 0 { 'x' } else { '-' });

    result.push(if mode & 0o004 != 0 { 'r' } else { '-' });
    result.push(if mode & 0o002 != 0 { 'w' } else { '-' });
    result.push(if mode & 0o001 != 0 { 'x' } else { '-' });

    result
}

fn format_speed(bytes_per_sec: f64) -> String {
    if bytes_per_sec >= 1024.0 * 1024.0 {
        format!("{:.1} MB/s", bytes_per_sec / (1024.0 * 1024.0))
    } else if bytes_per_sec >= 1024.0 {
        format!("{:.1} KB/s", bytes_per_sec / 1024.0)
    } else {
        format!("{:.0} B/s", bytes_per_sec)
    }
}

fn join_remote_path(base: &str, name: &str) -> String {
    if base == "." || base.is_empty() {
        name.to_string()
    } else if base == "/" {
        format!("/{}", name)
    } else {
        format!("{}/{}", base, name)
    }
}

fn remote_path_parent(path: &str) -> String {
    if path == "/" || path.is_empty() {
        ".".to_string()
    } else {
        let trimmed = path.trim_end_matches('/');
        match trimmed.rfind('/') {
            Some(0) => "/".to_string(),
            Some(pos) => trimmed[..pos].to_string(),
            None => ".".to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArchiveKind {
    Zip,
    Tar,
    TarGz,
    Tgz,
    TarBz2,
    Tbz2,
    TarXz,
    Txz,
    Gzip,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExtractConflictAction {
    Overwrite,
    SkipExisting,
}

pub(crate) fn archive_kind_for_name(name: &str) -> Option<ArchiveKind> {
    let lower = name.to_lowercase();
    [
        (".tar.gz", ArchiveKind::TarGz),
        (".tar.bz2", ArchiveKind::TarBz2),
        (".tar.xz", ArchiveKind::TarXz),
        (".tgz", ArchiveKind::Tgz),
        (".tbz2", ArchiveKind::Tbz2),
        (".txz", ArchiveKind::Txz),
        (".tar", ArchiveKind::Tar),
        (".zip", ArchiveKind::Zip),
        (".gz", ArchiveKind::Gzip),
    ]
    .into_iter()
    .find_map(|(suffix, kind)| lower.ends_with(suffix).then_some(kind))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub(crate) fn build_remote_extract_command(
    path: &str,
    name: &str,
    action: ExtractConflictAction,
) -> Option<String> {
    let quoted_path = shell_quote(path);
    let quoted_parent = shell_quote(&remote_path_parent(path));
    let tar_skip = match action {
        ExtractConflictAction::Overwrite => "",
        ExtractConflictAction::SkipExisting => " --skip-old-files",
    };

    match (archive_kind_for_name(name)?, action) {
        (ArchiveKind::Zip, ExtractConflictAction::Overwrite) => {
            Some(format!("unzip -o -- {quoted_path} -d {quoted_parent}"))
        }
        (ArchiveKind::Zip, ExtractConflictAction::SkipExisting) => {
            Some(format!("unzip -n -- {quoted_path} -d {quoted_parent}"))
        }
        (ArchiveKind::Tar, _) => Some(format!(
            "tar{tar_skip} -xf {quoted_path} -C {quoted_parent}"
        )),
        (ArchiveKind::TarGz | ArchiveKind::Tgz, _) => Some(format!(
            "tar{tar_skip} -xzf {quoted_path} -C {quoted_parent}"
        )),
        (ArchiveKind::TarBz2 | ArchiveKind::Tbz2, _) => Some(format!(
            "tar{tar_skip} -xjf {quoted_path} -C {quoted_parent}"
        )),
        (ArchiveKind::TarXz | ArchiveKind::Txz, _) => Some(format!(
            "tar{tar_skip} -xJf {quoted_path} -C {quoted_parent}"
        )),
        (ArchiveKind::Gzip, ExtractConflictAction::Overwrite) => {
            Some(format!("gzip -dkf -- {quoted_path}"))
        }
        (ArchiveKind::Gzip, ExtractConflictAction::SkipExisting) => Some(format!(
            "test -e {} || gzip -dk -- {quoted_path}",
            shell_quote(&remote_gzip_target_path(path))
        )),
    }
}

fn remote_gzip_target_path(path: &str) -> String {
    path.strip_suffix(".gz").unwrap_or(path).to_string()
}

fn build_archive_top_level_conflict_check_command(path: &str, list_command: String) -> String {
    let quoted_parent = shell_quote(&remote_path_parent(path));
    format!(
        "parent={quoted_parent}; tmp=$(mktemp) || exit 2; if ! {list_command} > \"$tmp\" 2>/dev/null; then rm -f \"$tmp\"; exit 2; fi; awk -F/ 'NF {{ print $1 }}' \"$tmp\" | sort -u | while IFS= read -r entry; do [ -n \"$entry\" ] || continue; if [ -e \"$parent/$entry\" ]; then printf '%s\\n' \"$entry\"; exit 7; fi; done; status=$?; rm -f \"$tmp\"; if [ \"$status\" -eq 7 ]; then exit 0; fi; exit 1"
    )
}

pub(crate) fn build_remote_extract_conflict_check_command(
    path: &str,
    name: &str,
) -> Option<String> {
    let quoted_path = shell_quote(path);
    match archive_kind_for_name(name)? {
        ArchiveKind::Zip => Some(build_archive_top_level_conflict_check_command(
            path,
            format!("unzip -Z1 -- {quoted_path}"),
        )),
        ArchiveKind::Tar
        | ArchiveKind::TarGz
        | ArchiveKind::Tgz
        | ArchiveKind::TarBz2
        | ArchiveKind::Tbz2
        | ArchiveKind::TarXz
        | ArchiveKind::Txz => Some(build_archive_top_level_conflict_check_command(
            path,
            format!("tar -tf {quoted_path}"),
        )),
        ArchiveKind::Gzip => Some(format!(
            "test -e {}",
            shell_quote(&remote_gzip_target_path(path))
        )),
    }
}

pub(crate) async fn exec_remote_command(
    session_manager: Arc<SshSessionManager>,
    command: &str,
) -> anyhow::Result<String> {
    let output = exec_remote_command_output(session_manager, command).await?;
    if output.exit_status != 0 {
        anyhow::bail!(
            "remote command exited with status {}: {}",
            output.exit_status,
            output.stderr
        );
    }

    Ok(output.stdout)
}

pub(crate) async fn exec_remote_command_output(
    session_manager: Arc<SshSessionManager>,
    command: &str,
) -> anyhow::Result<RemoteCommandOutput> {
    let mut channel = session_manager.open_channel().await?;
    channel.exec(command).await?;

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_status = 0u32;

    while let Some(event) = channel.recv().await {
        match event {
            ChannelEvent::Data(data) => stdout.extend(data),
            ChannelEvent::ExtendedData { data, .. } => stderr.extend(data),
            ChannelEvent::ExitStatus(status) => exit_status = status,
            ChannelEvent::ExitSignal {
                signal_name,
                error_message,
            } => {
                anyhow::bail!("remote command failed with signal {signal_name}: {error_message}");
            }
            ChannelEvent::Eof | ChannelEvent::Close => break,
        }
    }

    let _ = channel.close().await;
    Ok(RemoteCommandOutput {
        stdout: String::from_utf8_lossy(&stdout).to_string(),
        stderr: String::from_utf8_lossy(&stderr).to_string(),
        exit_status,
    })
}

pub(crate) async fn remote_extract_has_conflict(
    session_manager: Arc<SshSessionManager>,
    command: &str,
) -> anyhow::Result<bool> {
    let output = exec_remote_command_output(session_manager, command).await?;
    match output.exit_status {
        0 => Ok(true),
        1 => Ok(false),
        status => anyhow::bail!(
            "remote conflict check exited with status {}: {}",
            status,
            output.stderr
        ),
    }
}

fn should_apply_remote_listing(current_path: &str, listed_path: &str) -> bool {
    current_path == listed_path
}

fn should_apply_local_listing(
    current_path: &std::path::Path,
    listed_path: &std::path::Path,
) -> bool {
    current_path == listed_path
}

fn is_valid_entry_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
}

fn breadcrumb_item(label: impl Into<SharedString>) -> BreadcrumbItem {
    BreadcrumbItem::new(label)
        .flex_shrink()
        .min_w(px(0.))
        .max_w(px(BREADCRUMB_ITEM_MAX_WIDTH))
        .overflow_hidden()
        .text_ellipsis()
}

fn generate_unique_name(
    original_name: &str,
    existing_names: &std::collections::HashSet<String>,
) -> String {
    let (stem, ext) = if let Some(dot_pos) = original_name.rfind('.') {
        (
            original_name[..dot_pos].to_string(),
            Some(original_name[dot_pos..].to_string()),
        )
    } else {
        (original_name.to_string(), None)
    };

    let mut counter = 1;
    loop {
        let new_name = if counter == 1 {
            if let Some(ref ext) = ext {
                format!("{} (copy){}", stem, ext)
            } else {
                format!("{} (copy)", stem)
            }
        } else {
            if let Some(ref ext) = ext {
                format!("{} (copy {}){}", stem, counter, ext)
            } else {
                format!("{} (copy {})", stem, counter)
            }
        };

        if !existing_names.contains(&new_name) {
            return new_name;
        }
        counter += 1;
    }
}

fn rename_conflicting_transfers(
    mut transfers: Vec<PendingTransfer>,
    is_upload: bool,
    existing_names: std::collections::HashSet<String>,
) -> Vec<PendingTransfer> {
    let mut used_names = existing_names;

    for transfer in &mut transfers {
        if transfer.has_conflict {
            let new_name = generate_unique_name(&transfer.name, &used_names);
            used_names.insert(new_name.clone());

            if is_upload {
                let dir_part = if let Some(slash_pos) = transfer.remote_path.rfind('/') {
                    Some(transfer.remote_path[..=slash_pos].to_string())
                } else {
                    None
                };

                transfer.remote_path = if let Some(dir) = dir_part {
                    format!("{}{}", dir, new_name)
                } else {
                    new_name.clone()
                };
            } else {
                if let Some(parent) = transfer.local_path.parent() {
                    transfer.local_path = parent.join(&new_name);
                }
            }

            transfer.name = new_name;
        }
    }
    transfers
}

pub struct SftpView {
    connection_state: ConnectionState,
    sftp_config: SshConnectConfig,
    sftp_client: Option<Arc<Mutex<RusshSftpClient>>>,

    /// 原始连接信息，用于打开 SSH 终端
    stored_connection: StoredConnection,

    local_current_path: PathBuf,
    remote_current_path: String,

    local_history: Vec<PathBuf>,
    local_history_index: usize,
    remote_history: Vec<String>,
    remote_history_index: usize,

    local_panel: Entity<FileListPanel>,
    remote_panel: Entity<FileListPanel>,

    local_path_editing: bool,
    remote_path_editing: bool,
    local_path_input: Entity<InputState>,
    remote_path_input: Entity<InputState>,
    local_favorite_popover_open: bool,
    remote_favorite_popover_open: bool,
    local_favorite_search_input: Entity<InputState>,
    remote_favorite_search_input: Entity<InputState>,
    favorite_edit_input: Entity<InputState>,
    favorite_editing: Option<FavoritePathEdit>,

    transfer_queue: TransferQueue,
    next_task_id: usize,
    transfer_client_pool: Arc<Mutex<TransferClientPool>>,
    active_extract: Option<ActiveExtract>,

    focus_handle: FocusHandle,

    is_dragging_over_local: bool,
    is_dragging_over_remote: bool,

    remote_loading: bool,
    local_favorite_paths: Vec<String>,
    local_favorite_connection_key: String,
    favorite_paths: Vec<String>,
    favorite_connection_id: Option<i64>,
    favorite_connection_key: String,

    progress_refresh_task: Option<gpui::Task<()>>,
    _subscriptions: Vec<gpui::Subscription>,

    connection_name: String,

    /// 标签页序号（用于多实例显示）
    tab_index: Option<usize>,
}

impl SftpView {
    pub fn new(conn: StoredConnection, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new_with_index(conn, None, window, cx)
    }

    pub fn new_with_index(
        conn: StoredConnection,
        tab_index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let ssh_params = conn
            .to_ssh_params()
            .expect("StoredConnection should contain valid SSH params");

        let auth = match ssh_params.auth_method {
            SshAuthMethod::Password { password } => SshAuth::Password(password),
            SshAuthMethod::PrivateKey {
                key_path,
                passphrase,
            } => SshAuth::PrivateKey {
                key_path,
                passphrase,
                certificate_path: None,
            },
            SshAuthMethod::Agent => SshAuth::Agent,
            SshAuthMethod::AutoPublicKey => SshAuth::AutoPublicKey,
        };

        let config = SshConnectConfig {
            host: ssh_params.host,
            port: ssh_params.port,
            username: ssh_params.username,
            auth,
            timeout: ssh_params.connect_timeout.map(Duration::from_secs),
            keepalive_interval: ssh_params.keepalive_interval.map(Duration::from_secs),
            keepalive_max: ssh_params.keepalive_max,
            jump_server: ssh_params.jump_server.map(|jump| {
                let jump_auth = match jump.auth_method {
                    SshAuthMethod::Password { password } => SshAuth::Password(password),
                    SshAuthMethod::PrivateKey {
                        key_path,
                        passphrase,
                    } => SshAuth::PrivateKey {
                        key_path,
                        passphrase,
                        certificate_path: None,
                    },
                    SshAuthMethod::Agent => SshAuth::Agent,
                    SshAuthMethod::AutoPublicKey => SshAuth::AutoPublicKey,
                };
                JumpServerConnectConfig {
                    host: jump.host,
                    port: jump.port,
                    username: jump.username,
                    auth: jump_auth,
                }
            }),
            proxy: ssh_params.proxy.map(|p| {
                let proxy_type = match p.proxy_type {
                    StorageProxyType::Socks5 => ProxyType::Socks5,
                    StorageProxyType::Http => ProxyType::Http,
                };
                ProxyConnectConfig {
                    proxy_type,
                    host: p.host,
                    port: p.port,
                    username: p.username,
                    password: p.password,
                }
            }),
            keyboard_interactive_responder: None,
        };

        let focus_handle = cx.focus_handle();
        let local_current_path = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));

        let local_panel = cx.new(|cx| {
            FileListPanel::new(
                local_current_path.to_string_lossy().to_string(),
                false,
                window,
                cx,
            )
        });

        let remote_panel = cx.new(|cx| FileListPanel::new("/root".to_string(), true, window, cx));

        let local_path_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Enter path..."));
        let remote_path_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Enter path..."));
        let local_favorite_search_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("FavoritePath.search_placeholder"))
        });
        let remote_favorite_search_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("FavoritePath.search_placeholder"))
        });
        let favorite_edit_input = cx
            .new(|cx| InputState::new(window, cx).placeholder(t!("FavoritePath.edit_placeholder")));
        let favorite_connection_id = conn.id;
        let favorite_connection_key = sftp_favorite_connection_key(&conn);
        let local_favorite_connection_key = LOCAL_FAVORITE_CONNECTION_KEY.to_string();
        let local_favorite_paths = Self::load_favorite_paths(&local_favorite_connection_key, cx);
        let favorite_paths = Self::load_favorite_paths(&favorite_connection_key, cx);

        let mut subscriptions = Vec::new();

        subscriptions.push(cx.subscribe_in(
            &local_panel,
            window,
            |this, _state, event: &FileListPanelEvent, window, cx| match event {
                FileListPanelEvent::ItemDoubleClicked {
                    name,
                    full_path: _,
                    is_dir,
                } => {
                    this.on_local_item_double_click(name.clone(), *is_dir, cx);
                }
                FileListPanelEvent::PathChanged(path) => {
                    this.on_local_path_changed(path.clone(), cx);
                }
                _ => {
                    this.handle_local_context_menu_event(event, window, cx);
                }
            },
        ));

        subscriptions.push(cx.subscribe_in(
            &remote_panel,
            window,
            |this, _state, event: &FileListPanelEvent, window, cx| match event {
                FileListPanelEvent::ItemDoubleClicked {
                    name,
                    full_path,
                    is_dir,
                } => {
                    this.on_remote_item_double_click(
                        name.clone(),
                        full_path.clone(),
                        *is_dir,
                        window,
                        cx,
                    );
                }
                FileListPanelEvent::PathChanged(path) => {
                    this.on_remote_path_changed(path.clone(), cx);
                }
                _ => {
                    this.handle_remote_context_menu_event(event, window, cx);
                }
            },
        ));

        subscriptions.push(cx.subscribe_in(
            &local_path_input,
            window,
            |this, _, event: &gpui_component::input::InputEvent, window, cx| match event {
                gpui_component::input::InputEvent::PressEnter { .. } => {
                    this.confirm_local_path(window, cx);
                }
                gpui_component::input::InputEvent::Blur => {
                    this.cancel_local_path_editing(cx);
                }
                _ => {}
            },
        ));

        subscriptions.push(cx.subscribe_in(
            &remote_path_input,
            window,
            |this, _, event: &gpui_component::input::InputEvent, window, cx| match event {
                gpui_component::input::InputEvent::PressEnter { .. } => {
                    this.confirm_remote_path(window, cx);
                }
                gpui_component::input::InputEvent::Blur => {
                    this.cancel_remote_path_editing(cx);
                }
                _ => {}
            },
        ));

        subscriptions.push(cx.subscribe(
            &local_favorite_search_input,
            |_this, _, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    cx.notify();
                }
            },
        ));
        subscriptions.push(cx.subscribe(
            &remote_favorite_search_input,
            |_this, _, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    cx.notify();
                }
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &favorite_edit_input,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.save_editing_favorite_path(window, cx);
                }
            },
        ));

        let transfer_client_pool = Arc::new(Mutex::new(TransferClientPool::new(
            config.clone(),
            MAX_CONCURRENT_TRANSFERS,
        )));

        let mut view = Self {
            connection_state: ConnectionState::Disconnected { error: None },
            sftp_config: config,
            sftp_client: None,
            stored_connection: conn.clone(),
            local_current_path: local_current_path.clone(),
            remote_current_path: ".".to_string(),
            local_history: vec![local_current_path.clone()],
            local_history_index: 0,
            remote_history: vec![".".to_string()],
            remote_history_index: 0,
            local_panel,
            remote_panel,
            local_path_editing: false,
            remote_path_editing: false,
            local_path_input,
            remote_path_input,
            local_favorite_popover_open: false,
            remote_favorite_popover_open: false,
            local_favorite_search_input,
            remote_favorite_search_input,
            favorite_edit_input,
            favorite_editing: None,
            transfer_queue: TransferQueue::new(MAX_CONCURRENT_TRANSFERS),
            next_task_id: 0,
            transfer_client_pool,
            active_extract: None,
            focus_handle,
            is_dragging_over_local: false,
            is_dragging_over_remote: false,
            remote_loading: false,
            local_favorite_paths,
            local_favorite_connection_key,
            favorite_paths,
            favorite_connection_id,
            favorite_connection_key,
            progress_refresh_task: None,
            _subscriptions: subscriptions,
            connection_name: conn.name,
            tab_index,
        };

        view.refresh_local_dir(cx);
        view.connect(cx);

        view
    }

    fn connect(&mut self, cx: &mut Context<Self>) {
        self.connection_state = ConnectionState::Connecting;
        let config = self.sftp_config.clone();

        tracing::info!(
            "Connecting to SFTP server: {}@{}",
            config.username,
            config.host
        );

        let task = Tokio::spawn(cx, async move {
            let mut client = RusshSftpClient::connect(config).await?;
            // 连接成功后立即获取当前工作目录的真实路径
            let real_path = client.realpath(".").await.ok();
            Ok::<_, anyhow::Error>((client, real_path))
        });

        cx.spawn(async move |this, cx| match task.await {
            Ok(Ok((client, real_path))) => {
                tracing::info!("SFTP connection established successfully");
                let client = Arc::new(Mutex::new(client));

                let _ = this.update(cx, |this, cx| {
                    this.sftp_client = Some(client);
                    this.connection_state = ConnectionState::Connected;
                    this.set_connection_active(true, cx);

                    // 如果成功获取了真实路径，更新远程路径和历史记录
                    if let Some(path) = real_path {
                        tracing::info!("Remote working directory: {}", path);
                        this.remote_current_path = path.clone();
                        this.remote_history = vec![path];
                        this.remote_history_index = 0;
                    }

                    cx.notify();
                });
                let _ = this.update(cx, |this, cx| {
                    this.refresh_remote_dir(cx);
                });
            }
            Ok(Err(e)) => {
                let error_msg = format!("{}", e);
                tracing::error!("SFTP connection failed: {}", error_msg);
                let _ = this.update(cx, |this, cx| {
                    this.connection_state = ConnectionState::Disconnected {
                        error: Some(error_msg),
                    };
                    this.set_connection_active(false, cx);
                    cx.notify();
                });
            }
            Err(e) => {
                let error_msg = format!("Task error: {}", e);
                tracing::error!("SFTP connection task error: {}", error_msg);
                let _ = this.update(cx, |this, cx| {
                    this.connection_state = ConnectionState::Disconnected {
                        error: Some(error_msg),
                    };
                    this.set_connection_active(false, cx);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn set_connection_active(&self, active: bool, cx: &mut Context<Self>) {
        let Some(connection_id) = self.stored_connection.id else {
            return;
        };

        let global_state = cx.global_mut::<ActiveConnections>();
        if active {
            global_state.add(connection_id);
        } else {
            global_state.remove(connection_id);
        }
    }

    fn reconnect(&mut self, cx: &mut Context<Self>) {
        self.sftp_client = None;
        self.transfer_client_pool = Arc::new(Mutex::new(TransferClientPool::new(
            self.sftp_config.clone(),
            MAX_CONCURRENT_TRANSFERS,
        )));
        self.connect(cx);
    }

    fn refresh_local_dir(&mut self, cx: &mut Context<Self>) {
        self.refresh_local_dir_inner(None, cx);
    }

    fn refresh_local_dir_with_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_local_dir_inner(Some(window), cx);
    }

    fn refresh_local_dir_inner(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        let mut entries = Vec::new();

        tracing::info!("Refreshing local directory: {:?}", self.local_current_path);

        let path = self.local_current_path.clone();
        self.local_panel.update(cx, |panel, cx| {
            panel.set_current_path(path.to_string_lossy().to_string(), cx);
        });
        cx.notify();

        match std::fs::read_dir(&self.local_current_path) {
            Ok(dir_entries) => {
                for entry in dir_entries.flatten() {
                    if let Ok(metadata) = entry.metadata() {
                        entries.push(FileItem {
                            name: entry.file_name().to_string_lossy().to_string(),
                            size: metadata.len(),
                            modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                            is_dir: metadata.is_dir(),
                            permissions: String::new(),
                        });
                    }
                }

                tracing::info!("Found {} local entries", entries.len());
            }
            Err(e) => {
                tracing::error!("Failed to read local directory: {}", e);
                if let Some(window) = window {
                    window.push_notification(
                        Notification::error(t!("Error.read_dir_failed", error = e)),
                        cx,
                    );
                }
            }
        }

        self.local_panel.update(cx, |panel, cx| {
            panel.set_items(entries, cx);
        });
    }

    fn refresh_remote_dir(&mut self, cx: &mut Context<Self>) {
        self.refresh_remote_dir_inner(None, cx);
    }

    fn refresh_remote_dir_with_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_remote_dir_inner(Some(window), cx);
    }

    fn refresh_remote_dir_inner(&mut self, _window: Option<&mut Window>, cx: &mut Context<Self>) {
        let Some(client) = self.sftp_client.clone() else {
            tracing::warn!("Cannot refresh remote dir: client not connected");
            return;
        };

        let path = self.remote_current_path.clone();
        tracing::info!("Refreshing remote directory: {}", path);
        let remote_panel = self.remote_panel.clone();

        self.remote_loading = true;
        self.remote_panel.update(cx, |panel, cx| {
            panel.set_current_path(path.clone(), cx);
        });
        cx.notify();

        let listed_path = path.clone();
        let task = Tokio::spawn(cx, async move {
            let mut client = client.lock().await;
            client.list_dir(&path).await
        });

        let view = cx.entity().downgrade();
        cx.spawn(async move |_entity: WeakEntity<Self>, cx: &mut AsyncApp| {
            match task.await {
                Ok(Ok(entries)) => {
                    tracing::info!("Found {} remote entries", entries.len());
                    let items: Vec<FileItem> = entries
                        .into_iter()
                        .map(|e| FileItem {
                            name: e.name,
                            size: e.size,
                            modified: e.modified,
                            is_dir: e.is_dir,
                            permissions: format_permissions(e.permissions, e.is_dir),
                        })
                        .collect();
                    let _ = view.update(cx, |this, cx| {
                        if should_apply_remote_listing(&this.remote_current_path, &listed_path) {
                            let _ = remote_panel.update(cx, |panel, cx| {
                                panel.set_items(items, cx);
                            });
                        }
                    });
                }
                Ok(Err(e)) => {
                    let _ = view.update(cx, |this, _cx| {
                        if should_apply_remote_listing(&this.remote_current_path, &listed_path) {
                            tracing::error!("Failed to list remote directory: {}", e);
                        }
                    });
                }
                Err(e) => {
                    let _ = view.update(cx, |this, _cx| {
                        if should_apply_remote_listing(&this.remote_current_path, &listed_path) {
                            tracing::error!("Task error: {}", e);
                        }
                    });
                }
            }
            let _ = view.update(cx, |this, cx| {
                this.remote_loading = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn on_local_item_double_click(&mut self, name: String, is_dir: bool, cx: &mut Context<Self>) {
        if name == ".." {
            self.go_up_local(cx);
        } else if is_dir {
            self.local_current_path.push(&name);
            self.push_local_history(self.local_current_path.clone());
            self.refresh_local_dir(cx);
        }
        cx.notify();
    }

    fn on_remote_item_double_click(
        &mut self,
        name: String,
        full_path: String,
        is_dir: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if name == ".." {
            self.go_up_remote(cx);
        } else if is_dir {
            self.remote_current_path = join_remote_path(&self.remote_current_path, &name);
            self.push_remote_history(self.remote_current_path.clone());
            self.refresh_remote_dir(cx);
        } else {
            self.open_remote_editor(full_path, window, cx);
        }
        cx.notify();
    }

    fn open_remote_editor(&self, full_path: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(client) = self.sftp_client.clone() else {
            window.push_notification(
                Notification::error("SFTP client is not connected".to_string()),
                cx,
            );
            return;
        };

        open_remote_file_editor(full_path, client, cx);
    }

    fn navigate_local_to(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.local_current_path = path;
        self.push_local_history(self.local_current_path.clone());
        self.refresh_local_dir(cx);
        cx.notify();
    }

    fn navigate_remote_to(&mut self, path: String, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.remote_current_path = path;
        self.push_remote_history(self.remote_current_path.clone());
        self.refresh_remote_dir(cx);
        cx.notify();
    }

    fn on_local_path_changed(&mut self, path: String, cx: &mut Context<Self>) {
        self.local_current_path = PathBuf::from(&path);
        self.push_local_history(self.local_current_path.clone());
        self.refresh_local_dir(cx);
        cx.notify();
    }

    fn on_remote_path_changed(&mut self, path: String, cx: &mut Context<Self>) {
        self.remote_current_path = path;
        self.push_remote_history(self.remote_current_path.clone());
        self.refresh_remote_dir(cx);
        cx.notify();
    }

    fn go_up_local(&mut self, cx: &mut Context<Self>) {
        if let Some(parent) = self.local_current_path.parent() {
            self.local_current_path = parent.to_path_buf();
            self.push_local_history(self.local_current_path.clone());
            self.refresh_local_dir(cx);
            cx.notify();
        }
    }

    fn go_up_remote(&mut self, cx: &mut Context<Self>) {
        if self.remote_current_path != "." && self.remote_current_path != "/" {
            if let Some(pos) = self.remote_current_path.rfind('/') {
                self.remote_current_path = self.remote_current_path[..pos].to_string();
                if self.remote_current_path.is_empty() {
                    self.remote_current_path = "/".to_string();
                }
            } else {
                self.remote_current_path = ".".to_string();
            }
            self.push_remote_history(self.remote_current_path.clone());
            self.refresh_remote_dir(cx);
            cx.notify();
        }
    }

    fn push_local_history(&mut self, path: PathBuf) {
        if self.local_history_index + 1 < self.local_history.len() {
            self.local_history.truncate(self.local_history_index + 1);
        }
        if self.local_history.last() != Some(&path) {
            self.local_history.push(path);
            self.local_history_index = self.local_history.len() - 1;
        }
    }

    fn push_remote_history(&mut self, path: String) {
        if self.remote_history_index + 1 < self.remote_history.len() {
            self.remote_history.truncate(self.remote_history_index + 1);
        }
        if self.remote_history.last() != Some(&path) {
            self.remote_history.push(path);
            self.remote_history_index = self.remote_history.len() - 1;
        }
    }

    fn go_back_local(&mut self, cx: &mut Context<Self>) {
        if self.local_history_index > 0 {
            self.local_history_index -= 1;
            self.local_current_path = self.local_history[self.local_history_index].clone();
            self.refresh_local_dir(cx);
            cx.notify();
        }
    }

    fn go_forward_local(&mut self, cx: &mut Context<Self>) {
        if self.local_history_index + 1 < self.local_history.len() {
            self.local_history_index += 1;
            self.local_current_path = self.local_history[self.local_history_index].clone();
            self.refresh_local_dir(cx);
            cx.notify();
        }
    }

    fn go_back_remote(&mut self, cx: &mut Context<Self>) {
        if self.remote_history_index > 0 {
            self.remote_history_index -= 1;
            self.remote_current_path = self.remote_history[self.remote_history_index].clone();
            self.refresh_remote_dir(cx);
            cx.notify();
        }
    }

    fn go_forward_remote(&mut self, cx: &mut Context<Self>) {
        if self.remote_history_index + 1 < self.remote_history.len() {
            self.remote_history_index += 1;
            self.remote_current_path = self.remote_history[self.remote_history_index].clone();
            self.refresh_remote_dir(cx);
            cx.notify();
        }
    }

    fn can_go_back_local(&self) -> bool {
        self.local_history_index > 0
    }

    fn can_go_forward_local(&self) -> bool {
        self.local_history_index + 1 < self.local_history.len()
    }

    fn can_go_back_remote(&self) -> bool {
        self.remote_history_index > 0
    }

    fn can_go_forward_remote(&self) -> bool {
        self.remote_history_index + 1 < self.remote_history.len()
    }

    fn is_current_remote_path_favorite(&self) -> bool {
        let Some(path) = normalize_sftp_favorite_path(&self.remote_current_path) else {
            return false;
        };
        self.favorite_paths.iter().any(|existing| existing == &path)
    }

    fn is_current_local_path_favorite(&self) -> bool {
        let path = self.local_current_path.to_string_lossy();
        let Some(path) = normalize_sftp_favorite_path(&path) else {
            return false;
        };
        self.local_favorite_paths
            .iter()
            .any(|existing| existing == &path)
    }

    fn remote_favorite_paths(&self) -> Vec<String> {
        self.favorite_paths.clone()
    }

    fn local_favorite_paths(&self) -> Vec<String> {
        self.local_favorite_paths.clone()
    }

    fn toggle_current_remote_favorite(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = normalize_sftp_favorite_path(&self.remote_current_path) else {
            return;
        };
        let Some(repo) = Self::favorite_path_repository(cx) else {
            window.push_notification(
                Notification::error(
                    t!(
                        "FavoritePath.save_failed",
                        error = "SftpFavoritePathRepository not found"
                    )
                    .to_string(),
                ),
                cx,
            );
            return;
        };

        let is_favorite = self.is_current_remote_path_favorite();
        let result = if is_favorite {
            repo.remove_path(&self.favorite_connection_key, &path)
        } else {
            repo.add_path(
                self.favorite_connection_id,
                &self.favorite_connection_key,
                &path,
            )
        };

        match result {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                window.push_notification(
                    Notification::error(t!("FavoritePath.save_failed", error = error).to_string()),
                    cx,
                );
                return;
            }
        }

        self.refresh_remote_favorite_paths(cx);
        let message = if is_favorite {
            t!("FavoritePath.removed").to_string()
        } else {
            t!("FavoritePath.added").to_string()
        };
        window.push_notification(Notification::success(message), cx);
        cx.notify();
    }

    fn toggle_current_local_favorite(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.local_current_path.to_string_lossy().to_string();
        let is_favorite = self.is_current_local_path_favorite();
        let result = if is_favorite {
            self.remove_favorite_path(&self.local_favorite_connection_key, &path, cx)
        } else {
            self.insert_favorite_path(&self.local_favorite_connection_key, &path, cx)
        };

        match result {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                window.push_notification(
                    Notification::error(t!("FavoritePath.save_failed", error = error).to_string()),
                    cx,
                );
                return;
            }
        }

        self.refresh_local_favorite_paths(cx);
        let message = if is_favorite {
            t!("FavoritePath.removed").to_string()
        } else {
            t!("FavoritePath.added").to_string()
        };
        window.push_notification(Notification::success(message), cx);
        cx.notify();
    }

    fn add_remote_favorite_path(
        &mut self,
        path: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = normalize_sftp_favorite_path(path) else {
            return;
        };
        let Some(repo) = Self::favorite_path_repository(cx) else {
            window.push_notification(
                Notification::error(
                    t!(
                        "FavoritePath.save_failed",
                        error = "SftpFavoritePathRepository not found"
                    )
                    .to_string(),
                ),
                cx,
            );
            return;
        };

        match repo.add_path(
            self.favorite_connection_id,
            &self.favorite_connection_key,
            &path,
        ) {
            Ok(false) => return,
            Ok(true) => {
                self.refresh_remote_favorite_paths(cx);
                window.push_notification(
                    Notification::success(t!("FavoritePath.added").to_string()),
                    cx,
                );
                cx.notify();
            }
            Err(error) => {
                window.push_notification(
                    Notification::error(t!("FavoritePath.save_failed", error = error).to_string()),
                    cx,
                );
            }
        }
    }

    fn add_local_favorite_path(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        match self.insert_favorite_path(&self.local_favorite_connection_key, path, cx) {
            Ok(false) => return,
            Ok(true) => {
                self.refresh_local_favorite_paths(cx);
                window.push_notification(
                    Notification::success(t!("FavoritePath.added").to_string()),
                    cx,
                );
                cx.notify();
            }
            Err(error) => {
                window.push_notification(
                    Notification::error(t!("FavoritePath.save_failed", error = error).to_string()),
                    cx,
                );
            }
        }
    }

    fn refresh_remote_favorite_paths(&mut self, cx: &mut Context<Self>) {
        self.favorite_paths = Self::load_favorite_paths(&self.favorite_connection_key, cx);
    }

    fn refresh_local_favorite_paths(&mut self, cx: &mut Context<Self>) {
        self.local_favorite_paths =
            Self::load_favorite_paths(&self.local_favorite_connection_key, cx);
    }

    fn load_favorite_paths(connection_key: &str, cx: &mut Context<Self>) -> Vec<String> {
        let Some(repo) = Self::favorite_path_repository(cx) else {
            tracing::error!("SftpFavoritePathRepository not found");
            return Vec::new();
        };

        match repo.list_paths(connection_key) {
            Ok(paths) => paths,
            Err(error) => {
                tracing::error!("Failed to load SFTP favorite paths: {}", error);
                Vec::new()
            }
        }
    }

    fn favorite_path_repository(cx: &mut Context<Self>) -> Option<Arc<SftpFavoritePathRepository>> {
        let storage = cx.global::<GlobalStorageState>().storage.clone();
        storage.get::<SftpFavoritePathRepository>()
    }

    fn insert_favorite_path(
        &self,
        connection_key: &str,
        path: &str,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<bool> {
        let Some(repo) = Self::favorite_path_repository(cx) else {
            return Err(anyhow::anyhow!("SftpFavoritePathRepository not found"));
        };
        let connection_id = if connection_key == self.local_favorite_connection_key {
            None
        } else {
            self.favorite_connection_id
        };
        repo.add_path(connection_id, connection_key, path)
    }

    fn remove_favorite_path(
        &self,
        connection_key: &str,
        path: &str,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<bool> {
        let Some(repo) = Self::favorite_path_repository(cx) else {
            return Err(anyhow::anyhow!("SftpFavoritePathRepository not found"));
        };
        repo.remove_path(connection_key, path)
    }

    fn update_favorite_path(
        &self,
        connection_key: &str,
        old_path: &str,
        new_path: &str,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<bool> {
        let Some(repo) = Self::favorite_path_repository(cx) else {
            return Err(anyhow::anyhow!("SftpFavoritePathRepository not found"));
        };
        repo.update_path(connection_key, old_path, new_path)
    }

    fn favorite_connection_key_for_side(&self, side: PanelSide) -> &str {
        match side {
            PanelSide::Local => &self.local_favorite_connection_key,
            PanelSide::Remote => &self.favorite_connection_key,
        }
    }

    fn refresh_favorite_paths_for_side(&mut self, side: PanelSide, cx: &mut Context<Self>) {
        match side {
            PanelSide::Local => self.refresh_local_favorite_paths(cx),
            PanelSide::Remote => self.refresh_remote_favorite_paths(cx),
        }
    }

    fn remove_favorite_path_for_side(
        &mut self,
        side: PanelSide,
        path: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let connection_key = self.favorite_connection_key_for_side(side).to_string();
        match self.remove_favorite_path(&connection_key, path, cx) {
            Ok(false) => return,
            Ok(true) => {
                self.refresh_favorite_paths_for_side(side, cx);
                if self
                    .favorite_editing
                    .as_ref()
                    .is_some_and(|editing| editing.side == side && editing.original_path == path)
                {
                    self.favorite_editing = None;
                }
                window.push_notification(
                    Notification::success(t!("FavoritePath.removed").to_string()),
                    cx,
                );
                cx.notify();
            }
            Err(error) => {
                window.push_notification(
                    Notification::error(t!("FavoritePath.save_failed", error = error).to_string()),
                    cx,
                );
            }
        }
    }

    fn start_favorite_path_editing(
        &mut self,
        side: PanelSide,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.favorite_editing = Some(FavoritePathEdit {
            side,
            original_path: path.clone(),
        });
        self.favorite_edit_input.update(cx, |state, cx| {
            state.set_value(&path, window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    fn cancel_favorite_path_editing(&mut self, cx: &mut Context<Self>) {
        if self.favorite_editing.take().is_some() {
            cx.notify();
        }
    }

    fn save_editing_favorite_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editing) = self.favorite_editing.clone() else {
            return;
        };
        let new_path = self.favorite_edit_input.read(cx).text().to_string();
        let connection_key = self
            .favorite_connection_key_for_side(editing.side)
            .to_string();

        match self.update_favorite_path(&connection_key, &editing.original_path, &new_path, cx) {
            Ok(false) => return,
            Ok(true) => {
                self.favorite_editing = None;
                self.refresh_favorite_paths_for_side(editing.side, cx);
                window.push_notification(
                    Notification::success(t!("FavoritePath.updated").to_string()),
                    cx,
                );
                cx.notify();
            }
            Err(error) => {
                window.push_notification(
                    Notification::error(t!("FavoritePath.save_failed", error = error).to_string()),
                    cx,
                );
            }
        }
    }

    fn render_remote_favorites_menu(
        &self,
        favorite_paths: Vec<String>,
        is_connected: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        self.render_favorite_paths_popover(PanelSide::Remote, favorite_paths, is_connected, cx)
    }

    fn render_local_favorites_menu(
        &self,
        favorite_paths: Vec<String>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        self.render_favorite_paths_popover(PanelSide::Local, favorite_paths, true, cx)
    }

    fn render_favorite_paths_popover(
        &self,
        side: PanelSide,
        favorite_paths: Vec<String>,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let popover_id = match side {
            PanelSide::Local => "local_favorite_paths_popover",
            PanelSide::Remote => "remote_favorite_paths_popover",
        };
        let button_id = match side {
            PanelSide::Local => "local_favorite_paths",
            PanelSide::Remote => "remote_favorite_paths",
        };
        let open = match side {
            PanelSide::Local => self.local_favorite_popover_open,
            PanelSide::Remote => self.remote_favorite_popover_open,
        };
        let search_input = match side {
            PanelSide::Local => self.local_favorite_search_input.clone(),
            PanelSide::Remote => self.remote_favorite_search_input.clone(),
        };
        let edit_input = self.favorite_edit_input.clone();
        let editing = self.favorite_editing.clone();
        let view = cx.entity().clone();
        let query = search_input.read(cx).text().to_string().to_lowercase();
        let query = query.trim().to_string();
        let has_favorites = !favorite_paths.is_empty();
        let filtered_paths: Vec<String> = favorite_paths
            .into_iter()
            .filter(|path| query.is_empty() || path.to_lowercase().contains(&query))
            .collect();
        let disabled = !enabled || !has_favorites;

        Popover::new(popover_id)
            .open(open)
            .on_open_change(cx.listener(move |this, open, _window, cx| {
                match side {
                    PanelSide::Local => this.local_favorite_popover_open = *open,
                    PanelSide::Remote => this.remote_favorite_popover_open = *open,
                }
                if !*open
                    && this
                        .favorite_editing
                        .as_ref()
                        .is_some_and(|editing| editing.side == side)
                {
                    this.favorite_editing = None;
                }
                cx.notify();
            }))
            .trigger(
                Button::new(button_id)
                    .icon(IconName::FolderOpen)
                    .ghost()
                    .small()
                    .tooltip(t!("FavoritePath.open").to_string())
                    .disabled(disabled),
            )
            .content(move |_state, window, cx| {
                let mut list = v_flex().gap_1().max_h(px(320.0)).overflow_y_scrollbar();
                if filtered_paths.is_empty() {
                    list = list.child(
                        div()
                            .px_2()
                            .py_3()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("FavoritePath.no_results").to_string()),
                    );
                }

                for path in filtered_paths.iter().cloned() {
                    let is_editing = editing
                        .as_ref()
                        .is_some_and(|state| state.side == side && state.original_path == path);
                    list = list.child(Self::render_favorite_path_row(
                        side,
                        path,
                        is_editing,
                        edit_input.clone(),
                        view.clone(),
                        window,
                        cx,
                    ));
                }

                v_flex()
                    .w(px(360.0))
                    .max_h(px(420.0))
                    .gap_2()
                    .p_2()
                    .child(
                        Input::new(&search_input)
                            .small()
                            .prefix(Icon::new(IconName::Search).small())
                            .cleanable(true)
                            .w_full(),
                    )
                    .child(list)
            })
    }

    fn render_favorite_path_row(
        side: PanelSide,
        path: String,
        is_editing: bool,
        edit_input: Entity<InputState>,
        view: Entity<SftpView>,
        window: &mut Window,
        cx: &mut Context<PopoverState>,
    ) -> impl IntoElement {
        if is_editing {
            let save_path = path.clone();
            let cancel_path = path.clone();
            return h_flex()
                .id(SharedString::from(format!("favorite-edit-row-{path}")))
                .gap_1()
                .items_center()
                .child(Input::new(&edit_input).small().cleanable(false).flex_1())
                .child(
                    Button::new(SharedString::from(format!("favorite-save-{save_path}")))
                        .icon(IconName::Check)
                        .ghost()
                        .small()
                        .tooltip(t!("FavoritePath.save").to_string())
                        .on_click(window.listener_for(&view, |this, _, window, cx| {
                            this.save_editing_favorite_path(window, cx);
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("favorite-cancel-{cancel_path}")))
                        .icon(IconName::Close)
                        .ghost()
                        .small()
                        .tooltip(t!("FavoritePath.cancel").to_string())
                        .on_click(window.listener_for(&view, |this, _, _window, cx| {
                            this.cancel_favorite_path_editing(cx);
                        })),
                )
                .into_any_element();
        }

        let navigate_path = path.clone();
        let edit_path = path.clone();
        let remove_path = path.clone();

        h_flex()
            .id(SharedString::from(format!("favorite-row-{path}")))
            .items_center()
            .gap_1()
            .h_9()
            .px_1()
            .rounded_sm()
            .border_1()
            .border_color(cx.theme().border)
            .hover(|style| style.bg(cx.theme().list_active))
            .child(
                h_flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .h_full()
                    .gap_2()
                    .items_center()
                    .px_2()
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        window.listener_for(&view, move |this, _, _window, cx| match side {
                            PanelSide::Local => {
                                this.navigate_local_to(PathBuf::from(&navigate_path), cx)
                            }
                            PanelSide::Remote => this.navigate_remote_to(navigate_path.clone(), cx),
                        }),
                    )
                    .child(
                        Icon::new(IconName::Folder)
                            .with_size(Size::Small)
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_sm()
                            .text_color(cx.theme().foreground)
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .overflow_hidden()
                            .child(path),
                    ),
            )
            .child(
                Button::new(SharedString::from(format!("favorite-edit-{edit_path}")))
                    .icon(IconName::Edit)
                    .ghost()
                    .small()
                    .tooltip(t!("FavoritePath.edit").to_string())
                    .on_click(window.listener_for(&view, move |this, _, window, cx| {
                        this.start_favorite_path_editing(side, edit_path.clone(), window, cx);
                    })),
            )
            .child(
                Button::new(SharedString::from(format!("favorite-remove-{remove_path}")))
                    .icon(IconName::Remove)
                    .ghost()
                    .small()
                    .tooltip(t!("FavoritePath.delete").to_string())
                    .on_click(window.listener_for(&view, move |this, _, window, cx| {
                        this.remove_favorite_path_for_side(side, &remove_path, window, cx);
                    })),
            )
            .into_any_element()
    }

    fn start_local_path_editing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.local_path_editing = true;
        let path = self.local_current_path.to_string_lossy().to_string();
        self.local_path_input.update(cx, |state, cx| {
            state.set_value(&path, window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    fn start_remote_path_editing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.remote_path_editing = true;
        let remote_path = self.remote_current_path.clone();
        self.remote_path_input.update(cx, |state, cx| {
            state.set_value(&remote_path, window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    fn confirm_local_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let new_path = self.local_path_input.read(cx).text().to_string();
        self.local_path_editing = false;
        if !new_path.is_empty() {
            let path = PathBuf::from(&new_path);
            if path.exists() && path.is_dir() {
                self.local_current_path = path;
                self.refresh_local_dir(cx);
            } else {
                let error_msg = format!(
                    "Error: ENOENT: no such file or directory, lstat '{}'",
                    new_path
                );
                window.open_dialog(cx, move |dialog, _, _| {
                    dialog.title("Error").child(error_msg.clone()).alert()
                });
            }
        }
        cx.notify();
    }

    fn confirm_remote_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let new_path = self.remote_path_input.read(cx).text().to_string();
        self.remote_path_editing = false;
        if !new_path.is_empty() && new_path != self.remote_current_path {
            self.remote_current_path = new_path;
            self.push_remote_history(self.remote_current_path.clone());
            self.refresh_remote_dir_with_window(window, cx);
        }
        cx.notify();
    }

    fn cancel_local_path_editing(&mut self, cx: &mut Context<Self>) {
        self.local_path_editing = false;
        cx.notify();
    }

    fn cancel_remote_path_editing(&mut self, cx: &mut Context<Self>) {
        self.remote_path_editing = false;
        cx.notify();
    }

    fn upload_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(client) = self.sftp_client.clone() else {
            return;
        };

        let selected_entries = self.local_panel.read(cx).selected_items(cx);
        if selected_entries.is_empty() {
            return;
        }

        let local_path = self.local_current_path.clone();
        let remote_path = self.remote_current_path.clone();
        let view = cx.entity().clone();

        let list_task = Tokio::spawn(cx, {
            let client = client.clone();
            let remote_path = remote_path.clone();
            async move {
                let mut client_guard = client.lock().await;
                client_guard.list_dir(&remote_path).await
            }
        });

        window
            .spawn(cx, async move |cx| {
                let remote_names: std::collections::HashSet<String> = match list_task.await {
                    Ok(Ok(entries)) => entries.into_iter().map(|e| e.name).collect(),
                    Ok(Err(e)) => {
                        tracing::error!("Failed to list remote directory: {}", e);
                        let error_msg = t!("Error.read_dir_failed", error = e).to_string();
                        let _ = view.update_in(cx, |_this, window, cx| {
                            window.push_notification(Notification::error(error_msg.clone()), cx);
                        });
                        return;
                    }
                    Err(e) => {
                        tracing::error!("Task error: {}", e);
                        let error_msg = t!("Error.read_dir_failed", error = e).to_string();
                        let _ = view.update_in(cx, |_this, window, cx| {
                            window.push_notification(Notification::error(error_msg.clone()), cx);
                        });
                        return;
                    }
                };

                let _ = view.update_in(cx, |this, window, cx| {
                    let mut pending_transfers: Vec<PendingTransfer> = Vec::new();
                    let mut has_conflict = false;

                    for entry in &selected_entries {
                        let conflict = remote_names.contains(&entry.name);
                        if conflict {
                            has_conflict = true;
                        }

                        pending_transfers.push(PendingTransfer {
                            name: entry.name.clone(),
                            local_path: local_path.join(&entry.name),
                            remote_path: join_remote_path(&remote_path, &entry.name),
                            is_dir: entry.is_dir,
                            has_conflict: conflict,
                        });
                    }

                    if pending_transfers.is_empty() {
                        return;
                    }

                    if has_conflict {
                        let conflict_names: Vec<String> = pending_transfers
                            .iter()
                            .filter(|t| t.has_conflict)
                            .map(|t| t.name.clone())
                            .collect();

                        this.show_conflict_dialog(
                            conflict_names,
                            pending_transfers,
                            true,
                            remote_names,
                            window,
                            cx,
                        );
                    } else {
                        this.execute_uploads(pending_transfers, cx);
                    }
                });
            })
            .detach();
    }

    /// 将指定的本地路径上传到远程目录
    /// 用于文件选择器选择后的上传
    pub fn upload_paths_to_remote(
        &mut self,
        paths: Vec<PathBuf>,
        remote_base_path: String,
        client: Arc<Mutex<RusshSftpClient>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if paths.is_empty() {
            return;
        }

        let view = cx.entity().clone();

        // 先列出远程目录内容，检查冲突
        let list_task = Tokio::spawn(cx, {
            let client = client.clone();
            let remote_path = remote_base_path.clone();
            async move {
                let mut client_guard = client.lock().await;
                client_guard.list_dir(&remote_path).await
            }
        });

        window
            .spawn(cx, async move |cx| {
                let remote_names: std::collections::HashSet<String> = match list_task.await {
                    Ok(Ok(entries)) => entries.into_iter().map(|e| e.name).collect(),
                    Ok(Err(e)) => {
                        tracing::error!("Failed to list remote directory: {}", e);
                        let error_msg = t!("Error.read_dir_failed", error = e).to_string();
                        let _ = view.update_in(cx, |_this, window, cx| {
                            window.push_notification(Notification::error(error_msg.clone()), cx);
                        });
                        return;
                    }
                    Err(e) => {
                        tracing::error!("Task error: {}", e);
                        let error_msg = t!("Error.read_dir_failed", error = e).to_string();
                        let _ = view.update_in(cx, |_this, window, cx| {
                            window.push_notification(Notification::error(error_msg.clone()), cx);
                        });
                        return;
                    }
                };

                let _ = view.update_in(cx, |this, window, cx| {
                    let mut pending_transfers: Vec<PendingTransfer> = Vec::new();
                    let mut has_conflict = false;

                    for path in &paths {
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| "unknown".to_string());

                        let is_dir = path.is_dir();
                        let conflict = remote_names.contains(&name);
                        if conflict {
                            has_conflict = true;
                        }

                        pending_transfers.push(PendingTransfer {
                            name: name.clone(),
                            local_path: path.clone(),
                            remote_path: join_remote_path(&remote_base_path, &name),
                            is_dir,
                            has_conflict: conflict,
                        });
                    }

                    if pending_transfers.is_empty() {
                        return;
                    }

                    if has_conflict {
                        let conflict_names: Vec<String> = pending_transfers
                            .iter()
                            .filter(|t| t.has_conflict)
                            .map(|t| t.name.clone())
                            .collect();

                        this.show_conflict_dialog(
                            conflict_names,
                            pending_transfers,
                            true,
                            remote_names,
                            window,
                            cx,
                        );
                    } else {
                        this.execute_uploads(pending_transfers, cx);
                    }
                });
            })
            .detach();
    }

    fn show_conflict_dialog(
        &mut self,
        conflict_names: Vec<String>,
        pending_transfers: Vec<PendingTransfer>,
        is_upload: bool,
        existing_names: std::collections::HashSet<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.entity().clone();
        let conflict_count = conflict_names.len();
        let conflict_list = if conflict_count <= 3 {
            conflict_names.join(", ")
        } else {
            t!(
                "Conflict.n_files",
                name = conflict_names[..3].join(", "),
                count = conflict_count
            )
            .to_string()
        };

        // 检查是否有文件夹冲突（合并选项只对文件夹有意义）
        let has_dir_conflict = pending_transfers.iter().any(|t| t.has_conflict && t.is_dir);

        window.open_dialog(cx, move |dialog, _window, cx| {
            let view_overwrite = view.clone();
            let view_keep = view.clone();
            let view_skip = view.clone();
            let view_merge = view.clone();

            let transfers_overwrite = pending_transfers.clone();
            let transfers_keep = pending_transfers.clone();
            let transfers_skip = pending_transfers.clone();
            let transfers_merge = pending_transfers.clone();

            let existing_names_keep = existing_names.clone();

            dialog
                .title(t!("Dialog.file_conflict").to_string())
                .w(px(450.))
                .child(
                    v_flex()
                        .gap_2()
                        .child(t!("Conflict.files_exist").to_string())
                        .child(
                            div()
                                .p_2()
                                .bg(cx.theme().secondary)
                                .rounded_md()
                                .text_sm()
                                .child(conflict_list.clone()),
                        )
                        .child(t!("Conflict.choose_action").to_string()),
                )
                .footer(move |_, _, _window, _cx| {
                    let mut buttons: Vec<gpui::AnyElement> = vec![
                        Button::new("skip")
                            .label(t!("Conflict.skip").to_string())
                            .ghost()
                            .on_click({
                                let view = view_skip.clone();
                                let transfers = transfers_skip.clone();
                                move |_, window, cx| {
                                    window.close_dialog(cx);
                                    let transfers: Vec<_> = transfers
                                        .iter()
                                        .filter(|t| !t.has_conflict)
                                        .cloned()
                                        .collect();
                                    if !transfers.is_empty() {
                                        view.update(cx, |this, cx| {
                                            if is_upload {
                                                this.execute_uploads(transfers, cx);
                                            } else {
                                                this.execute_downloads(transfers, cx);
                                            }
                                        });
                                    }
                                }
                            })
                            .into_any_element(),
                        Button::new("keep_both")
                            .label(t!("Conflict.keep_both").to_string())
                            .ghost()
                            .on_click({
                                let view = view_keep.clone();
                                let transfers = transfers_keep.clone();
                                let existing = existing_names_keep.clone();
                                move |_, window, cx| {
                                    window.close_dialog(cx);
                                    let transfers = rename_conflicting_transfers(
                                        transfers.clone(),
                                        is_upload,
                                        existing.clone(),
                                    );
                                    view.update(cx, |this, cx| {
                                        if is_upload {
                                            this.execute_uploads(transfers, cx);
                                        } else {
                                            this.execute_downloads(transfers, cx);
                                        }
                                    });
                                }
                            })
                            .into_any_element(),
                    ];

                    // 如果有文件夹冲突且是上传操作，添加"合并"按钮
                    if has_dir_conflict && is_upload {
                        buttons.push(
                            Button::new("merge")
                                .label(t!("Conflict.merge").to_string())
                                .ghost()
                                .on_click({
                                    let view = view_merge.clone();
                                    let transfers = transfers_merge.clone();
                                    move |_, window, cx| {
                                        window.close_dialog(cx);
                                        // 合并逻辑：
                                        // - 冲突的文件夹：直接上传（会自动合并内容）
                                        // - 冲突的文件：跳过（不覆盖）
                                        // - 非冲突项：正常上传
                                        let transfers: Vec<_> = transfers
                                            .iter()
                                            .filter(|t| !t.has_conflict || t.is_dir)
                                            .cloned()
                                            .collect();
                                        if !transfers.is_empty() {
                                            view.update(cx, |this, cx| {
                                                this.execute_uploads(transfers, cx);
                                            });
                                        }
                                    }
                                })
                                .into_any_element(),
                        );
                    }

                    buttons.push(
                        Button::new("overwrite")
                            .label(t!("Conflict.overwrite").to_string())
                            .primary()
                            .on_click({
                                let view = view_overwrite.clone();
                                let transfers = transfers_overwrite.clone();
                                move |_, window, cx| {
                                    window.close_dialog(cx);
                                    view.update(cx, |this, cx| {
                                        if is_upload {
                                            this.execute_uploads(transfers.clone(), cx);
                                        } else {
                                            this.execute_downloads(transfers.clone(), cx);
                                        }
                                    });
                                }
                            })
                            .into_any_element(),
                    );

                    buttons
                })
                .overlay_closable(false)
                .close_button(true)
        });
    }

    fn schedule_transfers(&mut self, cx: &mut Context<Self>) {
        let startable = self.transfer_queue.next_startable();
        if startable.is_empty() {
            return;
        }

        for task in startable {
            self.start_transfer_task(task, cx);
        }

        self.start_progress_refresh(cx);
        cx.notify();
    }

    fn start_transfer_task(&mut self, task: TransferTask, cx: &mut Context<Self>) {
        match task.operation {
            TransferOperation::Upload {
                local_path,
                remote_path,
                is_dir,
                remote_dir,
            } => {
                self.start_upload_task(
                    task.id,
                    local_path,
                    remote_path,
                    is_dir,
                    remote_dir,
                    task.shared_progress,
                    cx,
                );
            }
            TransferOperation::Download {
                remote_path,
                local_path,
                is_dir,
                local_dir,
            } => {
                self.start_download_task(
                    task.id,
                    remote_path,
                    local_path,
                    is_dir,
                    local_dir,
                    task.shared_progress,
                    cx,
                );
            }
            TransferOperation::DeleteLocal { entries, local_dir } => {
                self.start_local_delete_task(task.id, entries, local_dir, task.shared_progress, cx);
            }
            TransferOperation::DeleteRemote {
                entries,
                remote_dir,
            } => {
                self.start_remote_delete_task(
                    task.id,
                    entries,
                    remote_dir,
                    task.shared_progress,
                    cx,
                );
            }
        }
    }

    fn start_upload_task(
        &mut self,
        task_id: usize,
        local_path: PathBuf,
        remote_path: String,
        is_dir: bool,
        remote_dir: String,
        shared_progress: Arc<SharedProgress>,
        cx: &mut Context<Self>,
    ) {
        let pool = self.transfer_client_pool.clone();
        let cancelled = shared_progress.cancelled.clone();
        let progress_for_callback = shared_progress.clone();
        let remote_dir_for_result = remote_dir.clone();

        if is_dir {
            shared_progress.scanning.store(true, Ordering::Relaxed);
            shared_progress.transferred.store(0, Ordering::Relaxed);
            shared_progress.total.store(0, Ordering::Relaxed);
            shared_progress.speed.store(0, Ordering::Relaxed);
        } else {
            shared_progress.scanning.store(false, Ordering::Relaxed);
        }

        let upload_task = Tokio::spawn(cx, async move {
            let client = acquire_transfer_client(pool.clone()).await?;
            let upload_result = {
                let mut client_guard = client.lock().await;
                if is_dir {
                    client_guard
                        .upload_dir_with_progress(
                            local_path.to_string_lossy().as_ref(),
                            &remote_path,
                            cancelled.clone(),
                            Box::new(move |progress: TransferProgress| {
                                progress_for_callback
                                    .scanning
                                    .store(false, Ordering::Relaxed);
                                progress_for_callback
                                    .transferred
                                    .store(progress.transferred, Ordering::Relaxed);
                                progress_for_callback
                                    .total
                                    .store(progress.total, Ordering::Relaxed);
                                progress_for_callback
                                    .speed
                                    .store(progress.speed.to_bits(), Ordering::Relaxed);
                                if let Some(file) = progress.current_file {
                                    if let Ok(mut guard) =
                                        progress_for_callback.current_file.write()
                                    {
                                        *guard = Some(file);
                                    }
                                }
                                progress_for_callback
                                    .current_file_transferred
                                    .store(progress.current_file_transferred, Ordering::Relaxed);
                                progress_for_callback
                                    .current_file_total
                                    .store(progress.current_file_total, Ordering::Relaxed);
                            }),
                        )
                        .await
                } else {
                    client_guard
                        .upload_with_progress(
                            local_path.to_string_lossy().as_ref(),
                            &remote_path,
                            cancelled.clone(),
                            Box::new(move |progress: TransferProgress| {
                                progress_for_callback
                                    .scanning
                                    .store(false, Ordering::Relaxed);
                                progress_for_callback
                                    .transferred
                                    .store(progress.transferred, Ordering::Relaxed);
                                progress_for_callback
                                    .total
                                    .store(progress.total, Ordering::Relaxed);
                                progress_for_callback
                                    .speed
                                    .store(progress.speed.to_bits(), Ordering::Relaxed);
                            }),
                        )
                        .await
                }
            };

            let entries_option = if upload_result.is_ok() {
                let mut client_guard = client.lock().await;
                client_guard.list_dir(&remote_dir).await.ok()
            } else {
                None
            };

            release_transfer_client(pool, client).await;
            Ok::<_, anyhow::Error>((upload_result, entries_option))
        });

        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let (upload_result, entries_option) = match upload_task.await {
                Ok(Ok((upload_result, entries_option))) => (upload_result, entries_option),
                Ok(Err(error)) => (Err(error), None),
                Err(error) => (Err(anyhow::Error::new(error)), None),
            };

            let should_refresh = upload_result.is_ok();

            let _ = this.update(cx, |this, cx| {
                this.update_task_state_from_result(task_id, upload_result, cx);
                this.schedule_transfers(cx);
                cx.notify();
            });

            if !should_refresh {
                return;
            }

            let _ = this.update(cx, |this, cx| {
                if !should_apply_remote_listing(&this.remote_current_path, &remote_dir_for_result) {
                    return;
                }

                let Some(entries) = entries_option else {
                    return;
                };

                let mut sorted_entries = entries;
                sorted_entries.sort_by(|a, b| {
                    if a.is_dir == b.is_dir {
                        a.name.to_lowercase().cmp(&b.name.to_lowercase())
                    } else if a.is_dir {
                        std::cmp::Ordering::Less
                    } else {
                        std::cmp::Ordering::Greater
                    }
                });
                let items: Vec<FileItem> = sorted_entries
                    .into_iter()
                    .map(|e| FileItem {
                        name: e.name,
                        size: e.size,
                        modified: e.modified,
                        is_dir: e.is_dir,
                        permissions: format_permissions(e.permissions, e.is_dir),
                    })
                    .collect();

                this.remote_panel.update(cx, |panel, cx| {
                    panel.set_items(items, cx);
                });
            });
        })
        .detach();
    }

    fn start_download_task(
        &mut self,
        task_id: usize,
        remote_path: String,
        local_path: PathBuf,
        is_dir: bool,
        local_dir: PathBuf,
        shared_progress: Arc<SharedProgress>,
        cx: &mut Context<Self>,
    ) {
        let pool = self.transfer_client_pool.clone();
        let local_panel = self.local_panel.clone();
        let cancelled = shared_progress.cancelled.clone();
        let progress_for_callback = shared_progress.clone();

        if is_dir {
            shared_progress.scanning.store(true, Ordering::Relaxed);
            shared_progress.transferred.store(0, Ordering::Relaxed);
            shared_progress.total.store(0, Ordering::Relaxed);
            shared_progress.speed.store(0, Ordering::Relaxed);
        } else {
            shared_progress.scanning.store(false, Ordering::Relaxed);
        }

        let download_task = Tokio::spawn(cx, async move {
            let client = acquire_transfer_client(pool.clone()).await?;
            let download_result = {
                let mut client_guard = client.lock().await;
                if is_dir {
                    client_guard
                        .download_dir_with_progress(
                            &remote_path,
                            local_path.to_string_lossy().as_ref(),
                            cancelled.clone(),
                            Box::new(move |progress: TransferProgress| {
                                progress_for_callback
                                    .scanning
                                    .store(false, Ordering::Relaxed);
                                progress_for_callback
                                    .transferred
                                    .store(progress.transferred, Ordering::Relaxed);
                                progress_for_callback
                                    .total
                                    .store(progress.total, Ordering::Relaxed);
                                progress_for_callback
                                    .speed
                                    .store(progress.speed.to_bits(), Ordering::Relaxed);
                                if let Some(file) = progress.current_file {
                                    if let Ok(mut guard) =
                                        progress_for_callback.current_file.write()
                                    {
                                        *guard = Some(file);
                                    }
                                }
                                progress_for_callback
                                    .current_file_transferred
                                    .store(progress.current_file_transferred, Ordering::Relaxed);
                                progress_for_callback
                                    .current_file_total
                                    .store(progress.current_file_total, Ordering::Relaxed);
                            }),
                        )
                        .await
                } else {
                    client_guard
                        .download_with_progress(
                            &remote_path,
                            local_path.to_string_lossy().as_ref(),
                            cancelled.clone(),
                            Box::new(move |progress: TransferProgress| {
                                progress_for_callback
                                    .scanning
                                    .store(false, Ordering::Relaxed);
                                progress_for_callback
                                    .transferred
                                    .store(progress.transferred, Ordering::Relaxed);
                                progress_for_callback
                                    .total
                                    .store(progress.total, Ordering::Relaxed);
                                progress_for_callback
                                    .speed
                                    .store(progress.speed.to_bits(), Ordering::Relaxed);
                            }),
                        )
                        .await
                }
            };

            release_transfer_client(pool, client).await;
            Ok::<_, anyhow::Error>(download_result)
        });

        cx.spawn(async move |this, cx| {
            let download_result = match download_task.await {
                Ok(Ok(result)) => result,
                Ok(Err(error)) => Err(error),
                Err(error) => Err(anyhow::Error::new(error)),
            };

            let should_refresh = download_result.is_ok();

            let _ = this.update(cx, |this, cx| {
                this.update_task_state_from_result(task_id, download_result, cx);
                this.schedule_transfers(cx);
                cx.notify();
            });

            if !should_refresh {
                return;
            }

            if let Ok(dir_entries) = std::fs::read_dir(&local_dir) {
                let mut entries = Vec::new();
                for entry in dir_entries.flatten() {
                    if let Ok(metadata) = entry.metadata() {
                        entries.push(LocalFileEntry {
                            name: entry.file_name().to_string_lossy().to_string(),
                            size: metadata.len(),
                            modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                            is_dir: metadata.is_dir(),
                        });
                    }
                }
                entries.sort_by(|a, b| {
                    if a.is_dir == b.is_dir {
                        a.name.to_lowercase().cmp(&b.name.to_lowercase())
                    } else if a.is_dir {
                        std::cmp::Ordering::Less
                    } else {
                        std::cmp::Ordering::Greater
                    }
                });
                let items: Vec<FileItem> = entries
                    .into_iter()
                    .map(|e| FileItem {
                        name: e.name,
                        size: e.size,
                        modified: e.modified,
                        is_dir: e.is_dir,
                        permissions: String::new(),
                    })
                    .collect();
                let local_dir_for_result = local_dir.clone();
                let _ = this.update(cx, |this, cx| {
                    if !should_apply_local_listing(&this.local_current_path, &local_dir_for_result)
                    {
                        return;
                    }

                    let _ = local_panel.update(cx, |panel, cx| {
                        panel.set_items(items, cx);
                    });
                });
            }
        })
        .detach();
    }

    fn start_local_delete_task(
        &mut self,
        task_id: usize,
        entries: Vec<FileItem>,
        local_dir: PathBuf,
        shared_progress: Arc<SharedProgress>,
        cx: &mut Context<Self>,
    ) {
        let progress_for_task = shared_progress.clone();
        let cancelled = shared_progress.cancelled.clone();
        let view = cx.entity().clone();

        let task = Tokio::spawn(cx, async move {
            let mut delete_errors: Vec<String> = Vec::new();

            for (idx, entry) in entries.iter().enumerate() {
                if cancelled.load(Ordering::Relaxed) {
                    return Err(anyhow::Error::from(TransferCancelled));
                }

                let path = local_dir.join(&entry.name);

                if let Ok(mut guard) = progress_for_task.current_file.write() {
                    *guard = Some(entry.name.clone());
                }
                progress_for_task
                    .current_file_transferred
                    .store(0, Ordering::Relaxed);
                progress_for_task
                    .current_file_total
                    .store(1, Ordering::Relaxed);

                let result = if entry.is_dir {
                    std::fs::remove_dir_all(&path)
                } else {
                    std::fs::remove_file(&path)
                };

                if let Err(error) = result {
                    tracing::error!("Failed to delete {}: {}", path.display(), error);
                    delete_errors.push(format!("{}: {}", entry.name, error));
                }

                progress_for_task
                    .transferred
                    .store((idx + 1) as u64, Ordering::Relaxed);
                progress_for_task
                    .current_file_transferred
                    .store(1, Ordering::Relaxed);
            }

            Ok::<_, anyhow::Error>(delete_errors)
        });

        cx.spawn(async move |_this, cx| {
            let delete_result = match task.await {
                Ok(Ok(errors)) => Ok(errors),
                Ok(Err(error)) => Err(error),
                Err(error) => Err(anyhow::Error::new(error)),
            };

            let _ = view.update(cx, |this, cx| {
                match delete_result {
                    Ok(errors) => {
                        if let Some(task) = this
                            .transfer_queue
                            .tasks
                            .iter_mut()
                            .find(|task| task.id == task_id)
                        {
                            task.state = if errors.is_empty() {
                                TransferTaskState::Completed
                            } else {
                                TransferTaskState::Failed
                            };
                        }

                        if !errors.is_empty() {
                            let error_msg = if errors.len() == 1 {
                                t!("Error.delete_failed", error = errors[0]).to_string()
                            } else {
                                t!("Error.delete_n_failed", count = errors.len()).to_string()
                            };
                            this.push_notification(Notification::error(error_msg), cx);
                        }
                    }
                    Err(error) => {
                        this.update_task_state_from_result(task_id, Err(error), cx);
                    }
                }

                this.refresh_local_dir(cx);
                this.schedule_transfers(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn start_remote_delete_task(
        &mut self,
        task_id: usize,
        entries: Vec<FileItem>,
        remote_dir: String,
        shared_progress: Arc<SharedProgress>,
        cx: &mut Context<Self>,
    ) {
        let pool = self.transfer_client_pool.clone();
        let progress_for_task = shared_progress.clone();
        let cancelled = shared_progress.cancelled.clone();
        let view = cx.entity().clone();

        let task = Tokio::spawn(cx, async move {
            let client = acquire_transfer_client(pool.clone()).await?;
            let mut delete_errors: Vec<String> = Vec::new();

            for entry in entries.iter() {
                if cancelled.load(Ordering::Relaxed) {
                    release_transfer_client(pool, client).await;
                    return Err(anyhow::Error::from(TransferCancelled));
                }

                let path = join_remote_path(&remote_dir, &entry.name);
                let progress_callback = progress_for_task.clone();

                let result = if entry.is_dir {
                    progress_for_task.scanning.store(true, Ordering::Relaxed);
                    progress_for_task.transferred.store(0, Ordering::Relaxed);
                    progress_for_task.total.store(0, Ordering::Relaxed);
                    progress_for_task
                        .current_file_transferred
                        .store(0, Ordering::Relaxed);
                    progress_for_task
                        .current_file_total
                        .store(0, Ordering::Relaxed);
                    if let Ok(mut guard) = progress_for_task.current_file.write() {
                        *guard = None;
                    }
                    let mut client_guard = client.lock().await;
                    client_guard
                        .delete_recursive(
                            &path,
                            cancelled.clone(),
                            Box::new(move |progress: TransferProgress| {
                                progress_callback.scanning.store(false, Ordering::Relaxed);
                                progress_callback
                                    .transferred
                                    .store(progress.transferred, Ordering::Relaxed);
                                progress_callback
                                    .total
                                    .store(progress.total, Ordering::Relaxed);
                                if let Some(file) = progress.current_file {
                                    if let Ok(mut guard) = progress_callback.current_file.write() {
                                        *guard = Some(file);
                                    }
                                }
                                progress_callback
                                    .current_file_transferred
                                    .store(progress.current_file_transferred, Ordering::Relaxed);
                                progress_callback
                                    .current_file_total
                                    .store(progress.current_file_total, Ordering::Relaxed);
                            }),
                        )
                        .await
                } else {
                    progress_for_task.scanning.store(false, Ordering::Relaxed);
                    if let Ok(mut guard) = progress_for_task.current_file.write() {
                        *guard = Some(entry.name.clone());
                    }
                    progress_for_task
                        .current_file_transferred
                        .store(0, Ordering::Relaxed);
                    progress_for_task
                        .current_file_total
                        .store(1, Ordering::Relaxed);

                    let mut client_guard = client.lock().await;
                    let result = client_guard.delete(&path, false).await;

                    progress_for_task
                        .transferred
                        .fetch_add(1, Ordering::Relaxed);
                    progress_for_task
                        .current_file_transferred
                        .store(1, Ordering::Relaxed);

                    result
                };

                if let Err(error) = result {
                    if error.downcast_ref::<TransferCancelled>().is_some() {
                        release_transfer_client(pool, client).await;
                        return Err(error);
                    }

                    tracing::error!("Failed to delete {}: {}", path, error);
                    delete_errors.push(format!("{}: {}", entry.name, error));
                }
            }

            release_transfer_client(pool, client).await;
            Ok::<_, anyhow::Error>(delete_errors)
        });

        cx.spawn(async move |_this, cx| {
            let delete_result = match task.await {
                Ok(Ok(result)) => Ok(result),
                Ok(Err(error)) => Err(error),
                Err(error) => Err(anyhow::Error::new(error)),
            };

            let _ = view.update(cx, |this, cx| {
                let mut should_refresh = true;
                match delete_result {
                    Ok(delete_errors) => {
                        if let Some(task) = this
                            .transfer_queue
                            .tasks
                            .iter_mut()
                            .find(|task| task.id == task_id)
                        {
                            task.state = if delete_errors.is_empty() {
                                TransferTaskState::Completed
                            } else {
                                TransferTaskState::Failed
                            };
                        }

                        if !delete_errors.is_empty() {
                            let error_msg = if delete_errors.len() == 1 {
                                t!("Error.delete_failed", error = delete_errors[0]).to_string()
                            } else {
                                t!("Error.delete_n_failed", count = delete_errors.len()).to_string()
                            };
                            this.push_notification(Notification::error(error_msg), cx);
                        }
                    }
                    Err(error) => {
                        if Self::is_transfer_cancelled(&error) {
                            should_refresh = false;
                        }
                        this.update_task_state_from_result(task_id, Err(error), cx);
                    }
                }

                if should_refresh {
                    this.refresh_remote_dir(cx);
                }
                this.schedule_transfers(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn update_task_state_from_result(
        &mut self,
        task_id: usize,
        result: Result<(), anyhow::Error>,
        cx: &mut Context<Self>,
    ) {
        let mut refresh_operation: Option<TransferOperation> = None;
        if let Some(task) = self
            .transfer_queue
            .tasks
            .iter_mut()
            .find(|task| task.id == task_id)
        {
            task.shared_progress
                .scanning
                .store(false, Ordering::Relaxed);
            match result {
                Ok(_) => {
                    task.state = TransferTaskState::Completed;
                    task.error = None;
                }
                Err(error) => {
                    if Self::is_transfer_cancelled(&error) {
                        task.state = TransferTaskState::Cancelled;
                        task.error = None;
                        refresh_operation = Some(task.operation.clone());
                    } else {
                        task.state = TransferTaskState::Failed;
                        task.error = Some(error.to_string());
                    }
                }
            }
        }

        if let Some(operation) = refresh_operation {
            self.refresh_panel_for_operation(&operation, cx);
        }
    }

    fn refresh_panel_for_operation(
        &mut self,
        operation: &TransferOperation,
        cx: &mut Context<Self>,
    ) {
        match operation {
            TransferOperation::Upload { .. } | TransferOperation::DeleteRemote { .. } => {
                self.refresh_remote_dir(cx);
            }
            TransferOperation::Download { .. } | TransferOperation::DeleteLocal { .. } => {
                self.refresh_local_dir(cx);
            }
        }
    }

    fn is_transfer_cancelled(error: &anyhow::Error) -> bool {
        error.downcast_ref::<TransferCancelled>().is_some()
    }

    fn execute_uploads(&mut self, transfers: Vec<PendingTransfer>, cx: &mut Context<Self>) {
        for transfer in transfers {
            let task_id = self.next_task_id;
            self.next_task_id += 1;

            let shared_progress = Arc::new(SharedProgress {
                transferred: AtomicU64::new(0),
                total: AtomicU64::new(0),
                speed: AtomicU64::new(0),
                cancelled: Arc::new(AtomicBool::new(false)),
                scanning: AtomicBool::new(false),
                current_file: std::sync::RwLock::new(None),
                current_file_transferred: AtomicU64::new(0),
                current_file_total: AtomicU64::new(0),
            });

            let remote_dir = if let Some(pos) = transfer.remote_path.rfind('/') {
                let dir = &transfer.remote_path[..pos];
                if dir.is_empty() {
                    "/".to_string()
                } else {
                    dir.to_string()
                }
            } else {
                ".".to_string()
            };

            self.transfer_queue.enqueue(TransferTask {
                id: task_id,
                operation: TransferOperation::Upload {
                    local_path: transfer.local_path,
                    remote_path: transfer.remote_path,
                    is_dir: transfer.is_dir,
                    remote_dir,
                },
                state: TransferTaskState::Pending,
                shared_progress,
                error: None,
            });
        }

        self.schedule_transfers(cx);
    }

    fn start_progress_refresh(&mut self, cx: &mut Context<Self>) {
        if self.progress_refresh_task.is_some() {
            // 即使任务已存在，也立即刷新一次
            cx.notify();
            return;
        }

        self.progress_refresh_task = Some(cx.spawn(async move |this, cx| {
            loop {
                // 先刷新，再等待
                let should_continue = this
                    .update(cx, |this, cx| {
                        let has_active = this.transfer_queue.has_active();

                        if has_active {
                            cx.notify();
                            true
                        } else {
                            this.progress_refresh_task = None;
                            false
                        }
                    })
                    .unwrap_or(false);

                if !should_continue {
                    break;
                }

                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
            }
        }));
    }

    fn cancel_transfer(&mut self, task_id: usize, cx: &mut Context<Self>) {
        let mut refresh_operation: Option<TransferOperation> = None;
        if let Some(task) = self
            .transfer_queue
            .tasks
            .iter_mut()
            .find(|t| t.id == task_id)
        {
            match task.state {
                TransferTaskState::Pending => {
                    task.state = TransferTaskState::Cancelled;
                    task.error = None;
                    refresh_operation = Some(task.operation.clone());
                }
                TransferTaskState::Running => {
                    task.shared_progress
                        .cancelled
                        .store(true, Ordering::Relaxed);
                }
                TransferTaskState::Completed
                | TransferTaskState::Failed
                | TransferTaskState::Cancelled => {}
            }
        }
        if let Some(operation) = refresh_operation {
            self.refresh_panel_for_operation(&operation, cx);
        }
        self.schedule_transfers(cx);
        cx.notify();
    }

    fn cancel_all_transfers(&mut self) {
        self.transfer_queue.pending.clear();
        for task in &mut self.transfer_queue.tasks {
            match task.state {
                TransferTaskState::Pending => {
                    task.state = TransferTaskState::Cancelled;
                    task.error = None;
                }
                TransferTaskState::Running => {
                    task.shared_progress
                        .cancelled
                        .store(true, Ordering::Relaxed);
                }
                TransferTaskState::Completed
                | TransferTaskState::Failed
                | TransferTaskState::Cancelled => {}
            }
        }
    }

    fn push_notification(&self, notification: Notification, cx: &mut Context<Self>) {
        if let Some(window) = cx.active_window() {
            if let Err(error) = window.update(cx, |_, window, cx| {
                window.push_notification(notification, cx);
            }) {
                tracing::error!("Failed to push notification: {}", error);
            }
        }
    }

    fn download_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sftp_client.is_none() {
            return;
        };

        let selected_entries = self.remote_panel.read(cx).selected_items(cx);
        if selected_entries.is_empty() {
            return;
        }

        let local_path = self.local_current_path.clone();
        let remote_path = self.remote_current_path.clone();

        let local_names: std::collections::HashSet<String> = match std::fs::read_dir(&local_path) {
            Ok(entries) => entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect(),
            Err(e) => {
                window.push_notification(
                    Notification::error(t!("Error.read_dir_failed", error = e)),
                    cx,
                );
                return;
            }
        };

        let mut pending_transfers: Vec<PendingTransfer> = Vec::new();
        let mut has_conflict = false;

        for entry in &selected_entries {
            let conflict = local_names.contains(&entry.name);
            if conflict {
                has_conflict = true;
            }

            pending_transfers.push(PendingTransfer {
                name: entry.name.clone(),
                local_path: local_path.join(&entry.name),
                remote_path: join_remote_path(&remote_path, &entry.name),
                is_dir: entry.is_dir,
                has_conflict: conflict,
            });
        }

        if pending_transfers.is_empty() {
            return;
        }

        if has_conflict {
            let conflict_names: Vec<String> = pending_transfers
                .iter()
                .filter(|t| t.has_conflict)
                .map(|t| t.name.clone())
                .collect();

            self.show_conflict_dialog(
                conflict_names,
                pending_transfers,
                false,
                local_names,
                window,
                cx,
            );
        } else {
            self.execute_downloads(pending_transfers, cx);
        }
    }

    fn execute_downloads(&mut self, transfers: Vec<PendingTransfer>, cx: &mut Context<Self>) {
        tracing::info!("execute_downloads: {} transfers", transfers.len());

        for transfer in transfers {
            tracing::info!(
                "execute_downloads: starting download for {:?}",
                transfer.name
            );
            let task_id = self.next_task_id;
            self.next_task_id += 1;

            let shared_progress = Arc::new(SharedProgress {
                transferred: AtomicU64::new(0),
                total: AtomicU64::new(0),
                speed: AtomicU64::new(0),
                cancelled: Arc::new(AtomicBool::new(false)),
                scanning: AtomicBool::new(false),
                current_file: std::sync::RwLock::new(None),
                current_file_transferred: AtomicU64::new(0),
                current_file_total: AtomicU64::new(0),
            });

            let local_dir = transfer
                .local_path
                .parent()
                .map(|path| path.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."));

            self.transfer_queue.enqueue(TransferTask {
                id: task_id,
                operation: TransferOperation::Download {
                    remote_path: transfer.remote_path,
                    local_path: transfer.local_path,
                    is_dir: transfer.is_dir,
                    local_dir,
                },
                state: TransferTaskState::Pending,
                shared_progress,
                error: None,
            });
        }

        self.schedule_transfers(cx);
    }

    fn delete_local_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected_entries = self.local_panel.read(cx).selected_items(cx);

        if selected_entries.is_empty() {
            return;
        }

        let local_path = self.local_current_path.clone();
        let view = cx.entity().downgrade();

        // 构建确认信息
        let file_count = selected_entries.iter().filter(|e| !e.is_dir).count();
        let dir_count = selected_entries.iter().filter(|e| e.is_dir).count();
        let confirm_msg = match (file_count, dir_count) {
            (0, 1) => t!("Delete.confirm_folder").to_string(),
            (0, d) => t!("Delete.confirm_folders", count = d).to_string(),
            (1, 0) => t!("Delete.confirm_file").to_string(),
            (f, 0) => t!("Delete.confirm_files", count = f).to_string(),
            (f, d) => t!("Delete.confirm_mixed", files = f, dirs = d).to_string(),
        };

        let file_list: String = selected_entries
            .iter()
            .take(5)
            .map(|e| {
                if e.is_dir {
                    format!("📁 {}", e.name)
                } else {
                    format!("📄 {}", e.name)
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        let file_list = if selected_entries.len() > 5 {
            format!(
                "{}\n{}",
                file_list,
                t!("Delete.and_more", count = selected_entries.len() - 5)
            )
        } else {
            file_list
        };

        window.open_dialog(cx, move |dialog, _window, cx| {
            let view_confirm = view.clone();
            let entries_confirm = selected_entries.clone();
            let local_path_confirm = local_path.clone();

            dialog
                .title(t!("Dialog.confirm_delete").to_string())
                .w(px(400.))
                .child(
                    v_flex().gap_2().child(confirm_msg.clone()).child(
                        div()
                            .p_2()
                            .bg(cx.theme().secondary)
                            .rounded_md()
                            .text_sm()
                            .overflow_hidden()
                            .child(file_list.clone()),
                    ),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("Common.delete").to_string())
                        .cancel_text(t!("Common.cancel").to_string()),
                )
                .on_ok(move |_, window, cx| {
                    window.close_dialog(cx);

                    let _ = view_confirm.update(cx, |this, cx| {
                        this.execute_local_delete(
                            entries_confirm.clone(),
                            local_path_confirm.clone(),
                            window,
                            cx,
                        );
                    });
                    true
                })
        });
    }

    fn execute_local_delete(
        &mut self,
        entries: Vec<FileItem>,
        local_path: PathBuf,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let first_file = entries.first().map(|e| e.name.clone());

        let shared_progress = Arc::new(SharedProgress {
            transferred: AtomicU64::new(0),
            total: AtomicU64::new(entries.len() as u64),
            speed: AtomicU64::new(0),
            cancelled: Arc::new(AtomicBool::new(false)),
            scanning: AtomicBool::new(false),
            current_file: std::sync::RwLock::new(first_file),
            current_file_transferred: AtomicU64::new(0),
            current_file_total: AtomicU64::new(1),
        });

        let task_id = self.next_task_id;
        self.next_task_id += 1;

        self.transfer_queue.enqueue(TransferTask {
            id: task_id,
            operation: TransferOperation::DeleteLocal {
                entries,
                local_dir: local_path,
            },
            state: TransferTaskState::Pending,
            shared_progress,
            error: None,
        });

        self.schedule_transfers(cx);
    }

    fn delete_remote_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sftp_client.is_none() {
            return;
        };

        let selected_entries = self.remote_panel.read(cx).selected_items(cx);

        if selected_entries.is_empty() {
            return;
        }

        let remote_path = self.remote_current_path.clone();
        let view = cx.entity().downgrade();

        // 构建确认信息
        let file_count = selected_entries.iter().filter(|e| !e.is_dir).count();
        let dir_count = selected_entries.iter().filter(|e| e.is_dir).count();
        let confirm_msg = match (file_count, dir_count) {
            (0, 1) => t!("Delete.confirm_folder").to_string(),
            (0, d) => t!("Delete.confirm_folders", count = d).to_string(),
            (1, 0) => t!("Delete.confirm_file").to_string(),
            (f, 0) => t!("Delete.confirm_files", count = f).to_string(),
            (f, d) => t!("Delete.confirm_mixed", files = f, dirs = d).to_string(),
        };

        let file_list: String = selected_entries
            .iter()
            .take(5)
            .map(|e| {
                if e.is_dir {
                    format!("📁 {}", e.name)
                } else {
                    format!("📄 {}", e.name)
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        let file_list = if selected_entries.len() > 5 {
            format!(
                "{}\n{}",
                file_list,
                t!("Delete.and_more", count = selected_entries.len() - 5)
            )
        } else {
            file_list
        };

        window.open_dialog(cx, move |dialog, _window, cx| {
            let view_confirm = view.clone();
            let entries_confirm = selected_entries.clone();
            let remote_path_confirm = remote_path.clone();

            dialog
                .title(t!("Dialog.confirm_delete").to_string())
                .w(px(400.))
                .child(
                    v_flex().gap_2().child(confirm_msg.clone()).child(
                        div()
                            .p_2()
                            .bg(cx.theme().secondary)
                            .rounded_md()
                            .text_sm()
                            .overflow_hidden()
                            .child(file_list.clone()),
                    ),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("Common.delete").to_string())
                        .cancel_text(t!("Common.cancel").to_string()),
                )
                .on_ok(move |_, window, cx| {
                    window.close_dialog(cx);

                    let _ = view_confirm.update(cx, |this, cx| {
                        this.execute_remote_delete(
                            entries_confirm.clone(),
                            remote_path_confirm.clone(),
                            window,
                            cx,
                        );
                    });
                    true
                })
        });
    }

    fn execute_remote_delete(
        &mut self,
        entries: Vec<FileItem>,
        remote_path: String,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let first_file = entries.first().map(|e| e.name.clone());

        let shared_progress = Arc::new(SharedProgress {
            transferred: AtomicU64::new(0),
            total: AtomicU64::new(entries.len() as u64),
            speed: AtomicU64::new(0),
            cancelled: Arc::new(AtomicBool::new(false)),
            scanning: AtomicBool::new(false),
            current_file: std::sync::RwLock::new(first_file),
            current_file_transferred: AtomicU64::new(0),
            current_file_total: AtomicU64::new(1),
        });

        let task_id = self.next_task_id;
        self.next_task_id += 1;

        self.transfer_queue.enqueue(TransferTask {
            id: task_id,
            operation: TransferOperation::DeleteRemote {
                entries,
                remote_dir: remote_path,
            },
            state: TransferTaskState::Pending,
            shared_progress,
            error: None,
        });

        self.schedule_transfers(cx);
    }

    fn show_new_folder_dialog(
        &mut self,
        side: PanelSide,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("Placeholder.filename")));
        let view = cx.entity().downgrade();

        // 在打开对话框前设置焦点，避免闪烁
        input.update(cx, |state, cx| {
            state.focus(window, cx);
        });

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let side = side;
            let view_clone = view.clone();
            let input_for_callback = input.clone();

            dialog
                .title(t!("File.new_folder").to_string())
                .w(px(360.))
                .child(Input::new(&input))
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("Common.create").to_string())
                        .cancel_text(t!("Common.cancel").to_string()),
                )
                .on_ok(move |_, window, cx| {
                    let folder_name = input_for_callback.read(cx).text().to_string();
                    if folder_name.is_empty() {
                        return false;
                    }
                    if !is_valid_entry_name(&folder_name) {
                        window.push_notification(Notification::error(t!("Error.invalid_name")), cx);
                        return false;
                    }

                    let _ = view_clone.update(cx, |this, cx| match side {
                        PanelSide::Local => {
                            let path = this.local_current_path.join(&folder_name);
                            if let Err(e) = std::fs::create_dir(&path) {
                                tracing::error!(
                                    "Failed to create folder {}: {}",
                                    path.display(),
                                    e
                                );
                                window.push_notification(
                                    Notification::error(t!(
                                        "Error.create_folder_failed",
                                        error = e
                                    )),
                                    cx,
                                );
                            } else {
                                window.close_dialog(cx);
                            }
                            this.refresh_local_dir(cx);
                        }
                        PanelSide::Remote => {
                            let Some(client) = this.sftp_client.clone() else {
                                return;
                            };

                            let remote_path =
                                join_remote_path(&this.remote_current_path, &folder_name);

                            let task = Tokio::spawn(cx, async move {
                                let mut client = client.lock().await;
                                client.mkdir(&remote_path).await
                            });

                            let view = cx.entity().clone();
                            window
                                .spawn(cx, async move |cx| match task.await {
                                    Ok(Ok(_)) => {
                                        let _ = view.update_in(cx, |this, window, cx| {
                                            window.close_dialog(cx);
                                            this.refresh_remote_dir(cx);
                                        });
                                    }
                                    Ok(Err(e)) => {
                                        tracing::error!("Failed to create remote folder: {}", e);
                                        let _ = view.update_in(cx, |_this, window, cx| {
                                            window.push_notification(
                                                Notification::error(t!(
                                                    "Error.create_folder_failed",
                                                    error = e
                                                )),
                                                cx,
                                            );
                                        });
                                    }
                                    Err(e) => {
                                        tracing::error!("Task error: {}", e);
                                        let _ = view.update_in(cx, |_this, window, cx| {
                                            window.push_notification(
                                                Notification::error(t!(
                                                    "Error.create_folder_failed",
                                                    error = e
                                                )),
                                                cx,
                                            );
                                        });
                                    }
                                })
                                .detach();
                        }
                    });
                    false
                })
        });
    }

    fn get_local_selected_count(&self, cx: &App) -> usize {
        self.local_panel.read(cx).selected_items(cx).len()
    }

    fn get_remote_selected_count(&self, cx: &App) -> usize {
        self.remote_panel.read(cx).selected_items(cx).len()
    }

    fn render_drop_overlay(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .inset_0()
            .m_4()
            .border_2()
            .border_color(cx.theme().link)
            .rounded_lg()
            .bg(gpui::rgba(0x3b82f610))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .child(
                Icon::new(IconName::ArrowDown)
                    .size_8()
                    .text_color(cx.theme().muted_foreground),
            )
            .child(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child("Drop files here"),
            )
    }

    fn handle_local_drop(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut copy_errors: Vec<String> = Vec::new();

        for path in paths {
            let file_name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let dest_path = self.local_current_path.join(&file_name);

            if path != dest_path {
                if let Err(e) = std::fs::copy(&path, &dest_path) {
                    tracing::error!("Failed to copy file: {}", e);
                    copy_errors.push(format!("{}: {}", file_name, e));
                }
            }
        }

        if !copy_errors.is_empty() {
            let error_msg = if copy_errors.len() == 1 {
                t!("Error.copy_failed", error = copy_errors[0]).to_string()
            } else {
                t!("Error.copy_n_failed", count = copy_errors.len()).to_string()
            };
            window.push_notification(Notification::error(error_msg), cx);
        }

        self.refresh_local_dir(cx);
    }

    fn handle_remote_drop(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(client) = self.sftp_client.clone() else {
            return;
        };

        let remote_path = self.remote_current_path.clone();
        let view = cx.entity().clone();

        // 先列出远程目录内容，检查冲突
        let list_task = Tokio::spawn(cx, {
            let client = client.clone();
            let remote_path = remote_path.clone();
            async move {
                let mut client_guard = client.lock().await;
                client_guard.list_dir(&remote_path).await
            }
        });

        window
            .spawn(cx, async move |cx| {
                let remote_names: std::collections::HashSet<String> = match list_task.await {
                    Ok(Ok(entries)) => entries.into_iter().map(|e| e.name).collect(),
                    Ok(Err(e)) => {
                        tracing::error!("Failed to list remote directory: {}", e);
                        let error_msg = t!("Error.read_dir_failed", error = e).to_string();
                        let _ = view.update_in(cx, |_this, window, cx| {
                            window.push_notification(Notification::error(error_msg.clone()), cx);
                        });
                        return;
                    }
                    Err(e) => {
                        tracing::error!("Task error: {}", e);
                        let error_msg = t!("Error.read_dir_failed", error = e).to_string();
                        let _ = view.update_in(cx, |_this, window, cx| {
                            window.push_notification(Notification::error(error_msg.clone()), cx);
                        });
                        return;
                    }
                };

                let _ = view.update_in(cx, |this, window, cx| {
                    let mut pending_transfers: Vec<PendingTransfer> = Vec::new();
                    let mut has_conflict = false;

                    for path in &paths {
                        if !path.exists() {
                            continue;
                        }

                        let file_name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();

                        let conflict = remote_names.contains(&file_name);
                        if conflict {
                            has_conflict = true;
                        }

                        pending_transfers.push(PendingTransfer {
                            name: file_name.clone(),
                            local_path: path.clone(),
                            remote_path: join_remote_path(&remote_path, &file_name),
                            is_dir: path.is_dir(),
                            has_conflict: conflict,
                        });
                    }

                    if pending_transfers.is_empty() {
                        return;
                    }

                    if has_conflict {
                        let conflict_names: Vec<String> = pending_transfers
                            .iter()
                            .filter(|t| t.has_conflict)
                            .map(|t| t.name.clone())
                            .collect();

                        this.show_conflict_dialog(
                            conflict_names,
                            pending_transfers,
                            true,
                            remote_names,
                            window,
                            cx,
                        );
                    } else {
                        this.execute_uploads(pending_transfers, cx);
                    }
                });
            })
            .detach();
    }

    fn handle_remote_files_drop_to_local(
        &mut self,
        dragged: DraggedFileItems,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        tracing::info!("handle_remote_files_drop_to_local: {} items", dragged.len());

        if self.sftp_client.is_none() {
            tracing::warn!("handle_remote_files_drop_to_local: no sftp client");
            return;
        };

        let local_path = self.local_current_path.clone();
        let local_names: std::collections::HashSet<String> = match std::fs::read_dir(&local_path) {
            Ok(entries) => entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect(),
            Err(e) => {
                window.push_notification(
                    Notification::error(t!("Error.read_dir_failed", error = e)),
                    cx,
                );
                return;
            }
        };

        let mut pending_transfers: Vec<PendingTransfer> = Vec::new();
        let mut has_conflict = false;

        for item in &dragged.items {
            let conflict = local_names.contains(&item.name);
            if conflict {
                has_conflict = true;
            }

            pending_transfers.push(PendingTransfer {
                name: item.name.clone(),
                local_path: local_path.join(&item.name),
                remote_path: item.full_path.clone(),
                is_dir: item.is_dir,
                has_conflict: conflict,
            });
        }

        if pending_transfers.is_empty() {
            return;
        }

        if has_conflict {
            let conflict_names: Vec<String> = pending_transfers
                .iter()
                .filter(|t| t.has_conflict)
                .map(|t| t.name.clone())
                .collect();

            self.show_conflict_dialog(
                conflict_names,
                pending_transfers,
                false,
                local_names,
                window,
                cx,
            );
        } else {
            self.execute_downloads(pending_transfers, cx);
        }
    }

    fn handle_local_files_drop_to_remote(
        &mut self,
        dragged: DraggedFileItems,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(client) = self.sftp_client.clone() else {
            return;
        };

        // 收集有效的本地文件路径
        let local_files: Vec<(PathBuf, DraggedFileItem)> = dragged
            .items
            .into_iter()
            .filter_map(|item| {
                let path = PathBuf::from(&item.full_path);
                if path.exists() {
                    Some((path, item))
                } else {
                    None
                }
            })
            .collect();

        if local_files.is_empty() {
            return;
        }

        let remote_path = self.remote_current_path.clone();
        let view = cx.entity().clone();

        let list_task = Tokio::spawn(cx, {
            let client = client.clone();
            let remote_path = remote_path.clone();
            async move {
                let mut client_guard = client.lock().await;
                client_guard.list_dir(&remote_path).await
            }
        });

        window
            .spawn(cx, async move |cx| {
                let remote_names: std::collections::HashSet<String> = match list_task.await {
                    Ok(Ok(entries)) => entries.into_iter().map(|e| e.name).collect(),
                    Ok(Err(e)) => {
                        tracing::error!("Failed to list remote directory: {}", e);
                        let error_msg = t!("Error.read_dir_failed", error = e).to_string();
                        let _ = view.update_in(cx, |_this, window, cx| {
                            window.push_notification(Notification::error(error_msg.clone()), cx);
                        });
                        return;
                    }
                    Err(e) => {
                        tracing::error!("Task error: {}", e);
                        let error_msg = t!("Error.read_dir_failed", error = e).to_string();
                        let _ = view.update_in(cx, |_this, window, cx| {
                            window.push_notification(Notification::error(error_msg.clone()), cx);
                        });
                        return;
                    }
                };

                let _ = view.update_in(cx, |this, window, cx| {
                    let mut pending_transfers: Vec<PendingTransfer> = Vec::new();
                    let mut has_conflict = false;

                    for (local_file, item) in &local_files {
                        let conflict = remote_names.contains(&item.name);
                        if conflict {
                            has_conflict = true;
                        }

                        pending_transfers.push(PendingTransfer {
                            name: item.name.clone(),
                            local_path: local_file.clone(),
                            remote_path: join_remote_path(&remote_path, &item.name),
                            is_dir: item.is_dir,
                            has_conflict: conflict,
                        });
                    }

                    if pending_transfers.is_empty() {
                        return;
                    }

                    if has_conflict {
                        let conflict_names: Vec<String> = pending_transfers
                            .iter()
                            .filter(|t| t.has_conflict)
                            .map(|t| t.name.clone())
                            .collect();

                        this.show_conflict_dialog(
                            conflict_names,
                            pending_transfers,
                            true,
                            remote_names,
                            window,
                            cx,
                        );
                    } else {
                        this.execute_uploads(pending_transfers, cx);
                    }
                });
            })
            .detach();
    }

    fn render_connection_overlay(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let is_connecting = matches!(self.connection_state, ConnectionState::Connecting);
        let error_msg = match &self.connection_state {
            ConnectionState::Disconnected { error } => error.clone(),
            _ => None,
        };

        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(Hsla {
                h: 0.,
                s: 0.,
                l: 0.,
                a: 0.7,
            })
            .child(
                v_flex()
                    .gap_4()
                    .items_center()
                    .p_6()
                    .bg(cx.theme().background)
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded_lg()
                    .shadow_lg()
                    .max_w(px(400.))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(if is_connecting {
                                Spinner::new().into_any_element()
                            } else {
                                Icon::new(IconName::CircleX)
                                    .with_size(px(24.))
                                    .text_color(cx.theme().danger)
                                    .into_any_element()
                            })
                            .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child(
                                if is_connecting {
                                    t!("Connection.connecting").to_string()
                                } else {
                                    t!("Connection.disconnected").to_string()
                                },
                            )),
                    )
                    .when_some(error_msg, |el, msg| {
                        el.child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().danger)
                                .max_w(px(350.))
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(msg),
                        )
                    })
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(if is_connecting {
                                t!("Connection.establishing").to_string()
                            } else {
                                t!("Connection.session_disconnected").to_string()
                            }),
                    )
                    .when(!is_connecting, |el| {
                        el.child(
                            Button::new("reconnect-btn")
                                .label(t!("Common.reconnect").to_string())
                                .primary()
                                .on_click(cx.listener(|this, _, _window, cx| {
                                    this.reconnect(cx);
                                })),
                        )
                    }),
            )
    }

    fn render_extract_queue_row(
        &self,
        extract: ActiveExtract,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tooltip_name = extract.path.clone();

        h_flex()
            .gap_2()
            .items_center()
            .child(Spinner::new().small())
            .child(Icon::new(IconName::Unarchive).small())
            .child(
                div()
                    .id("extract-name")
                    .text_sm()
                    .min_w(px(120.))
                    .max_w(px(250.))
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(extract.name)
                    .tooltip(move |window, cx| {
                        Tooltip::new(tooltip_name.clone()).build(window, cx)
                    }),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_xs()
                    .w(px(90.))
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("Extract.running").to_string()),
            )
            .into_any_element()
    }

    fn render_transfer_queue(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active_tasks = self.transfer_queue.active_tasks();

        if active_tasks.is_empty() && self.active_extract.is_none() {
            return div().into_any_element();
        }

        let mut rows = Vec::new();
        for task in active_tasks {
            let is_delete_op = matches!(
                &task.operation,
                TransferOperation::DeleteRemote { .. } | TransferOperation::DeleteLocal { .. }
            );

            let (icon, label) = match &task.operation {
                TransferOperation::Upload { local_path, .. } => (
                    IconName::ArrowUp,
                    local_path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default(),
                ),
                TransferOperation::Download { remote_path, .. } => {
                    let name = remote_path.rsplit('/').next().unwrap_or(remote_path);
                    (IconName::ArrowDown, name.to_string())
                }
                TransferOperation::DeleteRemote { entries, .. } => (
                    IconName::Remove,
                    t!("Delete.delete_n_items", count = entries.len()).to_string(),
                ),
                TransferOperation::DeleteLocal { entries, .. } => (
                    IconName::Remove,
                    t!("Delete.delete_n_items", count = entries.len()).to_string(),
                ),
            };

            let transferred = task.shared_progress.transferred.load(Ordering::Relaxed);
            let total = task.shared_progress.total.load(Ordering::Relaxed);
            let speed_bits = task.shared_progress.speed.load(Ordering::Relaxed);
            let speed = f64::from_bits(speed_bits);
            let is_scanning = task.shared_progress.scanning.load(Ordering::Relaxed);

            let current_file = task
                .shared_progress
                .current_file
                .read()
                .ok()
                .and_then(|g| g.clone());
            let current_file_transferred = task
                .shared_progress
                .current_file_transferred
                .load(Ordering::Relaxed);
            let current_file_total = task
                .shared_progress
                .current_file_total
                .load(Ordering::Relaxed);

            let progress_pct = if total > 0 {
                (transferred as f64 / total as f64 * 100.0) as u32
            } else {
                0
            };

            let current_file_pct = if current_file_total > 0 {
                (current_file_transferred as f64 / current_file_total as f64 * 100.0) as u32
            } else {
                0
            };

            let task_id = task.id;
            let is_running = task.state == TransferTaskState::Running;
            let has_current_file = current_file.is_some();

            let display_name = if is_delete_op {
                if is_scanning {
                    t!("Delete.scanning").to_string()
                } else if let Some(ref file) = current_file {
                    t!("Delete.deleting", name = file).to_string()
                } else {
                    label.clone()
                }
            } else if let Some(ref file) = current_file {
                format!("{} - {}", label, file)
            } else {
                label.clone()
            };
            let tooltip_name = display_name.clone();

            let display_progress = if is_scanning {
                0
            } else if is_delete_op {
                progress_pct
            } else if has_current_file {
                current_file_pct
            } else {
                progress_pct
            };

            rows.push(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(Icon::new(icon).small())
                    .child(
                        div()
                            .id(SharedString::from(format!("transfer-name-{}", task_id)))
                            .text_sm()
                            .min_w(px(120.))
                            .max_w(px(250.))
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(display_name)
                            .tooltip(move |window, cx| {
                                Tooltip::new(tooltip_name.clone()).build(window, cx)
                            }),
                    )
                    .child(div().flex_1().min_w(px(100.)).child(
                        Progress::new("file-transfer-process").value(display_progress as f32),
                    ))
                    .child(
                        div()
                            .text_xs()
                            .w(px(50.))
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_color(cx.theme().muted_foreground)
                            .child(match task.state {
                                TransferTaskState::Pending => "Pending".to_string(),
                                TransferTaskState::Running => {
                                    if is_scanning {
                                        t!("Common.scanning").to_string()
                                    } else if is_delete_op {
                                        format!("{}/{}", transferred, total)
                                    } else {
                                        format!("{}%", display_progress)
                                    }
                                }
                                TransferTaskState::Completed => "Done".to_string(),
                                TransferTaskState::Failed => "Failed".to_string(),
                                TransferTaskState::Cancelled => "Cancelled".to_string(),
                            }),
                    )
                    .when(has_current_file && !is_delete_op, |el| {
                        el.child(
                            div()
                                .text_xs()
                                .w(px(50.))
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("{}%", progress_pct)),
                        )
                    })
                    .child(
                        div()
                            .text_xs()
                            .w(px(80.))
                            .text_color(cx.theme().muted_foreground)
                            .child(if is_running && speed > 0.0 && !is_delete_op {
                                format_speed(speed)
                            } else {
                                String::new()
                            }),
                    )
                    .child(
                        Button::new(SharedString::from(format!("cancel-{}", task_id)))
                            .icon(IconName::Close)
                            .ghost()
                            .xsmall()
                            .on_click(cx.listener(move |this, _, _window, cx| {
                                this.cancel_transfer(task_id, cx);
                            })),
                    )
                    .into_any_element(),
            );
        }

        if let Some(extract) = self.active_extract.clone() {
            rows.push(self.render_extract_queue_row(extract, cx));
        }

        v_flex()
            .border_t_1()
            .border_color(cx.theme().border)
            .p_2()
            .gap_1()
            .children(rows)
            .into_any_element()
    }

    fn render_local_breadcrumb(&self, cx: &mut Context<Self>) -> Breadcrumb {
        let mut breadcrumb = Breadcrumb::new();
        let components: Vec<_> = self.local_current_path.components().collect();
        let total = components.len();
        const MAX_VISIBLE: usize = 4;

        if total <= MAX_VISIBLE {
            for (idx, component) in components.iter().enumerate() {
                let path_so_far: PathBuf = components[..=idx].iter().collect();
                let label = component.as_os_str().to_string_lossy().to_string();
                let label = if label.is_empty() || label == "/" {
                    "/".to_string()
                } else {
                    label
                };

                breadcrumb = breadcrumb.child(breadcrumb_item(label).on_click(cx.listener(
                    move |this, _, _window, cx| {
                        this.navigate_local_to(path_so_far.clone(), cx);
                    },
                )));
            }
        } else {
            let first_component = &components[0];
            let first_path: PathBuf = [first_component].iter().collect();
            let first_label = first_component.as_os_str().to_string_lossy().to_string();
            let first_label = if first_label.is_empty() || first_label == "/" {
                "/".to_string()
            } else {
                first_label
            };
            breadcrumb = breadcrumb.child(breadcrumb_item(first_label).on_click(cx.listener(
                move |this, _, _window, cx| {
                    this.navigate_local_to(first_path.clone(), cx);
                },
            )));

            breadcrumb = breadcrumb.child(breadcrumb_item("...").disabled(true));

            let visible_start = total - (MAX_VISIBLE - 2);
            for idx in visible_start..total {
                let path_so_far: PathBuf = components[..=idx].iter().collect();
                let label = components[idx].as_os_str().to_string_lossy().to_string();

                breadcrumb = breadcrumb.child(breadcrumb_item(label).on_click(cx.listener(
                    move |this, _, _window, cx| {
                        this.navigate_local_to(path_so_far.clone(), cx);
                    },
                )));
            }
        }

        breadcrumb
    }

    fn render_remote_breadcrumb(&self, cx: &mut Context<Self>) -> Breadcrumb {
        let mut breadcrumb = Breadcrumb::new();
        const MAX_VISIBLE: usize = 4;

        if self.remote_current_path == "." {
            breadcrumb = breadcrumb.child(breadcrumb_item("."));
        } else {
            let parts: Vec<&str> = self
                .remote_current_path
                .split('/')
                .filter(|s| !s.is_empty())
                .collect();

            let starts_with_slash = self.remote_current_path.starts_with('/');
            let total = parts.len() + if starts_with_slash { 1 } else { 0 };

            if total <= MAX_VISIBLE {
                if starts_with_slash {
                    breadcrumb = breadcrumb.child(breadcrumb_item("/").on_click(cx.listener(
                        |this, _, _window, cx| {
                            this.navigate_remote_to("/".to_string(), cx);
                        },
                    )));
                }

                for (idx, part) in parts.iter().enumerate() {
                    let path_so_far = if starts_with_slash {
                        format!("/{}", parts[..=idx].join("/"))
                    } else {
                        parts[..=idx].join("/")
                    };

                    breadcrumb = breadcrumb.child(breadcrumb_item(part.to_string()).on_click(
                        cx.listener(move |this, _, _window, cx| {
                            this.navigate_remote_to(path_so_far.clone(), cx);
                        }),
                    ));
                }
            } else {
                if starts_with_slash {
                    breadcrumb = breadcrumb.child(breadcrumb_item("/").on_click(cx.listener(
                        |this, _, _window, cx| {
                            this.navigate_remote_to("/".to_string(), cx);
                        },
                    )));
                }

                breadcrumb = breadcrumb.child(breadcrumb_item("...").disabled(true));

                let visible_count = MAX_VISIBLE - 2;
                let visible_start = parts.len().saturating_sub(visible_count);
                for idx in visible_start..parts.len() {
                    let path_so_far = if starts_with_slash {
                        format!("/{}", parts[..=idx].join("/"))
                    } else {
                        parts[..=idx].join("/")
                    };

                    breadcrumb =
                        breadcrumb.child(breadcrumb_item(parts[idx].to_string()).on_click(
                            cx.listener(move |this, _, _window, cx| {
                                this.navigate_remote_to(path_so_far.clone(), cx);
                            }),
                        ));
                }
            }
        }

        breadcrumb
    }

    fn render_local_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let breadcrumb = self.render_local_breadcrumb(cx);
        let selected_count = self.get_local_selected_count(cx);
        let has_selection = selected_count > 0;
        let local_path_input = self.local_path_input.clone();
        let is_editing = self.local_path_editing;
        let is_dragging = self.is_dragging_over_local;
        let can_go_back = self.can_go_back_local();
        let can_go_forward = self.can_go_forward_local();
        let is_favorite = self.is_current_local_path_favorite();
        let favorite_paths = self.local_favorite_paths();

        v_flex()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .h_10()
                    .px_2()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("back")
                                    .icon(IconName::ChevronLeft)
                                    .ghost()
                                    .small()
                                    .disabled(!can_go_back)
                                    .on_click(cx.listener(|this, _, _window, cx| {
                                        this.go_back_local(cx);
                                    })),
                            )
                            .child(
                                Button::new("forward")
                                    .icon(IconName::ChevronRight)
                                    .ghost()
                                    .small()
                                    .disabled(!can_go_forward)
                                    .on_click(cx.listener(|this, _, _window, cx| {
                                        this.go_forward_local(cx);
                                    })),
                            ),
                    )
                    .child(if is_editing {
                        h_flex()
                            .flex_1()
                            .min_w(px(0.))
                            .h_7()
                            .px_2()
                            .items_center()
                            .bg(cx.theme().secondary)
                            .rounded_md()
                            .child(
                                Input::new(&local_path_input)
                                    .small()
                                    .appearance(false)
                                    .cleanable(false)
                                    .w_full(),
                            )
                            .into_any_element()
                    } else {
                        h_flex()
                            .id("local-path-bar")
                            .flex_1()
                            .min_w(px(0.))
                            .h_7()
                            .px_2()
                            .items_center()
                            .bg(cx.theme().secondary)
                            .rounded_md()
                            .cursor_text()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.start_local_path_editing(window, cx);
                            }))
                            .child(breadcrumb.flex_1().min_w(px(0.)).overflow_hidden())
                            .into_any_element()
                    })
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("local_toggle_favorite")
                                    .icon(if is_favorite {
                                        IconName::StarFill
                                    } else {
                                        IconName::Star
                                    })
                                    .ghost()
                                    .small()
                                    .tooltip(if is_favorite {
                                        t!("FavoritePath.remove_current").to_string()
                                    } else {
                                        t!("FavoritePath.add_current").to_string()
                                    })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.toggle_current_local_favorite(window, cx);
                                    })),
                            )
                            .child(self.render_local_favorites_menu(favorite_paths, cx))
                            .child(
                                Button::new("refresh_local")
                                    .icon(IconName::Refresh)
                                    .ghost()
                                    .small()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.refresh_local_dir_with_window(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("local_upload")
                                    .icon(IconName::Upload)
                                    .ghost()
                                    .small()
                                    .disabled(!has_selection)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.upload_selected(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("local_new_folder")
                                    .icon(IconName::NewFolder)
                                    .ghost()
                                    .small()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.show_new_folder_dialog(PanelSide::Local, window, cx);
                                    })),
                            )
                            .child(
                                Button::new("local_delete")
                                    .icon(IconName::Remove)
                                    .ghost()
                                    .small()
                                    .disabled(!has_selection)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.delete_local_selected(window, cx);
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .id("local-drop-zone")
                    .flex_1()
                    .relative()
                    .drag_over::<ExternalPaths>(|el, _, _, _cx| el.bg(gpui::rgba(0x3b82f620)))
                    .drag_over::<DraggedFileItems>(|el, _, _, _cx| el.bg(gpui::rgba(0x3b82f620)))
                    .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                        this.is_dragging_over_local = false;
                        this.handle_local_drop(paths.paths().to_vec(), window, cx);
                    }))
                    .on_drop(cx.listener(|this, items: &DraggedFileItems, window, cx| {
                        tracing::info!("local-drop-zone on_drop: items count={}", items.len());
                        this.is_dragging_over_local = false;
                        if items.is_remote {
                            this.handle_remote_files_drop_to_local(items.clone(), window, cx);
                        }
                    }))
                    .child(self.local_panel.clone())
                    .when(is_dragging, |el| el.child(self.render_drop_overlay(cx))),
            )
    }

    fn render_remote_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let breadcrumb = self.render_remote_breadcrumb(cx);
        let selected_count = self.get_remote_selected_count(cx);
        let has_selection = selected_count > 0;
        let is_connected = self.connection_state == ConnectionState::Connected;
        let remote_path_input = self.remote_path_input.clone();
        let is_editing = self.remote_path_editing;
        let is_dragging = self.is_dragging_over_remote;
        let can_go_back = self.can_go_back_remote();
        let can_go_forward = self.can_go_forward_remote();
        let is_favorite = self.is_current_remote_path_favorite();
        let favorite_paths = self.remote_favorite_paths();

        v_flex()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .child(
                h_flex()
                    .h_10()
                    .px_2()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("remote_back")
                                    .icon(IconName::ChevronLeft)
                                    .ghost()
                                    .small()
                                    .disabled(!can_go_back)
                                    .on_click(cx.listener(|this, _, _window, cx| {
                                        this.go_back_remote(cx);
                                    })),
                            )
                            .child(
                                Button::new("remote_forward")
                                    .icon(IconName::ChevronRight)
                                    .ghost()
                                    .small()
                                    .disabled(!can_go_forward)
                                    .on_click(cx.listener(|this, _, _window, cx| {
                                        this.go_forward_remote(cx);
                                    })),
                            ),
                    )
                    .child(if is_editing {
                        h_flex()
                            .flex_1()
                            .min_w(px(0.))
                            .h_7()
                            .px_2()
                            .items_center()
                            .bg(cx.theme().secondary)
                            .rounded_md()
                            .child(
                                Input::new(&remote_path_input)
                                    .small()
                                    .appearance(false)
                                    .cleanable(false)
                                    .w_full(),
                            )
                            .into_any_element()
                    } else {
                        h_flex()
                            .id("remote-path-bar")
                            .flex_1()
                            .min_w(px(0.))
                            .h_7()
                            .px_2()
                            .items_center()
                            .bg(cx.theme().secondary)
                            .rounded_md()
                            .cursor_text()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.start_remote_path_editing(window, cx);
                            }))
                            .child(breadcrumb.flex_1().min_w(px(0.)).overflow_hidden())
                            .into_any_element()
                    })
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("remote_toggle_favorite")
                                    .icon(if is_favorite {
                                        IconName::StarFill
                                    } else {
                                        IconName::Star
                                    })
                                    .ghost()
                                    .small()
                                    .tooltip(if is_favorite {
                                        t!("FavoritePath.remove_current").to_string()
                                    } else {
                                        t!("FavoritePath.add_current").to_string()
                                    })
                                    .disabled(!is_connected)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.toggle_current_remote_favorite(window, cx);
                                    })),
                            )
                            .child(self.render_remote_favorites_menu(
                                favorite_paths,
                                is_connected,
                                cx,
                            ))
                            .child(
                                Button::new("refresh_remote")
                                    .icon(IconName::Refresh)
                                    .ghost()
                                    .small()
                                    .disabled(!is_connected)
                                    .tooltip(t!("Common.refresh"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.refresh_remote_dir_with_window(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("remote_download")
                                    .icon(IconName::ArrowDown)
                                    .ghost()
                                    .small()
                                    .tooltip(t!("Common.download"))
                                    .disabled(!has_selection || !is_connected)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.download_selected(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("remote_new_folder")
                                    .icon(IconName::NewFolder)
                                    .ghost()
                                    .small()
                                    .tooltip(t!("File.new_folder"))
                                    .disabled(!is_connected)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.show_new_folder_dialog(PanelSide::Remote, window, cx);
                                    })),
                            )
                            .child(
                                Button::new("remote_delete")
                                    .icon(IconName::Remove)
                                    .tooltip(t!("Common.delete"))
                                    .ghost()
                                    .small()
                                    .disabled(!has_selection || !is_connected)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.delete_remote_selected(window, cx);
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .id("remote-drop-zone")
                    .flex_1()
                    .relative()
                    .when(is_connected, |el| {
                        el.drag_over::<ExternalPaths>(|el, _, _, _cx| el.bg(gpui::rgba(0x3b82f620)))
                            .drag_over::<DraggedFileItems>(|el, _, _, _cx| {
                                el.bg(gpui::rgba(0x3b82f620))
                            })
                            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                                this.is_dragging_over_remote = false;
                                this.handle_remote_drop(paths.paths().to_vec(), window, cx);
                            }))
                            .on_drop(cx.listener(|this, items: &DraggedFileItems, window, cx| {
                                this.is_dragging_over_remote = false;
                                if !items.is_remote {
                                    this.handle_local_files_drop_to_remote(
                                        items.clone(),
                                        window,
                                        cx,
                                    );
                                }
                            }))
                    })
                    .child(match &self.connection_state {
                        ConnectionState::Connected => div()
                            .size_full()
                            .relative()
                            .child(self.remote_panel.clone())
                            .when(self.remote_loading, |el| {
                                el.child(
                                    div()
                                        .absolute()
                                        .inset_0()
                                        .bg(gpui::rgba(0x00000040))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(Spinner::new().with_size(Size::Large)),
                                )
                            })
                            .into_any_element(),
                        ConnectionState::Connecting => h_flex()
                            .size_full()
                            .justify_center()
                            .items_center()
                            .child(Spinner::new().with_size(Size::Large))
                            .child(
                                div()
                                    .ml_2()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(t!("Connection.connecting").to_string()),
                            )
                            .into_any_element(),
                        ConnectionState::Disconnected { .. } => h_flex()
                            .size_full()
                            .justify_center()
                            .items_center()
                            .child(
                                Icon::new(IconName::CircleX)
                                    .with_size(px(18.))
                                    .text_color(cx.theme().danger),
                            )
                            .child(
                                div()
                                    .ml_2()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(t!("Connection.disconnected").to_string()),
                            )
                            .into_any_element(),
                    })
                    .when(is_dragging && is_connected, |el| {
                        el.child(self.render_drop_overlay(cx))
                    }),
            )
    }
}

impl EventEmitter<TabContentEvent> for SftpView {}
impl EventEmitter<SftpViewEvent> for SftpView {}

impl TabContent for SftpView {
    fn content_key(&self) -> &'static str {
        "SFTP"
    }

    fn title(&self, _cx: &App) -> SharedString {
        // 如果有序号，添加到标题后
        if let Some(index) = self.tab_index {
            format!("{}({})", self.connection_name, index).into()
        } else {
            self.connection_name.clone().into()
        }
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        Some(Icon::new(IconName::Folder1).color().with_size(Size::Medium))
    }

    fn closeable(&self, _cx: &App) -> bool {
        true
    }

    fn can_split(&self, _cx: &App) -> bool {
        true
    }

    fn try_close(
        &mut self,
        _tab_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Task<bool> {
        // 检查是否有正在进行的传输任务
        let active_tasks = self.transfer_queue.active_tasks();

        if !active_tasks.is_empty() {
            // 有正在进行的任务，弹出确认对话框
            let (tx, rx) = tokio::sync::oneshot::channel::<bool>();
            let tx = Arc::new(std::sync::Mutex::new(Some(tx)));

            let task_count = active_tasks.len();
            let tx_ok = tx.clone();
            let tx_cancel = tx;

            window.open_dialog(cx, move |dialog, _window, _cx| {
                let tx_ok = tx_ok.clone();
                let tx_cancel = tx_cancel.clone();
                dialog
                    .title(t!("Dialog.confirm_close").to_string())
                    .w(px(400.))
                    .child(
                        v_flex()
                            .gap_2()
                            .child(t!("Transfer.has_active_tasks", count = task_count).to_string())
                            .child(t!("Transfer.confirm_close_warning").to_string()),
                    )
                    .confirm()
                    .button_props(
                        DialogButtonProps::default()
                            .ok_text(t!("Common.close").to_string())
                            .cancel_text(t!("Common.cancel").to_string()),
                    )
                    .on_ok(move |_, window, cx| {
                        window.close_dialog(cx);
                        if let Ok(mut guard) = tx_ok.lock() {
                            if let Some(sender) = guard.take() {
                                let _ = sender.send(true);
                            }
                        }
                        true
                    })
                    .on_cancel(move |_, window, cx| {
                        window.close_dialog(cx);
                        if let Ok(mut guard) = tx_cancel.lock() {
                            if let Some(sender) = guard.take() {
                                let _ = sender.send(false);
                            }
                        }
                        true
                    })
                    .overlay_closable(false)
                    .close_button(false)
            });

            let client = self.sftp_client.take();
            // 预先创建断开连接的 tokio task（但还不执行）
            let disconnect_task = client.clone().map(|c| {
                Tokio::spawn(cx, async move {
                    let mut guard = c.lock().await;
                    if let Err(e) = guard.disconnect().await {
                        tracing::error!("关闭 SFTP 连接失败: {}", e);
                    }
                })
            });

            return cx.spawn(async move |this, cx| {
                let confirmed = rx.await.unwrap_or(false);
                if confirmed {
                    let _ = this.update(cx, |this, cx| {
                        this.cancel_all_transfers();
                        cx.notify();
                    });
                    // 用户确认关闭，断开连接
                    if let Some(task) = disconnect_task {
                        let _ = task.await;
                    }
                    let _ = this.update(cx, |this, cx| {
                        this.set_connection_active(false, cx);
                    });
                    true
                } else {
                    // 用户取消，恢复 client
                    let _ = this.update(cx, |this, _cx| {
                        this.sftp_client = client;
                    });
                    false
                }
            });
        }

        // 没有活跃任务，直接关闭
        if let Some(client) = self.sftp_client.take() {
            let task = Tokio::spawn(cx, async move {
                let mut guard = client.lock().await;
                if let Err(e) = guard.disconnect().await {
                    tracing::error!("关闭 SFTP 连接失败: {}", e);
                }
            });
            return cx.spawn(async move |this, cx| {
                let _ = task.await;
                let _ = this.update(cx, |this, cx| {
                    this.set_connection_active(false, cx);
                });
                true
            });
        }
        self.set_connection_active(false, cx);
        gpui::Task::ready(true)
    }
}

impl Focusable for SftpView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SftpView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_disconnected = matches!(self.connection_state, ConnectionState::Disconnected { .. });

        v_flex()
            .size_full()
            .relative()
            .bg(cx.theme().background)
            .child(
                h_flex()
                    .flex_1()
                    .child(self.render_local_panel(cx))
                    .child(self.render_remote_panel(cx)),
            )
            .child(self.render_transfer_queue(cx))
            .when(is_disconnected, |el| {
                el.child(self.render_connection_overlay(cx))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{should_apply_local_listing, should_apply_remote_listing};
    use std::path::Path;

    #[test]
    fn only_apply_remote_listing_for_active_path() {
        assert!(should_apply_remote_listing("/srv/app", "/srv/app"));
        assert!(!should_apply_remote_listing("/srv/other", "/srv/app"));
    }

    #[test]
    fn only_apply_local_listing_for_active_path() {
        assert!(should_apply_local_listing(
            Path::new("/tmp/a"),
            Path::new("/tmp/a")
        ));
        assert!(!should_apply_local_listing(
            Path::new("/tmp/b"),
            Path::new("/tmp/a")
        ));
    }

    #[test]
    fn archive_kind_detects_supported_remote_archives() {
        assert_eq!(
            Some(super::ArchiveKind::Zip),
            super::archive_kind_for_name("APP.ZIP")
        );
        assert_eq!(
            Some(super::ArchiveKind::TarGz),
            super::archive_kind_for_name("release.tar.gz")
        );
        assert_eq!(
            Some(super::ArchiveKind::Tgz),
            super::archive_kind_for_name("release.tgz")
        );
        assert_eq!(None, super::archive_kind_for_name("notes.txt"));
    }

    #[test]
    fn build_remote_extract_command_quotes_paths_and_uses_archive_parent() {
        assert_eq!(
            Some("unzip -o -- '/srv/a'\\''b/app.zip' -d '/srv/a'\\''b'".to_string()),
            super::build_remote_extract_command(
                "/srv/a'b/app.zip",
                "app.zip",
                super::ExtractConflictAction::Overwrite
            )
        );
        assert_eq!(
            Some("tar -xzf '/tmp/release.tar.gz' -C '/tmp'".to_string()),
            super::build_remote_extract_command(
                "/tmp/release.tar.gz",
                "release.tar.gz",
                super::ExtractConflictAction::Overwrite
            )
        );
        assert_eq!(
            None,
            super::build_remote_extract_command(
                "/tmp/readme.md",
                "readme.md",
                super::ExtractConflictAction::Overwrite
            )
        );
    }

    #[test]
    fn build_remote_extract_command_can_skip_existing_targets() {
        assert_eq!(
            Some("unzip -n -- '/tmp/app.zip' -d '/tmp'".to_string()),
            super::build_remote_extract_command(
                "/tmp/app.zip",
                "app.zip",
                super::ExtractConflictAction::SkipExisting
            )
        );
        assert_eq!(
            Some("tar --skip-old-files -xzf '/tmp/release.tar.gz' -C '/tmp'".to_string()),
            super::build_remote_extract_command(
                "/tmp/release.tar.gz",
                "release.tar.gz",
                super::ExtractConflictAction::SkipExisting
            )
        );
        assert_eq!(
            Some("test -e '/tmp/app.log' || gzip -dk -- '/tmp/app.log.gz'".to_string()),
            super::build_remote_extract_command(
                "/tmp/app.log.gz",
                "app.log.gz",
                super::ExtractConflictAction::SkipExisting
            )
        );
    }

    #[test]
    fn build_remote_extract_conflict_check_command_detects_existing_targets() {
        assert_eq!(
            Some("test -e '/tmp/app.log'".to_string()),
            super::build_remote_extract_conflict_check_command("/tmp/app.log.gz", "app.log.gz")
        );

        let zip_command =
            super::build_remote_extract_conflict_check_command("/srv/a'b/app.zip", "app.zip")
                .unwrap();
        assert!(zip_command.contains("parent='/srv/a'\\''b'"));
        assert!(zip_command.contains("unzip -Z1 -- '/srv/a'\\''b/app.zip'"));
        assert!(zip_command.contains("[ -e \"$parent/$entry\" ]"));

        let tar_command = super::build_remote_extract_conflict_check_command(
            "/tmp/release.tar.gz",
            "release.tar.gz",
        )
        .unwrap();
        assert!(tar_command.contains("tar -tf '/tmp/release.tar.gz'"));
    }
}
