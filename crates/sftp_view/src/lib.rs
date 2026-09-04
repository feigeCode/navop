rust_i18n::i18n!("locales", fallback = "en");

mod context_menu_handler;
mod direct_copy_prompt;
mod direct_copy_prompt_ui;
mod endpoint;
mod endpoint_switcher;
mod file_clipboard;
mod file_list_panel;
mod file_list_preferences;
mod host_key_prompt;
mod left_remote;
mod left_remote_state;
mod ssh_config;

use context_menu_handler::ContextMenuHandler;
use endpoint::{
    DragSource, LeftEndpointKind, LeftEndpointValue, PaneSide, TransferRoute, transfer_route,
};
use file_clipboard::{ClipboardEndpoint, FileClipboard};
pub use file_list_panel::{
    DirectorySizeState, DraggedFileItem, DraggedFileItems, FileItem, FileListPanel,
    FileListPanelEvent,
};

use gpui::{
    Anchor, AnyElement, AnyWindowHandle, App, AsyncApp, ColorExt, Context, Entity, EventEmitter,
    ExternalPaths, FocusHandle, Focusable, FontWeight, IntoElement, MouseButton, ParentElement,
    Render, SharedString, Styled, WeakEntity, Window, actions, div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, IconSize, Sizable, Size, WindowExt,
    breadcrumb::{Breadcrumb, BreadcrumbItem},
    button::{Button, ButtonVariants},
    dialog::{DialogButtonProps, DialogFooter},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenuItem},
    notification::Notification,
    popover::{Popover, PopoverState},
    progress::Progress,
    scroll::ScrollableElement,
    spinner::Spinner,
    tooltip::Tooltip,
    v_flex,
};
use one_core::background_tasks::{
    BackgroundTaskCancellation, BackgroundTaskHandle, BackgroundTaskProgressUnit,
    BackgroundTaskSpec,
};
use one_core::gpui_tokio::Tokio;
use one_core::settings::AppSettings;
use one_core::storage::models::{ActiveConnections, StoredConnection};
use one_core::storage::{
    GlobalStorageState, SftpFavoritePathRepository, normalize_sftp_favorite_path,
    sftp_favorite_connection_key,
};
use one_core::tab_container::{TabContent, TabContentEvent};
use one_ui::file_conflict_prompt::{
    FileConflictChoice, FileConflictPrompt, FileConflictPromptLabels, FileConflictPromptSpec,
};
use one_ui::marquee_text::marquee_text;
use one_ui::{IconButton, IconButtonRole};
use remote_file_editor::{
    ExternalEditorOpenRequest, RemoteMutationCallback, open_remote_file_editor,
    open_remote_file_external_editor,
};
use remote_image_preview::{
    clipboard_upload_paths, image_format_for_path, open_remote_image_preview,
};
use rust_i18n::t;
use sftp::{DirectoryConflictPolicy, ServerCopyItem, ServerCopyRequest, copy_between_servers};
use sftp::{RusshSftpClient, SftpClient, TransferCancelled, TransferProgress};
use sftp_transfer::{
    SftpConnectionIdentity, SftpDeleteRemoteRequest, SftpDownloadRequest, SftpRemoteDeleteEntry,
    SftpTransferEvent, SftpTransferExecutor, SftpTransferId, SftpTransferOperation,
    SftpTransferReservation, SftpTransferSnapshot, SftpTransferState, SftpUploadConnection,
    SftpUploadRequest, UploadConflictResolver, delete_remote_task_key, download_task_key,
    upload_task_key,
};
use ssh::{ChannelEvent, SshChannel, SshConnectConfig, SshSessionManager};
use std::cell::RefCell;
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime};
use tokio::sync::Mutex;

use host_key_prompt::{HostKeyPromptTarget, host_key_prompt_request};
use left_remote_state::{LeftRemoteConnectionState, LeftRemoteEndpoint};

actions!(
    sftp_view,
    [
        ToggleFocus,
        RefreshFiles,
        Upload,
        Download,
        Delete,
        NewFolder,
        Rename,
        PasteUpload
    ]
);

pub const SFTP_VIEW_CONTEXT: &str = "SftpView";

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
    AwaitingCredentials,
    Connecting,
    Connected,
    Disconnected { error: Option<String> },
}

fn tab_connection_status(state: &ConnectionState) -> one_core::tab_container::TabConnectionStatus {
    match state {
        ConnectionState::AwaitingCredentials | ConnectionState::Connecting => {
            one_core::tab_container::TabConnectionStatus::Connecting
        }
        ConnectionState::Connected => one_core::tab_container::TabConnectionStatus::Connected,
        ConnectionState::Disconnected { .. } => {
            one_core::tab_container::TabConnectionStatus::Disconnected
        }
    }
}

struct SftpCredentialInputs {
    policy: ssh_config::SshCredentialPromptPolicy,
    username: Option<Entity<InputState>>,
    password: Option<Entity<InputState>>,
    error: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum PanelSide {
    Local,
    Remote,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CloseState {
    Open,
    AwaitingDecision,
    Closing,
}

impl CloseState {
    fn begin_confirmation(&mut self) -> bool {
        if !matches!(self, Self::Open) {
            return false;
        }
        *self = Self::AwaitingDecision;
        true
    }

    fn abort_confirmation(&mut self) {
        if matches!(self, Self::AwaitingDecision) {
            *self = Self::Open;
        }
    }

    fn begin_close(&mut self) -> bool {
        if matches!(self, Self::Closing) {
            return false;
        }
        *self = Self::Closing;
        true
    }

    fn is_closing(self) -> bool {
        matches!(self, Self::Closing)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CloseTransferStrategy {
    Wait,
    CancelTransfers,
    Background,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CloseChoice {
    Abort,
    Close(CloseTransferStrategy),
}

#[derive(Clone, PartialEq)]
struct FavoritePathEdit {
    side: PanelSide,
    original_path: String,
}

const MAX_CONCURRENT_TRANSFERS: usize = 2;
const SFTP_DISCONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const BREADCRUMB_ITEM_MAX_WIDTH: f32 = 180.0;
const LOCAL_FAVORITE_CONNECTION_KEY: &str = "local-file-list:global";

struct TransferClientPool {
    config: SshConnectConfig,
    max_size: usize,
    retired: AtomicBool,
    state: Mutex<TransferClientPoolState<Arc<Mutex<RusshSftpClient>>>>,
}

struct TransferClientPoolState<Client> {
    total_created: usize,
    available: Vec<Client>,
}

enum BoundedDisconnectOutcome {
    Disconnected,
    Failed(anyhow::Error),
    TimedOut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TransferAdmission {
    Open,
    Frozen,
}

struct TransferQueue {
    tasks: Vec<TransferTask>,
    pending: VecDeque<usize>,
    max_concurrent: usize,
    admission: TransferAdmission,
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

fn new_shared_progress() -> Arc<SharedProgress> {
    Arc::new(SharedProgress {
        transferred: AtomicU64::new(0),
        total: AtomicU64::new(0),
        speed: AtomicU64::new(0),
        cancelled: Arc::new(AtomicBool::new(false)),
        scanning: AtomicBool::new(false),
        current_file: std::sync::RwLock::new(None),
        current_file_transferred: AtomicU64::new(0),
        current_file_total: AtomicU64::new(0),
    })
}

const MAX_TRANSFER_ERROR_SUMMARY_CHARS: usize = 500;

fn transfer_error_summary(error: &anyhow::Error) -> String {
    let error = error.to_string();
    let mut summary = String::with_capacity(error.len().min(MAX_TRANSFER_ERROR_SUMMARY_CHARS));
    let mut previous_was_whitespace = false;
    let mut character_count = 0;
    for character in error.chars() {
        let character = if character.is_control() {
            ' '
        } else {
            character
        };
        if character.is_whitespace() {
            if previous_was_whitespace {
                continue;
            }
            previous_was_whitespace = true;
            summary.push(' ');
        } else {
            previous_was_whitespace = false;
            summary.push(character);
        }
        character_count += 1;
        if character_count >= MAX_TRANSFER_ERROR_SUMMARY_CHARS {
            summary.push('…');
            break;
        }
    }
    summary.trim().to_string()
}

#[derive(Clone)]
struct TransferTask {
    id: usize,
    external_transfer_id: Option<SftpTransferId>,
    operation: TransferOperation,
    state: TransferTaskState,
    shared_progress: Arc<SharedProgress>,
    error: Option<String>,
    background_task: Option<BackgroundTaskHandle>,
}

#[derive(Clone, Debug, PartialEq)]
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
        directory_conflict_policy: DirectoryConflictPolicy,
        remote_dir: String,
    },
    Download {
        remote_path: String,
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
    ServerCopy(Box<ServerCopyOperation>),
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
struct PendingUploadDecision {
    transfer: PendingTransfer,
    directory_conflict_policy: DirectoryConflictPolicy,
}

#[derive(Clone)]
struct UploadConflictSession {
    connection_generation: u64,
    resolver: Rc<RefCell<UploadConflictResolver<PendingUploadDecision>>>,
    existing_names: Rc<RefCell<HashSet<String>>>,
}

#[derive(Clone)]
struct SftpUploadContext {
    connection: SftpConnectionIdentity,
    connection_source: SftpUploadConnection,
    task_group: SharedString,
}

struct PreparedGlobalUpload {
    request: SftpUploadRequest,
    operation: TransferOperation,
}

struct PreparedGlobalDownload {
    request: SftpDownloadRequest,
    operation: TransferOperation,
}

struct PreparedGlobalRemoteDelete {
    request: SftpDeleteRemoteRequest,
    operation: TransferOperation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TransferRefreshTarget {
    Remote(String),
    Local(PathBuf),
}

#[derive(Clone)]
struct ActiveExtract {
    background_task: BackgroundTaskHandle,
}

struct RemoteCommandOutput {
    stdout: String,
    stderr: String,
    exit_status: u32,
}

struct ServerCopyTaskInput {
    task_id: usize,
    source_config: SshConnectConfig,
    target_config: SshConnectConfig,
    items: Vec<ServerCopyItem>,
    target_side: PaneSide,
    progress: Arc<SharedProgress>,
}

#[derive(Clone)]
struct ServerCopyOperation {
    source_config: SshConnectConfig,
    target_config: SshConnectConfig,
    items: Vec<ServerCopyItem>,
    target_side: PaneSide,
}

#[derive(Clone)]
struct PendingServerCopy {
    source_config: SshConnectConfig,
    target_config: SshConnectConfig,
    items: Vec<ServerCopyItem>,
    target_side: PaneSide,
    target_dir: String,
    existing_entries: std::collections::HashMap<String, bool>,
}

impl TransferClientPool {
    fn new(config: SshConnectConfig, max_size: usize) -> Self {
        Self {
            config,
            max_size,
            retired: AtomicBool::new(false),
            state: Mutex::new(TransferClientPoolState::default()),
        }
    }

    fn retire(&self) {
        self.retired.store(true, Ordering::Release);
    }

    fn is_retired(&self) -> bool {
        self.retired.load(Ordering::Acquire)
    }
}

impl<Client> Default for TransferClientPoolState<Client> {
    fn default() -> Self {
        Self {
            total_created: 0,
            available: Vec::new(),
        }
    }
}

impl<Client> TransferClientPoolState<Client> {
    fn take_available(&mut self) -> Option<Client> {
        self.available.pop()
    }

    fn reserve_new(&mut self, max_size: usize) -> bool {
        if self.total_created >= max_size {
            return false;
        }
        self.total_created += 1;
        true
    }

    fn discard_one(&mut self) {
        self.total_created = self.total_created.saturating_sub(1);
    }

    fn return_client(&mut self, client: Client, retired: bool) -> Option<Client> {
        if retired {
            self.discard_one();
            Some(client)
        } else {
            self.available.push(client);
            None
        }
    }

    fn drain_available(&mut self) -> Vec<Client> {
        self.total_created = self.total_created.saturating_sub(self.available.len());
        std::mem::take(&mut self.available)
    }
}

impl TransferQueue {
    fn new(max_concurrent: usize) -> Self {
        Self {
            tasks: Vec::new(),
            pending: VecDeque::new(),
            max_concurrent,
            admission: TransferAdmission::Open,
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

    fn enqueue(&mut self, task: TransferTask) -> bool {
        if matches!(self.admission, TransferAdmission::Frozen) {
            return false;
        }
        self.pending.push_back(task.id);
        self.tasks.push(task);
        true
    }

    fn track_external(&mut self, task: TransferTask) -> bool {
        if matches!(self.admission, TransferAdmission::Frozen) {
            return false;
        }
        debug_assert!(task.external_transfer_id.is_some());
        self.tasks.push(task);
        true
    }

    fn untrack_external(&mut self, transfer_id: SftpTransferId) -> bool {
        let Some(index) = self
            .tasks
            .iter()
            .position(|task| task.external_transfer_id == Some(transfer_id))
        else {
            return false;
        };
        self.tasks.remove(index);
        true
    }

    fn freeze_admission(&mut self) {
        self.admission = TransferAdmission::Frozen;
    }

    fn cancel_all(&mut self) {
        self.pending.clear();
        for task in &mut self.tasks {
            if task.external_transfer_id.is_some() {
                continue;
            }
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

    fn bottom_visible_tasks(&self) -> Vec<TransferTask> {
        self.active_tasks()
    }
}

async fn acquire_transfer_client(
    pool: Arc<TransferClientPool>,
) -> anyhow::Result<Arc<Mutex<RusshSftpClient>>> {
    if pool.is_retired() {
        return Err(anyhow::anyhow!("Transfer client pool retired"));
    }

    let config = {
        let mut state = pool.state.lock().await;
        if pool.is_retired() {
            return Err(anyhow::anyhow!("Transfer client pool retired"));
        }
        if let Some(client) = state.take_available() {
            return Ok(client);
        }

        if !state.reserve_new(pool.max_size) {
            return Err(anyhow::anyhow!("Transfer client pool exhausted"));
        }

        pool.config.clone()
    };

    match RusshSftpClient::connect(config).await {
        Ok(client) => {
            let client = Arc::new(Mutex::new(client));
            let disconnect = {
                let mut state = pool.state.lock().await;
                if pool.is_retired() {
                    state.discard_one();
                    true
                } else {
                    false
                }
            };
            if disconnect {
                disconnect_sftp_client(client).await;
                Err(anyhow::anyhow!("Transfer client pool retired"))
            } else {
                Ok(client)
            }
        }
        Err(error) => {
            let mut state = pool.state.lock().await;
            state.discard_one();
            Err(error)
        }
    }
}

async fn release_transfer_client(
    pool: Arc<TransferClientPool>,
    client: Arc<Mutex<RusshSftpClient>>,
) {
    let disconnect = {
        let mut state = pool.state.lock().await;
        state.return_client(client, pool.is_retired())
    };
    if let Some(client) = disconnect {
        disconnect_sftp_client(client).await;
    }
}

async fn bounded_disconnect<F>(timeout: Duration, disconnect: F) -> BoundedDisconnectOutcome
where
    F: std::future::Future<Output = anyhow::Result<()>>,
{
    match tokio::time::timeout(timeout, disconnect).await {
        Ok(Ok(())) => BoundedDisconnectOutcome::Disconnected,
        Ok(Err(error)) => BoundedDisconnectOutcome::Failed(error),
        Err(_) => BoundedDisconnectOutcome::TimedOut,
    }
}

pub(crate) async fn disconnect_sftp_client(client: Arc<Mutex<RusshSftpClient>>) {
    let disconnect = async move {
        let mut client = client.lock().await;
        client.disconnect().await
    };
    match bounded_disconnect(SFTP_DISCONNECT_TIMEOUT, disconnect).await {
        BoundedDisconnectOutcome::Disconnected => {}
        BoundedDisconnectOutcome::Failed(error) => {
            tracing::error!("Failed to disconnect SFTP client: {}", error);
        }
        BoundedDisconnectOutcome::TimedOut => {
            tracing::warn!(
                timeout_ms = SFTP_DISCONNECT_TIMEOUT.as_millis(),
                "Timed out disconnecting SFTP client"
            );
        }
    }
}

async fn disconnect_transfer_pool(pool: Arc<TransferClientPool>) {
    pool.retire();
    let clients = {
        let mut state = pool.state.lock().await;
        state.drain_available()
    };

    for client in clients {
        disconnect_sftp_client(client).await;
    }
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

#[cfg(unix)]
fn local_file_owner(metadata: &std::fs::Metadata) -> Option<String> {
    use std::os::unix::fs::MetadataExt;

    static USER_NAMES: std::sync::OnceLock<std::collections::BTreeMap<u32, String>> =
        std::sync::OnceLock::new();

    let uid = metadata.uid();
    let user_names = USER_NAMES.get_or_init(|| {
        sysinfo::Users::new_with_refreshed_list()
            .list()
            .iter()
            .map(|user| (**user.id(), user.name().to_owned()))
            .collect()
    });

    Some(
        user_names
            .get(&uid)
            .cloned()
            .unwrap_or_else(|| uid.to_string()),
    )
}

#[cfg(not(unix))]
fn local_file_owner(_metadata: &std::fs::Metadata) -> Option<String> {
    None
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
    let mut exit_status = None;

    while let Some(event) = channel.recv().await {
        match event {
            ChannelEvent::Data(data) => stdout.extend(data),
            ChannelEvent::ExtendedData { data, .. } => stderr.extend(data),
            ChannelEvent::ExitStatus(status) => exit_status = Some(status),
            ChannelEvent::ExitSignal {
                signal_name,
                error_message,
            } => {
                let _ = channel.close().await;
                anyhow::bail!("remote command failed with signal {signal_name}: {error_message}");
            }
            ChannelEvent::Eof | ChannelEvent::Close => break,
        }
    }

    let _ = channel.close().await;
    let Some(exit_status) = exit_status else {
        anyhow::bail!("remote command closed without reporting an exit status");
    };
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ConnectionGeneration(u64);

impl ConnectionGeneration {
    fn advance(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(1).max(1);
        self.0
    }

    fn current(self) -> u64 {
        self.0
    }

    fn is_current(self, generation: u64) -> bool {
        generation != 0 && self.0 == generation
    }
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

fn breadcrumb_item_min_width(label: &str) -> f32 {
    if label == "/" { 0. } else { 35. }
}

fn breadcrumb_item(label: impl Into<SharedString>) -> BreadcrumbItem {
    let label: SharedString = label.into();
    let min_width = breadcrumb_item_min_width(label.as_ref());

    BreadcrumbItem::new(label)
        .flex_shrink_1()
        .min_w(px(min_width))
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

fn server_copy_conflict_flags(
    items: &[ServerCopyItem],
    existing_entries: &std::collections::HashMap<String, bool>,
) -> (usize, bool, bool) {
    let mut conflict_count = 0;
    let mut has_mergeable_directory = false;
    let mut has_type_mismatch = false;

    for item in items {
        let Some(name) = item.target_path.rsplit('/').next() else {
            continue;
        };
        let Some(target_is_dir) = existing_entries.get(name) else {
            continue;
        };
        conflict_count += 1;
        if item.is_dir == *target_is_dir {
            has_mergeable_directory |= item.is_dir;
        } else {
            has_type_mismatch = true;
        }
    }

    (conflict_count, has_mergeable_directory, has_type_mismatch)
}

fn upload_directory_conflict_policy(
    transfer: &PendingTransfer,
    selected_policy: DirectoryConflictPolicy,
) -> DirectoryConflictPolicy {
    if transfer.is_dir && transfer.has_conflict {
        selected_policy
    } else {
        DirectoryConflictPolicy::Merge
    }
}

fn prepare_global_upload(
    transfer: &PendingTransfer,
    selected_policy: DirectoryConflictPolicy,
    context: &SftpUploadContext,
) -> PreparedGlobalUpload {
    let directory_conflict_policy = upload_directory_conflict_policy(transfer, selected_policy);
    let title_prefix = if transfer.is_dir {
        t!("File.upload_folder")
    } else {
        t!("File.upload_file")
    };
    let request = SftpUploadRequest {
        connection: context.connection.clone(),
        connection_source: context.connection_source.clone(),
        local_path: transfer.local_path.clone(),
        remote_path: transfer.remote_path.clone(),
        is_dir: transfer.is_dir,
        directory_conflict_policy,
        display_name: transfer.name.clone(),
        title: format!("{title_prefix} · {}", transfer.name).into(),
        task_group: Some(context.task_group.clone()),
        task_key: Some(upload_task_key(
            &context.connection,
            &transfer.local_path,
            &transfer.remote_path,
        )),
    };
    let operation = TransferOperation::Upload {
        local_path: transfer.local_path.clone(),
        remote_path: transfer.remote_path.clone(),
        is_dir: transfer.is_dir,
        directory_conflict_policy,
        remote_dir: remote_path_parent(&transfer.remote_path),
    };
    PreparedGlobalUpload { request, operation }
}

fn prepare_global_download(
    transfer: &PendingTransfer,
    context: &SftpUploadContext,
) -> PreparedGlobalDownload {
    let request = SftpDownloadRequest {
        connection: context.connection.clone(),
        connection_source: context.connection_source.clone(),
        remote_path: transfer.remote_path.clone(),
        local_path: transfer.local_path.clone(),
        is_dir: transfer.is_dir,
        display_name: transfer.name.clone(),
        title: format!("{} · {}", t!("Common.download"), transfer.name).into(),
        task_group: Some(context.task_group.clone()),
        task_key: Some(download_task_key(
            &context.connection,
            &transfer.remote_path,
            &transfer.local_path,
        )),
    };
    let local_dir = transfer
        .local_path
        .parent()
        .map(|path| path.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let operation = TransferOperation::Download {
        remote_path: transfer.remote_path.clone(),
        local_dir,
    };
    PreparedGlobalDownload { request, operation }
}

fn prepare_global_remote_delete(
    entries: &[FileItem],
    remote_dir: &str,
    context: &SftpUploadContext,
) -> PreparedGlobalRemoteDelete {
    let remote_entries = entries
        .iter()
        .map(|entry| SftpRemoteDeleteEntry {
            remote_path: join_remote_path(remote_dir, &entry.name),
            is_dir: entry.is_dir,
        })
        .collect::<Vec<_>>();
    let display_name = remote_delete_display_name(entries);
    let request = SftpDeleteRemoteRequest {
        connection: context.connection.clone(),
        connection_source: context.connection_source.clone(),
        entries: remote_entries,
        remote_dir: remote_dir.to_string(),
        display_name: display_name.clone(),
        title: format!("{} · {display_name}", t!("Common.delete")).into(),
        task_group: Some(context.task_group.clone()),
        task_key: None,
    };
    let request = SftpDeleteRemoteRequest {
        task_key: Some(delete_remote_task_key(
            &context.connection,
            remote_dir,
            &request.entries,
        )),
        ..request
    };
    let operation = TransferOperation::DeleteRemote {
        entries: entries.to_vec(),
        remote_dir: remote_dir.to_string(),
    };
    PreparedGlobalRemoteDelete { request, operation }
}

fn commit_external_transfer(
    queue: &mut TransferQueue,
    next_task_id: &mut usize,
    executor: &Entity<SftpTransferExecutor>,
    reservation: SftpTransferReservation,
    operation: TransferOperation,
    cx: &mut Context<SftpView>,
) -> Option<SftpTransferSnapshot> {
    let transfer_id = reservation.id();
    let task = TransferTask {
        id: *next_task_id,
        external_transfer_id: Some(transfer_id),
        operation,
        state: TransferTaskState::Pending,
        shared_progress: new_shared_progress(),
        error: None,
        background_task: None,
    };

    if !queue.track_external(task) {
        return None;
    }

    let committed_id =
        match executor.update(cx, |executor, cx| executor.commit_reserved(reservation, cx)) {
            Ok(id) => id,
            Err(_) => {
                let removed = queue.untrack_external(transfer_id);
                debug_assert!(removed);
                return None;
            }
        };
    debug_assert_eq!(committed_id, transfer_id);
    *next_task_id += 1;

    Some(
        executor
            .read(cx)
            .snapshot(committed_id)
            .expect("committed SFTP transfer must have an initial snapshot"),
    )
}

fn remote_delete_display_name(entries: &[FileItem]) -> String {
    match entries {
        [entry] => entry.name.clone(),
        _ => t!("Delete.delete_n_items", count = entries.len()).to_string(),
    }
}

fn transfer_task_state_from_global(state: &SftpTransferState) -> TransferTaskState {
    match state {
        SftpTransferState::Queued => TransferTaskState::Pending,
        SftpTransferState::Running | SftpTransferState::Cancelling => TransferTaskState::Running,
        SftpTransferState::Succeeded => TransferTaskState::Completed,
        SftpTransferState::Failed => TransferTaskState::Failed,
        SftpTransferState::Cancelled => TransferTaskState::Cancelled,
    }
}

fn apply_global_transfer_snapshot(task: &mut TransferTask, snapshot: &SftpTransferSnapshot) {
    task.state = transfer_task_state_from_global(&snapshot.state);
    task.error = snapshot.error.clone();
    task.shared_progress
        .transferred
        .store(snapshot.transferred, Ordering::Relaxed);
    task.shared_progress
        .total
        .store(snapshot.total.unwrap_or(0), Ordering::Relaxed);
    task.shared_progress
        .speed
        .store(snapshot.speed.to_bits(), Ordering::Relaxed);
    task.shared_progress.scanning.store(
        snapshot.total.is_none() && snapshot.state.is_active(),
        Ordering::Relaxed,
    );
    if let Ok(mut current_file) = task.shared_progress.current_file.write() {
        *current_file = snapshot.current_file.clone();
    }
    task.shared_progress
        .current_file_transferred
        .store(snapshot.transferred, Ordering::Relaxed);
    task.shared_progress
        .current_file_total
        .store(snapshot.total.unwrap_or(0), Ordering::Relaxed);
}

fn transfer_event_id(event: &SftpTransferEvent) -> SftpTransferId {
    match event {
        SftpTransferEvent::Added(id)
        | SftpTransferEvent::Updated(id)
        | SftpTransferEvent::Finished(id) => *id,
    }
}

fn active_transfer_state(state: &TransferTaskState) -> bool {
    matches!(
        state,
        TransferTaskState::Pending | TransferTaskState::Running
    )
}

fn active_external_transfer_ids(tasks: &[TransferTask]) -> Vec<SftpTransferId> {
    tasks
        .iter()
        .filter(|task| active_transfer_state(&task.state))
        .filter_map(|task| task.external_transfer_id)
        .collect()
}

fn should_refresh_transfer_target(
    was_active: bool,
    event: &SftpTransferEvent,
    state: &SftpTransferState,
) -> bool {
    was_active
        && matches!(event, SftpTransferEvent::Finished(_))
        && matches!(
            state,
            SftpTransferState::Succeeded | SftpTransferState::Failed | SftpTransferState::Cancelled
        )
}

fn reconcile_global_transfer_snapshot(
    task: &mut TransferTask,
    snapshot: &SftpTransferSnapshot,
) -> Option<TransferRefreshTarget> {
    let refresh_target = transfer_refresh_target(&task.operation, snapshot.operation)?;
    let was_active = active_transfer_state(&task.state);
    apply_global_transfer_snapshot(task, snapshot);
    let finished_now = was_active && !active_transfer_state(&task.state);
    let refreshable = matches!(
        snapshot.state,
        SftpTransferState::Succeeded | SftpTransferState::Failed | SftpTransferState::Cancelled
    );
    (finished_now && refreshable).then_some(refresh_target)
}

fn transfer_refresh_target(
    operation: &TransferOperation,
    snapshot_operation: SftpTransferOperation,
) -> Option<TransferRefreshTarget> {
    match (operation, snapshot_operation) {
        (TransferOperation::Upload { remote_dir, .. }, SftpTransferOperation::Upload) => {
            Some(TransferRefreshTarget::Remote(remote_dir.clone()))
        }
        (TransferOperation::Download { local_dir, .. }, SftpTransferOperation::Download) => {
            Some(TransferRefreshTarget::Local(local_dir.clone()))
        }
        (
            TransferOperation::DeleteRemote { remote_dir, .. },
            SftpTransferOperation::DeleteRemote,
        ) => Some(TransferRefreshTarget::Remote(remote_dir.clone())),
        _ => None,
    }
}

fn mark_server_copy_directory_replacements(
    items: &mut [ServerCopyItem],
    existing_entries: &std::collections::HashMap<String, bool>,
) {
    for item in items {
        let target_is_existing_directory = item
            .target_path
            .rsplit('/')
            .next()
            .and_then(|name| existing_entries.get(name).copied())
            == Some(true);
        if item.is_dir && target_is_existing_directory {
            item.directory_conflict_policy = DirectoryConflictPolicy::Replace;
        }
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

fn upload_conflict_prompt_labels(position: (usize, usize)) -> FileConflictPromptLabels {
    FileConflictPromptLabels {
        exists: t!("Conflict.item_exists").to_string().into(),
        progress: t!(
            "Conflict.progress",
            current = position.0,
            total = position.1
        )
        .to_string()
        .into(),
        choose_action: t!("Conflict.choose_action").to_string().into(),
        apply_all: t!("Conflict.apply_all").to_string().into(),
        skip: t!("Conflict.skip").to_string().into(),
        keep_both: t!("Conflict.keep_both").to_string().into(),
        merge: t!("Conflict.merge").to_string().into(),
        overwrite: t!("Conflict.overwrite").to_string().into(),
    }
}

fn resolve_upload_conflicts(
    session: &UploadConflictSession,
    decision: (FileConflictChoice, bool),
) -> Vec<PendingUploadDecision> {
    let (choice, apply_all) = decision;
    let mut existing_names = session.existing_names.borrow_mut();
    let mut resolver = session.resolver.borrow_mut();
    resolver.resolve_current(
        apply_all,
        |current, candidate| current.transfer.is_dir == candidate.transfer.is_dir,
        |transfer| resolve_upload_conflict(transfer, choice, &mut existing_names),
    );
    resolver.take_ready().unwrap_or_default()
}

fn resolve_upload_conflict(
    mut decision: PendingUploadDecision,
    choice: FileConflictChoice,
    existing_names: &mut HashSet<String>,
) -> Option<PendingUploadDecision> {
    match choice {
        FileConflictChoice::Skip => None,
        FileConflictChoice::KeepBoth => {
            let transfer = &mut decision.transfer;
            let new_name = generate_unique_name(&transfer.name, existing_names);
            existing_names.insert(new_name.clone());
            transfer.remote_path =
                join_remote_path(&remote_path_parent(&transfer.remote_path), &new_name);
            transfer.name = new_name;
            transfer.has_conflict = false;
            Some(decision)
        }
        FileConflictChoice::Merge => {
            decision.directory_conflict_policy = DirectoryConflictPolicy::Merge;
            Some(decision)
        }
        FileConflictChoice::Overwrite => {
            if decision.transfer.is_dir {
                decision.directory_conflict_policy = DirectoryConflictPolicy::Replace;
            }
            Some(decision)
        }
    }
}

pub struct SftpView {
    window_handle: AnyWindowHandle,
    connection_state: ConnectionState,
    close_state: CloseState,
    sftp_config: SshConnectConfig,
    /// 连接成功后进入的 SFTP 初始目录（配置的 `sftp_default_directory`）；`None` 时回退到服务器登录目录。
    sftp_initial_directory: Option<String>,
    credential_inputs: Option<SftpCredentialInputs>,
    sftp_client: Option<Arc<Mutex<RusshSftpClient>>>,
    /// 当前主 SFTP 连接尝试的代次；迟到的异步结果不能覆盖更新的连接。
    connection_generation: ConnectionGeneration,
    /// 左侧远程 SFTP 连接尝试的代次；即使切走后又选回同一连接，也不能应用旧结果。
    left_connection_generation: ConnectionGeneration,

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
    left_remote: Option<LeftRemoteEndpoint>,
    file_clipboard: Option<FileClipboard>,

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

    upload_executor: Entity<SftpTransferExecutor>,
    upload_connection_identity: SftpConnectionIdentity,
    transfer_queue: TransferQueue,
    next_task_id: usize,
    direct_copy_prompt_lock: Arc<tokio::sync::Mutex<()>>,
    direct_copy_prompt: Option<direct_copy_prompt::ActiveDirectCopyPrompt>,
    transfer_client_pool: Arc<TransferClientPool>,
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
        let resolved = ssh_config::resolve_ssh_connection(&conn)
            .expect("StoredConnection should contain valid SSH params");
        let config = resolved.config;
        let credential_prompt_policy = resolved.credential_prompt_policy;
        let sftp_initial_directory = resolved.sftp_initial_directory;

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
        let local_path_input = cx
            .new(|cx| InputState::new(window, cx).placeholder(t!("Placeholder.path").to_string()));
        let remote_path_input = cx
            .new(|cx| InputState::new(window, cx).placeholder(t!("Placeholder.path").to_string()));
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
        let upload_executor = sftp_transfer::global(cx);
        let upload_connection_identity =
            SftpConnectionIdentity::from_stored(&conn).unwrap_or_else(|| {
                upload_executor.update(cx, |executor, _| executor.allocate_runtime_connection())
            });

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
                    if this.left_remote.is_some() {
                        this.on_left_remote_item_double_click(name.clone(), *is_dir, cx);
                    } else {
                        this.on_local_item_double_click(name.clone(), *is_dir, cx);
                    }
                }
                FileListPanelEvent::PathChanged(path) => {
                    if this.left_remote.is_some() {
                        this.navigate_left_remote_to(path.clone(), cx);
                    } else {
                        this.on_local_path_changed(path.clone(), cx);
                    }
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
        subscriptions.push(cx.subscribe(
            &upload_executor,
            |this, executor, event: &SftpTransferEvent, cx| {
                this.handle_transfer_event(&executor, event, cx);
            },
        ));

        let credential_inputs = credential_prompt_policy.requires_prompt().then(|| {
            let username = credential_prompt_policy.username.then(|| {
                cx.new(|cx| {
                    InputState::new(window, cx).placeholder(t!("Credentials.username").to_string())
                })
            });
            let password = credential_prompt_policy.password.then(|| {
                cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder(t!("Credentials.password").to_string())
                        .masked(true)
                })
            });

            for input in username.iter().chain(password.iter()) {
                subscriptions.push(cx.subscribe_in(
                    input,
                    window,
                    |this, _, event: &InputEvent, window, cx| {
                        if matches!(
                            event,
                            InputEvent::PressEnter {
                                secondary: false,
                                ..
                            }
                        ) {
                            this.submit_credentials(window, cx);
                        }
                    },
                ));
            }

            SftpCredentialInputs {
                policy: credential_prompt_policy,
                username,
                password,
                error: None,
            }
        });

        let transfer_client_pool = Arc::new(TransferClientPool::new(
            config.clone(),
            MAX_CONCURRENT_TRANSFERS,
        ));

        let mut view = Self {
            window_handle: window.window_handle(),
            connection_state: if credential_inputs.is_some() {
                ConnectionState::AwaitingCredentials
            } else {
                ConnectionState::Disconnected { error: None }
            },
            close_state: CloseState::Open,
            sftp_config: config,
            sftp_initial_directory,
            credential_inputs,
            sftp_client: None,
            connection_generation: ConnectionGeneration::default(),
            left_connection_generation: ConnectionGeneration::default(),
            stored_connection: conn.clone(),
            local_current_path: local_current_path.clone(),
            remote_current_path: ".".to_string(),
            local_history: vec![local_current_path.clone()],
            local_history_index: 0,
            remote_history: vec![".".to_string()],
            remote_history_index: 0,
            local_panel,
            remote_panel,
            left_remote: None,
            file_clipboard: None,
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
            upload_executor,
            upload_connection_identity,
            transfer_queue: TransferQueue::new(MAX_CONCURRENT_TRANSFERS),
            next_task_id: 0,
            direct_copy_prompt_lock: Arc::new(tokio::sync::Mutex::new(())),
            direct_copy_prompt: None,
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
        if let Some(input) = view
            .credential_inputs
            .as_ref()
            .and_then(|inputs| inputs.username.as_ref().or(inputs.password.as_ref()))
            .cloned()
        {
            input.update(cx, |state, cx| state.focus(window, cx));
        } else {
            view.connect(cx);
        }

        view
    }

    fn next_connection_generation(&mut self) -> u64 {
        self.connection_generation.advance()
    }

    fn is_current_connection_generation(&self, generation: u64) -> bool {
        self.connection_generation.is_current(generation)
    }

    fn next_left_connection_generation(&mut self) -> u64 {
        self.left_connection_generation.advance()
    }

    fn is_current_left_connection_generation(&self, generation: u64) -> bool {
        self.left_connection_generation.is_current(generation)
    }

    fn connect(&mut self, cx: &mut Context<Self>) {
        if self.close_state.is_closing() {
            return;
        }
        let generation = self.next_connection_generation();
        self.set_connection_state(ConnectionState::Connecting, cx);
        let config = self.sftp_config.clone();
        let window_handle = self.window_handle.clone();
        let initial_directory = self.sftp_initial_directory.clone();

        tracing::info!(
            "Connecting to SFTP server: {}@{}",
            config.username,
            config.host
        );

        let task = Tokio::spawn(cx, async move {
            let mut client = RusshSftpClient::connect(config).await?;
            // 连接成功后确定远端工作目录：优先使用配置的初始目录，否则使用服务器登录目录
            let real_path = match initial_directory {
                Some(dir) => match client.realpath(&dir).await.ok() {
                    Some(path) => Some(path),
                    None => {
                        tracing::warn!(
                            "Configured SFTP initial directory could not be resolved: {dir}"
                        );
                        client.realpath(".").await.ok()
                    }
                },
                None => client.realpath(".").await.ok(),
            };
            Ok::<_, anyhow::Error>((client, real_path))
        });

        cx.spawn(async move |this, cx| match task.await {
            Ok(Ok((client, real_path))) => {
                tracing::info!("SFTP connection established successfully");
                let client = Arc::new(Mutex::new(client));

                let installed = this
                    .update(cx, |this, cx| {
                        if this.close_state.is_closing()
                            || !this.is_current_connection_generation(generation)
                        {
                            return false;
                        }
                        this.credential_inputs = None;
                        this.sftp_client = Some(client.clone());
                        this.set_connection_state(ConnectionState::Connected, cx);
                        this.set_connection_active(true, cx);

                        // 如果成功获取了真实路径，更新远程路径和历史记录
                        if let Some(path) = real_path {
                            tracing::info!("Remote working directory: {}", path);
                            this.remote_current_path = path.clone();
                            this.remote_history = vec![path];
                            this.remote_history_index = 0;
                        }

                        this.refresh_remote_dir(cx);
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);

                if !installed {
                    let task = Tokio::spawn(cx, disconnect_sftp_client(client));
                    let _ = task.await;
                }
            }
            Ok(Err(e)) => {
                let error_msg = format!("{}", e);
                tracing::error!("SFTP connection failed: {}", error_msg);
                if let Some(request) = host_key_prompt_request(&e) {
                    let should_prompt = this
                        .update(cx, |this, cx| {
                            if this.close_state.is_closing()
                                || !this.is_current_connection_generation(generation)
                            {
                                return false;
                            }
                            this.set_connection_active(false, cx);
                            true
                        })
                        .unwrap_or(false);
                    if should_prompt {
                        let prompt_result = cx.update_window(window_handle, |_, window, cx| {
                            let _ = this.update(cx, |this, cx| {
                                this.show_host_key_prompt(
                                    HostKeyPromptTarget::Main { generation },
                                    request,
                                    window,
                                    cx,
                                );
                            });
                        });
                        if prompt_result.is_err() {
                            let _ = this.update(cx, |this, cx| {
                                if this.close_state.is_closing()
                                    || !this.is_current_connection_generation(generation)
                                {
                                    return;
                                }
                                this.set_connection_error(error_msg, cx);
                            });
                        }
                    }
                    return;
                }
                let _ = this.update(cx, |this, cx| {
                    if this.close_state.is_closing()
                        || !this.is_current_connection_generation(generation)
                    {
                        return;
                    }
                    this.set_connection_error(error_msg, cx);
                });
            }
            Err(e) => {
                let error_msg = format!("Task error: {}", e);
                tracing::error!("SFTP connection task error: {}", error_msg);
                let _ = this.update(cx, |this, cx| {
                    if this.close_state.is_closing()
                        || !this.is_current_connection_generation(generation)
                    {
                        return;
                    }
                    this.set_connection_error(error_msg, cx);
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

    /// Update the connection state, notifying the owning tab bar whenever it
    /// actually transitions so the status badge refreshes.
    fn set_connection_state(&mut self, state: ConnectionState, cx: &mut Context<Self>) {
        if self.connection_state == state {
            return;
        }
        self.connection_state = state;
        cx.emit(TabContentEvent::StateChanged);
        cx.notify();
    }

    fn set_connection_error(&mut self, error: String, cx: &mut Context<Self>) {
        let state = if let Some(inputs) = self.credential_inputs.as_mut() {
            inputs.error = Some(error);
            ConnectionState::AwaitingCredentials
        } else {
            ConnectionState::Disconnected { error: Some(error) }
        };
        self.set_connection_state(state, cx);
        self.set_connection_active(false, cx);
        cx.notify();
    }

    fn reconnect(&mut self, cx: &mut Context<Self>) {
        if self.close_state.is_closing() {
            return;
        }
        let old_client = self.sftp_client.take();
        self.set_connection_active(false, cx);
        if let Some(old_client) = old_client {
            Tokio::spawn(cx, disconnect_sftp_client(old_client)).detach();
        }
        self.remote_loading = false;
        self.replace_transfer_client_pool(cx);
        self.connect(cx);
    }

    fn replace_transfer_client_pool(&mut self, cx: &mut Context<Self>) {
        let retired_pool = std::mem::replace(
            &mut self.transfer_client_pool,
            Arc::new(TransferClientPool::new(
                self.sftp_config.clone(),
                MAX_CONCURRENT_TRANSFERS,
            )),
        );
        retired_pool.retire();
        Tokio::spawn(cx, disconnect_transfer_pool(retired_pool)).detach();
    }

    fn submit_credentials(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(inputs) = self.credential_inputs.as_ref() else {
            return;
        };
        let username = inputs
            .username
            .as_ref()
            .map(|input| input.read(cx).text().to_string());
        let password = inputs
            .password
            .as_ref()
            .map(|input| input.read(cx).text().to_string());

        match ssh_config::ssh_config_with_runtime_credentials(
            &self.sftp_config,
            username.as_deref(),
            password.as_deref(),
        ) {
            Ok(config) => {
                self.sftp_config = config;
                if let Some(inputs) = self.credential_inputs.as_mut() {
                    inputs.error = None;
                }
                self.replace_transfer_client_pool(cx);
                self.connect(cx);
            }
            Err(error) => {
                let error = if inputs.policy.username
                    && username
                        .as_deref()
                        .is_some_and(|value| value.trim().is_empty())
                {
                    t!("Credentials.username_required").to_string()
                } else if inputs.policy.password && password.as_deref().is_some_and(str::is_empty) {
                    t!("Credentials.password_required").to_string()
                } else {
                    error.to_string()
                };
                if let Some(inputs) = self.credential_inputs.as_mut() {
                    inputs.error = Some(error);
                }
                if let Some(input) =
                    self.credential_inputs.as_ref().and_then(|inputs| {
                        if inputs.username.as_ref().is_some_and(|input| {
                            input.read(cx).text().to_string().trim().is_empty()
                        }) {
                            inputs.username.clone()
                        } else {
                            inputs.password.clone()
                        }
                    })
                {
                    input.update(cx, |state, cx| state.focus(window, cx));
                }
                cx.notify();
            }
        }
    }

    fn refresh_local_dir(&mut self, cx: &mut Context<Self>) {
        self.refresh_local_dir_inner(None, cx);
    }

    fn refresh_local_dir_with_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_local_dir_inner(Some(window), cx);
    }

    fn refresh_local_dir_inner(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        if self.close_state.is_closing() {
            return;
        }
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
                            owner: local_file_owner(&metadata),
                            directory_size: DirectorySizeState::Unknown,
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
        if self.close_state.is_closing() {
            return;
        }
        let Some(client) = self.sftp_client.clone() else {
            tracing::warn!("Cannot refresh remote dir: client not connected");
            return;
        };

        let path = self.remote_current_path.clone();
        let generation = self.connection_generation.current();
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
                        .map(|e| {
                            let owner = e.owner_display();
                            FileItem {
                                name: e.name,
                                size: e.size,
                                modified: e.modified,
                                is_dir: e.is_dir,
                                permissions: format_permissions(e.permissions, e.is_dir),
                                owner,
                                directory_size: DirectorySizeState::Unknown,
                            }
                        })
                        .collect();
                    let _ = view.update(cx, |this, cx| {
                        if !this.close_state.is_closing()
                            && this.is_current_connection_generation(generation)
                            && should_apply_remote_listing(&this.remote_current_path, &listed_path)
                        {
                            let _ = remote_panel.update(cx, |panel, cx| {
                                panel.set_items(items, cx);
                            });
                        }
                    });
                }
                Ok(Err(e)) => {
                    let _ = view.update(cx, |this, _cx| {
                        if !this.close_state.is_closing()
                            && this.is_current_connection_generation(generation)
                            && should_apply_remote_listing(&this.remote_current_path, &listed_path)
                        {
                            tracing::error!("Failed to list remote directory: {}", e);
                        }
                    });
                }
                Err(e) => {
                    let _ = view.update(cx, |this, _cx| {
                        if !this.close_state.is_closing()
                            && this.is_current_connection_generation(generation)
                            && should_apply_remote_listing(&this.remote_current_path, &listed_path)
                        {
                            tracing::error!("Task error: {}", e);
                        }
                    });
                }
            }
            let _ = view.update(cx, |this, cx| {
                if !this.close_state.is_closing()
                    && this.is_current_connection_generation(generation)
                    && should_apply_remote_listing(&this.remote_current_path, &listed_path)
                {
                    this.remote_loading = false;
                    cx.notify();
                }
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
            self.open_remote_file(full_path, window, cx);
        }
        cx.notify();
    }

    fn open_remote_file(&self, full_path: String, window: &mut Window, cx: &mut Context<Self>) {
        if image_format_for_path(&full_path).is_some() {
            let Some(client) = self.sftp_client.clone() else {
                window.push_notification(
                    Notification::error(t!("Error.sftp_not_connected").to_string()),
                    cx,
                );
                return;
            };
            open_remote_image_preview(full_path, client, window, cx);
        } else {
            self.open_remote_editor(full_path, window, cx);
        }
    }

    fn open_remote_editor(&self, full_path: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(client) = self.sftp_client.clone() else {
            window.push_notification(
                Notification::error(t!("Error.sftp_not_connected").to_string()),
                cx,
            );
            return;
        };

        open_remote_file_editor(full_path, client, self.remote_mutation_callback(cx), cx);
    }

    fn open_remote_external_editor(
        &self,
        selection: (String, String),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (full_path, editor_key) = selection;
        let Some(client) = self.sftp_client.clone() else {
            window.push_notification(
                Notification::error(t!("Error.sftp_not_connected").to_string()),
                cx,
            );
            return;
        };
        open_remote_file_external_editor(
            ExternalEditorOpenRequest {
                remote_path: full_path,
                editor_key,
                client,
                on_remote_changed: self.remote_mutation_callback(cx),
            },
            window,
            cx,
        );
    }

    fn remote_mutation_callback(&self, cx: &Context<Self>) -> RemoteMutationCallback {
        let view = cx.entity().downgrade();
        RemoteMutationCallback::new(move |cx| {
            let _ = view.update(cx, |this, cx| this.refresh_remote_dir(cx));
        })
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
        let path = self.left_remote.as_ref().map_or_else(
            || self.local_current_path.to_string_lossy().to_string(),
            |endpoint| endpoint.current_path.clone(),
        );
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
        if self.left_remote.is_some() {
            if !new_path.is_empty() {
                self.navigate_left_remote_to(new_path, cx);
            }
            return;
        }
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
                    dialog
                        .title(t!("Dialog.error").to_string())
                        .child(error_msg.clone())
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
        if self.close_state.is_closing() {
            return;
        }
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
                        let _ = view.update_in(cx, |this, window, cx| {
                            if this.close_state.is_closing() {
                                return;
                            }
                            window.push_notification(Notification::error(error_msg.clone()), cx);
                        });
                        return;
                    }
                    Err(e) => {
                        tracing::error!("Task error: {}", e);
                        let error_msg = t!("Error.read_dir_failed", error = e).to_string();
                        let _ = view.update_in(cx, |this, window, cx| {
                            if this.close_state.is_closing() {
                                return;
                            }
                            window.push_notification(Notification::error(error_msg.clone()), cx);
                        });
                        return;
                    }
                };

                let _ = view.update_in(cx, |this, window, cx| {
                    if this.close_state.is_closing() {
                        return;
                    }
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
                        this.show_upload_conflict_dialog(
                            pending_transfers,
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
        if self.close_state.is_closing() || paths.is_empty() {
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
                        let _ = view.update_in(cx, |this, window, cx| {
                            if this.close_state.is_closing() {
                                return;
                            }
                            window.push_notification(Notification::error(error_msg.clone()), cx);
                        });
                        return;
                    }
                    Err(e) => {
                        tracing::error!("Task error: {}", e);
                        let error_msg = t!("Error.read_dir_failed", error = e).to_string();
                        let _ = view.update_in(cx, |this, window, cx| {
                            if this.close_state.is_closing() {
                                return;
                            }
                            window.push_notification(Notification::error(error_msg.clone()), cx);
                        });
                        return;
                    }
                };

                let _ = view.update_in(cx, |this, window, cx| {
                    if this.close_state.is_closing() {
                        return;
                    }
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
                        this.show_upload_conflict_dialog(
                            pending_transfers,
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

    fn paste_upload_from_clipboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.close_state.is_closing() {
            return;
        }
        let Some(client) = self.sftp_client.clone() else {
            window.push_notification(
                Notification::warning(
                    t!("Notification.clipboard_upload_not_connected").to_string(),
                )
                .autohide(true),
                cx,
            );
            return;
        };

        let Some(item) = cx.read_from_clipboard() else {
            return;
        };

        let upload_paths = match clipboard_upload_paths(&item) {
            Ok(upload_paths) => upload_paths.paths,
            Err(error) => {
                window.push_notification(
                    Notification::error(
                        t!("Notification.clipboard_read_failed", error = error).to_string(),
                    )
                    .autohide(true),
                    cx,
                );
                return;
            }
        };

        if upload_paths.is_empty() {
            window.push_notification(
                Notification::info(t!("Notification.clipboard_no_uploads").to_string())
                    .autohide(true),
                cx,
            );
            return;
        }

        self.upload_paths_to_remote(
            upload_paths,
            self.remote_current_path.clone(),
            client,
            window,
            cx,
        );
    }

    fn show_download_conflict_dialog(
        &mut self,
        conflict_names: Vec<String>,
        pending_transfers: Vec<PendingTransfer>,
        existing_names: std::collections::HashSet<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.close_state.is_closing() {
            return;
        }
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

        window.open_dialog(cx, move |dialog, _window, cx| {
            let view_overwrite = view.clone();
            let view_keep = view.clone();
            let view_skip = view.clone();

            let transfers_overwrite = pending_transfers.clone();
            let transfers_keep = pending_transfers.clone();
            let transfers_skip = pending_transfers.clone();

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
                .footer({
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
                                            if this.close_state.is_closing() {
                                                return;
                                            }
                                            this.execute_downloads(transfers, cx);
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
                                        false,
                                        existing.clone(),
                                    );
                                    view.update(cx, |this, cx| {
                                        if this.close_state.is_closing() {
                                            return;
                                        }
                                        this.execute_downloads(transfers, cx);
                                    });
                                }
                            })
                            .into_any_element(),
                    ];

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
                                        if this.close_state.is_closing() {
                                            return;
                                        }
                                        this.execute_downloads(transfers.clone(), cx);
                                    });
                                }
                            })
                            .into_any_element(),
                    );

                    DialogFooter::new().children(buttons)
                })
                .overlay_closable(false)
                .close_button(true)
        });
    }

    fn show_upload_conflict_dialog(
        &mut self,
        pending_transfers: Vec<PendingTransfer>,
        existing_names: HashSet<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut existing_names = existing_names;
        existing_names.extend(pending_transfers.iter().map(|item| item.name.clone()));
        let decisions = pending_transfers
            .into_iter()
            .map(|transfer| PendingUploadDecision {
                transfer,
                directory_conflict_policy: DirectoryConflictPolicy::Merge,
            })
            .collect();
        let resolver = UploadConflictResolver::new(decisions, |item| item.transfer.has_conflict);
        if resolver.current().is_none() {
            return;
        }
        self.show_next_upload_conflict(
            UploadConflictSession {
                connection_generation: self.connection_generation.current(),
                resolver: Rc::new(RefCell::new(resolver)),
                existing_names: Rc::new(RefCell::new(existing_names)),
            },
            window,
            cx,
        );
    }

    fn show_next_upload_conflict(
        &mut self,
        session: UploadConflictSession,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (current, position) = {
            let resolver = session.resolver.borrow();
            let Some(current) = resolver.current().cloned() else {
                return;
            };
            let position = resolver.current_position().unwrap_or((1, 1));
            (current, position)
        };
        let view = cx.entity().clone();
        let apply_all = Arc::new(AtomicBool::new(false));
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let session = session.clone();
            let view = view.clone();
            dialog
                .title(t!("Dialog.file_conflict").to_string())
                .w(px(480.))
                .child(FileConflictPrompt::new(
                    FileConflictPromptSpec {
                        name: current.transfer.name.clone().into(),
                        is_directory: current.transfer.is_dir,
                        apply_all: apply_all.clone(),
                        labels: upload_conflict_prompt_labels(position),
                    },
                    move |choice, apply_all, window, cx| {
                        window.close_dialog(cx);
                        let uploads = resolve_upload_conflicts(&session, (choice, apply_all));
                        let has_next = session.resolver.borrow().current().is_some();
                        let _ = view.update(cx, |this, cx| {
                            if this.close_state.is_closing()
                                || !this
                                    .is_current_connection_generation(session.connection_generation)
                            {
                                return;
                            }
                            this.execute_upload_decisions(uploads, cx);
                            if has_next {
                                this.show_next_upload_conflict(session.clone(), window, cx);
                            }
                        });
                    },
                ))
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
            if let Some(handle) = task.background_task.as_ref() {
                handle.mark_running(cx);
            }
            self.start_transfer_task(task, cx);
        }

        self.start_progress_refresh(cx);
        cx.notify();
    }

    fn handle_transfer_event(
        &mut self,
        executor: &Entity<SftpTransferExecutor>,
        event: &SftpTransferEvent,
        cx: &mut Context<Self>,
    ) {
        let id = transfer_event_id(event);
        let Some(snapshot) = executor.read(cx).snapshot(id) else {
            return;
        };
        if snapshot.connection != self.upload_connection_identity {
            return;
        }
        let refresh_target = self.apply_transfer_event_to_mirror(event, &snapshot);
        self.refresh_transfer_target_if_visible(refresh_target, cx);
        self.schedule_transfers(cx);
        cx.notify();
    }

    fn refresh_transfer_target_if_visible(
        &mut self,
        refresh_target: Option<TransferRefreshTarget>,
        cx: &mut Context<Self>,
    ) {
        if self.close_state.is_closing() {
            return;
        }
        match refresh_target {
            Some(TransferRefreshTarget::Remote(remote_dir))
                if remote_dir == self.remote_current_path =>
            {
                self.refresh_remote_dir(cx);
            }
            Some(TransferRefreshTarget::Local(local_dir))
                if should_apply_local_listing(&self.local_current_path, &local_dir) =>
            {
                self.refresh_local_dir(cx);
            }
            _ => {}
        }
    }

    fn apply_transfer_event_to_mirror(
        &mut self,
        event: &SftpTransferEvent,
        snapshot: &SftpTransferSnapshot,
    ) -> Option<TransferRefreshTarget> {
        let task = self
            .transfer_queue
            .tasks
            .iter_mut()
            .find(|task| task.external_transfer_id == Some(snapshot.id))?;
        let refresh_target = transfer_refresh_target(&task.operation, snapshot.operation)?;
        let was_active = active_transfer_state(&task.state);
        apply_global_transfer_snapshot(task, snapshot);
        should_refresh_transfer_target(was_active, event, &snapshot.state).then_some(refresh_target)
    }

    fn reconcile_transfer_snapshot_to_mirror(
        &mut self,
        snapshot: &SftpTransferSnapshot,
    ) -> Option<TransferRefreshTarget> {
        let task = self
            .transfer_queue
            .tasks
            .iter_mut()
            .find(|task| task.external_transfer_id == Some(snapshot.id))?;
        reconcile_global_transfer_snapshot(task, snapshot)
    }

    fn start_transfer_task(&mut self, task: TransferTask, cx: &mut Context<Self>) {
        match task.operation {
            TransferOperation::Upload {
                local_path,
                remote_path,
                is_dir,
                directory_conflict_policy,
                remote_dir,
            } => {
                self.start_upload_task(
                    task.id,
                    local_path,
                    remote_path,
                    is_dir,
                    directory_conflict_policy,
                    remote_dir,
                    task.shared_progress,
                    cx,
                );
            }
            TransferOperation::Download { .. } => {
                unreachable!("global SFTP downloads must not enter the local transfer scheduler")
            }
            TransferOperation::DeleteLocal { entries, local_dir } => {
                self.start_local_delete_task(task.id, entries, local_dir, task.shared_progress, cx);
            }
            TransferOperation::DeleteRemote { .. } => {
                unreachable!(
                    "global SFTP remote deletes must not enter the local transfer scheduler"
                )
            }
            TransferOperation::ServerCopy(operation) => {
                let ServerCopyOperation {
                    source_config,
                    target_config,
                    items,
                    target_side,
                } = *operation;
                self.start_server_copy_task(
                    ServerCopyTaskInput {
                        task_id: task.id,
                        source_config,
                        target_config,
                        items,
                        target_side,
                        progress: task.shared_progress,
                    },
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
        directory_conflict_policy: DirectoryConflictPolicy,
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
                            directory_conflict_policy,
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
                if this.close_state.is_closing() {
                    return;
                }
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
                    .map(|e| {
                        let owner = e.owner_display();
                        FileItem {
                            name: e.name,
                            size: e.size,
                            modified: e.modified,
                            is_dir: e.is_dir,
                            permissions: format_permissions(e.permissions, e.is_dir),
                            owner,
                            directory_size: DirectorySizeState::Unknown,
                        }
                    })
                    .collect();

                this.remote_panel.update(cx, |panel, cx| {
                    panel.set_items(items, cx);
                });
            });
        })
        .detach();
    }

    fn start_server_copy_task(&mut self, input: ServerCopyTaskInput, cx: &mut Context<Self>) {
        input.progress.scanning.store(true, Ordering::Relaxed);
        let cancelled = input.progress.cancelled.clone();
        let direct_copy_enabled = AppSettings::global(cx).direct_server_transfer_enabled;
        let direct_copy_approval = if direct_copy_enabled {
            let (approval, prompt_request) = direct_copy_prompt::direct_copy_approval_bridge(
                input.task_id,
                cancelled.clone(),
                self.direct_copy_prompt_lock.clone(),
            );
            cx.spawn(async move |this, cx| {
                if let Ok(request) = prompt_request.await {
                    let _ = this.update_in(cx, |this, window, cx| {
                        this.open_direct_copy_prompt(request, window, cx);
                    });
                }
            })
            .detach();
            Some(approval)
        } else {
            None
        };

        let shared_progress = input.progress.clone();
        let task_id = input.task_id;
        let target_side = input.target_side;
        let copy_task = Tokio::spawn(cx, async move {
            copy_between_servers(ServerCopyRequest {
                source_config: input.source_config,
                target_config: input.target_config,
                items: input.items,
                cancelled,
                progress: Arc::new(move |progress| {
                    shared_progress.scanning.store(false, Ordering::Relaxed);
                    shared_progress
                        .transferred
                        .store(progress.transferred, Ordering::Relaxed);
                    shared_progress
                        .total
                        .store(progress.total, Ordering::Relaxed);
                    shared_progress
                        .speed
                        .store(progress.speed.to_bits(), Ordering::Relaxed);
                    if let Ok(mut current) = shared_progress.current_file.write() {
                        *current = progress.current_file;
                    }
                    shared_progress
                        .current_file_transferred
                        .store(progress.current_file_transferred, Ordering::Relaxed);
                    shared_progress
                        .current_file_total
                        .store(progress.current_file_total, Ordering::Relaxed);
                }),
                direct_copy_enabled,
                direct_copy_approval,
            })
            .await
            .map(|_| ())
        });

        cx.spawn(async move |this, cx| {
            let result = match copy_task.await {
                Ok(result) => result,
                Err(error) => Err(anyhow::Error::new(error)),
            };
            let succeeded = result.is_ok();
            let _ = this.update(cx, |this, cx| {
                let error_notification = result.as_ref().err().and_then(|error| {
                    (!Self::is_transfer_cancelled(error)).then(|| {
                        t!("Error.copy_failed", error = transfer_error_summary(error)).to_string()
                    })
                });
                this.update_task_state_from_result(task_id, result, cx);
                if let Some(error_message) = error_notification {
                    this.push_notification(Notification::error(error_message), cx);
                }
                if succeeded && !this.close_state.is_closing() {
                    match target_side {
                        PaneSide::Left => this.refresh_left_remote_dir(cx),
                        PaneSide::Right => this.refresh_remote_dir(cx),
                    }
                }
                this.schedule_transfers(cx);
                cx.notify();
            });
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
                let is_closing = this.close_state.is_closing();
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

                        if !errors.is_empty() && !is_closing {
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

                if !is_closing {
                    this.refresh_local_dir(cx);
                }
                this.sync_local_background_tasks(cx);
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
            if let Ok(mut current_file) = task.shared_progress.current_file.write() {
                *current_file = None;
            }
            task.shared_progress
                .current_file_transferred
                .store(0, Ordering::Relaxed);
            task.shared_progress
                .current_file_total
                .store(0, Ordering::Relaxed);
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

        if let Some(operation) = refresh_operation
            && !self.close_state.is_closing()
        {
            self.refresh_panel_for_operation(&operation, cx);
        }
        self.close_direct_copy_prompt_for_task(task_id, cx);
        self.sync_local_background_tasks(cx);
    }

    fn refresh_panel_for_operation(
        &mut self,
        operation: &TransferOperation,
        cx: &mut Context<Self>,
    ) {
        if self.close_state.is_closing() {
            return;
        }
        match operation {
            TransferOperation::Upload { .. } | TransferOperation::DeleteRemote { .. } => {
                self.refresh_remote_dir(cx);
            }
            TransferOperation::Download { .. } | TransferOperation::DeleteLocal { .. } => {
                self.refresh_local_dir(cx);
            }
            TransferOperation::ServerCopy(operation) => match operation.target_side {
                PaneSide::Left => self.refresh_left_remote_dir(cx),
                PaneSide::Right => self.refresh_remote_dir(cx),
            },
        }
    }

    fn is_transfer_cancelled(error: &anyhow::Error) -> bool {
        error.downcast_ref::<TransferCancelled>().is_some()
    }

    fn execute_uploads(&mut self, transfers: Vec<PendingTransfer>, cx: &mut Context<Self>) {
        self.execute_uploads_with_directory_policy(transfers, DirectoryConflictPolicy::Merge, cx);
    }

    fn execute_upload_decisions(
        &mut self,
        decisions: Vec<PendingUploadDecision>,
        cx: &mut Context<Self>,
    ) {
        for decision in decisions {
            self.execute_uploads_with_directory_policy(
                vec![decision.transfer],
                decision.directory_conflict_policy,
                cx,
            );
        }
    }

    fn execute_uploads_with_directory_policy(
        &mut self,
        transfers: Vec<PendingTransfer>,
        conflict_policy: DirectoryConflictPolicy,
        cx: &mut Context<Self>,
    ) {
        if matches!(self.transfer_queue.admission, TransferAdmission::Frozen) {
            tracing::debug!("Ignoring uploads after transfer admission was frozen");
            return;
        }
        let upload_context = SftpUploadContext {
            connection: self.upload_connection_identity.clone(),
            connection_source: SftpUploadConnection::Config(self.sftp_config.clone()),
            task_group: self.background_task_group(),
        };
        for transfer in transfers {
            let prepared = prepare_global_upload(&transfer, conflict_policy, &upload_context);
            let reservation = self
                .upload_executor
                .update(cx, |executor, _| executor.reserve(prepared.request));
            let Some(snapshot) = commit_external_transfer(
                &mut self.transfer_queue,
                &mut self.next_task_id,
                &self.upload_executor,
                reservation,
                prepared.operation,
                cx,
            ) else {
                tracing::debug!("Ignoring upload after transfer admission was frozen");
                continue;
            };
            let refresh_target = self.reconcile_transfer_snapshot_to_mirror(&snapshot);
            self.refresh_transfer_target_if_visible(refresh_target, cx);
        }

        self.start_progress_refresh(cx);
        cx.notify();
    }
    fn background_task_group(&self) -> SharedString {
        // 分组标题只保留「连接名称 - IP」，同一连接的多个面板合并到同一分组。
        format!("{} - {}", self.connection_name, self.sftp_config.host).into()
    }
    fn register_local_background_task(
        &self,
        kind: &'static str,
        title: String,
        progress_unit: BackgroundTaskProgressUnit,
        progress: &Arc<SharedProgress>,
        cx: &mut Context<Self>,
    ) -> BackgroundTaskHandle {
        let manager = one_core::background_tasks::global(cx);
        let spec = BackgroundTaskSpec::new(kind, title)
            .group(self.background_task_group())
            .progress_unit(progress_unit);
        let cancelled = progress.cancelled.clone();
        let id = manager.update(cx, |manager, cx| {
            let id = manager.register(spec, cx);
            manager.set_cancellation(
                id,
                BackgroundTaskCancellation::callback(move || {
                    cancelled.store(true, Ordering::Relaxed);
                }),
                cx,
            );
            id
        });
        BackgroundTaskHandle::new(manager.downgrade(), id)
    }

    fn register_non_cancellable_background_task(
        &self,
        kind: &'static str,
        title: String,
        cx: &mut Context<Self>,
    ) -> BackgroundTaskHandle {
        let manager = one_core::background_tasks::global(cx);
        let spec = BackgroundTaskSpec::new(kind, title)
            .group(self.background_task_group())
            .cancellable(false);
        let id = manager.update(cx, |manager, cx| {
            let id = manager.register(spec, cx);
            manager.mark_running(id, cx);
            id
        });
        BackgroundTaskHandle::new(manager.downgrade(), id)
    }

    fn sync_local_background_tasks(&mut self, cx: &mut Context<Self>) {
        for task in &mut self.transfer_queue.tasks {
            let Some(handle) = task.background_task.clone() else {
                continue;
            };
            match task.state {
                TransferTaskState::Pending => {}
                TransferTaskState::Running => {
                    let current = task.shared_progress.transferred.load(Ordering::Relaxed);
                    let total = task.shared_progress.total.load(Ordering::Relaxed);
                    let detail = task
                        .shared_progress
                        .current_file
                        .read()
                        .ok()
                        .and_then(|value| value.clone())
                        .map(Into::into);
                    let speed = f64::from_bits(task.shared_progress.speed.load(Ordering::Relaxed));
                    let speed = (speed > 0.0).then(|| format_speed(speed).into());
                    handle.update_progress(
                        current,
                        (total > 0).then_some(total),
                        detail,
                        speed,
                        cx,
                    );
                }
                TransferTaskState::Completed => {
                    handle.succeed(None, cx);
                    task.background_task = None;
                }
                TransferTaskState::Failed => {
                    handle.fail(
                        task.error
                            .clone()
                            .unwrap_or_else(|| "Task failed".to_string()),
                        cx,
                    );
                    task.background_task = None;
                }
                TransferTaskState::Cancelled => {
                    handle.cancel_confirmed(None, cx);
                    task.background_task = None;
                }
            }
        }
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
                            this.sync_local_background_tasks(cx);
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
        let external_transfer_id = self
            .transfer_queue
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .and_then(|task| task.external_transfer_id);
        if let Some(transfer_id) = external_transfer_id {
            self.upload_executor.update(cx, |executor, cx| {
                executor.cancel(transfer_id, cx);
            });
            self.close_direct_copy_prompt_for_task(task_id, cx);
            cx.notify();
            return;
        }

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
        self.close_direct_copy_prompt_for_task(task_id, cx);
        self.schedule_transfers(cx);
        cx.notify();
    }

    fn cancel_all_transfers(&mut self, cx: &mut Context<Self>) {
        let external_transfer_ids = active_external_transfer_ids(&self.transfer_queue.tasks);
        for transfer_id in external_transfer_ids {
            self.upload_executor.update(cx, |executor, cx| {
                executor.cancel(transfer_id, cx);
            });
        }
        self.transfer_queue.cancel_all();
        self.close_active_direct_copy_prompt(cx);
        cx.notify();
    }

    fn push_notification(&self, notification: Notification, cx: &mut Context<Self>) {
        if self.close_state.is_closing() {
            return;
        }
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

            self.show_download_conflict_dialog(
                conflict_names,
                pending_transfers,
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
        if matches!(self.transfer_queue.admission, TransferAdmission::Frozen) {
            tracing::debug!("Ignoring downloads after transfer admission was frozen");
            return;
        }
        let download_context = SftpUploadContext {
            connection: self.upload_connection_identity.clone(),
            connection_source: SftpUploadConnection::Config(self.sftp_config.clone()),
            task_group: self.background_task_group(),
        };
        for transfer in transfers {
            let prepared = prepare_global_download(&transfer, &download_context);
            let reservation = self.upload_executor.update(cx, |executor, _| {
                executor.reserve_download(prepared.request)
            });
            let Some(snapshot) = commit_external_transfer(
                &mut self.transfer_queue,
                &mut self.next_task_id,
                &self.upload_executor,
                reservation,
                prepared.operation,
                cx,
            ) else {
                tracing::debug!("Ignoring download after transfer admission was frozen");
                continue;
            };
            let refresh_target = self.reconcile_transfer_snapshot_to_mirror(&snapshot);
            self.refresh_transfer_target_if_visible(refresh_target, cx);
        }
        self.start_progress_refresh(cx);
        cx.notify();
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
                        .show_cancel(true)
                        .ok_text(t!("Common.delete").to_string())
                        .cancel_text(t!("Common.cancel").to_string()),
                )
                .on_ok(move |_, window, cx: &mut App| {
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
            current_file: std::sync::RwLock::new(first_file.clone()),
            current_file_transferred: AtomicU64::new(0),
            current_file_total: AtomicU64::new(1),
        });

        let task_id = self.next_task_id;
        self.next_task_id += 1;

        if !self.transfer_queue.enqueue(TransferTask {
            id: task_id,
            external_transfer_id: None,
            operation: TransferOperation::DeleteLocal {
                entries,
                local_dir: local_path,
            },
            state: TransferTaskState::Pending,
            shared_progress,
            error: None,
            background_task: None,
        }) {
            tracing::debug!("Ignoring local delete after transfer admission was frozen");
            return;
        }

        let task_progress = self
            .transfer_queue
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .map(|task| task.shared_progress.clone());
        if let Some(task_progress) = task_progress {
            let handle = self.register_local_background_task(
                "sftp-delete-local",
                format!(
                    "{} · {}",
                    t!("Common.delete"),
                    first_file.unwrap_or_default()
                ),
                BackgroundTaskProgressUnit::Items,
                &task_progress,
                cx,
            );
            if let Some(task) = self
                .transfer_queue
                .tasks
                .iter_mut()
                .find(|task| task.id == task_id)
            {
                task.background_task = Some(handle);
            }
        }

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
                        .show_cancel(true)
                        .ok_text(t!("Common.delete").to_string())
                        .cancel_text(t!("Common.cancel").to_string()),
                )
                .on_ok(move |_, window, cx: &mut App| {
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
        if matches!(self.transfer_queue.admission, TransferAdmission::Frozen) {
            tracing::debug!("Ignoring remote delete after transfer admission was frozen");
            return;
        }
        let context = SftpUploadContext {
            connection: self.upload_connection_identity.clone(),
            connection_source: SftpUploadConnection::Config(self.sftp_config.clone()),
            task_group: self.background_task_group(),
        };
        let prepared = prepare_global_remote_delete(&entries, &remote_path, &context);
        let reservation = self.upload_executor.update(cx, |executor, _| {
            executor.reserve_delete_remote(prepared.request)
        });
        let Some(snapshot) = commit_external_transfer(
            &mut self.transfer_queue,
            &mut self.next_task_id,
            &self.upload_executor,
            reservation,
            prepared.operation,
            cx,
        ) else {
            tracing::debug!("Ignoring remote delete after transfer admission was frozen");
            return;
        };
        let refresh_target = self.reconcile_transfer_snapshot_to_mirror(&snapshot);
        self.refresh_transfer_target_if_visible(refresh_target, cx);
        self.start_progress_refresh(cx);
        cx.notify();
    }

    fn show_new_folder_dialog(
        &mut self,
        side: PanelSide,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.close_state.is_closing() {
            return;
        }
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
                        .show_cancel(true)
                        .ok_text(t!("Common.create").to_string())
                        .cancel_text(t!("Common.cancel").to_string()),
                )
                .on_ok(move |_, window, cx: &mut App| {
                    let folder_name = input_for_callback.read(cx).text().to_string();
                    if folder_name.is_empty() {
                        return false;
                    }
                    if !is_valid_entry_name(&folder_name) {
                        window.push_notification(Notification::error(t!("Error.invalid_name")), cx);
                        return false;
                    }

                    let _ = view_clone.update(cx, |this, cx| {
                        if this.close_state.is_closing() {
                            return;
                        }
                        match side {
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
                                                if this.close_state.is_closing() {
                                                    return;
                                                }
                                                window.close_dialog(cx);
                                                this.refresh_remote_dir(cx);
                                            });
                                        }
                                        Ok(Err(e)) => {
                                            tracing::error!(
                                                "Failed to create remote folder: {}",
                                                e
                                            );
                                            let _ = view.update_in(cx, |this, window, cx| {
                                                if this.close_state.is_closing() {
                                                    return;
                                                }
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
                                            let _ = view.update_in(cx, |this, window, cx| {
                                                if this.close_state.is_closing() {
                                                    return;
                                                }
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
            .border_color(cx.theme().drag_border)
            .rounded_lg()
            .bg(cx.theme().drop_target)
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
                    .child(t!("File.drop_files_here").to_string()),
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

    fn selected_items_as_dragged(&self, side: PaneSide, cx: &App) -> Option<DraggedFileItems> {
        let (panel, current_path, source) = match side {
            PaneSide::Left => (
                &self.local_panel,
                self.left_remote
                    .as_ref()
                    .map(|endpoint| endpoint.current_path.clone())
                    .unwrap_or_else(|| self.local_current_path.to_string_lossy().to_string()),
                if self.left_remote.is_some() {
                    DragSource::RemoteLeft
                } else {
                    DragSource::LocalLeft
                },
            ),
            PaneSide::Right => (
                &self.remote_panel,
                self.remote_current_path.clone(),
                DragSource::RemoteRight,
            ),
        };
        let items = panel
            .read(cx)
            .selected_items(cx)
            .into_iter()
            .map(|item| {
                let full_path = if matches!(source, DragSource::LocalLeft) {
                    PathBuf::from(&current_path)
                        .join(&item.name)
                        .to_string_lossy()
                        .to_string()
                } else {
                    join_remote_path(&current_path, &item.name)
                };
                DraggedFileItem {
                    name: item.name,
                    size: item.size,
                    is_dir: item.is_dir,
                    full_path,
                    is_remote: !matches!(source, DragSource::LocalLeft),
                    source,
                }
            })
            .collect::<Vec<_>>();
        if items.is_empty() {
            None
        } else {
            Some(DraggedFileItems::multiple(
                items,
                !matches!(source, DragSource::LocalLeft),
                source,
            ))
        }
    }

    fn transfer_left_selection_to_right(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(items) = self.selected_items_as_dragged(PaneSide::Left, cx) else {
            return;
        };
        self.handle_dragged_drop_to_right(items, window, cx);
    }

    fn transfer_right_selection_to_left(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(items) = self.selected_items_as_dragged(PaneSide::Right, cx) else {
            return;
        };
        self.handle_dragged_drop_to_left(items, window, cx);
    }

    fn handle_remote_drop(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.close_state.is_closing() {
            return;
        }
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
                        let _ = view.update_in(cx, |this, window, cx| {
                            if this.close_state.is_closing() {
                                return;
                            }
                            window.push_notification(Notification::error(error_msg.clone()), cx);
                        });
                        return;
                    }
                    Err(e) => {
                        tracing::error!("Task error: {}", e);
                        let error_msg = t!("Error.read_dir_failed", error = e).to_string();
                        let _ = view.update_in(cx, |this, window, cx| {
                            if this.close_state.is_closing() {
                                return;
                            }
                            window.push_notification(Notification::error(error_msg.clone()), cx);
                        });
                        return;
                    }
                };

                let _ = view.update_in(cx, |this, window, cx| {
                    if this.close_state.is_closing() {
                        return;
                    }
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
                        this.show_upload_conflict_dialog(
                            pending_transfers,
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

    fn handle_dragged_drop_to_left(
        &mut self,
        dragged: DraggedFileItems,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match transfer_route(self.left_endpoint_kind(), dragged.source, PaneSide::Left) {
            Some(TransferRoute::Download) => {
                self.handle_remote_files_drop_to_local(dragged, window, cx);
            }
            Some(TransferRoute::ServerToServer { .. }) => {
                self.enqueue_server_copy(dragged, PaneSide::Left, window, cx);
            }
            _ => {}
        }
    }

    fn handle_dragged_drop_to_right(
        &mut self,
        dragged: DraggedFileItems,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match transfer_route(self.left_endpoint_kind(), dragged.source, PaneSide::Right) {
            Some(TransferRoute::Upload) => {
                self.handle_local_files_drop_to_remote(dragged, window, cx);
            }
            Some(TransferRoute::ServerToServer { .. }) => {
                self.enqueue_server_copy(dragged, PaneSide::Right, window, cx);
            }
            _ => {}
        }
    }

    fn left_endpoint_kind(&self) -> LeftEndpointKind {
        if self.left_remote.is_some() {
            LeftEndpointKind::Remote
        } else {
            LeftEndpointKind::Local
        }
    }

    fn enqueue_server_copy(
        &mut self,
        dragged: DraggedFileItems,
        target_side: PaneSide,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.close_state.is_closing() {
            return;
        }
        let (source_config, target_config, target_dir) = match (dragged.source, target_side) {
            (DragSource::RemoteLeft, PaneSide::Right) => {
                let Some(left) = self.left_remote.as_ref() else {
                    return;
                };
                (
                    left.config.clone(),
                    self.sftp_config.clone(),
                    self.remote_current_path.clone(),
                )
            }
            (DragSource::RemoteRight, PaneSide::Left) => {
                let Some(left) = self.left_remote.as_ref() else {
                    return;
                };
                (
                    self.sftp_config.clone(),
                    left.config.clone(),
                    left.current_path.clone(),
                )
            }
            _ => return,
        };
        let items = dragged
            .items
            .into_iter()
            .map(|item| ServerCopyItem {
                source_path: item.full_path,
                target_path: join_remote_path(&target_dir, &item.name),
                is_dir: item.is_dir,
                size: item.size,
                directory_conflict_policy: DirectoryConflictPolicy::Merge,
            })
            .collect::<Vec<_>>();
        if items.is_empty() {
            return;
        }

        let target_client = match target_side {
            PaneSide::Left => self
                .left_remote
                .as_ref()
                .and_then(|left| left.client.clone()),
            PaneSide::Right => self.sftp_client.clone(),
        };
        let Some(target_client) = target_client else {
            return;
        };
        let pending = PendingServerCopy {
            source_config,
            target_config,
            items,
            target_side,
            target_dir,
            existing_entries: std::collections::HashMap::new(),
        };
        let target_dir = pending.target_dir.clone();
        let view = cx.entity().clone();
        let list_task = Tokio::spawn(cx, async move {
            let mut client = target_client.lock().await;
            client.list_dir(&target_dir).await
        });
        window
            .spawn(cx, async move |cx| {
                let entries = match list_task.await {
                    Ok(Ok(entries)) => entries,
                    Ok(Err(error)) => {
                        let _ = view.update_in(cx, |this, window, cx| {
                            if this.close_state.is_closing() {
                                return;
                            }
                            window.push_notification(
                                Notification::error(
                                    t!("Error.read_dir_failed", error = error).to_string(),
                                ),
                                cx,
                            );
                        });
                        return;
                    }
                    Err(error) => {
                        let _ = view.update_in(cx, |this, window, cx| {
                            if this.close_state.is_closing() {
                                return;
                            }
                            window.push_notification(
                                Notification::error(
                                    t!("Error.read_dir_failed", error = error).to_string(),
                                ),
                                cx,
                            );
                        });
                        return;
                    }
                };
                let mut pending = pending;
                pending.existing_entries = entries
                    .into_iter()
                    .map(|entry| (entry.name, entry.is_dir))
                    .collect();
                let (conflicts, _, _) =
                    server_copy_conflict_flags(&pending.items, &pending.existing_entries);
                let _ = view.update_in(cx, |this, window, cx| {
                    if this.close_state.is_closing() {
                        return;
                    }
                    if conflicts == 0 {
                        this.enqueue_server_copy_now(pending, cx);
                    } else {
                        this.show_server_copy_conflict(pending, window, cx);
                    }
                });
            })
            .detach();
        return;
    }

    fn enqueue_server_copy_now(&mut self, pending: PendingServerCopy, cx: &mut Context<Self>) {
        if self.close_state.is_closing() {
            return;
        }
        let PendingServerCopy {
            source_config,
            target_config,
            items,
            target_side,
            ..
        } = pending;
        if items.is_empty() {
            return;
        }
        let item_count = items.len();

        let progress = Arc::new(SharedProgress {
            transferred: AtomicU64::new(0),
            total: AtomicU64::new(0),
            speed: AtomicU64::new(0),
            cancelled: Arc::new(AtomicBool::new(false)),
            scanning: AtomicBool::new(true),
            current_file: std::sync::RwLock::new(None),
            current_file_transferred: AtomicU64::new(0),
            current_file_total: AtomicU64::new(0),
        });
        let task_id = self.next_task_id;
        self.next_task_id += 1;
        if !self.transfer_queue.enqueue(TransferTask {
            id: task_id,
            external_transfer_id: None,
            operation: TransferOperation::ServerCopy(Box::new(ServerCopyOperation {
                source_config,
                target_config,
                items,
                target_side,
            })),
            state: TransferTaskState::Pending,
            shared_progress: progress,
            error: None,
            background_task: None,
        }) {
            tracing::debug!("Ignoring server copy after transfer admission was frozen");
            return;
        }
        let task_progress = self
            .transfer_queue
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .map(|task| task.shared_progress.clone());
        if let Some(task_progress) = task_progress {
            let handle = self.register_local_background_task(
                "sftp-server-copy",
                t!("Transfer.server_copy_n_items", count = item_count).to_string(),
                BackgroundTaskProgressUnit::Bytes,
                &task_progress,
                cx,
            );
            if let Some(task) = self
                .transfer_queue
                .tasks
                .iter_mut()
                .find(|task| task.id == task_id)
            {
                task.background_task = Some(handle);
            }
        }
        self.schedule_transfers(cx);
    }

    fn show_server_copy_conflict(
        &mut self,
        pending: PendingServerCopy,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.close_state.is_closing() {
            return;
        }
        let conflicts = pending
            .items
            .iter()
            .filter_map(|item| item.target_path.rsplit('/').next())
            .filter(|name| pending.existing_entries.contains_key(*name))
            .take(3)
            .collect::<Vec<_>>()
            .join(", ");
        let (_, has_mergeable_directory, has_type_mismatch) =
            server_copy_conflict_flags(&pending.items, &pending.existing_entries);
        let view = cx.entity().clone();
        let overwrite = pending.clone();
        let skip = pending.clone();
        let keep = pending.clone();
        let merge = pending.clone();
        window.open_dialog(cx, move |dialog, _window, cx| {
            let view_overwrite = view.clone();
            let view_skip = view.clone();
            let view_keep = view.clone();
            let view_merge = view.clone();
            let footer_overwrite = overwrite.clone();
            let footer_skip = skip.clone();
            let footer_keep = keep.clone();
            let footer_merge = merge.clone();
            let mut content = v_flex()
                .gap_2()
                .child(t!("Conflict.files_exist").to_string())
                .child(
                    div()
                        .p_2()
                        .bg(cx.theme().secondary)
                        .rounded_md()
                        .child(conflicts.clone()),
                );
            if has_type_mismatch {
                content = content.child(t!("Conflict.server_copy_type_mismatch_safe").to_string());
            }
            dialog
                .title(t!("Dialog.file_conflict").to_string())
                .w(px(450.))
                .child(content)
                .footer({
                    let view_overwrite = view_overwrite.clone();
                    let view_skip = view_skip.clone();
                    let view_keep = view_keep.clone();
                    let view_merge = view_merge.clone();
                    let skip = footer_skip.clone();
                    let keep = footer_keep.clone();
                    let merge = footer_merge.clone();
                    let overwrite = footer_overwrite.clone();
                    let mut buttons: Vec<gpui::AnyElement> = vec![
                        Button::new("server-copy-skip")
                            .label(t!("Conflict.skip").to_string())
                            .ghost()
                            .on_click({
                                let pending = skip.clone();
                                move |_, window, cx| {
                                    window.close_dialog(cx);
                                    let mut pending = pending.clone();
                                    pending.items.retain(|item| {
                                        item.target_path.rsplit('/').next().is_none_or(|name| {
                                            !pending.existing_entries.contains_key(name)
                                        })
                                    });
                                    let _ = view_skip.update(cx, |this, cx| {
                                        if this.close_state.is_closing() {
                                            return;
                                        }
                                        this.enqueue_server_copy_now(pending.clone(), cx);
                                    });
                                }
                            })
                            .into_any_element(),
                        Button::new("server-copy-keep")
                            .label(t!("Conflict.keep_both").to_string())
                            .ghost()
                            .on_click({
                                let pending = keep.clone();
                                move |_, window, cx| {
                                    window.close_dialog(cx);
                                    let mut pending = pending.clone();
                                    let mut names = pending
                                        .existing_entries
                                        .keys()
                                        .cloned()
                                        .collect::<std::collections::HashSet<_>>();
                                    for item in &mut pending.items {
                                        let Some(name) = item.target_path.rsplit('/').next() else {
                                            continue;
                                        };
                                        if pending.existing_entries.contains_key(name) {
                                            let renamed = generate_unique_name(name, &names);
                                            names.insert(renamed.clone());
                                            if let Some((parent, _)) =
                                                item.target_path.rsplit_once('/')
                                            {
                                                item.target_path = format!("{parent}/{renamed}");
                                            }
                                        }
                                    }
                                    let _ = view_keep.update(cx, |this, cx| {
                                        if this.close_state.is_closing() {
                                            return;
                                        }
                                        this.enqueue_server_copy_now(pending, cx);
                                    });
                                }
                            })
                            .into_any_element(),
                    ];
                    if has_mergeable_directory && !has_type_mismatch {
                        buttons.push(
                            Button::new("server-copy-merge")
                                .label(t!("Conflict.merge").to_string())
                                .ghost()
                                .on_click({
                                    let pending = merge.clone();
                                    move |_, window, cx| {
                                        window.close_dialog(cx);
                                        let mut pending = pending.clone();
                                        pending.items.retain(|item| {
                                            item.is_dir
                                                || item.target_path.rsplit('/').next().is_none_or(
                                                    |name| {
                                                        !pending.existing_entries.contains_key(name)
                                                    },
                                                )
                                        });
                                        let _ = view_merge.update(cx, |this, cx| {
                                            if this.close_state.is_closing() {
                                                return;
                                            }
                                            this.enqueue_server_copy_now(pending, cx);
                                        });
                                    }
                                })
                                .into_any_element(),
                        );
                    }
                    if !has_type_mismatch {
                        buttons.push(
                            Button::new("server-copy-overwrite")
                                .label(t!("Conflict.overwrite").to_string())
                                .primary()
                                .on_click({
                                    let pending = overwrite.clone();
                                    move |_, window, cx| {
                                        window.close_dialog(cx);
                                        let mut pending = pending.clone();
                                        mark_server_copy_directory_replacements(
                                            &mut pending.items,
                                            &pending.existing_entries,
                                        );
                                        let _ = view_overwrite.update(cx, |this, cx| {
                                            if this.close_state.is_closing() {
                                                return;
                                            }
                                            this.enqueue_server_copy_now(pending, cx);
                                        });
                                    }
                                })
                                .into_any_element(),
                        );
                    }
                    DialogFooter::new().children(buttons)
                })
                .overlay_closable(false)
                .close_button(true)
        });
    }

    fn handle_remote_files_drop_to_local(
        &mut self,
        dragged: DraggedFileItems,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.close_state.is_closing() {
            return;
        }
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

            self.show_download_conflict_dialog(
                conflict_names,
                pending_transfers,
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
        if self.close_state.is_closing() {
            return;
        }
        if dragged.source != DragSource::LocalLeft {
            return;
        }
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
                        let _ = view.update_in(cx, |this, window, cx| {
                            if this.close_state.is_closing() {
                                return;
                            }
                            window.push_notification(Notification::error(error_msg.clone()), cx);
                        });
                        return;
                    }
                    Err(e) => {
                        tracing::error!("Task error: {}", e);
                        let error_msg = t!("Error.read_dir_failed", error = e).to_string();
                        let _ = view.update_in(cx, |this, window, cx| {
                            if this.close_state.is_closing() {
                                return;
                            }
                            window.push_notification(Notification::error(error_msg.clone()), cx);
                        });
                        return;
                    }
                };

                let _ = view.update_in(cx, |this, window, cx| {
                    if this.close_state.is_closing() {
                        return;
                    }
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
                        this.show_upload_conflict_dialog(
                            pending_transfers,
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

    fn render_connection_overlay(&self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(inputs) = self.credential_inputs.as_ref() {
            return div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::black().opacity(cx.theme().geometry.opacity.scrim))
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
                        .w(px(450.))
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    Icon::new(IconName::Key)
                                        .with_size(IconSize::Large)
                                        .text_color(cx.theme().warning),
                                )
                                .child(
                                    div()
                                        .text_lg()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(t!("Credentials.title").to_string()),
                                ),
                        )
                        .child(
                            div()
                                .w_full()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("Credentials.hint").to_string()),
                        )
                        .when_some(inputs.username.as_ref(), |this, input| {
                            this.child(
                                v_flex()
                                    .w_full()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(t!("Credentials.username").to_string()),
                                    )
                                    .child(Input::new(input)),
                            )
                        })
                        .when_some(inputs.password.as_ref(), |this, input| {
                            this.child(
                                v_flex()
                                    .w_full()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(t!("Credentials.password").to_string()),
                                    )
                                    .child(Input::new(input).mask_toggle()),
                            )
                        })
                        .when_some(inputs.error.clone(), |this, error| {
                            this.child(
                                div()
                                    .w_full()
                                    .text_sm()
                                    .text_color(cx.theme().danger)
                                    .child(error),
                            )
                        })
                        .child(
                            Button::new("submit-sftp-credentials")
                                .label(t!("Credentials.connect").to_string())
                                .primary()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.submit_credentials(window, cx);
                                })),
                        ),
                )
                .into_any_element();
        }

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
            .bg(gpui::black().opacity(cx.theme().geometry.opacity.scrim))
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
                                    .with_size(IconSize::Large)
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
            .into_any_element()
    }

    fn render_transfer_queue(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active_tasks = self.transfer_queue.bottom_visible_tasks();

        if active_tasks.is_empty() {
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
                TransferOperation::ServerCopy(operation) => (
                    IconName::ArrowRight,
                    t!(
                        "Transfer.server_copy_n_items",
                        count = operation.items.len()
                    )
                    .to_string(),
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
                            .child(marquee_text(
                                SharedString::from(format!(
                                    "transfer-marquee-{task_id}-{display_name}"
                                )),
                                display_name,
                                is_running && has_current_file && !is_delete_op,
                            ))
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

    fn render_left_remote_breadcrumb(&self, cx: &mut Context<Self>) -> Breadcrumb {
        let Some(endpoint) = self.left_remote.as_ref() else {
            return Breadcrumb::new();
        };
        let path = endpoint.current_path.clone();
        if path == "." {
            return Breadcrumb::new().child(breadcrumb_item("."));
        }

        let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        let absolute = path.starts_with('/');
        let mut breadcrumb = Breadcrumb::new();
        if absolute {
            breadcrumb = breadcrumb.child(breadcrumb_item("/").on_click(cx.listener(
                |this, _, _window, cx| this.navigate_left_remote_to("/".to_string(), cx),
            )));
        }
        const MAX_VISIBLE_PARTS: usize = 3;
        let visible_start = parts.len().saturating_sub(MAX_VISIBLE_PARTS);
        if visible_start > 0 {
            breadcrumb = breadcrumb.child(breadcrumb_item("...").disabled(true));
        }
        for index in visible_start..parts.len() {
            let target = if absolute {
                format!("/{}", parts[..=index].join("/"))
            } else {
                parts[..=index].join("/")
            };
            breadcrumb = breadcrumb.child(breadcrumb_item(parts[index].to_string()).on_click(
                cx.listener(move |this, _, _window, cx| {
                    this.navigate_left_remote_to(target.clone(), cx);
                }),
            ));
        }
        breadcrumb
    }

    fn left_endpoint_title(&self) -> String {
        self.left_remote.as_ref().map_or_else(
            || t!("Endpoint.local").to_string(),
            |endpoint| endpoint::connection_title(&endpoint.connection),
        )
    }

    fn open_left_endpoint_switcher(&self, window: &mut Window, cx: &mut Context<Self>) {
        let active_value = self
            .left_remote
            .as_ref()
            .and_then(|endpoint| endpoint.connection.id)
            .map(LeftEndpointValue::Remote)
            .unwrap_or(LeftEndpointValue::Local);
        let entries = endpoint::endpoint_items(
            &self.stored_connection,
            t!("Endpoint.local").to_string(),
            cx,
        )
        .into_iter()
        .map(|item| endpoint_switcher::EndpointSwitcherEntry {
            active: item.value() == &active_value,
            value: item.value().clone(),
            title: item.title_text().to_string().into(),
            icon: item.icon(),
        })
        .collect();
        endpoint_switcher::open_endpoint_switcher_dialog(cx.entity(), entries, window, cx);
    }

    fn render_local_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let geometry = cx.theme().geometry.clone();
        let is_left_remote = self.left_remote.is_some();
        let breadcrumb = if is_left_remote {
            self.render_left_remote_breadcrumb(cx)
        } else {
            self.render_local_breadcrumb(cx)
        };
        let selected_count = self.get_local_selected_count(cx);
        let has_selection = selected_count > 0;
        let is_connected = self.connection_state == ConnectionState::Connected;
        let local_path_input = self.local_path_input.clone();
        let is_editing = self.local_path_editing;
        let is_dragging = self.is_dragging_over_local;
        let can_go_back = if is_left_remote {
            self.can_go_back_left_remote()
        } else {
            self.can_go_back_local()
        };
        let can_go_forward = if is_left_remote {
            self.can_go_forward_left_remote()
        } else {
            self.can_go_forward_local()
        };
        let is_favorite = !is_left_remote && self.is_current_local_path_favorite();
        let favorite_paths = if is_left_remote {
            Vec::new()
        } else {
            self.local_favorite_paths()
        };
        let clipboard_endpoint = if is_left_remote {
            ClipboardEndpoint::RemoteLeft
        } else {
            ClipboardEndpoint::Local
        };
        let paste_target_dir = self.left_remote.as_ref().map_or_else(
            || self.local_current_path.to_string_lossy().to_string(),
            |endpoint| endpoint.current_path.clone(),
        );
        let endpoint_ready = self
            .left_remote
            .as_ref()
            .is_none_or(|endpoint| endpoint.state == LeftRemoteConnectionState::Connected);
        let can_paste = endpoint_ready && self.can_paste_file_clipboard(clipboard_endpoint);
        let left_endpoint_title = self.left_endpoint_title();
        let local_actions_view = cx.entity();

        v_flex()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .h(geometry.layout.command_bar)
                    .px(geometry.spacing.space_2)
                    .gap(geometry.spacing.space_1)
                    .items_center()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        Button::new("left_endpoint_switcher")
                            .icon(if is_left_remote {
                                IconName::Server
                            } else {
                                IconName::HardDrive
                            })
                            .ghost()
                            .small()
                            .compact()
                            .dropdown_caret(true)
                            .tooltip(t!(
                                "Endpoint.switch_tooltip",
                                name = left_endpoint_title.clone()
                            ))
                            .child(
                                div()
                                    .max_w(px(96.))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(left_endpoint_title),
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_left_endpoint_switcher(window, cx);
                            })),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                IconButton::new("back", IconName::ChevronLeft)
                                    .role(IconButtonRole::Toolbar)
                                    .tooltip(t!("Navigation.back"))
                                    .disabled(!can_go_back)
                                    .on_click(cx.listener(|this, _, _window, cx| {
                                        if this.left_remote.is_some() {
                                            this.go_back_left_remote(cx);
                                        } else {
                                            this.go_back_local(cx);
                                        }
                                    })),
                            )
                            .child(
                                IconButton::new("forward", IconName::ChevronRight)
                                    .role(IconButtonRole::Toolbar)
                                    .tooltip(t!("Navigation.forward"))
                                    .disabled(!can_go_forward)
                                    .on_click(cx.listener(|this, _, _window, cx| {
                                        if this.left_remote.is_some() {
                                            this.go_forward_left_remote(cx);
                                        } else {
                                            this.go_forward_local(cx);
                                        }
                                    })),
                            ),
                    )
                    .child(if is_editing {
                        h_flex()
                            .flex_1()
                            .min_w(px(0.))
                            .h(geometry.control.small)
                            .px_2()
                            .items_center()
                            .bg(cx.theme().secondary)
                            .rounded(geometry.radius.sm)
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
                            .h(geometry.control.small)
                            .px_2()
                            .items_center()
                            .bg(cx.theme().secondary)
                            .rounded(geometry.radius.sm)
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
                            .when(!is_left_remote, |toolbar| {
                                toolbar
                                    .child(
                                        IconButton::new(
                                            "local_toggle_favorite",
                                            if is_favorite {
                                                IconName::StarFill
                                            } else {
                                                IconName::Star
                                            },
                                        )
                                        .role(IconButtonRole::Toolbar)
                                        .tooltip(if is_favorite {
                                            t!("FavoritePath.remove_current").to_string()
                                        } else {
                                            t!("FavoritePath.add_current").to_string()
                                        })
                                        .on_click(
                                            cx.listener(|this, _, window, cx| {
                                                this.toggle_current_local_favorite(window, cx);
                                            }),
                                        ),
                                    )
                                    .child(self.render_local_favorites_menu(favorite_paths, cx))
                            })
                            .child(
                                IconButton::new("refresh_local", IconName::Refresh)
                                    .role(IconButtonRole::Toolbar)
                                    .tooltip(t!("Common.refresh"))
                                    .disabled(!endpoint_ready)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        if this.left_remote.is_some() {
                                            this.refresh_left_remote_dir(cx);
                                        } else {
                                            this.refresh_local_dir_with_window(window, cx);
                                        }
                                    })),
                            )
                            .when(is_left_remote, |toolbar| {
                                toolbar.child(
                                    IconButton::new(
                                        "left_remote_transfer_to_right",
                                        IconName::ArrowRight,
                                    )
                                    .role(IconButtonRole::Toolbar)
                                    .tooltip(t!("Transfer.transfer_to_right"))
                                    .disabled(!has_selection || !endpoint_ready || !is_connected)
                                    .on_click(cx.listener(
                                        |this, _, window, cx| {
                                            this.transfer_left_selection_to_right(window, cx);
                                        },
                                    )),
                                )
                            })
                            .child(
                                Button::new("local_file_actions")
                                    .ghost()
                                    .small()
                                    .compact()
                                    .icon(IconName::Ellipsis)
                                    .tooltip(t!("File.actions"))
                                    .disabled(!endpoint_ready)
                                    .dropdown_menu_with_anchor(
                                        Anchor::TopRight,
                                        move |menu, window, _cx| {
                                            let paste_view = local_actions_view.clone();
                                            let paste_target_dir = paste_target_dir.clone();
                                            let menu = menu.item(
                                                PopupMenuItem::new(t!("File.paste").to_string())
                                                    .icon(IconName::Paste)
                                                    .disabled(!can_paste)
                                                    .on_click(window.listener_for(
                                                        &paste_view,
                                                        move |this, _, window, cx| {
                                                            this.paste_file_clipboard(
                                                                clipboard_endpoint,
                                                                paste_target_dir.clone(),
                                                                window,
                                                                cx,
                                                            );
                                                        },
                                                    )),
                                            );
                                            if is_left_remote {
                                                return menu;
                                            }

                                            let new_file_view = local_actions_view.clone();
                                            let new_folder_view = local_actions_view.clone();
                                            let delete_view = local_actions_view.clone();
                                            let upload_files_view = local_actions_view.clone();
                                            let upload_folder_view = local_actions_view.clone();
                                            menu.separator()
                                                .item(
                                                    PopupMenuItem::new(
                                                        t!("File.upload_file").to_string(),
                                                    )
                                                    .disabled(!is_connected)
                                                    .icon(IconName::Upload)
                                                    .on_click(window.listener_for(
                                                        &upload_files_view,
                                                        move |this, _, window, cx| {
                                                            this.select_and_upload_files(
                                                                window, cx,
                                                            );
                                                        },
                                                    )),
                                                )
                                                .item(
                                                    PopupMenuItem::new(
                                                        t!("File.upload_folder").to_string(),
                                                    )
                                                    .disabled(!is_connected)
                                                    .icon(IconName::Upload)
                                                    .on_click(window.listener_for(
                                                        &upload_folder_view,
                                                        move |this, _, window, cx| {
                                                            this.select_and_upload_folder(
                                                                window, cx,
                                                            );
                                                        },
                                                    )),
                                                )
                                                .item(
                                                    PopupMenuItem::new(
                                                        t!("File.new_file").to_string(),
                                                    )
                                                    .icon(IconName::File)
                                                    .on_click(window.listener_for(
                                                        &new_file_view,
                                                        move |this, _, window, cx| {
                                                            this.create_new_file(
                                                                PanelSide::Local,
                                                                window,
                                                                cx,
                                                            );
                                                        },
                                                    )),
                                                )
                                                .item(
                                                    PopupMenuItem::new(
                                                        t!("File.new_folder").to_string(),
                                                    )
                                                    .disabled(!is_connected)
                                                    .icon(IconName::NewFolder)
                                                    .on_click(window.listener_for(
                                                        &new_folder_view,
                                                        move |this, _, window, cx| {
                                                            this.show_new_folder_dialog(
                                                                PanelSide::Local,
                                                                window,
                                                                cx,
                                                            );
                                                        },
                                                    )),
                                                )
                                                .item(
                                                    PopupMenuItem::new(
                                                        t!("Common.delete").to_string(),
                                                    )
                                                    .icon(IconName::Remove)
                                                    .disabled(!has_selection)
                                                    .on_click(window.listener_for(
                                                        &delete_view,
                                                        move |this, _, window, cx| {
                                                            this.delete_local_selected(window, cx);
                                                        },
                                                    )),
                                                )
                                        },
                                    ),
                            ),
                    ),
            )
            .child(
                div()
                    .id("local-drop-zone")
                    .flex_1()
                    .relative()
                    .drag_over::<ExternalPaths>(|el, _, _, cx| el.bg(cx.theme().drop_target))
                    .drag_over::<DraggedFileItems>(|el, _, _, cx| el.bg(cx.theme().drop_target))
                    .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                        this.is_dragging_over_local = false;
                        if this.left_remote.is_some() {
                            window.push_notification(
                                Notification::info(t!("Endpoint.remote_edit_pending").to_string())
                                    .autohide(true),
                                cx,
                            );
                        } else {
                            this.handle_local_drop(paths.paths().to_vec(), window, cx);
                        }
                    }))
                    .on_drop(cx.listener(|this, items: &DraggedFileItems, window, cx| {
                        tracing::info!("local-drop-zone on_drop: items count={}", items.len());
                        this.is_dragging_over_local = false;
                        this.handle_dragged_drop_to_left(items.clone(), window, cx);
                    }))
                    .child(match self.left_remote.as_ref() {
                        None => self.local_panel.clone().into_any_element(),
                        Some(endpoint) => match &endpoint.state {
                            LeftRemoteConnectionState::Connected => div()
                                .size_full()
                                .relative()
                                .child(self.local_panel.clone())
                                .when(endpoint.loading, |element| {
                                    element.child(
                                        div()
                                            .absolute()
                                            .inset_0()
                                            .bg(gpui::black()
                                                .opacity(cx.theme().geometry.opacity.loading_scrim))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .child(Spinner::new().with_size(Size::Large)),
                                    )
                                })
                                .into_any_element(),
                            LeftRemoteConnectionState::Connecting => h_flex()
                                .size_full()
                                .justify_center()
                                .items_center()
                                .gap_2()
                                .child(Spinner::new().with_size(Size::Large))
                                .child(t!("Connection.connecting").to_string())
                                .into_any_element(),
                            LeftRemoteConnectionState::Disconnected(error) => v_flex()
                                .size_full()
                                .justify_center()
                                .items_center()
                                .gap_2()
                                .child(
                                    Icon::new(IconName::CircleX)
                                        .with_size(IconSize::Medium)
                                        .text_color(cx.theme().danger),
                                )
                                .child(t!("Connection.disconnected").to_string())
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(error.clone()),
                                )
                                .into_any_element(),
                        },
                    })
                    .when(is_dragging, |el| el.child(self.render_drop_overlay(cx))),
            )
    }

    fn render_remote_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let geometry = cx.theme().geometry.clone();
        let breadcrumb = self.render_remote_breadcrumb(cx);
        let selected_count = self.get_remote_selected_count(cx);
        let has_selection = selected_count > 0;
        let is_connected = self.connection_state == ConnectionState::Connected;
        let left_is_remote = self.left_remote.is_some();
        let left_ready = self
            .left_remote
            .as_ref()
            .is_none_or(|endpoint| endpoint.state == LeftRemoteConnectionState::Connected);
        let transfer_icon = if left_is_remote {
            IconName::ArrowLeft
        } else {
            IconName::ArrowDown
        };
        let transfer_tooltip = if left_is_remote {
            t!("Transfer.transfer_to_left").to_string()
        } else {
            t!("Common.download").to_string()
        };
        let remote_path_input = self.remote_path_input.clone();
        let is_editing = self.remote_path_editing;
        let is_dragging = self.is_dragging_over_remote;
        let can_go_back = self.can_go_back_remote();
        let can_go_forward = self.can_go_forward_remote();
        let is_favorite = self.is_current_remote_path_favorite();
        let favorite_paths = self.remote_favorite_paths();
        let paste_target_dir = self.remote_current_path.clone();
        let can_paste =
            is_connected && self.can_paste_file_clipboard(ClipboardEndpoint::RemoteRight);
        let remote_actions_view = cx.entity();

        v_flex()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .child(
                h_flex()
                    .h(geometry.layout.command_bar)
                    .px(geometry.spacing.space_2)
                    .gap(geometry.spacing.space_1)
                    .items_center()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                IconButton::new("remote_back", IconName::ChevronLeft)
                                    .role(IconButtonRole::Toolbar)
                                    .tooltip(t!("Navigation.back"))
                                    .disabled(!can_go_back)
                                    .on_click(cx.listener(|this, _, _window, cx| {
                                        this.go_back_remote(cx);
                                    })),
                            )
                            .child(
                                IconButton::new("remote_forward", IconName::ChevronRight)
                                    .role(IconButtonRole::Toolbar)
                                    .tooltip(t!("Navigation.forward"))
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
                            .h(geometry.control.small)
                            .px_2()
                            .items_center()
                            .bg(cx.theme().secondary)
                            .rounded(geometry.radius.sm)
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
                            .h(geometry.control.small)
                            .px_2()
                            .items_center()
                            .bg(cx.theme().secondary)
                            .rounded(geometry.radius.sm)
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
                                IconButton::new(
                                    "remote_toggle_favorite",
                                    if is_favorite {
                                        IconName::StarFill
                                    } else {
                                        IconName::Star
                                    },
                                )
                                .role(IconButtonRole::Toolbar)
                                .tooltip(if is_favorite {
                                    t!("FavoritePath.remove_current").to_string()
                                } else {
                                    t!("FavoritePath.add_current").to_string()
                                })
                                .disabled(!is_connected)
                                .on_click(cx.listener(
                                    |this, _, window, cx| {
                                        this.toggle_current_remote_favorite(window, cx);
                                    },
                                )),
                            )
                            .child(self.render_remote_favorites_menu(
                                favorite_paths,
                                is_connected,
                                cx,
                            ))
                            .child(
                                IconButton::new("refresh_remote", IconName::Refresh)
                                    .role(IconButtonRole::Toolbar)
                                    .disabled(!is_connected)
                                    .tooltip(t!("Common.refresh"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.refresh_remote_dir_with_window(window, cx);
                                    })),
                            )
                            .child(
                                IconButton::new("remote_download", transfer_icon)
                                    .role(IconButtonRole::Toolbar)
                                    .tooltip(transfer_tooltip)
                                    .disabled(!has_selection || !is_connected || !left_ready)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        if this.left_remote.is_some() {
                                            this.transfer_right_selection_to_left(window, cx);
                                        } else {
                                            this.download_selected(window, cx);
                                        }
                                    })),
                            )
                            .child(
                                Button::new("remote_file_actions")
                                    .ghost()
                                    .small()
                                    .compact()
                                    .icon(IconName::Ellipsis)
                                    .tooltip(t!("File.actions"))
                                    .disabled(!is_connected)
                                    .dropdown_menu_with_anchor(
                                        Anchor::TopRight,
                                        move |menu, window, _cx| {
                                            let paste_view = remote_actions_view.clone();
                                            let paste_target_dir = paste_target_dir.clone();
                                            let new_file_view = remote_actions_view.clone();
                                            let new_folder_view = remote_actions_view.clone();
                                            let delete_view = remote_actions_view.clone();

                                            menu.item(
                                                PopupMenuItem::new(t!("File.paste").to_string())
                                                    .icon(IconName::Paste)
                                                    .disabled(!can_paste)
                                                    .on_click(window.listener_for(
                                                        &paste_view,
                                                        move |this, _, window, cx| {
                                                            this.paste_file_clipboard(
                                                                ClipboardEndpoint::RemoteRight,
                                                                paste_target_dir.clone(),
                                                                window,
                                                                cx,
                                                            );
                                                        },
                                                    )),
                                            )
                                            .separator()
                                            .item(
                                                PopupMenuItem::new(t!("File.new_file").to_string())
                                                    .icon(IconName::File)
                                                    .disabled(!is_connected)
                                                    .on_click(window.listener_for(
                                                        &new_file_view,
                                                        move |this, _, window, cx| {
                                                            this.create_new_file(
                                                                PanelSide::Remote,
                                                                window,
                                                                cx,
                                                            );
                                                        },
                                                    )),
                                            )
                                            .item(
                                                PopupMenuItem::new(
                                                    t!("File.new_folder").to_string(),
                                                )
                                                .icon(IconName::NewFolder)
                                                .disabled(!is_connected)
                                                .on_click(window.listener_for(
                                                    &new_folder_view,
                                                    move |this, _, window, cx| {
                                                        this.show_new_folder_dialog(
                                                            PanelSide::Remote,
                                                            window,
                                                            cx,
                                                        );
                                                    },
                                                )),
                                            )
                                            .item(
                                                PopupMenuItem::new(t!("Common.delete").to_string())
                                                    .icon(IconName::Remove)
                                                    .disabled(!has_selection || !is_connected)
                                                    .on_click(window.listener_for(
                                                        &delete_view,
                                                        move |this, _, window, cx| {
                                                            this.delete_remote_selected(window, cx);
                                                        },
                                                    )),
                                            )
                                        },
                                    ),
                            ),
                    ),
            )
            .child(
                div()
                    .id("remote-drop-zone")
                    .flex_1()
                    .relative()
                    .when(is_connected, |el| {
                        el.drag_over::<ExternalPaths>(|el, _, _, cx| el.bg(cx.theme().drop_target))
                            .drag_over::<DraggedFileItems>(|el, _, _, cx| {
                                el.bg(cx.theme().drop_target)
                            })
                            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                                this.is_dragging_over_remote = false;
                                this.handle_remote_drop(paths.paths().to_vec(), window, cx);
                            }))
                            .on_drop(cx.listener(|this, items: &DraggedFileItems, window, cx| {
                                this.is_dragging_over_remote = false;
                                this.handle_dragged_drop_to_right(items.clone(), window, cx);
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
                                        .bg(gpui::black()
                                            .opacity(cx.theme().geometry.opacity.loading_scrim))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(Spinner::new().with_size(Size::Large)),
                                )
                            })
                            .into_any_element(),
                        ConnectionState::AwaitingCredentials => div()
                            .size_full()
                            .bg(cx.theme().background)
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
                                    .with_size(IconSize::Medium)
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

type CloseChoiceSender = Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<CloseChoice>>>>;

fn send_close_choice(sender: &CloseChoiceSender, choice: CloseChoice) {
    if let Ok(mut guard) = sender.lock()
        && let Some(sender) = guard.take()
    {
        let _ = sender.send(choice);
    }
}

enum CloseButtonStyle {
    Ghost,
    Primary,
    Danger,
}

struct CloseButtonSpec {
    id: &'static str,
    label: String,
    choice: CloseChoice,
    style: CloseButtonStyle,
}

fn close_choice_button(spec: CloseButtonSpec, sender: CloseChoiceSender) -> AnyElement {
    let button = Button::new(spec.id)
        .label(spec.label)
        .on_click(move |_, window, cx| {
            window.close_dialog(cx);
            send_close_choice(&sender, spec.choice);
        });
    let button = match spec.style {
        CloseButtonStyle::Ghost => button.ghost(),
        CloseButtonStyle::Primary => button.primary(),
        CloseButtonStyle::Danger => button.danger(),
    };
    button.into_any_element()
}

fn open_close_strategy_dialog(
    task_count: usize,
    window: &mut Window,
    cx: &mut Context<SftpView>,
    sender: CloseChoiceSender,
) {
    window.open_dialog(cx, move |dialog, _window, _cx| {
        let keyboard_cancel_sender = sender.clone();
        let keyboard_wait_sender = sender.clone();
        let footer_cancel_sender = sender.clone();
        let footer_wait_sender = sender.clone();
        let background_sender = sender.clone();
        let cancel_transfers_sender = sender.clone();

        dialog
            .title(t!("Dialog.confirm_close").to_string())
            .w(px(720.))
            .child(
                v_flex()
                    .gap_2()
                    .child(t!("Transfer.has_active_tasks", count = task_count).to_string())
                    .child(t!("Transfer.close_strategy_prompt").to_string()),
            )
            .on_cancel(move |_, _, _| {
                send_close_choice(&keyboard_cancel_sender, CloseChoice::Abort);
                true
            })
            .on_ok(move |_, _, _cx: &mut App| {
                send_close_choice(
                    &keyboard_wait_sender,
                    CloseChoice::Close(CloseTransferStrategy::Wait),
                );
                true
            })
            .footer(DialogFooter::new().children(vec![
                close_choice_button(
                    CloseButtonSpec {
                        id: "sftp-close-cancel",
                        label: t!("Common.cancel").to_string(),
                        choice: CloseChoice::Abort,
                        style: CloseButtonStyle::Ghost,
                    },
                    footer_cancel_sender.clone(),
                ),
                close_choice_button(
                    CloseButtonSpec {
                        id: "sftp-close-wait",
                        label: t!("Transfer.wait_and_close").to_string(),
                        choice: CloseChoice::Close(CloseTransferStrategy::Wait),
                        style: CloseButtonStyle::Primary,
                    },
                    footer_wait_sender.clone(),
                ),
                close_choice_button(
                    CloseButtonSpec {
                        id: "sftp-close-background",
                        label: t!("Transfer.continue_in_background").to_string(),
                        choice: CloseChoice::Close(CloseTransferStrategy::Background),
                        style: CloseButtonStyle::Ghost,
                    },
                    background_sender.clone(),
                ),
                close_choice_button(
                    CloseButtonSpec {
                        id: "sftp-close-cancel-transfers",
                        label: t!("Transfer.cancel_and_close").to_string(),
                        choice: CloseChoice::Close(CloseTransferStrategy::CancelTransfers),
                        style: CloseButtonStyle::Danger,
                    },
                    cancel_transfers_sender.clone(),
                ),
            ]))
            .overlay_closable(false)
            .close_button(false)
    });
}

async fn wait_for_active_transfers(view: Entity<SftpView>, cx: &mut AsyncApp) {
    loop {
        let active = view.update(cx, |this, cx| {
            this.schedule_transfers(cx);
            this.transfer_queue.has_active()
        });
        if !active {
            break;
        }
        cx.background_executor()
            .timer(Duration::from_millis(100))
            .await;
    }
}

async fn disconnect_view_clients(
    view: Entity<SftpView>,
    cx: &mut AsyncApp,
    disconnect_pool: bool,
    await_clients: bool,
) {
    let (clients, pool) = view.update(cx, |this, cx| {
        let mut clients = Vec::with_capacity(2);
        if let Some(client) = this.sftp_client.take() {
            clients.push(client);
        }
        if let Some(client) = this.take_left_remote_client(cx) {
            clients.push(client);
        }
        this.set_connection_active(false, cx);
        cx.notify();
        (clients, this.transfer_client_pool.clone())
    });

    for client in clients {
        let task = Tokio::spawn(cx, disconnect_sftp_client(client));
        if await_clients {
            let _ = task.await;
        } else {
            task.detach();
        }
    }
    if disconnect_pool {
        let task = Tokio::spawn(cx, disconnect_transfer_pool(pool));
        let _ = task.await;
    }
}

impl SftpView {
    fn start_background_transfer_cleanup(&mut self, cx: &mut Context<Self>) {
        let view = cx.entity();
        cx.spawn(async move |_this, cx| {
            wait_for_active_transfers(view.clone(), cx).await;
            disconnect_view_clients(view, cx, true, true).await;
        })
        .detach();
    }

    fn commit_close_choice(
        &mut self,
        strategy: CloseTransferStrategy,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.close_state.begin_close() {
            return false;
        }
        self.transfer_queue.freeze_admission();
        self.close_active_direct_copy_prompt(cx);
        if matches!(strategy, CloseTransferStrategy::CancelTransfers) {
            self.cancel_all_transfers(cx);
        } else {
            self.schedule_transfers(cx);
        }
        if !matches!(strategy, CloseTransferStrategy::Wait) {
            self.start_background_transfer_cleanup(cx);
        }
        cx.notify();
        true
    }
}

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
        Some(
            Icon::new(IconName::Folder1)
                .with_size(IconSize::Default)
                .color(),
        )
    }

    fn closeable(&self, _cx: &App) -> bool {
        true
    }

    fn is_disconnected(&self, _cx: &App) -> bool {
        matches!(self.connection_state, ConnectionState::Disconnected { .. })
    }

    fn connection_status(&self, _cx: &App) -> Option<one_core::tab_container::TabConnectionStatus> {
        Some(tab_connection_status(&self.connection_state))
    }

    fn try_close(
        &mut self,
        _tab_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Task<bool> {
        let active_count = self.transfer_queue.active_tasks().len();
        if active_count > 0 {
            if !self.close_state.begin_confirmation() {
                return gpui::Task::ready(false);
            }

            let (tx, rx) = tokio::sync::oneshot::channel();
            let sender = Arc::new(std::sync::Mutex::new(Some(tx)));
            open_close_strategy_dialog(active_count, window, cx, sender);

            let view = cx.entity();
            return cx.spawn(
                async move |_this, cx| match rx.await.unwrap_or(CloseChoice::Abort) {
                    CloseChoice::Abort => {
                        let _ = view.update(cx, |this, _cx| {
                            this.close_state.abort_confirmation();
                        });
                        false
                    }
                    CloseChoice::Close(strategy) => {
                        let committed =
                            view.update(cx, |this, cx| this.commit_close_choice(strategy, cx));
                        if !committed {
                            return false;
                        }

                        if matches!(strategy, CloseTransferStrategy::Wait) {
                            wait_for_active_transfers(view.clone(), cx).await;
                            disconnect_view_clients(view, cx, true, true).await;
                        } else {
                            // Cancel/background must not make tab closure wait
                            // for an in-flight listing to release the client
                            // mutex. The disconnect task owns the handle after
                            // the close decision has been committed.
                            disconnect_view_clients(view, cx, false, false).await;
                        }
                        true
                    }
                },
            );
        }

        if !self.close_state.begin_close() {
            return gpui::Task::ready(false);
        }
        self.transfer_queue.freeze_admission();
        let view = cx.entity();
        cx.spawn(async move |_this, cx| {
            disconnect_view_clients(view, cx, true, true).await;
            true
        })
    }
}

impl Focusable for SftpView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SftpView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shows_connection_overlay = matches!(
            self.connection_state,
            ConnectionState::AwaitingCredentials | ConnectionState::Disconnected { .. }
        );

        v_flex()
            .size_full()
            .relative()
            .track_focus(&self.focus_handle)
            .key_context(SFTP_VIEW_CONTEXT)
            .on_action(cx.listener(|this, _: &PasteUpload, window, cx| {
                this.paste_upload_from_clipboard(window, cx);
            }))
            .bg(cx.theme().background)
            .child(
                h_flex()
                    .flex_1()
                    .child(self.render_local_panel(cx))
                    .child(self.render_remote_panel(cx)),
            )
            .child(self.render_transfer_queue(cx))
            .when(shows_connection_overlay, |el| {
                el.child(self.render_connection_overlay(cx))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BoundedDisconnectOutcome, CloseState, ConnectionGeneration, ConnectionState,
        DirectorySizeState, FileConflictChoice, FileItem, GlobalStorageState, PendingTransfer,
        PendingUploadDecision, SftpFavoritePathRepository, SftpUploadContext, SftpView,
        SharedProgress, StoredConnection, TransferAdmission, TransferClientPool,
        TransferClientPoolState, TransferOperation, TransferQueue, TransferRefreshTarget,
        TransferTask, TransferTaskState, UploadConflictResolver, UploadConflictSession,
        acquire_transfer_client, active_external_transfer_ids, apply_global_transfer_snapshot,
        bounded_disconnect, breadcrumb_item_min_width, is_valid_entry_name, join_remote_path,
        mark_server_copy_directory_replacements, prepare_global_download,
        prepare_global_remote_delete, prepare_global_upload, reconcile_global_transfer_snapshot,
        remote_path_parent, resolve_upload_conflict, resolve_upload_conflicts,
        server_copy_conflict_flags, should_apply_local_listing, should_apply_remote_listing,
        should_refresh_transfer_target, tab_connection_status, transfer_error_summary,
        transfer_task_state_from_global, upload_directory_conflict_policy,
    };
    use anyhow::{Result, anyhow};
    use async_trait::async_trait;
    use gpui::{AppContext, Entity, TestAppContext, WindowHandle};
    use gpui_component::Root;
    use one_core::storage::connection::SqliteConnection;
    use one_core::storage::models::SshAuthMethod;
    use one_core::storage::{SshParams, StorageManager, migration::run_migrations};
    use one_core::tab_container::TabConnectionStatus;
    use sftp::ProgressCallback;
    use sftp::{DirectoryConflictPolicy, ServerCopyItem};
    use sftp_transfer::{
        SftpConnectionIdentity, SftpDeleteRemoteExecution, SftpDownloadExecution,
        SftpRemoteDeleteEntry, SftpTransferEvent, SftpTransferExecutor, SftpTransferId,
        SftpTransferOperation, SftpTransferProvider, SftpTransferSnapshot, SftpTransferState,
        SftpUploadConnection, SftpUploadExecution, download_task_key,
    };
    use ssh::{HostKeyVerifier, SshAuth, SshConnectConfig};
    use std::cell::RefCell;
    use std::collections::{HashMap, HashSet};
    use std::path::{Path, PathBuf};
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;
    use std::time::SystemTime;
    use tokio::sync::oneshot;

    #[test]
    fn root_breadcrumb_does_not_reserve_regular_item_width() {
        assert_eq!(breadcrumb_item_min_width("/"), 0.);
        assert_eq!(breadcrumb_item_min_width("home"), 35.);
    }

    #[test]
    fn awaiting_credentials_uses_connecting_tab_status() {
        assert_eq!(
            TabConnectionStatus::Connecting,
            tab_connection_status(&ConnectionState::AwaitingCredentials)
        );
    }

    fn transfer_task(id: usize) -> TransferTask {
        TransferTask {
            id,
            external_transfer_id: None,
            operation: TransferOperation::DeleteLocal {
                entries: Vec::new(),
                local_dir: PathBuf::from("."),
            },
            state: TransferTaskState::Pending,
            shared_progress: Arc::new(SharedProgress {
                transferred: AtomicU64::new(0),
                total: AtomicU64::new(0),
                speed: AtomicU64::new(0),
                cancelled: Arc::new(AtomicBool::new(false)),
                scanning: AtomicBool::new(false),
                current_file: std::sync::RwLock::new(None),
                current_file_transferred: AtomicU64::new(0),
                current_file_total: AtomicU64::new(0),
            }),
            error: None,
            background_task: None,
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum SftpViewTestOperation {
        Upload,
        Download,
        DeleteRemote,
    }

    #[derive(Clone, Default)]
    struct SftpViewTestProvider {
        state: Arc<Mutex<SftpViewTestProviderState>>,
    }

    #[derive(Default)]
    struct SftpViewTestProviderState {
        started: Vec<SftpTransferId>,
        operations: HashMap<SftpTransferId, SftpViewTestOperation>,
        paths: HashMap<SftpTransferId, (String, PathBuf, bool)>,
        delete_entries: HashMap<SftpTransferId, Vec<SftpRemoteDeleteEntry>>,
        completions: HashMap<SftpTransferId, oneshot::Sender<Result<()>>>,
        progress: HashMap<SftpTransferId, ProgressCallback>,
        cancellations: HashMap<SftpTransferId, Arc<AtomicBool>>,
    }

    struct SftpViewTestExecution {
        id: SftpTransferId,
        operation: SftpViewTestOperation,
        remote_path: String,
        local_path: PathBuf,
        is_dir: bool,
        cancelled: Arc<AtomicBool>,
    }

    impl SftpViewTestProvider {
        fn started(&self) -> Vec<SftpTransferId> {
            self.state.lock().unwrap().started.clone()
        }

        fn operation(&self, id: SftpTransferId) -> Option<SftpViewTestOperation> {
            self.state.lock().unwrap().operations.get(&id).copied()
        }

        fn paths(&self, id: SftpTransferId) -> Option<(String, PathBuf, bool)> {
            self.state.lock().unwrap().paths.get(&id).cloned()
        }

        fn delete_entries(&self, id: SftpTransferId) -> Option<Vec<SftpRemoteDeleteEntry>> {
            self.state.lock().unwrap().delete_entries.get(&id).cloned()
        }

        fn complete(&self, id: SftpTransferId, result: Result<()>) {
            let sender = {
                let mut state = self.state.lock().unwrap();
                state.progress.remove(&id);
                state.cancellations.remove(&id);
                state
                    .completions
                    .remove(&id)
                    .expect("transfer should be waiting for completion")
            };
            let _ = sender.send(result);
        }

        fn is_cancelled(&self, id: SftpTransferId) -> bool {
            self.state
                .lock()
                .unwrap()
                .cancellations
                .get(&id)
                .is_some_and(|cancelled| cancelled.load(Ordering::Relaxed))
        }

        async fn run(
            &self,
            execution: SftpViewTestExecution,
            progress: ProgressCallback,
        ) -> Result<()> {
            let receiver = {
                let (sender, receiver) = oneshot::channel();
                let mut state = self.state.lock().unwrap();
                state.started.push(execution.id);
                state.operations.insert(execution.id, execution.operation);
                state.paths.insert(
                    execution.id,
                    (
                        execution.remote_path,
                        execution.local_path,
                        execution.is_dir,
                    ),
                );
                state.completions.insert(execution.id, sender);
                state.progress.insert(execution.id, progress);
                state
                    .cancellations
                    .insert(execution.id, execution.cancelled);
                receiver
            };
            receiver
                .await
                .map_err(|_| anyhow!("test completion channel closed"))?
        }
    }

    #[async_trait]
    impl SftpTransferProvider for SftpViewTestProvider {
        async fn upload(
            &self,
            execution: SftpUploadExecution,
            progress: ProgressCallback,
        ) -> Result<()> {
            self.run(
                SftpViewTestExecution {
                    id: execution.id,
                    operation: SftpViewTestOperation::Upload,
                    remote_path: execution.remote_path,
                    local_path: execution.local_path,
                    is_dir: execution.is_dir,
                    cancelled: execution.cancelled,
                },
                progress,
            )
            .await
        }

        async fn download(
            &self,
            execution: SftpDownloadExecution,
            progress: ProgressCallback,
        ) -> Result<()> {
            self.run(
                SftpViewTestExecution {
                    id: execution.id,
                    operation: SftpViewTestOperation::Download,
                    remote_path: execution.remote_path,
                    local_path: execution.local_path,
                    is_dir: execution.is_dir,
                    cancelled: execution.cancelled,
                },
                progress,
            )
            .await
        }

        async fn delete_remote(
            &self,
            execution: SftpDeleteRemoteExecution,
            progress: ProgressCallback,
        ) -> Result<()> {
            self.state
                .lock()
                .unwrap()
                .delete_entries
                .insert(execution.id, execution.entries);
            self.run(
                SftpViewTestExecution {
                    id: execution.id,
                    operation: SftpViewTestOperation::DeleteRemote,
                    remote_path: execution.remote_dir,
                    local_path: PathBuf::new(),
                    is_dir: false,
                    cancelled: execution.cancelled,
                },
                progress,
            )
            .await
        }
    }

    fn test_stored_connection() -> StoredConnection {
        StoredConnection::new_ssh(
            "SFTP entity test".to_string(),
            SshParams {
                sftp_default_directory: None,
                disabled_jump_server: None,
                sftp_account: None,
                host: "sftp-entity-test.internal".to_string(),
                port: 2222,
                username: "deploy".to_string(),
                auth_method: SshAuthMethod::Agent,
                credential_reference: None,
                prompt_username: Some(true),
                prompt_password: None,
                keyboard_interactive: None,
                terminal_encoding: Default::default(),
                terminal_type: Default::default(),
                connect_timeout: Some(1),
                keepalive_interval: None,
                keepalive_max: None,
                default_directory: None,
                init_script: None,
                disable_shell_integration: None,
                x11_forwarding: None,
                allow_legacy_algorithms: Some(false),
                jump_server: None,
                proxy: None,
                os_id: None,
                icon: None,
                icon_file_path: None,
                account_expect: Default::default(),
            },
            None,
        )
    }

    fn wait_until(cx: &mut TestAppContext, mut predicate: impl FnMut(&TestAppContext) -> bool) {
        for _ in 0..100 {
            cx.run_until_parked();
            if predicate(cx) {
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("condition was not reached before timeout");
    }

    fn sftp_view_fixture(
        cx: &mut TestAppContext,
        temp_dir: &tempfile::TempDir,
    ) -> (
        SftpViewTestProvider,
        Entity<SftpTransferExecutor>,
        Entity<SftpView>,
        WindowHandle<Root>,
    ) {
        cx.update(one_core::gpui_tokio::init);
        cx.update(one_core::background_tasks::init);
        cx.executor().allow_parking();

        let db_path = temp_dir.path().join("sftp-view-fixture.db");
        let conn = SqliteConnection::open_with_pool_size(&db_path, 1)
            .expect("open isolated fixture database");
        conn.with_connection(|conn| run_migrations(conn))
            .expect("run isolated fixture migrations");
        let storage = StorageManager::new_with_connection(conn);
        storage.register(SftpFavoritePathRepository::new(storage.connection()));
        cx.update(|cx| {
            cx.set_global(GlobalStorageState { storage });
            gpui_component::init(cx);
            cx.set_global(gpui_component::Theme::default());
        });

        let provider = SftpViewTestProvider::default();
        let executor =
            cx.update(|cx| sftp_transfer::init_with_provider(cx, Arc::new(provider.clone())));

        let mut view_slot = None;
        let window = cx.open_window(Default::default(), |window, cx| {
            let view = cx.new(|cx| SftpView::new(test_stored_connection(), window, cx));
            view_slot = Some(view.clone());
            Root::new(view, window, cx)
        });

        let view = view_slot.expect("SFTP view fixture created");
        cx.update(|cx| {
            window
                .update(cx, |_, _, cx| {
                    view.update(cx, |this, cx| {
                        this.local_current_path = temp_dir.path().to_path_buf();
                        this.remote_current_path = "/remote".to_string();
                        this.refresh_local_dir(cx);
                        cx.notify();
                    });
                })
                .expect("SFTP fixture window remains open");
        });

        (provider, executor, view, window)
    }

    fn submit_upload(
        cx: &mut TestAppContext,
        view: &Entity<SftpView>,
        name: &str,
    ) -> SftpTransferId {
        let name = name.to_string();
        view.update(cx, |this, cx| {
            let local_path = PathBuf::from(format!("/tmp/entity/{name}"));
            let remote_path = format!("/remote/{name}");
            let transfer = PendingTransfer {
                name,
                local_path,
                remote_path,
                is_dir: false,
                has_conflict: false,
            };
            this.execute_uploads_with_directory_policy(
                vec![transfer],
                DirectoryConflictPolicy::Merge,
                cx,
            );
            this.transfer_queue
                .tasks
                .last()
                .and_then(|task| task.external_transfer_id)
                .expect("upload mirror should use an external transfer ID")
        })
    }

    fn submit_download(
        cx: &mut TestAppContext,
        view: &Entity<SftpView>,
        local_path: PathBuf,
    ) -> SftpTransferId {
        view.update(cx, |this, cx| {
            let transfer = PendingTransfer {
                name: "report.txt".to_string(),
                local_path,
                remote_path: "/remote/report.txt".to_string(),
                is_dir: false,
                has_conflict: false,
            };
            this.execute_downloads(vec![transfer], cx);
            this.transfer_queue
                .tasks
                .last()
                .and_then(|task| task.external_transfer_id)
                .expect("download mirror should use an external transfer ID")
        })
    }

    fn submit_remote_delete(
        cx: &mut TestAppContext,
        view: &Entity<SftpView>,
        window_handle: &WindowHandle<Root>,
    ) -> SftpTransferId {
        window_handle
            .update(cx, |_root, window, cx| {
                view.update(cx, |this, cx| {
                    this.execute_remote_delete(
                        vec![test_remote_file_item("report.txt", false)],
                        "/remote".to_string(),
                        window,
                        cx,
                    );
                    this.transfer_queue
                        .tasks
                        .last()
                        .and_then(|task| task.external_transfer_id)
                        .expect("remote delete mirror should use an external transfer ID")
                })
            })
            .expect("SFTP test window remains open")
    }

    fn test_remote_file_item(name: &str, is_dir: bool) -> FileItem {
        FileItem {
            name: name.to_string(),
            size: if is_dir { 0 } else { 128 },
            modified: SystemTime::UNIX_EPOCH,
            is_dir,
            permissions: "-rw-r--r--".to_string(),
            owner: Some("deploy".to_string()),
            directory_size: DirectorySizeState::Unknown,
        }
    }

    fn mirror_snapshot(
        cx: &TestAppContext,
        view: &Entity<SftpView>,
        transfer_id: SftpTransferId,
    ) -> (TransferTaskState, Option<String>) {
        view.read_with(cx, |view, _| {
            let task = view
                .transfer_queue
                .tasks
                .iter()
                .find(|task| task.external_transfer_id == Some(transfer_id))
                .expect("view mirror should retain the external transfer");
            (task.state.clone(), task.error.clone())
        })
    }

    #[gpui::test]
    fn sftp_view_upload_submission_reaches_global_executor_and_mirror(cx: &mut TestAppContext) {
        let temp_dir = tempfile::tempdir().expect("create temporary fixture directory");
        let (provider, executor, view, _window) = sftp_view_fixture(cx, &temp_dir);

        let transfer_id = submit_upload(cx, &view, "upload.txt");
        wait_until(cx, |_| provider.started().contains(&transfer_id));

        assert_eq!(
            Some((
                String::from("/remote/upload.txt"),
                PathBuf::from("/tmp/entity/upload.txt"),
                false
            )),
            provider.paths(transfer_id)
        );
        assert_eq!(
            Some(SftpViewTestOperation::Upload),
            provider.operation(transfer_id)
        );
        assert!(matches!(
            mirror_snapshot(cx, &view, transfer_id),
            (
                TransferTaskState::Pending | TransferTaskState::Running,
                None
            )
        ));
        assert_eq!(
            Some(SftpTransferState::Running),
            executor.read_with(cx, |executor, _| {
                executor
                    .snapshot(transfer_id)
                    .map(|snapshot| snapshot.state)
            })
        );

        provider.complete(transfer_id, Ok(()));
        wait_until(cx, |cx| {
            matches!(
                mirror_snapshot(cx, &view, transfer_id),
                (TransferTaskState::Completed, _)
            )
        });
        assert_eq!(
            Some(SftpTransferState::Succeeded),
            executor.read_with(cx, |executor, _| {
                executor
                    .snapshot(transfer_id)
                    .map(|snapshot| snapshot.state)
            })
        );
    }

    #[gpui::test]
    fn sftp_view_download_submission_refreshes_local_directory(cx: &mut TestAppContext) {
        let temp_dir = tempfile::tempdir().expect("create temporary fixture directory");
        let (provider, _executor, view, _window) = sftp_view_fixture(cx, &temp_dir);
        let local_path = temp_dir.path().join("downloaded.txt");

        let transfer_id = submit_download(cx, &view, local_path.clone());
        wait_until(cx, |_| provider.started().contains(&transfer_id));

        assert_eq!(
            Some((
                String::from("/remote/report.txt"),
                local_path.clone(),
                false
            )),
            provider.paths(transfer_id)
        );
        assert_eq!(
            Some(SftpViewTestOperation::Download),
            provider.operation(transfer_id)
        );

        std::fs::write(&local_path, "downloaded").expect("simulate committed local download");
        provider.complete(transfer_id, Ok(()));
        wait_until(cx, |cx| {
            matches!(
                mirror_snapshot(cx, &view, transfer_id),
                (TransferTaskState::Completed, _)
            )
        });

        view.read_with(cx, |view, cx| {
            assert!(
                view.local_panel
                    .read(cx)
                    .items()
                    .iter()
                    .any(|item| { item.name == "downloaded.txt" && !item.is_dir })
            );
        });
    }

    #[gpui::test]
    fn sftp_view_remote_delete_submission_reaches_provider(cx: &mut TestAppContext) {
        let temp_dir = tempfile::tempdir().expect("create temporary fixture directory");
        let (provider, _executor, view, window) = sftp_view_fixture(cx, &temp_dir);

        let transfer_id = submit_remote_delete(cx, &view, &window);
        wait_until(cx, |_| provider.started().contains(&transfer_id));

        assert_eq!(
            Some(vec![SftpRemoteDeleteEntry {
                remote_path: "/remote/report.txt".to_string(),
                is_dir: false,
            }]),
            provider.delete_entries(transfer_id)
        );
        assert_eq!(
            Some(SftpViewTestOperation::DeleteRemote),
            provider.operation(transfer_id)
        );

        provider.complete(transfer_id, Ok(()));
        wait_until(cx, |cx| {
            matches!(
                mirror_snapshot(cx, &view, transfer_id),
                (TransferTaskState::Completed, _)
            )
        });
    }

    #[gpui::test]
    fn frozen_sftp_view_admission_never_creates_executor_tasks(cx: &mut TestAppContext) {
        let temp_dir = tempfile::tempdir().expect("create temporary fixture directory");
        let (provider, executor, view, window) = sftp_view_fixture(cx, &temp_dir);

        window
            .update(cx, |_root, window, cx| {
                view.update(cx, |view, cx| {
                    view.transfer_queue.freeze_admission();
                    view.execute_uploads_with_directory_policy(
                        vec![PendingTransfer {
                            name: "frozen.txt".to_string(),
                            local_path: PathBuf::from("/tmp/entity/frozen.txt"),
                            remote_path: "/remote/frozen.txt".to_string(),
                            is_dir: false,
                            has_conflict: false,
                        }],
                        DirectoryConflictPolicy::Merge,
                        cx,
                    );
                    view.execute_downloads(
                        vec![PendingTransfer {
                            name: "frozen.txt".to_string(),
                            local_path: temp_dir.path().join("frozen.txt"),
                            remote_path: "/remote/frozen.txt".to_string(),
                            is_dir: false,
                            has_conflict: false,
                        }],
                        cx,
                    );
                    view.execute_remote_delete(
                        vec![test_remote_file_item("frozen.txt", false)],
                        "/remote".to_string(),
                        window,
                        cx,
                    );
                });
            })
            .expect("SFTP test window remains open");
        cx.run_until_parked();

        assert!(view.read_with(cx, |view, _| view.transfer_queue.tasks.is_empty()));
        assert!(provider.started().is_empty());
        executor.read_with(cx, |executor, _| {
            for id in 1..=3 {
                assert!(executor.snapshot(SftpTransferId::new(id)).is_none());
            }
        });
    }

    #[gpui::test]
    fn provider_failure_is_reflected_in_sftp_view_mirror(cx: &mut TestAppContext) {
        let temp_dir = tempfile::tempdir().expect("create temporary fixture directory");
        let (provider, executor, view, _window) = sftp_view_fixture(cx, &temp_dir);
        let transfer_id = submit_upload(cx, &view, "failed.txt");
        wait_until(cx, |_| provider.started().contains(&transfer_id));

        provider.complete(transfer_id, Err(anyhow!("entity upload failed")));
        wait_until(cx, |cx| {
            matches!(
                mirror_snapshot(cx, &view, transfer_id),
                (TransferTaskState::Failed, Some(_))
            )
        });
        assert_eq!(
            Some(SftpTransferState::Failed),
            executor.read_with(cx, |executor, _| {
                executor
                    .snapshot(transfer_id)
                    .map(|snapshot| snapshot.state)
            })
        );
    }

    #[gpui::test]
    fn queued_sftp_transfer_can_be_cancelled_before_provider_start(cx: &mut TestAppContext) {
        let temp_dir = tempfile::tempdir().expect("create temporary fixture directory");
        let (provider, _executor, view, _window) = sftp_view_fixture(cx, &temp_dir);
        let first = submit_upload(cx, &view, "first.txt");
        let second = submit_upload(cx, &view, "second.txt");

        let second_task_id = view.read_with(cx, |view, _| {
            view.transfer_queue
                .tasks
                .iter()
                .find(|task| task.external_transfer_id == Some(second))
                .map(|task| task.id)
                .expect("queued transfer should have a view task ID")
        });
        view.update(cx, |view, cx| {
            view.cancel_transfer(second_task_id, cx);
        });
        wait_until(cx, |cx| {
            matches!(
                mirror_snapshot(cx, &view, second),
                (TransferTaskState::Cancelled, _)
            )
        });

        wait_until(cx, |_| provider.started().contains(&first));
        provider.complete(first, Ok(()));
        wait_until(cx, |cx| {
            matches!(
                mirror_snapshot(cx, &view, first),
                (TransferTaskState::Completed, _)
            )
        });

        assert_eq!(vec![first], provider.started());
    }

    #[gpui::test]
    fn running_sftp_transfer_cancel_reaches_provider(cx: &mut TestAppContext) {
        let temp_dir = tempfile::tempdir().expect("create temporary fixture directory");
        let (provider, _executor, view, _window) = sftp_view_fixture(cx, &temp_dir);
        let transfer_id = submit_upload(cx, &view, "running-cancel.txt");
        wait_until(cx, |_| provider.started().contains(&transfer_id));
        let task_id = view.read_with(cx, |view, _| {
            view.transfer_queue
                .tasks
                .iter()
                .find(|task| task.external_transfer_id == Some(transfer_id))
                .map(|task| task.id)
                .expect("running transfer should have a view task ID")
        });

        view.update(cx, |view, cx| {
            view.cancel_transfer(task_id, cx);
        });

        wait_until(cx, |_| provider.is_cancelled(transfer_id));
        provider.complete(transfer_id, Ok(()));
        wait_until(cx, |cx| {
            matches!(
                mirror_snapshot(cx, &view, transfer_id),
                (TransferTaskState::Cancelled, _)
            )
        });
    }

    #[test]
    fn tracked_external_transfer_is_active_but_not_startable() {
        let mut queue = TransferQueue::new(1);
        let mut task = transfer_task(0);
        task.external_transfer_id = Some(SftpTransferId::new(7));

        assert!(queue.track_external(task));
        assert!(queue.has_active());
        assert_eq!(1, queue.active_tasks().len());
        assert!(queue.next_startable().is_empty());
        assert!(queue.pending.is_empty());
    }

    #[test]
    fn frozen_queue_rejects_external_transfer_tracking() {
        let mut queue = TransferQueue::new(1);
        queue.freeze_admission();
        let mut task = transfer_task(0);
        task.external_transfer_id = Some(SftpTransferId::new(7));

        assert!(!queue.track_external(task));
        assert!(queue.tasks.is_empty());
    }

    #[test]
    fn external_transfer_mirror_can_be_rolled_back_after_failed_commit() {
        let mut queue = TransferQueue::new(1);
        let mut first = transfer_task(0);
        first.external_transfer_id = Some(SftpTransferId::new(7));
        let mut second = transfer_task(1);
        second.external_transfer_id = Some(SftpTransferId::new(8));

        assert!(queue.track_external(first));
        assert!(queue.track_external(second));
        assert!(queue.untrack_external(SftpTransferId::new(7)));
        assert!(!queue.untrack_external(SftpTransferId::new(7)));
        assert_eq!(queue.tasks.len(), 1);
        assert_eq!(
            queue.tasks[0].external_transfer_id,
            Some(SftpTransferId::new(8))
        );
    }

    #[test]
    fn cancel_all_leaves_external_transfers_for_executor_events() {
        let mut queue = TransferQueue::new(2);
        let mut pending_external = transfer_task(0);
        pending_external.external_transfer_id = Some(SftpTransferId::new(7));
        let pending_cancelled = pending_external.shared_progress.cancelled.clone();
        let mut running_external = transfer_task(1);
        running_external.external_transfer_id = Some(SftpTransferId::new(8));
        running_external.state = TransferTaskState::Running;
        let running_cancelled = running_external.shared_progress.cancelled.clone();

        assert!(queue.track_external(pending_external));
        assert!(queue.track_external(running_external));
        queue.cancel_all();

        assert!(matches!(queue.tasks[0].state, TransferTaskState::Pending));
        assert!(matches!(queue.tasks[1].state, TransferTaskState::Running));
        assert!(!pending_cancelled.load(Ordering::Relaxed));
        assert!(!running_cancelled.load(Ordering::Relaxed));
    }

    #[test]
    fn mixed_queue_never_starts_external_transfer_locally() {
        let mut queue = TransferQueue::new(2);
        assert!(queue.enqueue(transfer_task(1)));
        let mut external = transfer_task(2);
        external.external_transfer_id = Some(SftpTransferId::new(7));
        assert!(queue.track_external(external));

        let startable = queue.next_startable();

        assert_eq!(
            vec![1],
            startable.iter().map(|task| task.id).collect::<Vec<_>>()
        );
        assert!(queue.pending.is_empty());
        assert_eq!(TransferTaskState::Pending, queue.tasks[1].state);
    }

    #[test]
    fn bottom_queue_shows_all_active_tasks_including_uploads_and_downloads() {
        let mut queue = TransferQueue::new(2);
        let mut upload = transfer_task(1);
        upload.external_transfer_id = Some(SftpTransferId::new(7));
        upload.operation = TransferOperation::Upload {
            local_path: PathBuf::from("/local/file.txt"),
            remote_path: "/remote/file.txt".to_string(),
            is_dir: false,
            directory_conflict_policy: DirectoryConflictPolicy::Merge,
            remote_dir: "/remote".to_string(),
        };
        let mut download = transfer_task(2);
        download.external_transfer_id = Some(SftpTransferId::new(8));
        download.operation = TransferOperation::Download {
            remote_path: "/remote/file.txt".to_string(),
            local_dir: PathBuf::from("/local"),
        };
        let local_delete = TransferTask {
            operation: TransferOperation::DeleteLocal {
                entries: Vec::new(),
                local_dir: PathBuf::from("/local"),
            },
            ..transfer_task(3)
        };

        assert!(queue.track_external(upload));
        assert!(queue.track_external(download));
        assert!(queue.enqueue(local_delete));

        let visible = queue.bottom_visible_tasks();

        assert_eq!(
            vec![1, 2, 3],
            visible.iter().map(|task| task.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn active_external_transfer_ids_exclude_terminal_and_internal_tasks() {
        let mut pending_external = transfer_task(1);
        pending_external.external_transfer_id = Some(SftpTransferId::new(7));
        let mut running_external = transfer_task(2);
        running_external.external_transfer_id = Some(SftpTransferId::new(8));
        running_external.state = TransferTaskState::Running;
        let mut completed_external = transfer_task(3);
        completed_external.external_transfer_id = Some(SftpTransferId::new(9));
        completed_external.state = TransferTaskState::Completed;
        let internal = transfer_task(4);

        assert_eq!(
            vec![SftpTransferId::new(7), SftpTransferId::new(8)],
            active_external_transfer_ids(&[
                pending_external,
                running_external,
                completed_external,
                internal,
            ])
        );
    }

    #[test]
    fn transfer_refresh_requires_first_finished_transition() {
        let id = SftpTransferId::new(7);

        assert!(!should_refresh_transfer_target(
            true,
            &SftpTransferEvent::Updated(id),
            &SftpTransferState::Succeeded,
        ));
        assert!(should_refresh_transfer_target(
            true,
            &SftpTransferEvent::Finished(id),
            &SftpTransferState::Succeeded,
        ));
        assert!(should_refresh_transfer_target(
            true,
            &SftpTransferEvent::Finished(id),
            &SftpTransferState::Cancelled,
        ));
        assert!(should_refresh_transfer_target(
            true,
            &SftpTransferEvent::Finished(id),
            &SftpTransferState::Failed,
        ));
        assert!(!should_refresh_transfer_target(
            false,
            &SftpTransferEvent::Finished(id),
            &SftpTransferState::Succeeded,
        ));
    }

    #[test]
    fn reconciled_terminal_upload_snapshot_updates_mirror_and_requests_refresh() {
        let mut task = transfer_task(0);
        task.external_transfer_id = Some(SftpTransferId::new(7));
        task.operation = TransferOperation::Upload {
            local_path: PathBuf::from("/tmp/archive"),
            remote_path: "/srv/archive".to_string(),
            is_dir: true,
            directory_conflict_policy: DirectoryConflictPolicy::Merge,
            remote_dir: "/srv".to_string(),
        };
        let snapshot = SftpTransferSnapshot {
            id: SftpTransferId::new(7),
            operation: SftpTransferOperation::Upload,
            connection: SftpConnectionIdentity::Runtime(3),
            local_path: PathBuf::from("/tmp/archive"),
            remote_path: "/srv/archive".to_string(),
            display_name: "archive".to_string(),
            state: SftpTransferState::Succeeded,
            transferred: 512,
            total: Some(512),
            speed: 0.0,
            current_file: None,
            error: None,
        };

        assert_eq!(
            Some(TransferRefreshTarget::Remote("/srv".to_string())),
            reconcile_global_transfer_snapshot(&mut task, &snapshot)
        );
        assert_eq!(TransferTaskState::Completed, task.state);
        assert_eq!(
            512,
            task.shared_progress.transferred.load(Ordering::Relaxed)
        );
        assert_eq!(
            None,
            reconcile_global_transfer_snapshot(&mut task, &snapshot)
        );
    }

    #[test]
    fn reconciled_terminal_download_snapshot_updates_mirror_and_requests_local_refresh() {
        let mut task = transfer_task(0);
        task.external_transfer_id = Some(SftpTransferId::new(8));
        task.operation = TransferOperation::Download {
            remote_path: "/srv/archive".to_string(),
            local_dir: PathBuf::from("/tmp"),
        };
        let snapshot = SftpTransferSnapshot {
            id: SftpTransferId::new(8),
            operation: SftpTransferOperation::Download,
            connection: SftpConnectionIdentity::Runtime(3),
            local_path: PathBuf::from("/tmp/archive"),
            remote_path: "/srv/archive".to_string(),
            display_name: "archive".to_string(),
            state: SftpTransferState::Succeeded,
            transferred: 512,
            total: Some(512),
            speed: 0.0,
            current_file: None,
            error: None,
        };

        assert_eq!(
            Some(TransferRefreshTarget::Local(PathBuf::from("/tmp"))),
            reconcile_global_transfer_snapshot(&mut task, &snapshot)
        );
        assert_eq!(TransferTaskState::Completed, task.state);
        assert_eq!(
            None,
            reconcile_global_transfer_snapshot(&mut task, &snapshot)
        );
    }

    #[test]
    fn mismatched_snapshot_operation_does_not_update_or_refresh_mirror() {
        let mut task = transfer_task(0);
        task.operation = TransferOperation::Download {
            remote_path: "/srv/archive".to_string(),
            local_dir: PathBuf::from("/tmp"),
        };
        let snapshot = SftpTransferSnapshot {
            id: SftpTransferId::new(8),
            operation: SftpTransferOperation::Upload,
            connection: SftpConnectionIdentity::Runtime(3),
            local_path: PathBuf::from("/tmp/archive"),
            remote_path: "/srv/archive".to_string(),
            display_name: "archive".to_string(),
            state: SftpTransferState::Succeeded,
            transferred: 512,
            total: Some(512),
            speed: 0.0,
            current_file: None,
            error: None,
        };

        assert_eq!(
            None,
            reconcile_global_transfer_snapshot(&mut task, &snapshot)
        );
        assert_eq!(TransferTaskState::Pending, task.state);
        assert_eq!(0, task.shared_progress.transferred.load(Ordering::Relaxed));
    }

    #[test]
    fn remote_path_parent_handles_absolute_and_relative_upload_targets() {
        assert_eq!("/", remote_path_parent("/file"));
        assert_eq!("/dir", remote_path_parent("/dir/file"));
        assert_eq!(".", remote_path_parent("file"));
        assert_eq!("dir", remote_path_parent("dir/file"));
        assert_eq!("/dir", remote_path_parent("/dir/file/"));
    }

    #[test]
    fn global_transfer_states_map_to_local_mirror_states() {
        assert_eq!(
            TransferTaskState::Pending,
            transfer_task_state_from_global(&SftpTransferState::Queued)
        );
        assert_eq!(
            TransferTaskState::Running,
            transfer_task_state_from_global(&SftpTransferState::Running)
        );
        assert_eq!(
            TransferTaskState::Running,
            transfer_task_state_from_global(&SftpTransferState::Cancelling)
        );
        assert_eq!(
            TransferTaskState::Completed,
            transfer_task_state_from_global(&SftpTransferState::Succeeded)
        );
        assert_eq!(
            TransferTaskState::Failed,
            transfer_task_state_from_global(&SftpTransferState::Failed)
        );
        assert_eq!(
            TransferTaskState::Cancelled,
            transfer_task_state_from_global(&SftpTransferState::Cancelled)
        );
    }

    #[test]
    fn global_upload_snapshot_updates_local_progress_mirror() {
        let mut task = transfer_task(0);
        let snapshot = SftpTransferSnapshot {
            id: SftpTransferId::new(7),
            operation: SftpTransferOperation::Upload,
            connection: SftpConnectionIdentity::Runtime(3),
            local_path: PathBuf::from("/tmp/archive.zip"),
            remote_path: "/srv/archive.zip".to_string(),
            display_name: "archive.zip".to_string(),
            state: SftpTransferState::Running,
            transferred: 128,
            total: Some(512),
            speed: 42.5,
            current_file: Some("nested/file.txt".to_string()),
            error: Some("transient".to_string()),
        };

        apply_global_transfer_snapshot(&mut task, &snapshot);

        assert_eq!(TransferTaskState::Running, task.state);
        assert_eq!(Some("transient"), task.error.as_deref());
        assert_eq!(
            128,
            task.shared_progress.transferred.load(Ordering::Relaxed)
        );
        assert_eq!(512, task.shared_progress.total.load(Ordering::Relaxed));
        assert_eq!(
            42.5,
            f64::from_bits(task.shared_progress.speed.load(Ordering::Relaxed))
        );
        assert_eq!(
            Some("nested/file.txt".to_string()),
            task.shared_progress.current_file.read().unwrap().clone()
        );
        assert!(!task.shared_progress.scanning.load(Ordering::Relaxed));
        assert_eq!(
            128,
            task.shared_progress
                .current_file_transferred
                .load(Ordering::Relaxed)
        );
        assert_eq!(
            512,
            task.shared_progress
                .current_file_total
                .load(Ordering::Relaxed)
        );
    }

    #[test]
    fn prepared_global_upload_preserves_request_and_remote_parent() {
        let context = SftpUploadContext {
            connection: SftpConnectionIdentity::Runtime(3),
            connection_source: SftpUploadConnection::Config(transfer_pool_config()),
            task_group: "Test SFTP".into(),
        };
        let transfer = PendingTransfer {
            name: "archive".to_string(),
            local_path: PathBuf::from("/local/archive"),
            remote_path: "/srv/backups/archive".to_string(),
            is_dir: true,
            has_conflict: true,
        };

        let prepared = prepare_global_upload(&transfer, DirectoryConflictPolicy::Replace, &context);

        assert_eq!(context.connection, prepared.request.connection);
        assert_eq!(transfer.local_path, prepared.request.local_path);
        assert_eq!(transfer.remote_path, prepared.request.remote_path);
        assert!(prepared.request.is_dir);
        assert_eq!(
            DirectoryConflictPolicy::Replace,
            prepared.request.directory_conflict_policy
        );
        assert_eq!("archive", prepared.request.display_name);
        assert!(matches!(
            prepared.operation,
            TransferOperation::Upload { ref remote_dir, .. } if remote_dir == "/srv/backups"
        ));
    }

    #[test]
    fn prepared_global_download_preserves_request_and_local_parent() {
        let context = SftpUploadContext {
            connection: SftpConnectionIdentity::Runtime(3),
            connection_source: SftpUploadConnection::Config(transfer_pool_config()),
            task_group: "Test SFTP".into(),
        };
        let transfer = PendingTransfer {
            name: "archive".to_string(),
            local_path: PathBuf::from("/local/backups/archive"),
            remote_path: "/srv/archive".to_string(),
            is_dir: true,
            has_conflict: true,
        };

        let prepared = prepare_global_download(&transfer, &context);

        assert_eq!(context.connection, prepared.request.connection);
        assert_eq!(transfer.local_path, prepared.request.local_path);
        assert_eq!(transfer.remote_path, prepared.request.remote_path);
        assert!(prepared.request.is_dir);
        assert_eq!("archive", prepared.request.display_name);
        assert_eq!(
            Some(download_task_key(
                &context.connection,
                &transfer.remote_path,
                &transfer.local_path,
            )),
            prepared.request.task_key
        );
        assert!(matches!(
            prepared.operation,
            TransferOperation::Download { ref local_dir, .. }
                if local_dir == Path::new("/local/backups")
        ));
    }

    #[test]
    fn prepared_global_remote_delete_preserves_entries_and_remote_dir() {
        let context = SftpUploadContext {
            connection: SftpConnectionIdentity::Runtime(3),
            connection_source: SftpUploadConnection::Config(transfer_pool_config()),
            task_group: "Test SFTP".into(),
        };
        let entries = vec![
            remote_delete_entry("archive", true),
            remote_delete_entry("readme.txt", false),
        ];

        let prepared = prepare_global_remote_delete(&entries, "/srv/backups", &context);

        assert_eq!(context.connection, prepared.request.connection);
        assert_eq!("/srv/backups", prepared.request.remote_dir);
        assert_eq!(2, prepared.request.entries.len());
        assert_eq!(
            "/srv/backups/archive",
            prepared.request.entries[0].remote_path
        );
        assert!(prepared.request.entries[0].is_dir);
        assert_eq!(
            "/srv/backups/readme.txt",
            prepared.request.entries[1].remote_path
        );
        assert!(!prepared.request.entries[1].is_dir);
        assert!(matches!(
            prepared.operation,
            TransferOperation::DeleteRemote {
                ref entries,
                ref remote_dir,
            } if entries.len() == 2 && remote_dir == "/srv/backups"
        ));
    }

    #[test]
    fn reconciled_terminal_remote_delete_snapshot_requests_remote_refresh() {
        let mut task = transfer_task(0);
        task.external_transfer_id = Some(SftpTransferId::new(9));
        task.operation = TransferOperation::DeleteRemote {
            entries: vec![remote_delete_entry("archive", true)],
            remote_dir: "/srv/backups".to_string(),
        };
        let snapshot = SftpTransferSnapshot {
            id: SftpTransferId::new(9),
            operation: SftpTransferOperation::DeleteRemote,
            connection: SftpConnectionIdentity::Runtime(3),
            local_path: PathBuf::new(),
            remote_path: "/srv/backups".to_string(),
            display_name: "archive".to_string(),
            state: SftpTransferState::Succeeded,
            transferred: 1,
            total: Some(1),
            speed: 0.0,
            current_file: None,
            error: None,
        };

        assert_eq!(
            Some(TransferRefreshTarget::Remote("/srv/backups".to_string())),
            reconcile_global_transfer_snapshot(&mut task, &snapshot)
        );
        assert_eq!(TransferTaskState::Completed, task.state);
    }

    #[test]
    fn reconciled_failed_remote_delete_snapshot_requests_remote_refresh_once() {
        let mut task = transfer_task(0);
        task.external_transfer_id = Some(SftpTransferId::new(10));
        task.operation = TransferOperation::DeleteRemote {
            entries: vec![
                remote_delete_entry("deleted.txt", false),
                remote_delete_entry("permission-denied.txt", false),
            ],
            remote_dir: "/srv/backups".to_string(),
        };
        let snapshot = SftpTransferSnapshot {
            id: SftpTransferId::new(10),
            operation: SftpTransferOperation::DeleteRemote,
            connection: SftpConnectionIdentity::Runtime(3),
            local_path: PathBuf::new(),
            remote_path: "/srv/backups".to_string(),
            display_name: "2 items".to_string(),
            state: SftpTransferState::Failed,
            transferred: 2,
            total: Some(2),
            speed: 0.0,
            current_file: Some("/srv/backups/permission-denied.txt".to_string()),
            error: Some("permission denied".to_string()),
        };

        assert_eq!(
            Some(TransferRefreshTarget::Remote("/srv/backups".to_string())),
            reconcile_global_transfer_snapshot(&mut task, &snapshot)
        );
        assert_eq!(TransferTaskState::Failed, task.state);
        assert_eq!(
            None,
            reconcile_global_transfer_snapshot(&mut task, &snapshot)
        );
    }

    #[test]
    fn transfer_error_summary_removes_control_characters_and_limits_length() {
        let long_error = format!("password failed\r\n{}\0secret", "x".repeat(600));
        let summary = transfer_error_summary(&anyhow::anyhow!(long_error));

        assert!(!summary.chars().any(char::is_control));
        assert!(summary.starts_with("password failed "));
        assert!(summary.ends_with('…'));
        assert!(summary.chars().count() <= 501);
    }

    fn transfer_pool_config() -> SshConnectConfig {
        SshConnectConfig {
            host: "should-not-dial.invalid".to_string(),
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

    fn remote_delete_entry(name: &str, is_dir: bool) -> super::FileItem {
        super::FileItem {
            name: name.to_string(),
            size: 0,
            modified: std::time::SystemTime::UNIX_EPOCH,
            is_dir,
            permissions: String::new(),
            owner: None,
            directory_size: super::DirectorySizeState::Unknown,
        }
    }

    #[test]
    fn server_copy_conflicts_distinguish_mergeable_directories_and_type_mismatches() {
        let items = vec![
            ServerCopyItem {
                source_path: "/source/app".to_string(),
                target_path: "/target/app".to_string(),
                is_dir: true,
                size: 0,
                directory_conflict_policy: DirectoryConflictPolicy::Merge,
            },
            ServerCopyItem {
                source_path: "/source/report.txt".to_string(),
                target_path: "/target/report.txt".to_string(),
                is_dir: false,
                size: 42,
                directory_conflict_policy: DirectoryConflictPolicy::Merge,
            },
        ];
        let existing_entries =
            HashMap::from([("app".to_string(), true), ("report.txt".to_string(), true)]);

        assert_eq!(
            (2, true, true),
            server_copy_conflict_flags(&items, &existing_entries)
        );
    }

    #[test]
    fn upload_overwrite_only_marks_conflicting_directories_for_replace() {
        let transfer = |is_dir, has_conflict| PendingTransfer {
            name: "app".to_string(),
            local_path: PathBuf::from("/local/app"),
            remote_path: "/remote/app".to_string(),
            is_dir,
            has_conflict,
        };

        assert_eq!(
            DirectoryConflictPolicy::Replace,
            upload_directory_conflict_policy(
                &transfer(true, true),
                DirectoryConflictPolicy::Replace
            )
        );
        assert_eq!(
            DirectoryConflictPolicy::Merge,
            upload_directory_conflict_policy(
                &transfer(true, false),
                DirectoryConflictPolicy::Replace
            )
        );
        assert_eq!(
            DirectoryConflictPolicy::Merge,
            upload_directory_conflict_policy(
                &transfer(false, true),
                DirectoryConflictPolicy::Replace
            )
        );
    }

    #[test]
    fn upload_keep_both_renames_only_the_current_conflict() {
        let transfer = PendingTransfer {
            name: "report.txt".to_string(),
            local_path: PathBuf::from("/local/report.txt"),
            remote_path: "/remote/report.txt".to_string(),
            is_dir: false,
            has_conflict: true,
        };
        let resolved = resolve_upload_conflict(
            PendingUploadDecision {
                transfer,
                directory_conflict_policy: DirectoryConflictPolicy::Merge,
            },
            FileConflictChoice::KeepBoth,
            &mut HashSet::from(["report.txt".to_string()]),
        )
        .expect("keep both retains the upload");

        assert_eq!(resolved.transfer.name, "report (copy).txt");
        assert_eq!(resolved.transfer.remote_path, "/remote/report (copy).txt");
        assert!(!resolved.transfer.has_conflict);
    }

    #[test]
    fn sequential_directory_choices_keep_per_item_policies() {
        let directory = |name: &str| PendingUploadDecision {
            transfer: PendingTransfer {
                name: name.to_string(),
                local_path: PathBuf::from(format!("/local/{name}")),
                remote_path: format!("/remote/{name}"),
                is_dir: true,
                has_conflict: true,
            },
            directory_conflict_policy: DirectoryConflictPolicy::Merge,
        };
        let session = UploadConflictSession {
            connection_generation: 1,
            resolver: Rc::new(RefCell::new(UploadConflictResolver::new(
                vec![directory("replace-me"), directory("merge-me")],
                |_| true,
            ))),
            existing_names: Rc::new(RefCell::new(HashSet::new())),
        };

        assert!(
            resolve_upload_conflicts(&session, (FileConflictChoice::Overwrite, false)).is_empty()
        );
        let resolved = resolve_upload_conflicts(&session, (FileConflictChoice::Merge, false));

        assert_eq!(resolved.len(), 2);
        assert_eq!(
            resolved[0].directory_conflict_policy,
            DirectoryConflictPolicy::Replace
        );
        assert_eq!(
            resolved[1].directory_conflict_policy,
            DirectoryConflictPolicy::Merge
        );
    }

    #[test]
    fn server_copy_overwrite_marks_only_directory_to_directory_conflicts() {
        let mut items = vec![
            ServerCopyItem {
                source_path: "/source/app".to_string(),
                target_path: "/target/app".to_string(),
                is_dir: true,
                size: 0,
                directory_conflict_policy: DirectoryConflictPolicy::Merge,
            },
            ServerCopyItem {
                source_path: "/source/report.txt".to_string(),
                target_path: "/target/report.txt".to_string(),
                is_dir: false,
                size: 42,
                directory_conflict_policy: DirectoryConflictPolicy::Merge,
            },
            ServerCopyItem {
                source_path: "/source/new".to_string(),
                target_path: "/target/new".to_string(),
                is_dir: true,
                size: 0,
                directory_conflict_policy: DirectoryConflictPolicy::Merge,
            },
        ];
        let existing_entries =
            HashMap::from([("app".to_string(), true), ("report.txt".to_string(), false)]);

        mark_server_copy_directory_replacements(&mut items, &existing_entries);

        assert_eq!(
            DirectoryConflictPolicy::Replace,
            items[0].directory_conflict_policy
        );
        assert_eq!(
            DirectoryConflictPolicy::Merge,
            items[1].directory_conflict_policy
        );
        assert_eq!(
            DirectoryConflictPolicy::Merge,
            items[2].directory_conflict_policy
        );
    }

    #[test]
    fn close_confirmation_can_be_aborted_without_committing_close() {
        let mut state = CloseState::Open;

        assert!(state.begin_confirmation());
        assert!(matches!(state, CloseState::AwaitingDecision));

        state.abort_confirmation();
        assert!(matches!(state, CloseState::Open));
    }

    #[test]
    fn committed_close_cannot_be_confirmed_or_started_again() {
        let mut state = CloseState::Open;

        assert!(state.begin_confirmation());
        assert!(state.begin_close());
        assert!(matches!(state, CloseState::Closing));
        assert!(!state.begin_confirmation());
        assert!(!state.begin_close());
    }

    #[test]
    fn transfer_admission_freeze_rejects_late_tasks_but_keeps_existing_work() {
        let mut queue = TransferQueue::new(1);

        assert!(queue.enqueue(transfer_task(0)));
        queue.freeze_admission();

        assert!(matches!(queue.admission, TransferAdmission::Frozen));
        assert!(!queue.enqueue(transfer_task(1)));
        assert_eq!(1, queue.tasks.len());

        let startable = queue.next_startable();
        assert_eq!(1, startable.len());
        assert_eq!(0, startable[0].id);
        assert!(matches!(queue.tasks[0].state, TransferTaskState::Running));
    }

    #[test]
    fn cancel_all_cancels_pending_tasks_and_signals_running_tasks() {
        let mut queue = TransferQueue::new(1);
        let running = transfer_task(0);
        let running_cancelled = running.shared_progress.cancelled.clone();

        assert!(queue.enqueue(running));
        assert!(queue.enqueue(transfer_task(1)));
        assert_eq!(1, queue.next_startable().len());

        queue.cancel_all();

        assert!(queue.pending.is_empty());
        assert!(running_cancelled.load(Ordering::Relaxed));
        assert!(matches!(queue.tasks[0].state, TransferTaskState::Running));
        assert!(matches!(queue.tasks[1].state, TransferTaskState::Cancelled));
    }

    #[test]
    fn only_apply_remote_listing_for_active_path() {
        assert!(should_apply_remote_listing("/srv/app", "/srv/app"));
        assert!(!should_apply_remote_listing("/srv/other", "/srv/app"));
    }

    #[test]
    fn connection_generation_rejects_stale_and_reserved_values() {
        let mut generation = ConnectionGeneration::default();

        assert!(!generation.is_current(0));

        let first = generation.advance();
        assert_eq!(1, first);
        assert!(generation.is_current(first));

        let second = generation.advance();
        assert_eq!(2, second);
        assert!(!generation.is_current(first));
        assert!(generation.is_current(second));
    }

    #[test]
    fn connection_generation_wraps_without_reusing_zero() {
        let mut generation = ConnectionGeneration(u64::MAX);

        let wrapped = generation.advance();

        assert_eq!(1, wrapped);
        assert!(generation.is_current(wrapped));
        assert!(!generation.is_current(0));
    }

    #[test]
    fn retired_transfer_pool_drains_idle_and_disconnects_late_returns() {
        let mut state = TransferClientPoolState::default();

        assert!(state.reserve_new(2));
        assert!(state.reserve_new(2));
        assert!(!state.reserve_new(2));
        assert_eq!(2, state.total_created);

        assert_eq!(None, state.return_client(10, false));
        assert_eq!(vec![10], state.drain_available());
        assert_eq!(1, state.total_created);

        assert_eq!(Some(20), state.return_client(20, true));
        assert!(state.available.is_empty());
        assert_eq!(0, state.total_created);
    }

    #[tokio::test]
    async fn retired_transfer_pool_rejects_acquire_before_dial() {
        let pool = Arc::new(TransferClientPool::new(transfer_pool_config(), 1));
        pool.retire();

        let error = match acquire_transfer_client(pool).await {
            Ok(_) => panic!("retired pool must reject new acquisitions"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("pool retired"));
    }

    #[tokio::test]
    async fn bounded_disconnect_reports_completion_and_failure() {
        assert!(matches!(
            bounded_disconnect(Duration::from_secs(1), async { Ok(()) }).await,
            BoundedDisconnectOutcome::Disconnected
        ));

        let failed = bounded_disconnect(Duration::from_secs(1), async {
            Err(anyhow::anyhow!("disconnect failed"))
        })
        .await;
        assert!(matches!(
            failed,
            BoundedDisconnectOutcome::Failed(error)
                if error.to_string() == "disconnect failed"
        ));
    }

    #[tokio::test]
    async fn bounded_disconnect_times_out_a_stalled_shutdown() {
        let outcome = bounded_disconnect(
            Duration::from_millis(20),
            std::future::pending::<anyhow::Result<()>>(),
        )
        .await;

        assert!(matches!(outcome, BoundedDisconnectOutcome::TimedOut));
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
    fn new_file_target_path_keeps_current_directory() {
        assert_eq!("/srv/app/new.log", join_remote_path("/srv/app", "new.log"));
        assert_eq!("/new.log", join_remote_path("/", "new.log"));
    }

    #[test]
    fn new_file_names_reject_path_traversal_and_special_entries() {
        assert!(is_valid_entry_name("notes.txt"));
        assert!(!is_valid_entry_name("../notes.txt"));
        assert!(!is_valid_entry_name("."));
        assert!(!is_valid_entry_name(""));
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
