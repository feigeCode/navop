//! 终端侧边栏文件管理器面板
//!
//! 仅针对 SSH 终端，通过独立的 SFTP 连接浏览远程文件系统。
//! UI 参考 `sftp_view` 的 `FileListPanel`，但为侧边栏场景做了精简和适配。
//! 支持文件传输（上传/下载/拖拽），使用独立的传输连接避免阻塞浏览。

use super::remote_path::{join_remote_path, normalize_remote_path, resolve_remote_path};
use crate::theme::TerminalColors;
use chrono::{DateTime, Local};
use gpui::{
    Anchor, App, ClipboardItem, ColorExt as _, Context, Entity, EventEmitter, ExternalPaths,
    FocusHandle, Focusable, Hsla, IntoElement, KeyBinding, ListSizingBehavior, MouseButton,
    MouseDownEvent, ParentElement, PathPromptOptions, Render, SharedString, Styled,
    UniformListScrollHandle, Window, actions, div, prelude::*, px, uniform_list,
};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, IconSize, InteractiveElementExt, Sizable, Size,
    WindowExt,
    breadcrumb::{Breadcrumb, BreadcrumbItem},
    button::{Button, ButtonVariants},
    dialog::DialogButtonProps,
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{ContextMenuExt, DropdownMenu, PopupMenu, PopupMenuItem},
    notification::Notification,
    popover::{Popover, PopoverState},
    progress::Progress,
    scroll::ScrollableElement,
    spinner::Spinner,
    tooltip::Tooltip,
    v_flex,
};
use one_core::background_tasks::{BackgroundTaskHandle, BackgroundTaskSpec};
use one_core::gpui_tokio::Tokio;
use one_core::sidebar_contribution::SidebarPlacement;
use one_core::storage::models::StoredConnection;
use one_core::storage::{
    GlobalStorageState, SftpFavoritePathRepository, normalize_sftp_favorite_path,
    sftp_favorite_connection_key,
};
use one_ui::file_conflict_prompt::{
    FileConflictChoice, FileConflictPrompt, FileConflictPromptLabels, FileConflictPromptSpec,
};
use one_ui::marquee_text::marquee_text;
use remote_file_editor::{
    ExternalEditorOpenRequest, RemoteMutationCallback, external_editor_menu_label,
    external_editors_for_file, open_remote_file_editor, open_remote_file_external_editor,
};
use remote_image_preview::{
    clipboard_upload_paths, image_format_for_path, open_remote_image_preview,
};
use rust_i18n::t;
use sftp::{
    DirectoryConflictPolicy, RemoteFileOperation, RusshSftpClient, ServerCopyItem, SftpClient,
    TransferCancelled, TransferProgress, build_remote_file_command, calculate_directory_size,
    remote_path_is_same_or_descendant,
};
use sftp_transfer::{
    self, SftpConnectionIdentity, SftpDeleteRemoteRequest, SftpRemoteDeleteEntry,
    SftpTransferEvent, SftpTransferExecutor, SftpTransferId, SftpTransferOperation,
    SftpTransferSnapshot, SftpTransferState, SftpUploadConnection, SftpUploadRequest,
    UploadConflictResolver, delete_remote_task_key, download_task_key, upload_task_key,
};
use ssh::{ChannelEvent, SshChannel, SshSessionManager};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime};
use tokio::sync::Mutex;

actions!(terminal_file_manager, [PasteUpload, NavigateParent]);

pub const FILE_MANAGER_CONTEXT: &str = "TerminalFileManager";

const FILE_ROW_HEIGHT: gpui::Pixels = px(36.);
const SIZE_COLUMN_WIDTH: gpui::Pixels = px(72.);
const MODIFIED_COLUMN_WIDTH: gpui::Pixels = px(70.);

pub fn init_keybindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new(
            file_manager_paste_shortcut(),
            PasteUpload,
            Some(FILE_MANAGER_CONTEXT),
        ),
        KeyBinding::new("backspace", NavigateParent, Some(FILE_MANAGER_CONTEXT)),
    ]
}

fn file_manager_paste_shortcut() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-v"
    } else {
        "ctrl-v"
    }
}

fn upload_progress_state(state: &SftpTransferState) -> TransferProgressState {
    match state {
        SftpTransferState::Queued => TransferProgressState::Pending,
        SftpTransferState::Running => TransferProgressState::Running,
        SftpTransferState::Cancelling
        | SftpTransferState::Succeeded
        | SftpTransferState::Failed
        | SftpTransferState::Cancelled => TransferProgressState::Cancelling,
    }
}

/// 后台任务分组标题：只保留「连接名称 - IP」，同一连接的面板合并到同一分组。
fn background_task_group_label(connection: &StoredConnection) -> SharedString {
    let host = connection
        .to_ssh_params()
        .map(|params| params.host)
        .unwrap_or_default();
    if host.is_empty() {
        connection.name.clone().into()
    } else {
        format!("{} - {}", connection.name, host).into()
    }
}

fn local_progress_state(state: &TransferTaskState) -> TransferProgressState {
    match state {
        TransferTaskState::Pending => TransferProgressState::Pending,
        TransferTaskState::Running => TransferProgressState::Running,
        TransferTaskState::Completed | TransferTaskState::Failed | TransferTaskState::Cancelled => {
            TransferProgressState::Cancelling
        }
    }
}

#[derive(Clone)]
#[allow(dead_code)]
enum TransferOperation {
    Download {
        remote_path: String,
        local_path: PathBuf,
        is_dir: bool,
    },
}

#[derive(Clone, PartialEq)]
enum TransferTaskState {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

struct SharedProgress {
    transferred: AtomicU64,
    total: AtomicU64,
    speed: AtomicU64,
    cancelled: Arc<AtomicBool>,
    current_file: std::sync::RwLock<Option<String>>,
}

impl SharedProgress {
    #[allow(dead_code)]
    fn new() -> Arc<Self> {
        Arc::new(Self {
            transferred: AtomicU64::new(0),
            total: AtomicU64::new(0),
            speed: AtomicU64::new(0),
            cancelled: Arc::new(AtomicBool::new(false)),
            current_file: std::sync::RwLock::new(None),
        })
    }
}

#[derive(Clone)]
struct TransferTask {
    id: usize,
    operation: TransferOperation,
    state: TransferTaskState,
    shared_progress: Arc<SharedProgress>,
    error: Option<String>,
}

#[derive(Clone, Copy)]
enum TransferCancelTarget {
    Global(SftpTransferId),
    Local(usize),
}

#[derive(Clone)]
struct GlobalDeleteView {
    remote_dir: String,
}

#[derive(Clone, Copy)]
enum TransferProgressState {
    Pending,
    Running,
    Cancelling,
}

struct TransferProgressView {
    icon: IconName,
    label: String,
    transferred: u64,
    total: u64,
    speed: f64,
    current_file: Option<String>,
    state: TransferProgressState,
    pending_count: usize,
    cancel_target: TransferCancelTarget,
}

#[derive(Clone)]
struct PendingUpload {
    name: String,
    local_path: PathBuf,
    remote_path: String,
    is_dir: bool,
    has_conflict: bool,
    directory_conflict_policy: DirectoryConflictPolicy,
}

#[derive(Clone)]
struct UploadConflictSession {
    connection_generation: u64,
    resolver: Rc<RefCell<UploadConflictResolver<PendingUpload>>>,
    existing_names: Rc<RefCell<HashSet<String>>>,
}

struct UploadPreparation {
    paths: Vec<PathBuf>,
    remote_dir: String,
    connection_generation: u64,
}

struct CompletedUploadPreparation {
    request: UploadPreparation,
    remote_names: Result<HashSet<String>, String>,
}

struct UploadPreparationTask {
    request: UploadPreparation,
    client: Arc<Mutex<RusshSftpClient>>,
    view: Entity<FileManagerPanel>,
}

struct UploadConflictDialog {
    connection_generation: u64,
    pending_uploads: Vec<PendingUpload>,
    existing_names: HashSet<String>,
}

fn build_pending_uploads(
    paths: Vec<PathBuf>,
    remote_dir: &str,
    existing_names: Option<&HashSet<String>>,
) -> Vec<PendingUpload> {
    paths
        .into_iter()
        .map(|path| {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default();
            PendingUpload {
                remote_path: join_remote_path(remote_dir, &name),
                is_dir: path.is_dir(),
                local_path: path,
                has_conflict: existing_names.is_some_and(|names| names.contains(&name)),
                name,
                directory_conflict_policy: DirectoryConflictPolicy::Merge,
            }
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DeleteTarget {
    name: String,
    path: String,
    is_dir: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DownloadTarget {
    name: String,
    path: String,
    is_dir: bool,
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

struct TransferQueue {
    tasks: Vec<TransferTask>,
    pending: VecDeque<usize>,
}

impl TransferQueue {
    fn new() -> Self {
        Self {
            tasks: Vec::new(),
            pending: VecDeque::new(),
        }
    }

    fn has_active(&self) -> bool {
        self.tasks.iter().any(|task| {
            task.state == TransferTaskState::Running || task.state == TransferTaskState::Pending
        })
    }

    fn running_count(&self) -> usize {
        self.tasks
            .iter()
            .filter(|task| task.state == TransferTaskState::Running)
            .count()
    }

    #[allow(dead_code)]
    fn enqueue(&mut self, task: TransferTask) {
        self.pending.push_back(task.id);
        self.tasks.push(task);
    }

    fn take_cancelled_pending(&mut self) -> Vec<TransferTask> {
        let mut retained = VecDeque::with_capacity(self.pending.len());
        let mut cancelled = Vec::new();
        while let Some(task_id) = self.pending.pop_front() {
            let Some(task) = self.tasks.iter_mut().find(|task| task.id == task_id) else {
                continue;
            };
            if task.state == TransferTaskState::Pending
                && task.shared_progress.cancelled.load(Ordering::Relaxed)
            {
                task.state = TransferTaskState::Cancelled;
                task.error = None;
                cancelled.push(task.clone());
            } else if task.state == TransferTaskState::Pending {
                retained.push_back(task_id);
            }
        }
        self.pending = retained;
        cancelled
    }

    fn next_startable(&mut self) -> Option<TransferTask> {
        if self.running_count() > 0 {
            return None;
        }
        while let Some(task_id) = self.pending.pop_front() {
            let Some(task) = self.tasks.iter_mut().find(|task| task.id == task_id) else {
                continue;
            };
            if task.state == TransferTaskState::Pending {
                task.state = TransferTaskState::Running;
                return Some(task.clone());
            }
        }
        None
    }

    fn active_task(&self) -> Option<&TransferTask> {
        self.tasks
            .iter()
            .find(|task| task.state == TransferTaskState::Running)
            .or_else(|| {
                self.tasks
                    .iter()
                    .find(|task| task.state == TransferTaskState::Pending)
            })
    }

    fn pending_count(&self) -> usize {
        self.tasks
            .iter()
            .filter(|task| task.state == TransferTaskState::Pending)
            .count()
    }
}

// ── 基础类型 ──────────────────────────────────────────────────

/// SFTP 连接状态
#[derive(Debug, Clone, PartialEq, Eq)]
enum ConnectionState {
    /// 初始状态，尚未连接
    Idle,
    /// 连接中
    Connecting,
    /// 已连接
    Connected,
    /// 连接失败
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RetryResetPlan {
    next_state: ConnectionState,
    initial_working_dir: Option<String>,
    clear_listing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NavigationRecoveryPlan {
    fallback_path: String,
}

fn build_retry_reset_plan(current_path: &str, working_dir: Option<String>) -> RetryResetPlan {
    RetryResetPlan {
        next_state: ConnectionState::Idle,
        initial_working_dir: working_dir.or_else(|| Some(current_path.to_string())),
        clear_listing: true,
    }
}

fn build_navigation_recovery_plan(
    current_path: &str,
    working_dir: Option<&str>,
    history: &[String],
    history_index: usize,
) -> NavigationRecoveryPlan {
    let fallback_path = working_dir
        .filter(|path| !path.is_empty() && *path != current_path)
        .map(ToString::to_string)
        .or_else(|| {
            history
                .get(history_index.saturating_sub(1))
                .filter(|path| !path.is_empty() && path.as_str() != current_path)
                .cloned()
        })
        .unwrap_or_else(|| "/".to_string());

    NavigationRecoveryPlan { fallback_path }
}

fn clear_remote_listing_state<T>(
    items: &mut Vec<T>,
    filtered_indices: &mut Vec<usize>,
    selected_indices: &mut HashSet<usize>,
) {
    items.clear();
    filtered_indices.clear();
    selected_indices.clear();
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectionMode {
    Replace,
    Toggle,
    Range,
}

fn selection_mode(shift_pressed: bool, multi_select: bool) -> SelectionMode {
    if shift_pressed {
        SelectionMode::Range
    } else if multi_select {
        SelectionMode::Toggle
    } else {
        SelectionMode::Replace
    }
}

fn apply_selection_mode(
    selected_indices: &mut HashSet<usize>,
    anchor_index: &mut Option<usize>,
    row_ix: usize,
    mode: SelectionMode,
) {
    match mode {
        SelectionMode::Replace => {
            selected_indices.clear();
            selected_indices.insert(row_ix);
            *anchor_index = Some(row_ix);
        }
        SelectionMode::Toggle => {
            if !selected_indices.remove(&row_ix) {
                selected_indices.insert(row_ix);
            }
            *anchor_index = Some(row_ix);
        }
        SelectionMode::Range => {
            let anchor = anchor_index.unwrap_or(row_ix);
            let start = anchor.min(row_ix);
            let end = anchor.max(row_ix);
            selected_indices.clear();
            selected_indices.extend(start..=end);
            anchor_index.get_or_insert(row_ix);
        }
    }
}

/// 排序列
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SortColumn {
    Name,
    Size,
    Modified,
}

/// 排序方向
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SortOrder {
    Ascending,
    Descending,
}

/// 远程文件项
#[derive(Clone, Debug)]
struct RemoteFileItem {
    name: String,
    size: u64,
    modified: SystemTime,
    is_dir: bool,
    permissions: String,
    owner: Option<String>,
    directory_size: DirectorySizeState,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum DirectorySizeState {
    #[default]
    Unknown,
    Calculating,
    Ready(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RemoteClipboardKind {
    Copy,
    Cut,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RemoteClipboardEntry {
    name: String,
    source_path: String,
    is_dir: bool,
    size: u64,
}

#[derive(Clone, Debug)]
struct RemoteFileClipboard {
    kind: RemoteClipboardKind,
    entries: Vec<RemoteClipboardEntry>,
}

fn can_paste_remote_file_clipboard(
    clipboard: Option<&RemoteFileClipboard>,
    is_connected: bool,
) -> bool {
    is_connected && clipboard.is_some_and(|clipboard| !clipboard.entries.is_empty())
}

/// 文件管理器面板事件
#[derive(Clone, Debug)]
pub enum FileManagerPanelEvent {
    /// 关闭面板
    Close,
    /// 请求宿主把面板移动到指定位置
    MoveTo(SidebarPlacement),
    /// 在独立页签中打开当前连接的 SFTP 文件管理器
    OpenSftp(StoredConnection),
    /// 在终端中 cd 到指定路径
    CdToTerminal(String),
    /// 请求将终端当前工作目录同步到文件管理器
    SyncWorkingDir,
    /// 切换“自动跟随终端工作目录”开关（宿主负责持久化并回写视觉态）
    ToggleFollowTerminalCwd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FrameMoveOption {
    placement: SidebarPlacement,
    disabled: bool,
}

fn frame_move_options(current: SidebarPlacement) -> Vec<FrameMoveOption> {
    [
        SidebarPlacement::Left,
        SidebarPlacement::Right,
        SidebarPlacement::Bottom,
    ]
    .into_iter()
    .map(|placement| FrameMoveOption {
        placement,
        disabled: placement == current,
    })
    .collect()
}

fn frame_placement_label(placement: SidebarPlacement) -> &'static str {
    match placement {
        SidebarPlacement::Left => "Left",
        SidebarPlacement::Right => "Right",
        SidebarPlacement::Bottom => "Bottom",
    }
}

fn frame_placement_icon(placement: SidebarPlacement) -> IconName {
    match placement {
        SidebarPlacement::Left => IconName::PanelLeft,
        SidebarPlacement::Right => IconName::PanelRight,
        SidebarPlacement::Bottom => IconName::PanelBottom,
    }
}

fn build_frame_options_menu(
    menu: PopupMenu,
    panel: Entity<FileManagerPanel>,
    placement: SidebarPlacement,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let move_panel = panel.clone();
    let close_panel = panel.clone();
    menu.min_w(px(220.0))
        .submenu_with_icon(
            Some(IconName::PanelRight.into()),
            "Move to",
            window,
            cx,
            move |submenu, _window, _cx| {
                frame_move_options(placement)
                    .into_iter()
                    .fold(submenu, |submenu, option| {
                        let panel = move_panel.clone();
                        submenu.item(
                            PopupMenuItem::new(frame_placement_label(option.placement))
                                .icon(frame_placement_icon(option.placement))
                                .checked(option.disabled)
                                .disabled(option.disabled)
                                .on_click(move |_, _, cx| {
                                    panel.update(cx, |_this, cx| {
                                        cx.emit(FileManagerPanelEvent::MoveTo(option.placement));
                                    });
                                }),
                        )
                    })
            },
        )
        .separator()
        .item(
            PopupMenuItem::new(t!("Sidebar.remove_from_sidebar").to_string())
                .icon(IconName::Close)
                .on_click(move |_, _, cx| {
                    close_panel.update(cx, |_this, cx| {
                        cx.emit(FileManagerPanelEvent::Close);
                    });
                }),
        )
}

// ── 工具函数 ──────────────────────────────────────────────────

/// 格式化文件大小（紧凑格式，适合侧边栏窄列）
fn format_file_size(size: u64) -> String {
    if size == 0 {
        return "0 B".to_string();
    }
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if size >= GB {
        format!("{:.1}G", size as f64 / GB as f64)
    } else if size >= MB {
        format!("{:.1}M", size as f64 / MB as f64)
    } else if size >= KB {
        format!("{:.1}K", size as f64 / KB as f64)
    } else {
        format!("{}B", size)
    }
}

fn size_sort_key(item: &RemoteFileItem) -> (u8, u64) {
    if !item.is_dir {
        return (0, item.size);
    }

    match item.directory_size {
        DirectorySizeState::Ready(size) => (0, size),
        DirectorySizeState::Calculating => (1, 0),
        DirectorySizeState::Unknown => (2, 0),
    }
}

fn size_label(item: &RemoteFileItem) -> String {
    if !item.is_dir {
        return format_file_size(item.size);
    }

    match item.directory_size {
        DirectorySizeState::Unknown => t!("FileManager.calculate").to_string(),
        DirectorySizeState::Calculating => t!("FileManager.calculating").to_string(),
        DirectorySizeState::Ready(size) => format_file_size(size),
    }
}

fn property_row(label: String, value: String) -> impl IntoElement {
    h_flex()
        .gap_3()
        .child(div().w(px(96.)).text_sm().child(label))
        .child(div().flex_1().text_sm().child(value))
}

/// 格式化修改时间（短格式，适合侧边栏）
fn format_modified_time(time: SystemTime) -> String {
    let datetime: DateTime<Local> = time.into();
    let now = Local::now();
    // 同年使用 M/D HH:MM，不同年使用 YYYY/M/D
    if datetime.format("%Y").to_string() == now.format("%Y").to_string() {
        datetime.format("%-m/%-d %H:%M").to_string()
    } else {
        datetime.format("%Y/%-m/%-d").to_string()
    }
}

/// 格式化传输速度
fn format_speed(bytes_per_sec: f64) -> String {
    if bytes_per_sec >= 1024.0 * 1024.0 {
        format!("{:.1} MB/s", bytes_per_sec / (1024.0 * 1024.0))
    } else if bytes_per_sec >= 1024.0 {
        format!("{:.1} KB/s", bytes_per_sec / 1024.0)
    } else {
        format!("{:.0} B/s", bytes_per_sec)
    }
}

// 远程路径拼接与归一化见 `super::remote_path`（`join_remote_path` 已在文件头部引入）。

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ArchiveKind {
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
enum ExtractConflictAction {
    Overwrite,
    SkipExisting,
}

fn archive_kind_for_name(name: &str) -> Option<ArchiveKind> {
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

fn build_rename_target_path(old_path: &str, new_name: &str) -> String {
    let parent = remote_path_parent(old_path);
    join_remote_path(&parent, new_name)
}

fn build_new_file_target_path(current_path: &str, file_name: &str) -> String {
    join_remote_path(current_path, file_name)
}

fn build_remote_extract_command(
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

fn build_remote_extract_conflict_check_command(path: &str, name: &str) -> Option<String> {
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

fn should_apply_directory_result(current_path: &str, listed_path: &str) -> bool {
    current_path == listed_path
}

fn is_current_generation(current: u64, expected: u64) -> bool {
    current == expected
}

fn should_refresh_after_upload(current_path: &str, remote_path: &str) -> bool {
    current_path == remote_path_parent(remote_path)
}

fn transfer_progress_display_label(label: String, current_file: Option<String>) -> String {
    current_file
        .map(|current_file| format!("{label} - {current_file}"))
        .unwrap_or(label)
}

async fn load_upload_remote_names(
    client: Arc<Mutex<RusshSftpClient>>,
    remote_dir: String,
) -> Result<HashSet<String>, String> {
    let mut client = client.lock().await;
    let entries = client.list_dir(&remote_dir).await.map_err(|error| {
        tracing::error!("读取远程目录失败: {}", error);
        t!("FileManager.read_dir_failed", error = error).to_string()
    })?;
    Ok(entries
        .into_iter()
        .filter(|entry| entry.name != "." && entry.name != "..")
        .map(|entry| entry.name)
        .collect())
}

fn spawn_upload_preparation(
    task: UploadPreparationTask,
    window: &mut Window,
    cx: &mut Context<FileManagerPanel>,
) {
    let list_task = Tokio::spawn(
        cx,
        load_upload_remote_names(task.client, task.request.remote_dir.clone()),
    );
    window
        .spawn(cx, async move |cx| {
            let remote_names = match list_task.await {
                Ok(remote_names) => remote_names,
                Err(error) => {
                    tracing::error!("远程目录检查任务失败: {}", error);
                    Err(t!("FileManager.read_dir_failed", error = error).to_string())
                }
            };
            let completed = CompletedUploadPreparation {
                request: task.request,
                remote_names,
            };
            let _ = task.view.update_in(cx, |this, window, cx| {
                this.handle_upload_preparation(completed, window, cx);
            });
        })
        .detach();
}

fn should_refresh_after_delete(current_path: &str, remote_dir: &str) -> bool {
    current_path == remote_dir
}

fn remote_delete_display_name(targets: &[DeleteTarget]) -> String {
    match targets {
        [target] => target.name.clone(),
        _ => t!("FileManager.delete_n_items", count = targets.len()).to_string(),
    }
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
    const BREADCRUMB_ITEM_MAX_WIDTH: f32 = 180.;

    BreadcrumbItem::new(label)
        .flex_shrink_1()
        .min_w(px(0.))
        .max_w(px(BREADCRUMB_ITEM_MAX_WIDTH))
        .truncate()
}

/// 判断传输错误是否为取消
fn is_transfer_cancelled(error: &anyhow::Error) -> bool {
    error.downcast_ref::<TransferCancelled>().is_some()
}

fn generate_unique_name(original_name: &str, existing_names: &HashSet<String>) -> String {
    let (stem, ext) = if let Some(dot_pos) = original_name.rfind('.') {
        if dot_pos > 0 {
            (
                original_name[..dot_pos].to_string(),
                Some(original_name[dot_pos..].to_string()),
            )
        } else {
            (original_name.to_string(), None)
        }
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
        } else if let Some(ref ext) = ext {
            format!("{} (copy {}){}", stem, counter, ext)
        } else {
            format!("{} (copy {})", stem, counter)
        };

        if !existing_names.contains(&new_name) {
            return new_name;
        }
        counter += 1;
    }
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
) -> Vec<PendingUpload> {
    let (choice, apply_all) = decision;
    let mut existing_names = session.existing_names.borrow_mut();
    let mut resolver = session.resolver.borrow_mut();
    resolver.resolve_current(
        apply_all,
        |current, candidate| current.is_dir == candidate.is_dir,
        |upload| resolve_upload_conflict(upload, choice, &mut existing_names),
    );
    resolver.take_ready().unwrap_or_default()
}

fn resolve_upload_conflict(
    mut upload: PendingUpload,
    choice: FileConflictChoice,
    existing_names: &mut HashSet<String>,
) -> Option<PendingUpload> {
    match choice {
        FileConflictChoice::Skip => None,
        FileConflictChoice::KeepBoth => {
            let new_name = generate_unique_name(&upload.name, existing_names);
            existing_names.insert(new_name.clone());
            upload.remote_path =
                join_remote_path(&remote_path_parent(&upload.remote_path), &new_name);
            upload.name = new_name;
            upload.has_conflict = false;
            Some(upload)
        }
        FileConflictChoice::Merge => {
            upload.directory_conflict_policy = DirectoryConflictPolicy::Merge;
            Some(upload)
        }
        FileConflictChoice::Overwrite => {
            if upload.is_dir {
                upload.directory_conflict_policy = DirectoryConflictPolicy::Replace;
            }
            Some(upload)
        }
    }
}

fn delete_targets_for_selection(
    current_path: &str,
    items: &[RemoteFileItem],
    filtered_indices: &[usize],
    selected_indices: &HashSet<usize>,
) -> Vec<DeleteTarget> {
    let mut selected: Vec<_> = selected_indices.iter().copied().collect();
    selected.sort_unstable();

    selected
        .into_iter()
        .filter_map(|filtered_ix| {
            let real_ix = *filtered_indices.get(filtered_ix)?;
            let item = items.get(real_ix)?;
            Some(DeleteTarget {
                name: item.name.clone(),
                path: join_remote_path(current_path, &item.name),
                is_dir: item.is_dir,
            })
        })
        .collect()
}

fn download_targets_for_selection(
    current_path: &str,
    items: &[RemoteFileItem],
    filtered_indices: &[usize],
    selected_indices: &HashSet<usize>,
) -> Vec<DownloadTarget> {
    let mut selected: Vec<_> = selected_indices.iter().copied().collect();
    selected.sort_unstable();

    selected
        .into_iter()
        .filter_map(|filtered_ix| {
            let real_ix = *filtered_indices.get(filtered_ix)?;
            let item = items.get(real_ix)?;
            Some(DownloadTarget {
                name: item.name.clone(),
                path: join_remote_path(current_path, &item.name),
                is_dir: item.is_dir,
            })
        })
        .collect()
}

fn clipboard_entries_for_selection(
    current_path: &str,
    items: &[RemoteFileItem],
    filtered_indices: &[usize],
    selected_indices: &HashSet<usize>,
) -> Vec<RemoteClipboardEntry> {
    let mut selected: Vec<_> = selected_indices.iter().copied().collect();
    selected.sort_unstable();

    selected
        .into_iter()
        .filter_map(|filtered_ix| {
            let real_ix = *filtered_indices.get(filtered_ix)?;
            let item = items.get(real_ix)?;
            Some(RemoteClipboardEntry {
                name: item.name.clone(),
                source_path: join_remote_path(current_path, &item.name),
                is_dir: item.is_dir,
                size: item.size,
            })
        })
        .collect()
}

fn should_use_context_selection(selected_indices: &HashSet<usize>, filtered_ix: usize) -> bool {
    selected_indices.contains(&filtered_ix) && selected_indices.len() > 1
}

fn delete_target_preview(targets: &[DeleteTarget]) -> String {
    let mut lines: Vec<String> = targets
        .iter()
        .take(5)
        .map(|target| {
            let prefix = if target.is_dir { "[dir]" } else { "[file]" };
            format!("{} {}", prefix, target.name)
        })
        .collect();

    if targets.len() > 5 {
        lines.push(t!("FileManager.and_more", count = targets.len() - 5).to_string());
    }

    lines.join("\n")
}

async fn exec_remote_command(
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

async fn exec_remote_command_output(
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

async fn remote_extract_has_conflict(
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

// ── FileManagerPanel ──────────────────────────────────────────

/// 终端侧边栏文件管理器面板
pub struct FileManagerPanel {
    /// 当前 SSH 连接，用于在独立页签中打开同一连接的 SFTP 视图
    stored_connection: StoredConnection,
    /// 共享 SSH 会话管理器
    session_manager: Arc<SshSessionManager>,
    /// SFTP 客户端（浏览用）
    sftp_client: Option<Arc<Mutex<RusshSftpClient>>>,
    /// 浏览连接代次；新连接或重试会使旧连接 future 的结果失效。
    connection_generation: u64,
    /// 连接状态
    connection_state: ConnectionState,
    /// 当前远程路径
    current_path: String,
    /// 文件列表
    items: Vec<RemoteFileItem>,
    /// 过滤后的索引
    filtered_indices: Vec<usize>,
    /// 选中项索引（基于 filtered_indices 的下标）
    selected_indices: HashSet<usize>,
    /// Shift 范围选择的锚点（基于 filtered_indices 的下标）
    selection_anchor_index: Option<usize>,
    /// 排序列
    sort_column: SortColumn,
    /// 排序方向
    sort_order: SortOrder,
    /// 是否显示隐藏文件
    show_hidden: bool,
    /// 是否自动跟随终端工作目录（视觉态由宿主同步，持久化在应用设置）
    follow_terminal_cwd: bool,
    /// 搜索输入框
    search_input: Entity<InputState>,
    /// 路径输入框
    path_input: Entity<InputState>,
    /// 搜索关键词
    search_query: String,
    /// 是否正在编辑路径
    path_editing: bool,
    /// 导航历史
    history: Vec<String>,
    /// 当前历史位置
    history_index: usize,
    /// 滚动句柄
    scroll_handle: UniformListScrollHandle,
    /// 焦点句柄
    focus_handle: FocusHandle,
    /// 是否正在加载目录
    loading: bool,
    /// 文件复制/剪切缓冲区
    file_clipboard: Option<RemoteFileClipboard>,
    favorite_paths: Vec<String>,
    favorite_connection_id: Option<i64>,
    favorite_connection_key: String,
    favorite_popover_open: bool,
    favorite_search_input: Entity<InputState>,
    favorite_edit_input: Entity<InputState>,
    favorite_editing_path: Option<String>,
    /// 订阅
    _subscriptions: Vec<gpui::Subscription>,

    // ── 传输相关字段 ──
    /// 全局 SFTP 传输执行器，任务生命周期不依赖面板。
    global_executor: Entity<SftpTransferExecutor>,
    /// 当前连接在全局传输执行器中的稳定身份。
    upload_connection_identity: SftpConnectionIdentity,
    background_task_group: SharedString,
    /// 面板提交的、尚未终结的全局远程删除。
    pending_global_deletes: HashMap<SftpTransferId, GlobalDeleteView>,
    transfer_client: Option<Arc<Mutex<RusshSftpClient>>>,
    transfer_connecting: bool,
    transfer_generation: u64,
    transfer_queue: TransferQueue,
    #[allow(dead_code)]
    next_task_id: usize,
    progress_refresh_task: Option<gpui::Task<()>>,
    active_extract: Option<ActiveExtract>,
    /// 仅测试注入；记录远程目录刷新次数。
    #[cfg(test)]
    test_refresh_count: Arc<AtomicU64>,
    /// 终端当前工作目录缓存，用于首次连接和导航失败恢复
    working_dir_hint: Option<String>,
    /// 终端主题配色，用于嵌入侧边栏时保持和终端一致
    colors: TerminalColors,
    /// 宿主工具面板当前所在位置
    frame_placement: SidebarPlacement,
}

impl FileManagerPanel {
    pub fn new(
        stored_connection: StoredConnection,
        session_manager: Arc<SshSessionManager>,
        colors: TerminalColors,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        let search_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("FileManager.search_placeholder"))
        });
        let path_input = cx
            .new(|cx| InputState::new(window, cx).placeholder(t!("FileManager.path_placeholder")));
        let favorite_search_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("FileManager.favorite_search_placeholder"))
        });
        let favorite_edit_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("FileManager.favorite_edit_placeholder"))
        });
        let favorite_connection_id = stored_connection.id;
        let favorite_connection_key = sftp_favorite_connection_key(&stored_connection);
        let favorite_paths = Self::load_favorite_paths(&favorite_connection_key, cx);
        let global_executor = sftp_transfer::global(cx);
        let upload_connection_identity = SftpConnectionIdentity::from_stored(&stored_connection)
            .unwrap_or_else(|| {
                global_executor.update(cx, |executor, _| executor.allocate_runtime_connection())
            });
        let background_task_group = background_task_group_label(&stored_connection);

        let mut subscriptions = Vec::new();
        subscriptions.push(
            cx.subscribe(&search_input, |this, input, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    let text = input.read(cx).text().to_string();
                    this.search_query = text;
                    this.apply_filter();
                    this.clear_selection();
                    cx.notify();
                }
            }),
        );
        subscriptions.push(cx.subscribe_in(
            &path_input,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => {
                    this.confirm_path(window, cx);
                }
                InputEvent::Blur => {
                    this.cancel_path_editing(cx);
                }
                _ => {}
            },
        ));
        subscriptions.push(cx.subscribe(
            &favorite_search_input,
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
            &global_executor,
            |this, executor, event: &SftpTransferEvent, cx| {
                this.handle_global_transfer_event(&executor, event, cx);
            },
        ));

        Self {
            stored_connection,
            session_manager,
            sftp_client: None,
            connection_generation: 0,
            connection_state: ConnectionState::Idle,
            current_path: "/".to_string(),
            items: Vec::new(),
            filtered_indices: Vec::new(),
            selected_indices: HashSet::new(),
            selection_anchor_index: None,
            sort_column: SortColumn::Name,
            sort_order: SortOrder::Ascending,
            show_hidden: false,
            follow_terminal_cwd: true,
            search_input,
            path_input,
            search_query: String::new(),
            path_editing: false,
            history: vec!["/".to_string()],
            history_index: 0,
            scroll_handle: UniformListScrollHandle::new(),
            focus_handle,
            loading: false,
            file_clipboard: None,
            favorite_paths,
            favorite_connection_id,
            favorite_connection_key,
            favorite_popover_open: false,
            favorite_search_input,
            favorite_edit_input,
            favorite_editing_path: None,
            _subscriptions: subscriptions,
            global_executor,
            upload_connection_identity,
            background_task_group,
            pending_global_deletes: HashMap::new(),
            transfer_client: None,
            transfer_connecting: false,
            transfer_generation: 0,
            transfer_queue: TransferQueue::new(),
            next_task_id: 0,
            progress_refresh_task: None,
            active_extract: None,
            #[cfg(test)]
            test_refresh_count: Arc::new(AtomicU64::new(0)),
            working_dir_hint: None,
            colors,
            frame_placement: SidebarPlacement::Right,
        }
    }

    pub fn set_colors(&mut self, colors: TerminalColors, cx: &mut Context<Self>) {
        self.colors = colors;
        cx.notify();
    }

    /// 设置测试用刷新计数句柄。仅供同文件实体测试注入，不参与生产行为。
    #[cfg(test)]
    pub(crate) fn set_test_refresh_count(&mut self, count: Arc<AtomicU64>) {
        self.test_refresh_count = count;
    }

    pub fn set_frame_placement(&mut self, placement: SidebarPlacement, cx: &mut Context<Self>) {
        if self.frame_placement == placement {
            return;
        }
        self.frame_placement = placement;
        cx.notify();
    }

    // ── 连接管理 ──────────────────────────────────────────────

    /// 建立 SFTP 连接
    pub fn connect(&mut self, cx: &mut Context<Self>) {
        if self.connection_state == ConnectionState::Connecting {
            return;
        }

        self.connection_generation = self.connection_generation.wrapping_add(1);
        let connection_generation = self.connection_generation;
        self.connection_state = ConnectionState::Connecting;
        cx.notify();

        let initial_dir = self.working_dir_hint.clone();
        let session_manager = self.session_manager.clone();
        let task = Tokio::spawn(cx, async move {
            let shared_client = session_manager.client().await?;
            let mut client = RusshSftpClient::connect_with_client(shared_client).await?;
            // 优先使用终端当前工作目录，否则回退到 realpath(".")
            let real_path = if let Some(dir) = initial_dir {
                dir
            } else {
                client
                    .realpath(".")
                    .await
                    .unwrap_or_else(|_| "/".to_string())
            };
            Ok::<_, anyhow::Error>((client, real_path))
        });

        cx.spawn(async move |this, cx| match task.await {
            Ok(Ok((client, real_path))) => {
                let _ = this.update(cx, |this, cx| {
                    if this.connection_generation != connection_generation {
                        return;
                    }
                    this.sftp_client = Some(Arc::new(Mutex::new(client)));
                    this.connection_state = ConnectionState::Connected;
                    let real_path = normalize_remote_path(&real_path);
                    this.current_path = real_path.clone();
                    this.working_dir_hint = Some(real_path.clone());
                    this.history = vec![real_path];
                    this.history_index = 0;
                    this.refresh_dir(cx);
                });
            }
            Ok(Err(e)) => {
                let _ = this.update(cx, |this, cx| {
                    if this.connection_generation != connection_generation {
                        return;
                    }
                    this.connection_state = ConnectionState::Error(format!(
                        "{}: {}",
                        t!("FileManager.connect_failed"),
                        e
                    ));
                    cx.notify();
                });
            }
            Err(e) => {
                let _ = this.update(cx, |this, cx| {
                    if this.connection_generation != connection_generation {
                        return;
                    }
                    this.connection_state = ConnectionState::Error(format!(
                        "{}: {}",
                        t!("FileManager.connect_failed"),
                        e
                    ));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// 仅在 Idle 状态时自动连接（用于面板首次激活）
    pub fn connect_if_idle(&mut self, cx: &mut Context<Self>) {
        if self.connection_state == ConnectionState::Idle {
            self.connect(cx);
        }
    }

    fn apply_retry_reset_plan(&mut self, plan: RetryResetPlan) {
        self.connection_state = plan.next_state;
        self.working_dir_hint = plan.initial_working_dir;
        self.sftp_client = None;
        self.connection_generation = self.connection_generation.wrapping_add(1);
        self.transfer_client = None;
        self.transfer_connecting = false;
        self.transfer_generation = self.transfer_generation.wrapping_add(1);
        self.loading = false;

        if plan.clear_listing {
            clear_remote_listing_state(
                &mut self.items,
                &mut self.filtered_indices,
                &mut self.selected_indices,
            );
            self.selection_anchor_index = None;
        }
    }

    fn reset_connection_for_retry(&mut self, working_dir: Option<String>) {
        let plan = build_retry_reset_plan(&self.current_path, working_dir);
        self.apply_retry_reset_plan(plan);
    }

    fn repair_history_after_navigation_failure(&mut self, failed_path: &str, fallback_path: &str) {
        if self.history_index < self.history.len() {
            self.history.truncate(self.history_index + 1);
        }

        if self.history.last().map(String::as_str) == Some(failed_path) {
            self.history.pop();
        }

        if self.history.last().map(String::as_str) != Some(fallback_path) {
            self.history.push(fallback_path.to_string());
        }

        if self.history.is_empty() {
            self.history.push(fallback_path.to_string());
        }

        self.history_index = self.history.len().saturating_sub(1);
    }

    fn recover_from_navigation_error(&mut self, message: String, cx: &mut Context<Self>) {
        if let Some(window) = cx.active_window() {
            let notification =
                Notification::error(t!("FileManager.read_dir_failed_recovered", error = message))
                    .autohide(true);
            let _ = window.update(cx, |_, window, cx| {
                window.push_notification(notification, cx);
            });
        }

        let plan = build_navigation_recovery_plan(
            &self.current_path,
            self.working_dir_hint.as_deref(),
            &self.history,
            self.history_index,
        );
        let failed_path = self.current_path.clone();

        self.connection_state = ConnectionState::Connected;
        self.loading = false;
        self.current_path = plan.fallback_path.clone();
        self.repair_history_after_navigation_failure(&failed_path, &plan.fallback_path);
        self.refresh_dir(cx);
    }

    pub fn reconnect_with_working_dir(
        &mut self,
        working_dir: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let should_reconnect = self.connection_state != ConnectionState::Idle
            || self.sftp_client.is_some()
            || !self.items.is_empty();
        if !should_reconnect {
            return;
        }

        self.reset_connection_for_retry(working_dir);
        self.connect(cx);
    }

    /// 设置初始工作目录（连接前由终端 OSC 7 提供）
    ///
    /// 仅在尚未连接时有效，连接后应使用 `sync_navigate_to`。
    pub fn set_initial_working_dir(&mut self, path: String) {
        let path = resolve_remote_path(&self.current_path, &path);
        self.working_dir_hint = Some(path.clone());
        if self.connection_state == ConnectionState::Idle {
            self.current_path = path;
        }
    }

    /// 同步“自动跟随终端工作目录”的视觉态（真值在应用设置中，由宿主回写）
    pub fn set_follow_terminal_cwd(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.follow_terminal_cwd != enabled {
            self.follow_terminal_cwd = enabled;
            cx.notify();
        }
    }

    /// 从终端 OSC 7 同步导航到指定路径
    ///
    /// 仅在已连接且路径不同时才导航，避免不必要的刷新。
    pub fn sync_navigate_to(&mut self, path: String, cx: &mut Context<Self>) {
        let path = resolve_remote_path(&self.current_path, &path);
        self.working_dir_hint = Some(path.clone());
        if self.connection_state != ConnectionState::Connected {
            return;
        }
        if path == self.current_path {
            return;
        }
        self.navigate_to(path, cx);
    }

    fn start_path_editing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.path_editing = true;
        let path = self.current_path.clone();
        self.path_input.update(cx, |state, cx| {
            state.set_value(&path, window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    fn confirm_path(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let new_path = self.path_input.read(cx).text().to_string();
        let new_path = new_path.trim().to_string();
        self.path_editing = false;

        if !new_path.is_empty() && new_path != self.current_path {
            self.navigate_to(new_path, cx);
        } else {
            cx.notify();
        }
    }

    fn cancel_path_editing(&mut self, cx: &mut Context<Self>) {
        if self.path_editing {
            self.path_editing = false;
            cx.notify();
        }
    }

    fn is_current_path_favorite(&self) -> bool {
        let Some(path) = normalize_sftp_favorite_path(&self.current_path) else {
            return false;
        };
        self.favorite_paths.iter().any(|existing| existing == &path)
    }

    fn toggle_current_favorite(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = normalize_sftp_favorite_path(&self.current_path) else {
            return;
        };
        let Some(repo) = Self::favorite_path_repository(cx) else {
            window.push_notification(
                Notification::error(
                    t!(
                        "FileManager.favorite_save_failed",
                        error = "SftpFavoritePathRepository not found"
                    )
                    .to_string(),
                ),
                cx,
            );
            return;
        };

        let is_favorite = self.is_current_path_favorite();
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
                    Notification::error(
                        t!("FileManager.favorite_save_failed", error = error).to_string(),
                    ),
                    cx,
                );
                return;
            }
        }

        self.refresh_favorite_paths(cx);
        let message = if is_favorite {
            t!("FileManager.favorite_removed").to_string()
        } else {
            t!("FileManager.favorite_added").to_string()
        };
        window.push_notification(Notification::success(message), cx);
        cx.notify();
    }

    fn add_favorite_path(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = normalize_sftp_favorite_path(path) else {
            return;
        };
        let Some(repo) = Self::favorite_path_repository(cx) else {
            window.push_notification(
                Notification::error(
                    t!(
                        "FileManager.favorite_save_failed",
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
                self.refresh_favorite_paths(cx);
                window.push_notification(
                    Notification::success(t!("FileManager.favorite_added").to_string()),
                    cx,
                );
                cx.notify();
            }
            Err(error) => {
                window.push_notification(
                    Notification::error(
                        t!("FileManager.favorite_save_failed", error = error).to_string(),
                    ),
                    cx,
                );
            }
        }
    }

    fn refresh_favorite_paths(&mut self, cx: &mut Context<Self>) {
        self.favorite_paths = Self::load_favorite_paths(&self.favorite_connection_key, cx);
    }

    fn load_favorite_paths(connection_key: &str, cx: &mut Context<Self>) -> Vec<String> {
        let Some(repo) = Self::favorite_path_repository(cx) else {
            tracing::error!("SftpFavoritePathRepository not found");
            return Vec::new();
        };

        repo.list_paths(connection_key).unwrap_or_else(|error| {
            tracing::error!("Failed to load SFTP favorite paths: {}", error);
            Vec::new()
        })
    }

    fn favorite_path_repository(cx: &mut Context<Self>) -> Option<Arc<SftpFavoritePathRepository>> {
        let storage = cx.global::<GlobalStorageState>().storage.clone();
        storage.get::<SftpFavoritePathRepository>()
    }

    fn remove_favorite_path(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo) = Self::favorite_path_repository(cx) else {
            window.push_notification(
                Notification::error(
                    t!(
                        "FileManager.favorite_save_failed",
                        error = "SftpFavoritePathRepository not found"
                    )
                    .to_string(),
                ),
                cx,
            );
            return;
        };

        match repo.remove_path(&self.favorite_connection_key, path) {
            Ok(false) => return,
            Ok(true) => {
                self.refresh_favorite_paths(cx);
                if self.favorite_editing_path.as_deref() == Some(path) {
                    self.favorite_editing_path = None;
                }
                window.push_notification(
                    Notification::success(t!("FileManager.favorite_removed").to_string()),
                    cx,
                );
                cx.notify();
            }
            Err(error) => {
                window.push_notification(
                    Notification::error(
                        t!("FileManager.favorite_save_failed", error = error).to_string(),
                    ),
                    cx,
                );
            }
        }
    }

    fn start_favorite_path_editing(
        &mut self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.favorite_editing_path = Some(path.clone());
        self.favorite_edit_input.update(cx, |state, cx| {
            state.set_value(&path, window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    fn cancel_favorite_path_editing(&mut self, cx: &mut Context<Self>) {
        if self.favorite_editing_path.take().is_some() {
            cx.notify();
        }
    }

    fn save_editing_favorite_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(old_path) = self.favorite_editing_path.clone() else {
            return;
        };
        let new_path = self.favorite_edit_input.read(cx).text().to_string();
        let Some(repo) = Self::favorite_path_repository(cx) else {
            window.push_notification(
                Notification::error(
                    t!(
                        "FileManager.favorite_save_failed",
                        error = "SftpFavoritePathRepository not found"
                    )
                    .to_string(),
                ),
                cx,
            );
            return;
        };

        match repo.update_path(&self.favorite_connection_key, &old_path, &new_path) {
            Ok(false) => return,
            Ok(true) => {
                self.favorite_editing_path = None;
                self.refresh_favorite_paths(cx);
                window.push_notification(
                    Notification::success(t!("FileManager.favorite_updated").to_string()),
                    cx,
                );
                cx.notify();
            }
            Err(error) => {
                window.push_notification(
                    Notification::error(
                        t!("FileManager.favorite_save_failed", error = error).to_string(),
                    ),
                    cx,
                );
            }
        }
    }

    fn render_path_breadcrumb(&self, cx: &mut Context<Self>) -> Breadcrumb {
        let mut breadcrumb = Breadcrumb::new();
        const MAX_VISIBLE: usize = 4;

        if self.current_path == "." {
            return breadcrumb.child(breadcrumb_item("."));
        }

        let parts: Vec<&str> = self
            .current_path
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect();
        let starts_with_slash = self.current_path.starts_with('/');
        let total = parts.len() + if starts_with_slash { 1 } else { 0 };

        if total <= MAX_VISIBLE {
            if starts_with_slash {
                breadcrumb = breadcrumb.child(breadcrumb_item("/").on_click(cx.listener(
                    |this, _, _window, cx| {
                        cx.stop_propagation();
                        this.navigate_to("/".to_string(), cx);
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
                        cx.stop_propagation();
                        this.navigate_to(path_so_far.clone(), cx);
                    }),
                ));
            }
        } else {
            if starts_with_slash {
                breadcrumb = breadcrumb.child(breadcrumb_item("/").on_click(cx.listener(
                    |this, _, _window, cx| {
                        cx.stop_propagation();
                        this.navigate_to("/".to_string(), cx);
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

                breadcrumb = breadcrumb.child(breadcrumb_item(parts[idx].to_string()).on_click(
                    cx.listener(move |this, _, _window, cx| {
                        cx.stop_propagation();
                        this.navigate_to(path_so_far.clone(), cx);
                    }),
                ));
            }
        }

        breadcrumb
    }

    // ── 目录浏览 ──────────────────────────────────────────────

    /// 刷新当前目录
    fn refresh_dir(&mut self, cx: &mut Context<Self>) {
        #[cfg(test)]
        self.test_refresh_count.fetch_add(1, Ordering::Relaxed);

        let Some(client) = self.sftp_client.clone() else {
            return;
        };

        self.loading = true;
        cx.notify();

        let path = self.current_path.clone();
        let listed_path = path.clone();
        let task = Tokio::spawn(cx, async move {
            let mut client: tokio::sync::MutexGuard<'_, RusshSftpClient> = client.lock().await;
            client.list_dir(&path).await
        });

        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                if !should_apply_directory_result(&this.current_path, &listed_path) {
                    cx.notify();
                    return;
                }

                match result {
                    Ok(Ok(entries)) => {
                        this.items = entries
                            .into_iter()
                            .filter(|e| e.name != "." && e.name != "..")
                            .map(|e| {
                                let owner = e.owner_display();
                                RemoteFileItem {
                                    name: e.name,
                                    size: e.size,
                                    modified: e.modified,
                                    is_dir: e.is_dir,
                                    permissions: format!("{:o}", e.permissions & 0o7777),
                                    owner,
                                    directory_size: DirectorySizeState::Unknown,
                                }
                            })
                            .collect();
                        this.sort_items();
                        this.apply_filter();
                        this.clear_selection();
                    }
                    Ok(Err(e)) => {
                        tracing::error!("列出目录失败: {}", e);
                        this.recover_from_navigation_error(e.to_string(), cx);
                    }
                    Err(e) => {
                        tracing::error!("SFTP 任务失败: {}", e);
                        this.recover_from_navigation_error(e.to_string(), cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 导航到指定路径
    fn navigate_to(&mut self, path: String, cx: &mut Context<Self>) {
        let path = resolve_remote_path(&self.current_path, &path);
        if path == self.current_path {
            self.refresh_dir(cx);
            return;
        }

        self.current_path = path.clone();

        // 截断当前位置之后的历史记录，再追加新路径
        if self.history_index + 1 < self.history.len() {
            self.history.truncate(self.history_index + 1);
        }
        self.history.push(path);
        self.history_index = self.history.len() - 1;

        self.scroll_handle = UniformListScrollHandle::new();
        self.refresh_dir(cx);
    }

    /// 后退
    fn go_back(&mut self, cx: &mut Context<Self>) {
        if self.history_index > 0 {
            self.history_index -= 1;
            self.current_path = self.history[self.history_index].clone();
            self.scroll_handle = UniformListScrollHandle::new();
            self.refresh_dir(cx);
        }
    }

    /// 导航到 Home（SFTP realpath "." 返回的初始路径）
    fn go_home(&mut self, cx: &mut Context<Self>) {
        let home = self.history.first().cloned().unwrap_or("/".to_string());
        self.navigate_to(home, cx);
    }

    /// 导航到上层目录
    fn go_parent(&mut self, cx: &mut Context<Self>) {
        let parent = if self.current_path == "/" {
            "/".to_string()
        } else {
            let path = self.current_path.trim_end_matches('/');
            match path.rfind('/') {
                Some(0) => "/".to_string(),
                Some(pos) => path[..pos].to_string(),
                None => "/".to_string(),
            }
        };
        self.navigate_to(parent, cx);
    }

    /// 是否在根目录
    fn is_at_root(&self) -> bool {
        self.current_path == "/" || self.current_path.is_empty()
    }

    // ── 排序和过滤 ───────────────────────────────────────────

    /// 排序文件列表
    fn sort_items(&mut self) {
        let sort_column = self.sort_column;
        let sort_order = self.sort_order;

        self.items.sort_by(|a, b| {
            // 文件夹始终排在前面
            if a.is_dir != b.is_dir {
                return if a.is_dir {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                };
            }

            let cmp = match sort_column {
                SortColumn::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                SortColumn::Size => size_sort_key(a).cmp(&size_sort_key(b)),
                SortColumn::Modified => a.modified.cmp(&b.modified),
            };

            match sort_order {
                SortOrder::Ascending => cmp,
                SortOrder::Descending => cmp.reverse(),
            }
        });
    }

    /// 设置排序
    fn set_sort(&mut self, column: SortColumn, cx: &mut Context<Self>) {
        if self.sort_column == column {
            self.sort_order = match self.sort_order {
                SortOrder::Ascending => SortOrder::Descending,
                SortOrder::Descending => SortOrder::Ascending,
            };
        } else {
            self.sort_column = column;
            self.sort_order = SortOrder::Ascending;
        }
        self.sort_items();
        self.apply_filter();
        self.clear_selection();
        cx.notify();
    }

    /// 应用过滤
    fn apply_filter(&mut self) {
        let query = self.search_query.to_lowercase();
        let show_hidden = self.show_hidden;

        self.filtered_indices = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                if !show_hidden && item.name.starts_with('.') {
                    return false;
                }
                if query.is_empty() {
                    true
                } else {
                    item.name.to_lowercase().contains(&query)
                }
            })
            .map(|(i, _)| i)
            .collect();
    }

    fn clear_selection(&mut self) {
        self.selected_indices.clear();
        self.selection_anchor_index = None;
    }

    fn set_directory_size_state(
        &mut self,
        full_path: &str,
        state: DirectorySizeState,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(item) = self
            .items
            .iter_mut()
            .find(|item| join_remote_path(&self.current_path, &item.name) == full_path)
        else {
            return false;
        };

        item.directory_size = state;
        if self.sort_column == SortColumn::Size {
            self.sort_items();
            self.apply_filter();
            self.clear_selection();
        }
        cx.notify();
        true
    }

    /// 更新选中状态
    fn select_row(&mut self, row_ix: usize, mode: SelectionMode) {
        apply_selection_mode(
            &mut self.selected_indices,
            &mut self.selection_anchor_index,
            row_ix,
            mode,
        );
    }

    fn store_remote_file_clipboard(
        &mut self,
        filtered_ix: usize,
        kind: RemoteClipboardKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !should_use_context_selection(&self.selected_indices, filtered_ix) {
            self.selected_indices.clear();
            self.selected_indices.insert(filtered_ix);
            self.selection_anchor_index = Some(filtered_ix);
        }

        let entries = clipboard_entries_for_selection(
            &self.current_path,
            &self.items,
            &self.filtered_indices,
            &self.selected_indices,
        );
        if entries.is_empty() {
            return;
        }

        self.file_clipboard = Some(RemoteFileClipboard { kind, entries });
        window.push_notification(
            Notification::success(match kind {
                RemoteClipboardKind::Copy => t!("FileManager.copy_ready"),
                RemoteClipboardKind::Cut => t!("FileManager.cut_ready"),
            }),
            cx,
        );
        cx.notify();
    }

    fn paste_remote_file_clipboard(
        &mut self,
        target_dir: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(clipboard) = self.file_clipboard.clone() else {
            window.push_notification(Notification::info(t!("FileManager.clipboard_empty")), cx);
            return;
        };

        if clipboard.entries.iter().any(|entry| {
            entry.is_dir && remote_path_is_same_or_descendant(&entry.source_path, &target_dir)
        }) {
            window.push_notification(
                Notification::error(t!("FileManager.invalid_paste_target")),
                cx,
            );
            return;
        }

        let Some(client) = self.sftp_client.clone() else {
            window.push_notification(
                Notification::error(t!("FileManager.sftp_not_connected")),
                cx,
            );
            return;
        };

        let session_manager = self.session_manager.clone();
        let kind = clipboard.kind;
        let task = Tokio::spawn(cx, async move {
            let mut client_guard = client.lock().await;
            let mut used_names = client_guard
                .list_dir(&target_dir)
                .await?
                .into_iter()
                .map(|entry| entry.name)
                .collect::<HashSet<_>>();
            let items = clipboard
                .entries
                .iter()
                .map(|entry| {
                    let target_name = if used_names.contains(&entry.name) {
                        generate_unique_name(&entry.name, &used_names)
                    } else {
                        entry.name.clone()
                    };
                    used_names.insert(target_name.clone());
                    ServerCopyItem {
                        source_path: entry.source_path.clone(),
                        target_path: join_remote_path(&target_dir, &target_name),
                        is_dir: entry.is_dir,
                        size: entry.size,
                        directory_conflict_policy: DirectoryConflictPolicy::Merge,
                    }
                })
                .collect::<Vec<_>>();

            drop(client_guard);
            let operation = match kind {
                RemoteClipboardKind::Copy => RemoteFileOperation::Copy,
                RemoteClipboardKind::Cut => RemoteFileOperation::Move,
            };
            let command = build_remote_file_command(operation, &items)?;
            exec_remote_command(session_manager, &command).await?;
            Ok::<_, anyhow::Error>(())
        });

        let view = cx.entity().downgrade();
        window
            .spawn(cx, async move |cx| {
                let result = task.await;
                let _ = view.update_in(cx, |this, window, cx| match result {
                    Ok(Ok(())) => {
                        if kind == RemoteClipboardKind::Cut {
                            this.file_clipboard = None;
                        }
                        this.refresh_dir(cx);
                        window.push_notification(
                            Notification::success(t!("FileManager.paste_success")),
                            cx,
                        );
                    }
                    Ok(Err(error)) => window.push_notification(
                        Notification::error(t!("FileManager.paste_failed", error = error)),
                        cx,
                    ),
                    Err(error) => window.push_notification(
                        Notification::error(t!("FileManager.paste_failed", error = error)),
                        cx,
                    ),
                });
            })
            .detach();
    }

    fn show_file_properties(
        &self,
        item: RemoteFileItem,
        full_path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let size = size_label(&item);
        let modified: DateTime<Local> = item.modified.into();
        let permissions = if item.permissions.is_empty() {
            "-".to_string()
        } else {
            item.permissions.clone()
        };
        let owner = item.owner.clone().unwrap_or_else(|| "-".to_string());

        window.open_dialog(cx, move |dialog, _window, _cx| {
            dialog
                .title(t!("FileManager.properties").to_string())
                .w(px(480.))
                .child(
                    v_flex()
                        .gap_2()
                        .child(property_row(
                            t!("FileManager.property_name").to_string(),
                            item.name.clone(),
                        ))
                        .child(property_row(
                            t!("FileManager.property_path").to_string(),
                            full_path.clone(),
                        ))
                        .child(property_row(
                            t!("FileManager.property_type").to_string(),
                            if item.is_dir {
                                t!("FileManager.property_folder").to_string()
                            } else {
                                t!("FileManager.property_file").to_string()
                            },
                        ))
                        .child(property_row(
                            t!("FileManager.property_size").to_string(),
                            size.clone(),
                        ))
                        .child(property_row(
                            t!("FileManager.property_modified").to_string(),
                            modified.format("%Y-%m-%d %H:%M:%S").to_string(),
                        ))
                        .child(property_row(
                            t!("FileManager.property_permissions").to_string(),
                            permissions.clone(),
                        ))
                        .child(property_row(
                            t!("FileManager.property_owner").to_string(),
                            owner.clone(),
                        )),
                )
                .close_button(true)
        });
    }

    fn calculate_remote_directory_size(
        &mut self,
        full_path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.set_directory_size_state(&full_path, DirectorySizeState::Calculating, cx) {
            return;
        }

        let Some(client) = self.sftp_client.clone() else {
            self.set_directory_size_state(&full_path, DirectorySizeState::Unknown, cx);
            return;
        };
        let path = full_path.clone();
        let task = Tokio::spawn(cx, async move {
            let mut client = client.lock().await;
            calculate_directory_size(&mut *client, &path, Arc::new(AtomicBool::new(false))).await
        });
        let view = cx.entity().downgrade();
        window
            .spawn(cx, async move |cx| {
                let result = task.await;
                let _ = view.update_in(cx, |this, window, cx| match result {
                    Ok(Ok(size)) => {
                        this.set_directory_size_state(
                            &full_path,
                            DirectorySizeState::Ready(size),
                            cx,
                        );
                    }
                    Ok(Err(error)) => {
                        this.set_directory_size_state(&full_path, DirectorySizeState::Unknown, cx);
                        window.push_notification(
                            Notification::error(t!(
                                "FileManager.calculate_size_failed",
                                error = error
                            )),
                            cx,
                        );
                    }
                    Err(error) => {
                        this.set_directory_size_state(&full_path, DirectorySizeState::Unknown, cx);
                        window.push_notification(
                            Notification::error(t!(
                                "FileManager.calculate_size_failed",
                                error = error
                            )),
                            cx,
                        );
                    }
                });
            })
            .detach();
    }

    // ── 传输调度 ──────────────────────────────────────────────

    /// 分配下一个任务 ID
    #[allow(dead_code)]
    fn alloc_task_id(&mut self) -> usize {
        let id = self.next_task_id;
        self.next_task_id += 1;
        id
    }

    fn handle_global_transfer_event(
        &mut self,
        executor: &Entity<SftpTransferExecutor>,
        event: &SftpTransferEvent,
        cx: &mut Context<Self>,
    ) {
        let id = match event {
            SftpTransferEvent::Added(id)
            | SftpTransferEvent::Updated(id)
            | SftpTransferEvent::Finished(id) => *id,
        };
        let Some(snapshot) = executor.read(cx).snapshot(id) else {
            return;
        };
        if snapshot.connection != self.upload_connection_identity {
            return;
        }
        if matches!(event, SftpTransferEvent::Finished(_)) {
            self.finish_global_delete(&snapshot, cx);
            if snapshot.state == SftpTransferState::Succeeded
                && snapshot.operation == SftpTransferOperation::Upload
                && should_refresh_after_upload(&self.current_path, &snapshot.remote_path)
            {
                self.refresh_dir(cx);
            }
        }
        cx.notify();
    }

    fn finish_global_delete(&mut self, snapshot: &SftpTransferSnapshot, cx: &mut Context<Self>) {
        if snapshot.operation != SftpTransferOperation::DeleteRemote {
            return;
        }
        let Some(delete) = self.pending_global_deletes.remove(&snapshot.id) else {
            return;
        };
        if should_refresh_after_delete(&self.current_path, &delete.remote_dir) {
            self.clear_selection();
            self.refresh_dir(cx);
        }
    }

    fn finish_cancelled_pending_tasks(&mut self) {
        self.transfer_queue.take_cancelled_pending();
    }

    /// 创建传输专用连接（首次传输时懒创建），然后执行排队任务
    #[allow(dead_code)]
    fn ensure_transfer_client_and_schedule(&mut self, cx: &mut Context<Self>) {
        if self.transfer_client.is_some() {
            self.schedule_transfers(cx);
            return;
        }
        if self.transfer_connecting {
            return;
        }

        self.transfer_connecting = true;
        let transfer_generation = self.transfer_generation;
        let session_manager = self.session_manager.clone();
        let connect_task = Tokio::spawn(cx, async move {
            let shared_client = session_manager.client().await?;
            let client = RusshSftpClient::connect_with_client(shared_client).await?;
            Ok::<_, anyhow::Error>(client)
        });

        cx.spawn(async move |this, cx| {
            let result = match connect_task.await {
                Ok(Ok(client)) => Ok(client),
                Ok(Err(e)) => Err(e),
                Err(e) => Err(anyhow::Error::new(e)),
            };

            let _ = this.update(cx, |this, cx| {
                if this.transfer_generation != transfer_generation {
                    return;
                }

                this.transfer_connecting = false;
                match result {
                    Ok(client) => {
                        this.transfer_client = Some(Arc::new(Mutex::new(client)));
                        this.schedule_transfers(cx);
                    }
                    Err(e) => {
                        let error_msg =
                            format!("{}: {}", t!("FileManager.transfer_connect_failed"), e);
                        tracing::error!("{}", error_msg);
                        for task in &mut this.transfer_queue.tasks {
                            if task.state == TransferTaskState::Pending {
                                let cancelled =
                                    task.shared_progress.cancelled.load(Ordering::Relaxed);
                                if cancelled {
                                    task.state = TransferTaskState::Cancelled;
                                    task.error = None;
                                } else {
                                    task.state = TransferTaskState::Failed;
                                    task.error = Some(error_msg.clone());
                                }
                            }
                        }
                        this.transfer_queue.pending.clear();
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    /// 调度下一个待执行的传输任务
    fn schedule_transfers(&mut self, cx: &mut Context<Self>) {
        self.finish_cancelled_pending_tasks();
        let Some(task) = self.transfer_queue.next_startable() else {
            return;
        };

        match task.operation.clone() {
            TransferOperation::Download {
                remote_path,
                local_path,
                is_dir,
            } => {
                self.start_download_task(
                    task.id,
                    remote_path,
                    local_path,
                    is_dir,
                    task.shared_progress,
                    cx,
                );
            }
        }

        self.start_progress_refresh(cx);
        cx.notify();
    }

    /// 执行下载任务
    fn start_download_task(
        &mut self,
        task_id: usize,
        remote_path: String,
        local_path: PathBuf,
        is_dir: bool,
        shared_progress: Arc<SharedProgress>,
        cx: &mut Context<Self>,
    ) {
        let Some(client) = self.transfer_client.clone() else {
            return;
        };

        let cancelled = shared_progress.cancelled.clone();
        let progress_for_callback = shared_progress.clone();

        let download_task = Tokio::spawn(cx, async move {
            let mut client_guard = client.lock().await;
            if is_dir {
                client_guard
                    .download_dir_with_progress(
                        &remote_path,
                        local_path.to_string_lossy().as_ref(),
                        cancelled,
                        Box::new(move |progress: TransferProgress| {
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
                                if let Ok(mut guard) = progress_for_callback.current_file.write() {
                                    *guard = Some(file);
                                }
                            }
                        }),
                    )
                    .await
            } else {
                client_guard
                    .download_with_progress(
                        &remote_path,
                        local_path.to_string_lossy().as_ref(),
                        cancelled,
                        Box::new(move |progress: TransferProgress| {
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
        });

        cx.spawn(async move |this, cx| {
            let result = match download_task.await {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => Err(e),
                Err(e) => Err(anyhow::Error::new(e)),
            };

            let _ = this.update(cx, |this, cx| {
                this.update_task_state(task_id, result);
                this.schedule_transfers(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// 更新任务状态
    fn update_task_state(&mut self, task_id: usize, result: Result<(), anyhow::Error>) {
        if let Some(task) = self
            .transfer_queue
            .tasks
            .iter_mut()
            .find(|t| t.id == task_id)
        {
            match result {
                Ok(()) => {
                    task.state = TransferTaskState::Completed;
                    task.error = None;
                }
                Err(error) => {
                    if is_transfer_cancelled(&error) {
                        task.state = TransferTaskState::Cancelled;
                        task.error = None;
                    } else {
                        let error = error.to_string();
                        task.state = TransferTaskState::Failed;
                        task.error = Some(error.clone());
                    }
                }
            }
        }
    }

    /// 取消传输
    fn cancel_transfer(&mut self, task_id: usize, cx: &mut Context<Self>) {
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
                    self.transfer_queue
                        .pending
                        .retain(|pending_id| *pending_id != task_id);
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
        self.schedule_transfers(cx);
        cx.notify();
    }

    /// 100ms 定时刷新进度
    fn start_progress_refresh(&mut self, cx: &mut Context<Self>) {
        if self.progress_refresh_task.is_some() {
            cx.notify();
            return;
        }

        self.progress_refresh_task = Some(cx.spawn(async move |this, cx| {
            loop {
                let should_continue = this
                    .update(cx, |this, cx| {
                        this.finish_cancelled_pending_tasks();
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

    // ── 传输入口 ──────────────────────────────────────────────

    /// 将待上传项加入传输队列
    fn enqueue_pending_uploads(&mut self, uploads: Vec<PendingUpload>, cx: &mut Context<Self>) {
        if uploads.is_empty() {
            return;
        }
        for upload in uploads {
            let title_prefix = if upload.is_dir {
                t!("FileManager.upload_folder")
            } else {
                t!("FileManager.upload_file")
            };
            let task_key = upload_task_key(
                &self.upload_connection_identity,
                &upload.local_path,
                &upload.remote_path,
            );
            let request = SftpUploadRequest {
                connection: self.upload_connection_identity.clone(),
                connection_source: SftpUploadConnection::SessionManager(
                    self.session_manager.clone(),
                ),
                local_path: upload.local_path,
                remote_path: upload.remote_path,
                is_dir: upload.is_dir,
                directory_conflict_policy: upload.directory_conflict_policy,
                display_name: upload.name.clone(),
                title: format!("{title_prefix} · {}", upload.name).into(),
                task_group: Some(self.background_task_group()),
                task_key: Some(task_key),
            };
            self.global_executor
                .update(cx, |executor, cx| executor.submit(request, cx));
        }

        cx.notify();
    }

    /// 上传前先检测目标目录中的重名项，必要时弹出冲突提示
    fn prepare_uploads(
        &mut self,
        paths: Vec<PathBuf>,
        remote_dir: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if paths.is_empty() {
            return;
        }

        let request = UploadPreparation {
            paths,
            remote_dir: remote_dir.to_string(),
            connection_generation: self.connection_generation,
        };
        let Some(client) = self.sftp_client.clone() else {
            let uploads = build_pending_uploads(request.paths, &request.remote_dir, None);
            self.enqueue_pending_uploads(uploads, cx);
            return;
        };

        spawn_upload_preparation(
            UploadPreparationTask {
                request,
                client,
                view: cx.entity().clone(),
            },
            window,
            cx,
        );
    }

    fn handle_upload_preparation(
        &mut self,
        completed: CompletedUploadPreparation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let request = completed.request;
        if !is_current_generation(self.connection_generation, request.connection_generation) {
            return;
        }
        let remote_names = match completed.remote_names {
            Ok(remote_names) => remote_names,
            Err(error) => {
                window.push_notification(Notification::error(error), cx);
                return;
            }
        };
        let pending_uploads =
            build_pending_uploads(request.paths, &request.remote_dir, Some(&remote_names));
        if pending_uploads.iter().all(|upload| !upload.has_conflict) {
            self.enqueue_pending_uploads(pending_uploads, cx);
            return;
        }
        self.show_upload_conflict_dialog(
            UploadConflictDialog {
                connection_generation: request.connection_generation,
                pending_uploads,
                existing_names: remote_names,
            },
            window,
            cx,
        );
    }

    fn show_upload_conflict_dialog(
        &mut self,
        conflict: UploadConflictDialog,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut existing_names = conflict.existing_names;
        existing_names.extend(
            conflict
                .pending_uploads
                .iter()
                .map(|upload| upload.name.clone()),
        );
        let resolver =
            UploadConflictResolver::new(conflict.pending_uploads, |upload| upload.has_conflict);
        if resolver.current().is_none() {
            return;
        }
        self.show_next_upload_conflict(
            UploadConflictSession {
                connection_generation: conflict.connection_generation,
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
                        name: current.name.clone().into(),
                        is_directory: current.is_dir,
                        apply_all: apply_all.clone(),
                        labels: upload_conflict_prompt_labels(position),
                    },
                    move |choice, apply_all, window, cx| {
                        window.close_dialog(cx);
                        let uploads = resolve_upload_conflicts(&session, (choice, apply_all));
                        let has_next = session.resolver.borrow().current().is_some();
                        view.update(cx, |this, cx| {
                            if !is_current_generation(
                                this.connection_generation,
                                session.connection_generation,
                            ) {
                                return;
                            }
                            this.enqueue_pending_uploads(uploads, cx);
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

    /// 入队下载任务
    fn enqueue_download(
        &mut self,
        remote_path: String,
        local_path: PathBuf,
        is_dir: bool,
        cx: &mut Context<Self>,
    ) {
        let name = local_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&remote_path)
            .to_string();
        let request = sftp_transfer::SftpDownloadRequest {
            connection: self.upload_connection_identity.clone(),
            connection_source: SftpUploadConnection::SessionManager(self.session_manager.clone()),
            remote_path: remote_path.clone(),
            local_path: local_path.clone(),
            is_dir,
            display_name: name.clone(),
            title: format!("{} · {name}", t!("FileManager.download")).into(),
            task_group: Some(self.background_task_group()),
            task_key: Some(download_task_key(
                &self.upload_connection_identity,
                &remote_path,
                &local_path,
            )),
        };
        self.global_executor
            .update(cx, |executor, cx| executor.submit_download(request, cx));
    }

    fn background_task_group(&self) -> SharedString {
        self.background_task_group.clone()
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

    fn enqueue_delete(
        &mut self,
        targets: Vec<DeleteTarget>,
        remote_dir: String,
        cx: &mut Context<Self>,
    ) {
        if targets.is_empty() {
            return;
        }

        let entries = targets
            .iter()
            .map(|target| SftpRemoteDeleteEntry {
                remote_path: target.path.clone(),
                is_dir: target.is_dir,
            })
            .collect::<Vec<_>>();
        let task_key =
            delete_remote_task_key(&self.upload_connection_identity, &remote_dir, &entries);
        let display_name = remote_delete_display_name(&targets);
        let request = SftpDeleteRemoteRequest {
            connection: self.upload_connection_identity.clone(),
            connection_source: SftpUploadConnection::SessionManager(self.session_manager.clone()),
            entries,
            remote_dir: remote_dir.clone(),
            display_name: display_name.clone(),
            title: format!("{} · {}", t!("FileManager.delete"), display_name).into(),
            task_group: Some(self.background_task_group()),
            task_key: Some(task_key),
        };
        let id = self.global_executor.update(cx, |executor, cx| {
            executor.submit_delete_remote(request, cx)
        });
        self.pending_global_deletes
            .insert(id, GlobalDeleteView { remote_dir });
        cx.notify();
    }

    /// 通过文件选择器上传文件
    fn select_and_upload_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let remote_dir = self.current_path.clone();
        let view = cx.entity().clone();

        let future = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            multiple: true,
            directories: false,
            prompt: Some(t!("FileManager.select_upload_files").to_string().into()),
        });

        window
            .spawn(cx, async move |cx| {
                if let Ok(Ok(Some(paths))) = future.await {
                    if paths.is_empty() {
                        return;
                    }
                    let _ = view.update_in(cx, |this, window, cx| {
                        this.prepare_uploads(paths, &remote_dir, window, cx);
                    });
                }
            })
            .detach();
    }

    /// 通过文件夹选择器上传文件夹
    fn select_and_upload_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let remote_dir = self.current_path.clone();
        let view = cx.entity().clone();

        let future = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            multiple: true,
            directories: true,
            prompt: Some(t!("FileManager.select_upload_folder").to_string().into()),
        });

        window
            .spawn(cx, async move |cx| {
                if let Ok(Ok(Some(paths))) = future.await {
                    if paths.is_empty() {
                        return;
                    }
                    let _ = view.update_in(cx, |this, window, cx| {
                        this.prepare_uploads(paths, &remote_dir, window, cx);
                    });
                }
            })
            .detach();
    }

    fn paste_upload_from_clipboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.connection_state != ConnectionState::Connected {
            window.push_notification(
                Notification::warning(t!("FileManager.clipboard_upload_not_connected").to_string())
                    .autohide(true),
                cx,
            );
            return;
        }

        let Some(item) = cx.read_from_clipboard() else {
            return;
        };

        let upload_paths = match clipboard_upload_paths(&item) {
            Ok(upload_paths) => upload_paths.paths,
            Err(error) => {
                window.push_notification(
                    Notification::error(
                        t!("FileManager.clipboard_read_failed", error = error).to_string(),
                    )
                    .autohide(true),
                    cx,
                );
                return;
            }
        };

        if upload_paths.is_empty() {
            window.push_notification(
                Notification::info(t!("FileManager.clipboard_no_uploads").to_string())
                    .autohide(true),
                cx,
            );
            return;
        }

        let remote_dir = self.current_path.clone();
        self.prepare_uploads(upload_paths, &remote_dir, window, cx);
    }

    fn show_new_folder_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("FileManager.new_folder_placeholder"))
        });
        let view = cx.entity().downgrade();

        input.update(cx, |state, cx| {
            state.focus(window, cx);
        });

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let view_clone = view.clone();
            let input_for_callback = input.clone();

            dialog
                .title(t!("FileManager.new_folder").to_string())
                .w(px(360.))
                .child(Input::new(&input))
                .button_props(
                    DialogButtonProps::default()
                        .show_cancel(true)
                        .ok_text(t!("Common.create").to_string())
                        .cancel_text(t!("Common.cancel").to_string()),
                )
                .on_ok(move |_, window, cx| {
                    let folder_name = input_for_callback.read(cx).text().to_string();
                    let folder_name = folder_name.trim().to_string();
                    if folder_name.is_empty() {
                        return false;
                    }
                    if !is_valid_entry_name(&folder_name) {
                        window.push_notification(
                            Notification::error(t!("FileManager.invalid_name")),
                            cx,
                        );
                        return false;
                    }

                    let _ = view_clone.update(cx, |this, cx| {
                        let Some(client) = this.sftp_client.clone() else {
                            return;
                        };

                        let remote_path = join_remote_path(&this.current_path, &folder_name);
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
                                        this.refresh_dir(cx);
                                    });
                                }
                                Ok(Err(e)) => {
                                    tracing::error!("创建远程文件夹失败: {}", e);
                                    let error_msg =
                                        t!("FileManager.create_folder_failed", error = e)
                                            .to_string();
                                    let _ = view.update_in(cx, |_this, window, cx| {
                                        window.push_notification(
                                            Notification::error(error_msg.clone()),
                                            cx,
                                        );
                                    });
                                }
                                Err(e) => {
                                    tracing::error!("远程创建文件夹任务失败: {}", e);
                                    let error_msg =
                                        t!("FileManager.create_folder_failed", error = e)
                                            .to_string();
                                    let _ = view.update_in(cx, |_this, window, cx| {
                                        window.push_notification(
                                            Notification::error(error_msg.clone()),
                                            cx,
                                        );
                                    });
                                }
                            })
                            .detach();
                    });
                    false
                })
        });
    }

    fn show_new_file_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("FileManager.new_file_placeholder"))
        });
        let view = cx.entity().downgrade();

        input.update(cx, |state, cx| {
            state.focus(window, cx);
        });

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let view_clone = view.clone();
            let input_for_callback = input.clone();

            dialog
                .title(t!("FileManager.new_file").to_string())
                .w(px(360.))
                .child(Input::new(&input))
                .button_props(
                    DialogButtonProps::default()
                        .show_cancel(true)
                        .ok_text(t!("Common.create").to_string())
                        .cancel_text(t!("Common.cancel").to_string()),
                )
                .on_ok(move |_, window, cx| {
                    let file_name = input_for_callback.read(cx).text().to_string();
                    let file_name = file_name.trim().to_string();
                    if file_name.is_empty() {
                        return false;
                    }
                    if !is_valid_entry_name(&file_name) {
                        window.push_notification(
                            Notification::error(t!("FileManager.invalid_name")),
                            cx,
                        );
                        return false;
                    }

                    let _ = view_clone.update(cx, |this, cx| {
                        let Some(client) = this.sftp_client.clone() else {
                            return;
                        };

                        let remote_path =
                            build_new_file_target_path(&this.current_path, &file_name);
                        let task = Tokio::spawn(cx, async move {
                            let mut client = client.lock().await;
                            client.write_file(&remote_path, &[]).await
                        });

                        let view = cx.entity().clone();
                        window
                            .spawn(cx, async move |cx| match task.await {
                                Ok(Ok(_)) => {
                                    let _ = view.update_in(cx, |this, window, cx| {
                                        window.close_dialog(cx);
                                        this.refresh_dir(cx);
                                    });
                                }
                                Ok(Err(error)) => {
                                    let message =
                                        t!("FileManager.create_file_failed", error = error)
                                            .to_string();
                                    let _ = view.update_in(cx, |_this, window, cx| {
                                        window.push_notification(Notification::error(message), cx);
                                    });
                                }
                                Err(error) => {
                                    let message =
                                        t!("FileManager.create_file_failed", error = error)
                                            .to_string();
                                    let _ = view.update_in(cx, |_this, window, cx| {
                                        window.push_notification(Notification::error(message), cx);
                                    });
                                }
                            })
                            .detach();
                    });
                    false
                })
        });
    }

    fn rename_item(
        &mut self,
        name: String,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("FileManager.rename_placeholder"))
        });
        let view = cx.entity().downgrade();

        input.update(cx, |state, cx| {
            state.set_value(&name, window, cx);
            state.focus(window, cx);
        });

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let view_clone = view.clone();
            let input_for_callback = input.clone();
            let old_path = path.clone();

            dialog
                .title(t!("FileManager.rename").to_string())
                .w(px(360.))
                .child(Input::new(&input))
                .button_props(
                    DialogButtonProps::default()
                        .show_cancel(true)
                        .ok_text(t!("FileManager.rename").to_string())
                        .cancel_text(t!("Common.cancel").to_string()),
                )
                .on_ok(move |_, window, cx| {
                    let new_name = input_for_callback.read(cx).text().to_string();
                    let new_name = new_name.trim().to_string();
                    if new_name.is_empty() {
                        return false;
                    }
                    if !is_valid_entry_name(&new_name) {
                        window.push_notification(
                            Notification::error(t!("FileManager.invalid_name")),
                            cx,
                        );
                        return false;
                    }

                    let old_path_for_task = old_path.clone();
                    let _ = view_clone.update(cx, |this, cx| {
                        let Some(client) = this.sftp_client.clone() else {
                            return;
                        };

                        let old_path = old_path_for_task.clone();
                        let new_path = build_rename_target_path(&old_path, &new_name);
                        let task = Tokio::spawn(cx, async move {
                            let mut client = client.lock().await;
                            client.rename(&old_path, &new_path).await
                        });

                        let view = cx.entity().clone();
                        window
                            .spawn(cx, async move |cx| match task.await {
                                Ok(Ok(())) => {
                                    let _ = view.update_in(cx, |this, window, cx| {
                                        window.close_dialog(cx);
                                        this.refresh_dir(cx);
                                    });
                                }
                                Ok(Err(error)) => {
                                    let message =
                                        t!("FileManager.rename_failed", error = error).to_string();
                                    let _ = view.update_in(cx, |_this, window, cx| {
                                        window.push_notification(Notification::error(message), cx);
                                    });
                                }
                                Err(error) => {
                                    let message =
                                        t!("FileManager.rename_failed", error = error).to_string();
                                    let _ = view.update_in(cx, |_this, window, cx| {
                                        window.push_notification(Notification::error(message), cx);
                                    });
                                }
                            })
                            .detach();
                    });
                    false
                })
        });
    }

    fn extract_archive(
        &mut self,
        name: String,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_extract.is_some() {
            window.push_notification(Notification::info(t!("FileManager.extract_running")), cx);
            return;
        }

        let Some(command) =
            build_remote_extract_command(&path, &name, ExtractConflictAction::Overwrite)
        else {
            window.push_notification(
                Notification::error(t!("FileManager.extract_unsupported")),
                cx,
            );
            return;
        };

        let Some(check_command) = build_remote_extract_conflict_check_command(&path, &name) else {
            window.push_notification(
                Notification::error(t!("FileManager.extract_unsupported")),
                cx,
            );
            return;
        };

        let session_manager = self.session_manager.clone();
        let view = cx.entity().clone();
        let task = Tokio::spawn(cx, async move {
            remote_extract_has_conflict(session_manager, &check_command).await
        });

        window
            .spawn(cx, async move |cx| match task.await {
                Ok(Ok(true)) => {
                    let _ = view.update_in(cx, |this, window, cx| {
                        this.show_extract_conflict_dialog(name, path, command, window, cx);
                    });
                }
                Ok(Ok(false)) => {
                    let _ = view.update_in(cx, |this, window, cx| {
                        this.start_extract_archive(name, path, command, window, cx);
                    });
                }
                Ok(Err(error)) => {
                    let message = t!("FileManager.extract_check_failed", error = error).to_string();
                    let _ = view.update_in(cx, |_this, window, cx| {
                        window.push_notification(Notification::error(message), cx);
                    });
                }
                Err(error) => {
                    let message = t!("FileManager.extract_check_failed", error = error).to_string();
                    let _ = view.update_in(cx, |_this, window, cx| {
                        window.push_notification(Notification::error(message), cx);
                    });
                }
            })
            .detach();
    }

    fn show_extract_conflict_dialog(
        &mut self,
        name: String,
        path: String,
        overwrite_command: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(skip_command) =
            build_remote_extract_command(&path, &name, ExtractConflictAction::SkipExisting)
        else {
            window.push_notification(
                Notification::error(t!("FileManager.extract_unsupported")),
                cx,
            );
            return;
        };

        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let view_skip = view.clone();
            let view_overwrite = view.clone();
            let skip_name = name.clone();
            let skip_path = path.clone();
            let overwrite_name = name.clone();
            let overwrite_path = path.clone();
            let skip_command = skip_command.clone();
            let overwrite_command = overwrite_command.clone();

            dialog
                .title(t!("FileManager.extract_conflict_title").to_string())
                .w(px(380.))
                .child(div().text_sm().child(t!(
                    "FileManager.extract_conflict_message",
                    name = name.clone()
                )))
                .child(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("extract-cancel")
                                .label(t!("Common.cancel").to_string())
                                .ghost()
                                .on_click(|_, window, cx| {
                                    window.close_dialog(cx);
                                }),
                        )
                        .child(
                            Button::new("extract-skip-existing")
                                .label(t!("FileManager.extract_skip_existing").to_string())
                                .ghost()
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    let _ = view_skip.update(cx, |this, cx| {
                                        this.start_extract_archive(
                                            skip_name.clone(),
                                            skip_path.clone(),
                                            skip_command.clone(),
                                            window,
                                            cx,
                                        );
                                    });
                                }),
                        )
                        .child(
                            Button::new("extract-overwrite")
                                .label(t!("Conflict.overwrite").to_string())
                                .primary()
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    let _ = view_overwrite.update(cx, |this, cx| {
                                        this.start_extract_archive(
                                            overwrite_name.clone(),
                                            overwrite_path.clone(),
                                            overwrite_command.clone(),
                                            window,
                                            cx,
                                        );
                                    });
                                }),
                        ),
                )
        });
    }

    fn start_extract_archive(
        &mut self,
        name: String,
        _path: String,
        command: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_extract.is_some() {
            window.push_notification(Notification::info(t!("FileManager.extract_running")), cx);
            return;
        }

        let background_task = self.register_non_cancellable_background_task(
            "sftp-extract",
            format!("{} · {name}", t!("FileManager.extract_running")),
            cx,
        );
        self.active_extract = Some(ActiveExtract { background_task });
        cx.notify();

        let session_manager = self.session_manager.clone();
        let view = cx.entity().clone();
        let task = Tokio::spawn(cx, async move {
            exec_remote_command(session_manager, &command).await
        });

        window
            .spawn(cx, async move |cx| match task.await {
                Ok(Ok(_)) => {
                    let _ = view.update_in(cx, |this, window, cx| {
                        if let Some(extract) = this.active_extract.take() {
                            extract.background_task.succeed(None, cx);
                        }
                        window.push_notification(
                            Notification::success(t!("FileManager.extract_success")),
                            cx,
                        );
                        this.refresh_dir(cx);
                    });
                }
                Ok(Err(error)) => {
                    let message = t!("FileManager.extract_failed", error = error).to_string();
                    let _ = view.update_in(cx, |this, window, cx| {
                        if let Some(extract) = this.active_extract.take() {
                            extract.background_task.fail(message.clone(), cx);
                        }
                        window.push_notification(Notification::error(message), cx);
                    });
                }
                Err(error) => {
                    let message = t!("FileManager.extract_failed", error = error).to_string();
                    let _ = view.update_in(cx, |this, window, cx| {
                        if let Some(extract) = this.active_extract.take() {
                            extract.background_task.fail(message.clone(), cx);
                        }
                        window.push_notification(Notification::error(message), cx);
                    });
                }
            })
            .detach();
    }

    fn delete_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let targets = delete_targets_for_selection(
            &self.current_path,
            &self.items,
            &self.filtered_indices,
            &self.selected_indices,
        );
        self.show_delete_confirmation(targets, window, cx);
    }

    fn delete_item(
        &mut self,
        name: String,
        path: String,
        is_dir: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_delete_confirmation(vec![DeleteTarget { name, path, is_dir }], window, cx);
    }

    fn delete_context_item_or_selection(
        &mut self,
        filtered_ix: usize,
        name: String,
        path: String,
        is_dir: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if should_use_context_selection(&self.selected_indices, filtered_ix) {
            self.delete_selected(window, cx);
        } else {
            self.delete_item(name, path, is_dir, window, cx);
        }
    }

    fn show_delete_confirmation(
        &mut self,
        targets: Vec<DeleteTarget>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if targets.is_empty() {
            return;
        }

        let remote_dir = self.current_path.clone();
        let view = cx.entity().downgrade();
        let file_count = targets.iter().filter(|target| !target.is_dir).count();
        let dir_count = targets.iter().filter(|target| target.is_dir).count();
        let confirm_msg = match (file_count, dir_count) {
            (0, 1) => t!("FileManager.confirm_delete_folder").to_string(),
            (0, d) => t!("FileManager.confirm_delete_folders", count = d).to_string(),
            (1, 0) => t!("FileManager.confirm_delete_file").to_string(),
            (f, 0) => t!("FileManager.confirm_delete_files", count = f).to_string(),
            (f, d) => t!("FileManager.confirm_delete_mixed", files = f, dirs = d).to_string(),
        };
        let target_list = delete_target_preview(&targets);

        window.open_dialog(cx, move |dialog, _window, cx| {
            let view_confirm = view.clone();
            let targets_confirm = targets.clone();
            let remote_dir_confirm = remote_dir.clone();

            dialog
                .title(t!("FileManager.confirm_delete_title").to_string())
                .w(px(400.))
                .child(
                    v_flex().gap_2().child(confirm_msg.clone()).child(
                        div()
                            .p_2()
                            .bg(cx.theme().secondary)
                            .rounded_md()
                            .text_sm()
                            .overflow_hidden()
                            .child(target_list.clone()),
                    ),
                )
                .button_props(
                    DialogButtonProps::default()
                        .show_cancel(true)
                        .ok_text(t!("FileManager.delete").to_string())
                        .cancel_text(t!("Common.cancel").to_string()),
                )
                .on_ok(move |_, window, cx| {
                    window.close_dialog(cx);
                    let _ = view_confirm.update(cx, |this, cx| {
                        this.enqueue_delete(
                            targets_confirm.clone(),
                            remote_dir_confirm.clone(),
                            cx,
                        );
                    });
                    true
                })
        });
    }

    /// 通过保存目录选择器下载远程文件/文件夹
    fn download_item(
        &mut self,
        remote_path: String,
        is_dir: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let remote_name = remote_path
            .rsplit('/')
            .next()
            .unwrap_or(&remote_path)
            .to_string();

        self.download_targets(
            vec![DownloadTarget {
                name: remote_name,
                path: remote_path,
                is_dir,
            }],
            window,
            cx,
        );
    }

    fn download_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let targets = download_targets_for_selection(
            &self.current_path,
            &self.items,
            &self.filtered_indices,
            &self.selected_indices,
        );
        self.download_targets(targets, window, cx);
    }

    fn download_context_item_or_selection(
        &mut self,
        filtered_ix: usize,
        remote_path: String,
        is_dir: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if should_use_context_selection(&self.selected_indices, filtered_ix) {
            self.download_selected(window, cx);
        } else {
            self.download_item(remote_path, is_dir, window, cx);
        }
    }

    fn download_targets(
        &mut self,
        targets: Vec<DownloadTarget>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if targets.is_empty() {
            return;
        }

        let view = cx.entity().clone();

        let future = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            multiple: false,
            directories: true,
            prompt: Some(t!("FileManager.select_download_dir").to_string().into()),
        });

        cx.spawn(async move |_this, cx| {
            if let Ok(Ok(Some(paths))) = future.await {
                if let Some(dir) = paths.first() {
                    view.update(cx, |this, cx| {
                        for target in &targets {
                            this.enqueue_download(
                                target.path.clone(),
                                dir.join(&target.name),
                                target.is_dir,
                                cx,
                            );
                        }
                    });
                }
            }
        })
        .detach();
    }

    fn open_remote_file(&self, full_path: String, window: &mut Window, cx: &mut Context<Self>) {
        if image_format_for_path(&full_path).is_some() {
            let Some(client) = self.sftp_client.clone() else {
                window.push_notification(
                    Notification::error(t!("FileManager.sftp_not_connected").to_string()),
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
                Notification::error(t!("FileManager.sftp_not_connected").to_string()),
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
                Notification::error(t!("FileManager.sftp_not_connected").to_string()),
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
        let panel = cx.entity().downgrade();
        RemoteMutationCallback::new(move |cx| {
            let _ = panel.update(cx, |this, cx| this.refresh_dir(cx));
        })
    }

    // ── 渲染方法 ──────────────────────────────────────────────

    /// 渲染工具栏
    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let can_go_back = self.history_index > 0;
        let upload_panel = cx.entity();
        let has_selection = !self.selected_indices.is_empty();
        let is_connected = self.connection_state == ConnectionState::Connected;
        let can_paste = can_paste_remote_file_clipboard(self.file_clipboard.as_ref(), is_connected);
        let paste_target_dir = self.current_path.clone();
        let is_favorite = self.is_current_path_favorite();
        let favorite_paths = self.favorite_paths.clone();
        let border = self.colors.border;
        let panel = self.colors.muted;
        let hover = self.colors.muted.opacity(0.72);
        let field_bg = self.colors.background;
        let foreground = self.colors.foreground;
        let muted_foreground = self.colors.muted_foreground;
        let breadcrumb = self
            .render_path_breadcrumb(cx)
            .colors(foreground, muted_foreground);
        v_flex()
            .border_b_1()
            .border_color(border)
            .bg(panel)
            .child(
                h_flex()
                    .h_9()
                    .px_2()
                    .gap_1()
                    .items_center()
                    // 后退按钮
                    .child(
                        div()
                            .id("fm-back")
                            .cursor_pointer()
                            .rounded_md()
                            .p(px(5.))
                            .when(!can_go_back, |el| el.opacity(0.4))
                            .when(can_go_back, |el| el.hover(move |s| s.bg(hover)))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _window, cx| {
                                    this.go_back(cx);
                                }),
                            )
                            .tooltip(move |window, cx| {
                                Tooltip::new(t!("FileManager.go_back").to_string())
                                    .build(window, cx)
                            })
                            .child(
                                Icon::new(IconName::ArrowLeft)
                                    .small()
                                    .text_color(muted_foreground),
                            ),
                    )
                    // Home 按钮
                    .child(
                        div()
                            .id("fm-home")
                            .cursor_pointer()
                            .rounded_md()
                            .p(px(5.))
                            .hover(move |s| s.bg(hover))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _window, cx| {
                                    this.go_home(cx);
                                }),
                            )
                            .tooltip(move |window, cx| {
                                Tooltip::new(t!("FileManager.go_home").to_string())
                                    .build(window, cx)
                            })
                            .child(
                                Icon::new(IconName::Home)
                                    .small()
                                    .text_color(muted_foreground),
                            ),
                    )
                    // 上级目录按钮
                    .child(
                        div()
                            .id("fm-parent")
                            .cursor_pointer()
                            .rounded_md()
                            .p(px(5.))
                            .when(self.is_at_root(), |el| el.opacity(0.4))
                            .when(!self.is_at_root(), |el| el.hover(move |s| s.bg(hover)))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _window, cx| {
                                    this.go_parent(cx);
                                }),
                            )
                            .tooltip(move |window, cx| {
                                Tooltip::new(t!("FileManager.go_parent").to_string())
                                    .build(window, cx)
                            })
                            .child(
                                Icon::new(IconName::ArrowUp)
                                    .small()
                                    .text_color(muted_foreground),
                            ),
                    )
                    .child(
                        Button::new("fm-upload")
                            .ghost()
                            .small()
                            .compact()
                            .icon(IconName::Ellipsis)
                            .text_color(muted_foreground)
                            .tooltip(t!("File.actions"))
                            .dropdown_menu_with_anchor(
                                Anchor::TopRight,
                                move |menu, window, _cx| {
                                    let paste_panel = upload_panel.clone();
                                    let paste_target_dir = paste_target_dir.clone();
                                    let upload_files_panel = upload_panel.clone();
                                    let upload_folder_panel = upload_panel.clone();
                                    let new_file_panel = upload_panel.clone();
                                    let new_folder_panel = upload_panel.clone();
                                    let download_panel = upload_panel.clone();
                                    let delete_panel = upload_panel.clone();
                                    menu.item(
                                        PopupMenuItem::new(t!("FileManager.paste"))
                                            .icon(IconName::Paste)
                                            .disabled(!can_paste)
                                            .on_click(window.listener_for(
                                                &paste_panel,
                                                move |this, _, window, cx| {
                                                    this.paste_remote_file_clipboard(
                                                        paste_target_dir.clone(),
                                                        window,
                                                        cx,
                                                    );
                                                },
                                            )),
                                    )
                                    .separator()
                                    .item(
                                        PopupMenuItem::new(t!("FileManager.upload_file"))
                                            .icon(IconName::Upload)
                                            .on_click(window.listener_for(
                                                &upload_files_panel,
                                                move |this, _, window, cx| {
                                                    this.select_and_upload_files(window, cx);
                                                },
                                            )),
                                    )
                                    .item(
                                        PopupMenuItem::new(t!("FileManager.upload_folder"))
                                            .icon(IconName::Upload)
                                            .on_click(window.listener_for(
                                                &upload_folder_panel,
                                                move |this, _, window, cx| {
                                                    this.select_and_upload_folder(window, cx);
                                                },
                                            )),
                                    )
                                    .separator()
                                    .item(
                                        PopupMenuItem::new(t!("FileManager.new_file"))
                                            .icon(IconName::File)
                                            .on_click(window.listener_for(
                                                &new_file_panel,
                                                move |this, _, window, cx| {
                                                    this.show_new_file_dialog(window, cx);
                                                },
                                            )),
                                    )
                                    .item(
                                        PopupMenuItem::new(t!("FileManager.new_folder"))
                                            .icon(IconName::NewFolder)
                                            .on_click(window.listener_for(
                                                &new_folder_panel,
                                                move |this, _, window, cx| {
                                                    this.show_new_folder_dialog(window, cx);
                                                },
                                            )),
                                    )
                                    .item(
                                        PopupMenuItem::new(t!("FileManager.download"))
                                            .icon(IconName::ArrowDown)
                                            .disabled(!has_selection)
                                            .on_click(window.listener_for(
                                                &download_panel,
                                                move |this, _, window, cx| {
                                                    this.download_selected(window, cx);
                                                },
                                            )),
                                    )
                                    .item(
                                        PopupMenuItem::new(t!("FileManager.delete"))
                                            .icon(IconName::Remove)
                                            .disabled(!has_selection)
                                            .on_click(window.listener_for(
                                                &delete_panel,
                                                move |this, _, window, cx| {
                                                    this.delete_selected(window, cx);
                                                },
                                            )),
                                    )
                                },
                            ),
                    )
                    .child(div().flex_1())
                    // 在独立页签中打开 SFTP
                    .child(
                        div()
                            .id("fm-open-sftp")
                            .cursor_pointer()
                            .rounded_md()
                            .p(px(5.))
                            .hover(move |s| s.bg(hover))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _window, cx| {
                                    cx.emit(FileManagerPanelEvent::OpenSftp(
                                        this.stored_connection.clone(),
                                    ));
                                }),
                            )
                            .tooltip(move |window, cx| {
                                Tooltip::new(t!("FileManager.open_sftp").to_string())
                                    .build(window, cx)
                            })
                            .child(
                                Icon::new(IconName::FolderOpen)
                                    .small()
                                    .text_color(muted_foreground),
                            ),
                    )
                    // 自动跟随终端工作目录开关
                    .child({
                        let follow_terminal_cwd = self.follow_terminal_cwd;
                        div()
                            .id("fm-follow-terminal")
                            .cursor_pointer()
                            .rounded_md()
                            .p(px(5.))
                            .hover(move |s| s.bg(hover))
                            .when(follow_terminal_cwd, |el| el.bg(hover))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |_this, _, _window, cx| {
                                    cx.emit(FileManagerPanelEvent::ToggleFollowTerminalCwd);
                                }),
                            )
                            .tooltip(move |window, cx| {
                                let key = if follow_terminal_cwd {
                                    "FileManager.follow_terminal_dir_on"
                                } else {
                                    "FileManager.follow_terminal_dir_off"
                                };
                                Tooltip::new(t!(key).to_string()).build(window, cx)
                            })
                            .child(
                                Icon::new(IconName::LocateActiveTab)
                                    .small()
                                    .text_color(muted_foreground),
                            )
                    })
                    // 同步终端工作目录按钮
                    .child(
                        div()
                            .id("fm-sync-terminal")
                            .cursor_pointer()
                            .rounded_md()
                            .p(px(5.))
                            .hover(move |s| s.bg(hover))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |_this, _, _window, cx| {
                                    cx.emit(FileManagerPanelEvent::SyncWorkingDir);
                                }),
                            )
                            .tooltip(move |window, cx| {
                                Tooltip::new(t!("FileManager.sync_terminal_dir").to_string())
                                    .build(window, cx)
                            })
                            .child(
                                Icon::new(IconName::Sync)
                                    .small()
                                    .text_color(muted_foreground),
                            ),
                    )
                    // 刷新按钮
                    .child(
                        div()
                            .id("fm-refresh")
                            .cursor_pointer()
                            .rounded_md()
                            .p(px(5.))
                            .hover(move |s| s.bg(hover))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _window, cx| {
                                    this.refresh_dir(cx);
                                }),
                            )
                            .tooltip(move |window, cx| {
                                Tooltip::new(t!("FileManager.refresh").to_string())
                                    .build(window, cx)
                            })
                            .child(
                                Icon::new(IconName::Refresh)
                                    .small()
                                    .text_color(muted_foreground),
                            ),
                    )
                    // 隐藏文件开关
                    .child(
                        div()
                            .id("fm-hidden")
                            .cursor_pointer()
                            .rounded_md()
                            .p(px(5.))
                            .hover(move |s| s.bg(hover))
                            .when(self.show_hidden, |el| el.bg(hover))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _window, cx| {
                                    this.show_hidden = !this.show_hidden;
                                    this.apply_filter();
                                    this.clear_selection();
                                    cx.notify();
                                }),
                            )
                            .tooltip(move |window, cx| {
                                Tooltip::new(t!("FileManager.toggle_hidden").to_string())
                                    .build(window, cx)
                            })
                            .child(
                                Icon::new(IconName::Eye)
                                    .small()
                                    .text_color(muted_foreground),
                            ),
                    )
                    .child(self.render_frame_options_button(cx))
                    // 关闭按钮
                    .child(
                        div()
                            .id("fm-close")
                            .cursor_pointer()
                            .rounded_md()
                            .p(px(5.))
                            .hover(move |s| s.bg(hover))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |_this, _, _window, cx| {
                                    cx.emit(FileManagerPanelEvent::Close);
                                }),
                            )
                            .child(
                                Icon::new(IconName::Close)
                                    .small()
                                    .text_color(muted_foreground),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .h_8()
                    .px_2()
                    .pb_2()
                    .gap_1()
                    .items_center()
                    .child(if self.path_editing {
                        h_flex()
                            .id("fm-path-editor")
                            .flex_1()
                            .min_w(px(0.))
                            .h_7()
                            .px_2()
                            .items_center()
                            .bg(field_bg)
                            .rounded_md()
                            .child(
                                Input::new(&self.path_input)
                                    .small()
                                    .appearance(false)
                                    .cleanable(false)
                                    .text_color(foreground)
                                    .w_full(),
                            )
                            .into_any_element()
                    } else {
                        h_flex()
                            .id("fm-path")
                            .flex_1()
                            .min_w(px(0.))
                            .h_7()
                            .px_2()
                            .items_center()
                            .bg(field_bg)
                            .text_color(foreground)
                            .cursor_text()
                            .rounded_md()
                            .hover(move |style| style.bg(hover))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.start_path_editing(window, cx);
                            }))
                            .child(breadcrumb.flex_1().min_w(px(0.)).overflow_hidden())
                            .tooltip(move |window, cx| {
                                Tooltip::new(t!("FileManager.edit_path").to_string())
                                    .build(window, cx)
                            })
                            .into_any_element()
                    })
                    .child(
                        Button::new("fm-toggle-favorite")
                            .custom(self.colors.icon_button_variant(muted_foreground, cx))
                            .small()
                            .icon(if is_favorite {
                                IconName::StarFill
                            } else {
                                IconName::Star
                            })
                            .text_color(muted_foreground)
                            .tooltip(if is_favorite {
                                t!("FileManager.favorite_remove_current").to_string()
                            } else {
                                t!("FileManager.favorite_add_current").to_string()
                            })
                            .disabled(!is_connected)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_current_favorite(window, cx);
                            })),
                    )
                    .child(self.render_favorites_menu(favorite_paths, is_connected, cx)),
            )
    }

    fn render_frame_options_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = cx.entity();
        let placement = self.frame_placement;
        Button::new("fm-frame-options")
            .custom(
                self.colors
                    .icon_button_variant(self.colors.muted_foreground, cx),
            )
            .small()
            .icon(IconName::Ellipsis)
            .text_color(self.colors.muted_foreground)
            .tooltip(t!("FileManager.panel_options").to_string())
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, window, cx| {
                build_frame_options_menu(menu, panel.clone(), placement, window, cx)
            })
    }

    fn render_favorites_menu(
        &self,
        favorite_paths: Vec<String>,
        is_connected: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let has_favorites = !favorite_paths.is_empty();
        let search_input = self.favorite_search_input.clone();
        let edit_input = self.favorite_edit_input.clone();
        let editing_path = self.favorite_editing_path.clone();
        let view = cx.entity().clone();
        let query = search_input.read(cx).text().to_string().to_lowercase();
        let query = query.trim().to_string();
        let filtered_paths: Vec<String> = favorite_paths
            .into_iter()
            .filter(|path| query.is_empty() || path.to_lowercase().contains(&query))
            .collect();

        Popover::new("fm-favorite-paths-popover")
            .open(self.favorite_popover_open)
            .on_open_change(cx.listener(|this, open, _window, cx| {
                this.favorite_popover_open = *open;
                if !*open {
                    this.favorite_editing_path = None;
                }
                cx.notify();
            }))
            .trigger(
                Button::new("fm-favorite-paths")
                    .ghost()
                    .small()
                    .icon(IconName::FolderOpen)
                    .text_color(self.colors.muted_foreground)
                    .tooltip(t!("FileManager.favorite_open").to_string())
                    .disabled(!is_connected || !has_favorites),
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
                            .child(t!("FileManager.favorite_no_results").to_string()),
                    );
                }

                for path in filtered_paths.iter().cloned() {
                    let is_editing = editing_path.as_deref() == Some(path.as_str());
                    list = list.child(Self::render_favorite_path_row(
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
        path: String,
        is_editing: bool,
        edit_input: Entity<InputState>,
        view: Entity<FileManagerPanel>,
        window: &mut Window,
        cx: &mut Context<PopoverState>,
    ) -> impl IntoElement {
        if is_editing {
            let save_path = path.clone();
            let cancel_path = path.clone();
            return h_flex()
                .id(SharedString::from(format!("fm-favorite-edit-row-{path}")))
                .gap_1()
                .items_center()
                .child(Input::new(&edit_input).small().cleanable(false).flex_1())
                .child(
                    Button::new(SharedString::from(format!("fm-favorite-save-{save_path}")))
                        .icon(IconName::Check)
                        .ghost()
                        .small()
                        .tooltip(t!("FileManager.favorite_save").to_string())
                        .on_click(window.listener_for(&view, |this, _, window, cx| {
                            this.save_editing_favorite_path(window, cx);
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!(
                        "fm-favorite-cancel-{cancel_path}"
                    )))
                    .icon(IconName::Close)
                    .ghost()
                    .small()
                    .tooltip(t!("FileManager.favorite_cancel").to_string())
                    .on_click(window.listener_for(
                        &view,
                        |this, _, _window, cx| {
                            this.cancel_favorite_path_editing(cx);
                        },
                    )),
                )
                .into_any_element();
        }

        let navigate_path = path.clone();
        let edit_path = path.clone();
        let remove_path = path.clone();

        h_flex()
            .id(SharedString::from(format!("fm-favorite-row-{path}")))
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
                        window.listener_for(&view, move |this, _, _window, cx| {
                            this.navigate_to(navigate_path.clone(), cx);
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
                Button::new(SharedString::from(format!("fm-favorite-edit-{edit_path}")))
                    .icon(IconName::Edit)
                    .ghost()
                    .small()
                    .tooltip(t!("FileManager.favorite_edit").to_string())
                    .on_click(window.listener_for(&view, move |this, _, window, cx| {
                        this.start_favorite_path_editing(edit_path.clone(), window, cx);
                    })),
            )
            .child(
                Button::new(SharedString::from(format!(
                    "fm-favorite-remove-{remove_path}"
                )))
                .icon(IconName::Remove)
                .ghost()
                .small()
                .tooltip(t!("FileManager.favorite_delete").to_string())
                .on_click(window.listener_for(
                    &view,
                    move |this, _, window, cx| {
                        this.remove_favorite_path(&remove_path, window, cx);
                    },
                )),
            )
            .into_any_element()
    }

    /// 渲染搜索栏
    fn render_search_bar(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        let has_query = !self.search_query.is_empty();
        let filtered_count = self.filtered_indices.len();
        let total_count = self.items.len();
        let border = self.colors.border;
        let background = self.colors.background;
        let foreground = self.colors.foreground;
        let muted_foreground = self.colors.muted_foreground;

        h_flex()
            .h_8()
            .px_2()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(border)
            .bg(background)
            .child(
                Icon::new(IconName::Search)
                    .xsmall()
                    .text_color(muted_foreground),
            )
            .child(
                div().flex_1().child(
                    Input::new(&self.search_input)
                        .xsmall()
                        .appearance(false)
                        .text_color(foreground)
                        .cleanable(has_query),
                ),
            )
            .when(has_query, |el| {
                el.child(
                    div()
                        .text_xs()
                        .text_color(muted_foreground)
                        .child(format!("{}/{}", filtered_count, total_count)),
                )
            })
    }

    /// 渲染排序表头
    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let border = self.colors.border;
        let panel = self.colors.muted;

        h_flex()
            .h_7()
            .px_2()
            .items_center()
            .border_b_1()
            .border_color(border)
            .bg(panel)
            .child(self.render_header_cell(&t!("FileManager.name"), SortColumn::Name, true, cx))
            .child(self.render_header_cell(&t!("FileManager.size"), SortColumn::Size, false, cx))
            .child(self.render_header_cell(
                &t!("FileManager.time"),
                SortColumn::Modified,
                false,
                cx,
            ))
    }

    /// 渲染单个表头列
    fn render_header_cell(
        &self,
        label: &str,
        column: SortColumn,
        is_flex: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_sorted = self.sort_column == column;
        let sort_order = self.sort_order;
        let label = label.to_string();
        let hover = self.colors.muted.opacity(0.72);
        let muted_foreground = self.colors.muted_foreground;

        let base = h_flex()
            .h_full()
            .px_1()
            .items_center()
            .gap_0p5()
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _window, cx| {
                    this.set_sort(column, cx);
                }),
            )
            .child(div().text_xs().text_color(muted_foreground).child(label))
            .when(is_sorted, |el| {
                el.child(
                    Icon::new(if sort_order == SortOrder::Ascending {
                        IconName::ChevronUp
                    } else {
                        IconName::ChevronDown
                    })
                    .xsmall()
                    .text_color(muted_foreground),
                )
            });

        if is_flex {
            base.flex_1()
        } else {
            match column {
                SortColumn::Size => base.w(SIZE_COLUMN_WIDTH),
                SortColumn::Modified => base.w(MODIFIED_COLUMN_WIDTH),
                SortColumn::Name => base,
            }
        }
    }

    /// 渲染单行文件项
    fn render_file_row(
        &self,
        filtered_ix: usize,
        item: &RemoteFileItem,
        full_path: &str,
        is_selected: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let name = item.name.clone();
        let is_dir = item.is_dir;
        let directory_size = item.directory_size;
        let size = size_label(item);
        let path_for_size = full_path.to_string();
        let foreground = self.colors.foreground;
        let muted_foreground = self.colors.muted_foreground;
        let accent = self.colors.accent;
        let selection = self.colors.accent.opacity(0.24);

        h_flex()
            .h(FILE_ROW_HEIGHT)
            .px_2()
            .items_center()
            .text_color(foreground)
            .when(is_selected, |el| el.bg(selection))
            // 名称列
            .child(
                h_flex()
                    .flex_1()
                    .gap_1()
                    .items_center()
                    .overflow_hidden()
                    .child(
                        Icon::new(if is_dir {
                            IconName::Folder1
                        } else {
                            IconName::File
                        })
                        .with_size(IconSize::Small),
                    )
                    .child({
                        let tooltip_name = name.clone();
                        div()
                            .id(SharedString::from(name.clone()))
                            .flex_1()
                            .text_sm()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(name)
                            .tooltip(move |window, cx| {
                                Tooltip::new(tooltip_name.clone()).build(window, cx)
                            })
                    }),
            )
            // 大小列
            .child(
                div()
                    .id(("fm-file-size", filtered_ix))
                    .w(SIZE_COLUMN_WIDTH)
                    .text_xs()
                    .text_color(if is_dir && directory_size == DirectorySizeState::Unknown {
                        accent
                    } else {
                        muted_foreground
                    })
                    .when(
                        is_dir && directory_size == DirectorySizeState::Unknown,
                        |el| {
                            el.cursor_pointer()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.calculate_remote_directory_size(
                                        path_for_size.clone(),
                                        window,
                                        cx,
                                    );
                                }))
                        },
                    )
                    .child(size),
            )
            // 时间列
            .child(
                div()
                    .w(MODIFIED_COLUMN_WIDTH)
                    .text_xs()
                    .text_color(muted_foreground)
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(format_modified_time(item.modified)),
            )
    }

    /// 渲染上级目录行（..）
    fn render_parent_row(&self, _cx: &App) -> impl IntoElement {
        let foreground = self.colors.foreground;

        h_flex()
            .h(FILE_ROW_HEIGHT)
            .px_2()
            .items_center()
            .text_color(foreground)
            .child(
                h_flex()
                    .flex_1()
                    .gap_1()
                    .items_center()
                    .child(Icon::new(IconName::Folder1).with_size(IconSize::Small))
                    .child(div().text_sm().child("..")),
            )
            .child(div().w(SIZE_COLUMN_WIDTH))
            .child(div().w(MODIFIED_COLUMN_WIDTH))
    }

    /// 构建文件项右键菜单
    fn build_context_menu(
        menu: PopupMenu,
        filtered_ix: usize,
        name: &str,
        full_path: &str,
        is_dir: bool,
        view: &Entity<Self>,
        window: &mut Window,
        cx: &mut Context<PopupMenu>,
    ) -> PopupMenu {
        let path_for_cd = full_path.to_string();
        let path_for_copy = full_path.to_string();
        let name_for_copy = name.to_string();
        let name_for_rename = name.to_string();
        let path_for_rename = full_path.to_string();
        let path_for_download = full_path.to_string();
        let is_dir_for_download = is_dir;
        let path_for_edit = full_path.to_string();
        let name_for_extract = name.to_string();
        let path_for_extract = full_path.to_string();
        let path_for_favorite = full_path.to_string();
        let name_for_delete = name.to_string();
        let path_for_delete = full_path.to_string();
        let is_dir_for_delete = is_dir;
        let target_dir_for_paste = if is_dir {
            full_path.to_string()
        } else {
            view.read(cx).current_path.clone()
        };
        let can_paste = {
            let view = view.read(cx);
            can_paste_remote_file_clipboard(
                view.file_clipboard.as_ref(),
                view.connection_state == ConnectionState::Connected,
            )
        };
        let item_for_properties = view
            .read(cx)
            .filtered_indices
            .get(filtered_ix)
            .and_then(|&real_ix| view.read(cx).items.get(real_ix))
            .cloned();

        let mut menu = menu;

        let view_copy_entries = view.clone();
        let view_cut_entries = view.clone();
        let view_paste = view.clone();
        menu = menu
            .item(
                PopupMenuItem::new(t!("FileManager.copy"))
                    .icon(IconName::Copy)
                    .on_click(window.listener_for(
                        &view_copy_entries,
                        move |this, _, window, cx| {
                            this.store_remote_file_clipboard(
                                filtered_ix,
                                RemoteClipboardKind::Copy,
                                window,
                                cx,
                            );
                        },
                    )),
            )
            .item(
                PopupMenuItem::new(t!("FileManager.cut")).on_click(window.listener_for(
                    &view_cut_entries,
                    move |this, _, window, cx| {
                        this.store_remote_file_clipboard(
                            filtered_ix,
                            RemoteClipboardKind::Cut,
                            window,
                            cx,
                        );
                    },
                )),
            )
            .item(
                PopupMenuItem::new(t!("FileManager.paste"))
                    .icon(IconName::Paste)
                    .disabled(!can_paste)
                    .on_click(
                        window.listener_for(&view_paste, move |this, _, window, cx| {
                            this.paste_remote_file_clipboard(
                                target_dir_for_paste.clone(),
                                window,
                                cx,
                            );
                        }),
                    ),
            )
            .separator();

        // 下载
        let view_download = view.clone();
        menu = menu.item(
            PopupMenuItem::new(t!("FileManager.download"))
                .icon(IconName::ArrowDown)
                .on_click(
                    window.listener_for(&view_download, move |this, _, window, cx| {
                        this.download_context_item_or_selection(
                            filtered_ix,
                            path_for_download.clone(),
                            is_dir_for_download,
                            window,
                            cx,
                        );
                    }),
                ),
        );

        let view_rename = view.clone();
        menu = menu.item(
            PopupMenuItem::new(t!("FileManager.rename"))
                .icon(IconName::Edit)
                .on_click(
                    window.listener_for(&view_rename, move |this, _, window, cx| {
                        this.rename_item(
                            name_for_rename.clone(),
                            path_for_rename.clone(),
                            window,
                            cx,
                        );
                    }),
                ),
        );

        if !is_dir {
            let view_edit = view.clone();
            menu = menu.item(
                PopupMenuItem::new(t!("Common.edit"))
                    .icon(IconName::Edit)
                    .on_click(window.listener_for(&view_edit, move |this, _, window, cx| {
                        this.open_remote_file(path_for_edit.clone(), window, cx);
                    })),
            );

            for editor in external_editors_for_file(name, cx) {
                let view_external = view.clone();
                let path_for_external = full_path.to_string();
                let editor_key = editor.editor_key;
                menu = menu.item(
                    PopupMenuItem::new(external_editor_menu_label(&editor.display_name))
                        .icon(IconName::Edit)
                        .on_click(window.listener_for(
                            &view_external,
                            move |this, _, window, cx| {
                                this.open_remote_external_editor(
                                    (path_for_external.clone(), editor_key.clone()),
                                    window,
                                    cx,
                                );
                            },
                        )),
                );
            }

            if archive_kind_for_name(name).is_some() {
                let view_extract = view.clone();
                menu = menu.item(
                    PopupMenuItem::new(t!("FileManager.extract"))
                        .icon(IconName::Unarchive)
                        .on_click(window.listener_for(
                            &view_extract,
                            move |this, _, window, cx| {
                                this.extract_archive(
                                    name_for_extract.clone(),
                                    path_for_extract.clone(),
                                    window,
                                    cx,
                                );
                            },
                        )),
                );
            }
        }

        // 文件夹：在终端中 CD
        if is_dir {
            let view_cd = view.clone();
            let view_favorite = view.clone();
            menu = menu.item(
                PopupMenuItem::new(t!("FileManager.cd_to_terminal"))
                    .icon(IconName::SquareTerminal)
                    .on_click(window.listener_for(&view_cd, move |_this, _, _, cx| {
                        cx.emit(FileManagerPanelEvent::CdToTerminal(path_for_cd.clone()));
                    })),
            );
            menu = menu.item(
                PopupMenuItem::new(t!("FileManager.favorite_add_path"))
                    .icon(IconName::Star)
                    .on_click(
                        window.listener_for(&view_favorite, move |this, _, window, cx| {
                            this.add_favorite_path(&path_for_favorite, window, cx);
                        }),
                    ),
            );
        }

        // 复制路径
        let view_copy_path = view.clone();
        menu = menu.item(
            PopupMenuItem::new(t!("FileManager.copy_path"))
                .icon(IconName::Copy)
                .on_click(
                    window.listener_for(&view_copy_path, move |_this, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(path_for_copy.clone()));
                    }),
                ),
        );

        // 复制名称
        let view_copy_name = view.clone();
        menu = menu.item(
            PopupMenuItem::new(t!("FileManager.copy_name"))
                .icon(IconName::Copy)
                .on_click(
                    window.listener_for(&view_copy_name, move |_this, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(name_for_copy.clone()));
                    }),
                ),
        );

        // 分隔线 + 上传文件 + 上传文件夹 + 刷新
        let view_upload_files = view.clone();
        let view_upload_folder = view.clone();
        let view_delete = view.clone();
        let view_refresh = view.clone();
        menu = menu.separator().item(
            PopupMenuItem::new(t!("FileManager.delete"))
                .icon(IconName::Remove)
                .on_click(
                    window.listener_for(&view_delete, move |this, _, window, cx| {
                        this.delete_context_item_or_selection(
                            filtered_ix,
                            name_for_delete.clone(),
                            path_for_delete.clone(),
                            is_dir_for_delete,
                            window,
                            cx,
                        );
                    }),
                ),
        );

        if let Some(item) = item_for_properties {
            let view_properties = view.clone();
            let path_for_properties = full_path.to_string();
            menu = menu.item(
                PopupMenuItem::new(t!("FileManager.properties"))
                    .icon(IconName::Info)
                    .on_click(
                        window.listener_for(&view_properties, move |this, _, window, cx| {
                            this.show_file_properties(
                                item.clone(),
                                path_for_properties.clone(),
                                window,
                                cx,
                            );
                        }),
                    ),
            );
        }

        menu = menu
            .separator()
            .item(
                PopupMenuItem::new(t!("FileManager.upload_file"))
                    .icon(IconName::Upload)
                    .on_click(window.listener_for(
                        &view_upload_files,
                        move |this, _, window, cx| {
                            this.select_and_upload_files(window, cx);
                        },
                    )),
            )
            .item(
                PopupMenuItem::new(t!("FileManager.upload_folder"))
                    .icon(IconName::Upload)
                    .on_click(window.listener_for(
                        &view_upload_folder,
                        move |this, _, window, cx| {
                            this.select_and_upload_folder(window, cx);
                        },
                    )),
            )
            .separator()
            .item(
                PopupMenuItem::new(t!("FileManager.refresh"))
                    .icon(IconName::Refresh)
                    .on_click(window.listener_for(&view_refresh, move |this, _, _, cx| {
                        this.refresh_dir(cx);
                    })),
            );

        menu
    }

    /// 渲染底部传输进度条（紧凑型，适合侧边栏窄宽度）
    fn render_transfer_progress(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = self
            .upload_progress_view(cx)
            .or_else(|| self.local_progress_view());
        let Some(view) = view else {
            return div().into_any_element();
        };
        self.render_transfer_progress_view(view, cx)
    }

    fn upload_progress_view(&self, cx: &mut Context<Self>) -> Option<TransferProgressView> {
        self.global_executor.read_with(cx, |executor, _| {
            let snapshot = executor.active_for_connection(&self.upload_connection_identity)?;
            let icon = match snapshot.operation {
                SftpTransferOperation::Upload => IconName::ArrowUp,
                SftpTransferOperation::Download => IconName::ArrowDown,
                SftpTransferOperation::DeleteRemote => IconName::Remove,
            };
            Some(TransferProgressView {
                icon,
                label: snapshot.display_name,
                transferred: snapshot.transferred,
                total: snapshot.total.unwrap_or(0),
                speed: snapshot.speed,
                current_file: snapshot.current_file,
                state: upload_progress_state(&snapshot.state),
                pending_count: executor.pending_count(&self.upload_connection_identity),
                cancel_target: TransferCancelTarget::Global(snapshot.id),
            })
        })
    }

    fn local_progress_view(&self) -> Option<TransferProgressView> {
        let task = self.transfer_queue.active_task()?;
        let (icon, label) = match &task.operation {
            TransferOperation::Download { remote_path, .. } => {
                let name = remote_path.rsplit('/').next().unwrap_or(remote_path);
                (IconName::ArrowDown, name.to_string())
            }
        };
        Some(TransferProgressView {
            icon,
            label,
            transferred: task.shared_progress.transferred.load(Ordering::Relaxed),
            total: task.shared_progress.total.load(Ordering::Relaxed),
            speed: f64::from_bits(task.shared_progress.speed.load(Ordering::Relaxed)),
            current_file: task
                .shared_progress
                .current_file
                .read()
                .ok()
                .and_then(|current| current.clone()),
            state: local_progress_state(&task.state),
            pending_count: self.transfer_queue.pending_count(),
            cancel_target: TransferCancelTarget::Local(task.id),
        })
    }

    fn render_transfer_progress_view(
        &self,
        view: TransferProgressView,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let border = self.colors.border;
        let panel = self.colors.muted;
        let hover = self.colors.muted.opacity(0.72);
        let muted_foreground = self.colors.muted_foreground;
        let TransferProgressView {
            icon,
            label,
            transferred,
            total,
            speed,
            current_file,
            state,
            pending_count,
            cancel_target,
        } = view;
        let progress_pct = if total > 0 {
            (transferred as f64 / total as f64 * 100.0) as u32
        } else {
            0
        };
        let status_text = match state {
            TransferProgressState::Pending => t!("FileManager.transfer_pending").to_string(),
            TransferProgressState::Running => {
                if speed > 0.0 {
                    format!("{}% {}", progress_pct, format_speed(speed))
                } else {
                    format!("{}%", progress_pct)
                }
            }
            TransferProgressState::Cancelling => t!("FileManager.transfer_cancelled").to_string(),
        };
        let has_current_file = current_file.is_some();
        let display_label = transfer_progress_display_label(label, current_file);
        let tooltip_label = display_label.clone();
        let can_cancel = !matches!(state, TransferProgressState::Cancelling);

        v_flex()
            .border_t_1()
            .border_color(border)
            .bg(panel)
            .px_2()
            .py_1()
            .gap_1()
            // 第一行：图标 + 文件名 + 状态文本 + 取消按钮
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(Icon::new(icon).xsmall().text_color(muted_foreground))
                    .child(
                        div()
                            .id("fm-transfer-name")
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .overflow_hidden()
                            .child(marquee_text(
                                "fm-transfer-name-marquee",
                                display_label,
                                matches!(state, TransferProgressState::Running) && has_current_file,
                            ))
                            .tooltip(move |window, cx| {
                                Tooltip::new(tooltip_label.clone()).build(window, cx)
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted_foreground)
                            .child(status_text),
                    )
                    .when(can_cancel, |el| {
                        el.child(
                            div()
                                .id("fm-cancel-transfer")
                                .cursor_pointer()
                                .rounded_md()
                                .p(px(2.))
                                .hover(move |s| s.bg(hover))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _window, cx| {
                                        this.cancel_transfer_target(cancel_target, cx);
                                    }),
                                )
                                .child(
                                    Icon::new(IconName::Close)
                                        .xsmall()
                                        .text_color(muted_foreground),
                                ),
                        )
                    }),
            )
            // 第二行：进度条 + 排队数
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        div().flex_1().child(
                            Progress::new("fm-transfer-progress").value(progress_pct as f32),
                        ),
                    )
                    .when(pending_count > 0, |el| {
                        el.child(
                            div()
                                .text_xs()
                                .text_color(muted_foreground)
                                .child(format!("+{}", pending_count)),
                        )
                    }),
            )
            .into_any_element()
    }

    fn cancel_transfer_target(&mut self, target: TransferCancelTarget, cx: &mut Context<Self>) {
        match target {
            TransferCancelTarget::Global(id) => {
                self.global_executor
                    .update(cx, |executor, cx| executor.cancel(id, cx));
            }
            TransferCancelTarget::Local(id) => self.cancel_transfer(id, cx),
        }
    }

    /// 渲染连接中状态
    fn render_connecting(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        let muted_foreground = self.colors.muted_foreground;

        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .child(Spinner::new().small())
            .child(
                div()
                    .text_sm()
                    .text_color(muted_foreground)
                    .child(t!("FileManager.connecting")),
            )
    }

    /// 渲染错误状态
    fn render_error(&self, error: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let accent = self.colors.accent;
        let accent_foreground = self.colors.accent_foreground;

        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .p_4()
            .child(
                Icon::new(IconName::CircleX)
                    .color()
                    .with_size(Size::Large)
                    .text_color(cx.theme().danger),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .text_center()
                    .max_w(px(200.))
                    .child(error.to_string()),
            )
            .child(
                div()
                    .id("fm-retry")
                    .cursor_pointer()
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .bg(accent)
                    .text_color(accent_foreground)
                    .text_sm()
                    .hover(|s| s.opacity(0.9))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _window, cx| {
                            this.connect(cx);
                        }),
                    )
                    .child(t!("FileManager.retry")),
            )
    }

    /// 渲染初始状态（提示连接）
    fn render_idle(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let accent = self.colors.accent;
        let accent_foreground = self.colors.accent_foreground;
        let muted_foreground = self.colors.muted_foreground;

        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .p_4()
            .child(
                Icon::new(IconName::FolderOpen)
                    .color()
                    .with_size(Size::Large)
                    .text_color(muted_foreground),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(muted_foreground)
                    .child(t!("FileManager.title")),
            )
            .child(
                div()
                    .id("fm-connect")
                    .cursor_pointer()
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .bg(accent)
                    .text_color(accent_foreground)
                    .text_sm()
                    .hover(|s| s.opacity(0.9))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _window, cx| {
                            this.connect(cx);
                        }),
                    )
                    .child(t!("FileManager.connect")),
            )
    }

    /// 渲染已连接的文件列表
    fn render_file_list(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let filtered_count = self.filtered_indices.len();
        let show_parent = !self.is_at_root();
        let total_count = if show_parent {
            filtered_count + 1
        } else {
            filtered_count
        };
        let scroll_handle = self.scroll_handle.clone();
        let is_loading = self.loading;
        let has_active_transfer = self.transfer_queue.has_active()
            || self.global_executor.read_with(cx, |executor, _| {
                executor
                    .active_for_connection(&self.upload_connection_identity)
                    .is_some()
            });
        let background = self.colors.background;
        let foreground = self.colors.foreground;
        let hover = self.colors.muted.opacity(0.72);

        v_flex()
            .size_full()
            .bg(background)
            .child(self.render_toolbar(cx))
            .child(self.render_search_bar(cx))
            .child(self.render_header(cx))
            .when(is_loading, |el| {
                el.child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(Spinner::new().small()),
                )
            })
            .when(!is_loading, |el| {
                el.child(
                    div()
                        .id("fm-file-list-drop-zone")
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .relative()
                        .overflow_hidden()
                        .bg(background)
                        // 拖拽上传支持
                        .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                            let file_paths = paths.paths().to_vec();
                            if !file_paths.is_empty() {
                                let remote_dir = this.current_path.clone();
                                this.prepare_uploads(file_paths, &remote_dir, window, cx);
                            }
                        }))
                        .child(
                            uniform_list("fm-file-list", total_count, {
                                cx.processor(
                                    move |state: &mut Self, range: Range<usize>, _window, cx| {
                                        let current_path = state.current_path.clone();
                                        let has_parent = !state.is_at_root();
                                        let view = cx.entity();
                                        range
                                            .map(|list_ix| {
                                                // 上级目录行
                                                if has_parent && list_ix == 0 {
                                                    return div()
                                                        .id(list_ix)
                                                        .cursor_pointer()
                                                        .hover(move |s| s.bg(hover))
                                                        .on_double_click(cx.listener(
                                                            move |this, _, _window, cx| {
                                                                this.go_parent(cx);
                                                            },
                                                        ))
                                                        .child(state.render_parent_row(cx))
                                                        .into_any_element();
                                                }

                                                let filtered_ix =
                                                    if has_parent { list_ix - 1 } else { list_ix };
                                                let real_ix = state.filtered_indices[filtered_ix];
                                                let item = state.items[real_ix].clone();
                                                let is_selected =
                                                    state.selected_indices.contains(&filtered_ix);
                                                let item_name = item.name.clone();
                                                let is_dir = item.is_dir;
                                                let full_path = if current_path.ends_with('/') {
                                                    format!("{}{}", current_path, item_name)
                                                } else {
                                                    format!("{}/{}", current_path, item_name)
                                                };

                                                // 右键菜单变量
                                                let ctx_name = item_name.clone();
                                                let ctx_full_path = full_path.clone();
                                                let ctx_is_dir = is_dir;
                                                let ctx_view = view.clone();
                                                div()
                                                    .id(list_ix)
                                                    .cursor_pointer()
                                                    .hover(move |s| s.bg(hover))
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        cx.listener(
                                                            move |this,
                                                                  event: &MouseDownEvent,
                                                                  _window,
                                                                  cx| {
                                                                let mode = selection_mode(
                                                                    event.modifiers.shift,
                                                                    event.modifiers.secondary(),
                                                                );
                                                                this.select_row(
                                                                    filtered_ix,
                                                                    mode,
                                                                );
                                                                cx.notify();
                                                            },
                                                        ),
                                                    )
                                                    .on_double_click(cx.listener({
                                                        let fp = full_path.clone();
                                                        move |this, _, window, cx| {
                                                            if is_dir {
                                                                this.navigate_to(
                                                                    fp.clone(),
                                                                    cx,
                                                                );
                                                            } else {
                                                                this.open_remote_file(
                                                                    fp.clone(),
                                                                    window,
                                                                    cx,
                                                                );
                                                            }
                                                        }
                                                    }))
                                                    .context_menu(
                                                        move |menu, window, cx| {
                                                            Self::build_context_menu(
                                                                menu,
                                                                filtered_ix,
                                                                &ctx_name,
                                                                &ctx_full_path,
                                                                ctx_is_dir,
                                                                &ctx_view,
                                                                window,
                                                                cx,
                                                            )
                                                        },
                                                    )
                                                    .child(state.render_file_row(
                                                        filtered_ix,
                                                        &item,
                                                        &full_path,
                                                        is_selected,
                                                        cx,
                                                    ))
                                                    .into_any_element()
                                            })
                                            .collect()
                                    },
                                )
                            })
                            .flex_1()
                            .size_full()
                            .track_scroll(&scroll_handle)
                            .with_sizing_behavior(ListSizingBehavior::Auto),
                        )
                        .vertical_scrollbar(&scroll_handle)
                        .child(render_file_drop_overlay(
                            foreground,
                            cx.theme().drop_target,
                            cx.theme().drag_border,
                        )),
                )
            })
            // 底部传输进度条
            .when(has_active_transfer, |el| {
                el.child(self.render_transfer_progress(cx))
            })
    }
}

fn render_file_drop_overlay(
    foreground: Hsla,
    drop_target: Hsla,
    drag_border: Hsla,
) -> impl IntoElement {
    div()
        .invisible()
        .absolute()
        .inset_0()
        .m_2()
        .bg(drop_target)
        .border_2()
        .border_color(drag_border)
        .rounded_lg()
        .flex()
        .items_center()
        .justify_center()
        .drag_over::<ExternalPaths>(|style, _, _, _| style.visible())
        .child(
            v_flex().items_center().gap_2().child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(foreground)
                    .child(t!("FileManager.drop_files_here")),
            ),
        )
}

/// 获取远程路径的父目录
fn remote_path_parent(path: &str) -> String {
    if path == "/" || path.is_empty() {
        "/".to_string()
    } else {
        let trimmed = path.trim_end_matches('/');
        match trimmed.rfind('/') {
            Some(0) => "/".to_string(),
            Some(pos) => trimmed[..pos].to_string(),
            None => "/".to_string(),
        }
    }
}

impl EventEmitter<FileManagerPanelEvent> for FileManagerPanel {}

impl Focusable for FileManagerPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for FileManagerPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.connection_state.clone();
        let background = self.colors.background;
        let foreground = self.colors.foreground;

        v_flex()
            .size_full()
            .track_focus(&self.focus_handle)
            .key_context(FILE_MANAGER_CONTEXT)
            .on_action(cx.listener(|this, _: &PasteUpload, window, cx| {
                this.paste_upload_from_clipboard(window, cx);
            }))
            .on_action(cx.listener(|this, _: &NavigateParent, _, cx| {
                if this.connection_state == ConnectionState::Connected {
                    this.go_parent(cx);
                }
            }))
            .bg(background)
            .text_color(foreground)
            .child(match state {
                ConnectionState::Idle => self.render_idle(cx).into_any_element(),
                ConnectionState::Connecting => self.render_connecting(cx).into_any_element(),
                ConnectionState::Connected => self.render_file_list(cx).into_any_element(),
                ConnectionState::Error(ref msg) => self.render_error(msg, cx).into_any_element(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ConnectionState, FileConflictChoice, NavigationRecoveryPlan, PendingUpload,
        RemoteClipboardEntry, RemoteClipboardKind, RemoteFileClipboard, SharedProgress,
        TransferCancelTarget, TransferOperation, TransferQueue, TransferTask, TransferTaskState,
        build_navigation_recovery_plan, build_retry_reset_plan, can_paste_remote_file_clipboard,
        clear_remote_listing_state, frame_move_options, resolve_upload_conflict,
        should_apply_directory_result, should_refresh_after_delete, should_refresh_after_upload,
        transfer_progress_display_label,
    };
    use anyhow::{Result, anyhow};
    use async_trait::async_trait;
    use gpui::{AppContext, Entity, TestAppContext, WindowHandle};
    use gpui_component::Root;
    use one_core::sidebar_contribution::SidebarPlacement;
    use one_core::storage::connection::SqliteConnection;
    use one_core::storage::models::SshAuthMethod;
    use one_core::storage::{
        GlobalStorageState, SftpFavoritePathRepository, SshParams, StorageManager,
        migration::run_migrations,
    };
    use sftp::DirectoryConflictPolicy;
    use sftp::ProgressCallback;
    use sftp_transfer::{
        SftpConnectionIdentity, SftpDeleteRemoteExecution, SftpDownloadExecution,
        SftpRemoteDeleteEntry, SftpTransferEvent, SftpTransferExecutor, SftpTransferId,
        SftpTransferOperation, SftpTransferProvider, SftpTransferState, SftpUploadExecution,
        delete_remote_task_key,
    };
    use ssh::{HostKeyVerifier, SshAuth, SshConnectConfig, SshSessionManager};
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;
    use tokio::sync::oneshot;

    fn test_download_task(id: usize, cancelled: bool) -> TransferTask {
        let shared_progress = SharedProgress::new();
        shared_progress
            .cancelled
            .store(cancelled, Ordering::Relaxed);
        TransferTask {
            id,
            operation: TransferOperation::Download {
                remote_path: format!("/remote/{id}.txt"),
                local_path: PathBuf::from(format!("/tmp/{id}.txt")),
                is_dir: false,
            },
            state: TransferTaskState::Pending,
            shared_progress,
            error: None,
        }
    }

    fn pending_directory(has_conflict: bool) -> PendingUpload {
        PendingUpload {
            name: "folder".to_string(),
            local_path: PathBuf::from("/tmp/folder"),
            remote_path: "/remote/folder".to_string(),
            is_dir: true,
            has_conflict,
            directory_conflict_policy: DirectoryConflictPolicy::Merge,
        }
    }

    fn delete_target(name: &str, is_dir: bool) -> super::DeleteTarget {
        super::DeleteTarget {
            name: name.to_string(),
            path: format!("/remote/{name}"),
            is_dir,
        }
    }

    fn test_stored_connection() -> one_core::storage::models::StoredConnection {
        let mut connection = one_core::storage::models::StoredConnection::new_ssh(
            "Terminal file manager test".to_string(),
            SshParams {
                sftp_default_directory: None,
                disabled_jump_server: None,
                sftp_account: None,
                host: "terminal-file-manager-test.internal".to_string(),
                port: 2222,
                username: "deploy".to_string(),
                auth_method: SshAuthMethod::Agent,
                credential_reference: None,
                prompt_username: None,
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
                allow_legacy_algorithms: None,
                jump_server: None,
                proxy: None,
                os_id: None,
                icon: None,
                icon_file_path: None,
                account_expect: Default::default(),
            },
            None,
        );
        connection.id = Some(7);
        connection
    }

    fn test_session_manager() -> Arc<SshSessionManager> {
        Arc::new(SshSessionManager::new(SshConnectConfig {
            host: "terminal-file-manager-test.internal".to_string(),
            port: 2222,
            username: "deploy".to_string(),
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
        }))
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum FileManagerTestOperation {
        Upload,
        Download,
        DeleteRemote,
    }

    #[derive(Clone, Default)]
    struct FileManagerTestProvider {
        state: Arc<Mutex<FileManagerTestProviderState>>,
    }

    #[derive(Default)]
    struct FileManagerTestProviderState {
        started: Vec<SftpTransferId>,
        operations: HashMap<SftpTransferId, FileManagerTestOperation>,
        remote_paths: HashMap<SftpTransferId, String>,
        delete_entries: HashMap<SftpTransferId, Vec<SftpRemoteDeleteEntry>>,
        completions: HashMap<SftpTransferId, oneshot::Sender<Result<()>>>,
        cancellations: HashMap<SftpTransferId, Arc<AtomicBool>>,
    }

    impl FileManagerTestProvider {
        fn started(&self) -> Vec<SftpTransferId> {
            self.state.lock().unwrap().started.clone()
        }

        fn operation(&self, id: SftpTransferId) -> Option<FileManagerTestOperation> {
            self.state.lock().unwrap().operations.get(&id).copied()
        }

        fn remote_path(&self, id: SftpTransferId) -> Option<String> {
            self.state.lock().unwrap().remote_paths.get(&id).cloned()
        }

        fn delete_entries(&self, id: SftpTransferId) -> Option<Vec<SftpRemoteDeleteEntry>> {
            self.state.lock().unwrap().delete_entries.get(&id).cloned()
        }

        fn complete(&self, id: SftpTransferId, result: Result<()>) {
            let sender = {
                let mut state = self.state.lock().unwrap();
                state.cancellations.remove(&id);
                state
                    .completions
                    .remove(&id)
                    .expect("test transfer should be waiting for completion")
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
            id: SftpTransferId,
            operation: FileManagerTestOperation,
            remote_path: String,
            entries: Option<Vec<SftpRemoteDeleteEntry>>,
            cancelled: Arc<AtomicBool>,
        ) -> Result<()> {
            let (sender, receiver) = oneshot::channel();
            {
                let mut state = self.state.lock().unwrap();
                state.started.push(id);
                state.operations.insert(id, operation);
                state.remote_paths.insert(id, remote_path);
                if let Some(entries) = entries {
                    state.delete_entries.insert(id, entries);
                }
                state.completions.insert(id, sender);
                state.cancellations.insert(id, cancelled);
            }
            receiver
                .await
                .map_err(|_| anyhow!("test completion channel closed"))?
        }
    }

    #[async_trait]
    impl SftpTransferProvider for FileManagerTestProvider {
        async fn upload(
            &self,
            execution: SftpUploadExecution,
            _progress: ProgressCallback,
        ) -> Result<()> {
            self.run(
                execution.id,
                FileManagerTestOperation::Upload,
                execution.remote_path,
                None,
                execution.cancelled,
            )
            .await
        }

        async fn download(
            &self,
            execution: SftpDownloadExecution,
            _progress: ProgressCallback,
        ) -> Result<()> {
            self.run(
                execution.id,
                FileManagerTestOperation::Download,
                execution.remote_path,
                None,
                execution.cancelled,
            )
            .await
        }

        async fn delete_remote(
            &self,
            execution: SftpDeleteRemoteExecution,
            _progress: ProgressCallback,
        ) -> Result<()> {
            self.run(
                execution.id,
                FileManagerTestOperation::DeleteRemote,
                execution.remote_dir,
                Some(execution.entries),
                execution.cancelled,
            )
            .await
        }
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

    fn file_manager_fixture(
        cx: &mut TestAppContext,
        temp_dir: &tempfile::TempDir,
    ) -> (
        FileManagerTestProvider,
        Entity<SftpTransferExecutor>,
        Entity<super::FileManagerPanel>,
        WindowHandle<Root>,
    ) {
        cx.update(one_core::gpui_tokio::init);
        cx.update(one_core::background_tasks::init);
        cx.executor().allow_parking();

        let db_path = temp_dir.path().join("terminal-file-manager-fixture.db");
        let conn = SqliteConnection::open_with_pool_size(&db_path, 1)
            .expect("open isolated fixture database");
        conn.with_connection(run_migrations)
            .expect("run isolated fixture migrations");
        let storage = StorageManager::new_with_connection(conn);
        storage.register(SftpFavoritePathRepository::new(storage.connection()));
        cx.update(|cx| {
            cx.set_global(GlobalStorageState { storage });
            gpui_component::init(cx);
            cx.set_global(gpui_component::Theme::default());
        });

        let provider = FileManagerTestProvider::default();
        let executor =
            cx.update(|cx| sftp_transfer::init_with_provider(cx, Arc::new(provider.clone())));

        let mut panel_slot = None;
        let window = cx.open_window(Default::default(), |window, cx| {
            let panel = cx.new(|cx| {
                super::FileManagerPanel::new(
                    test_stored_connection(),
                    test_session_manager(),
                    super::TerminalColors {
                        background: gpui::black(),
                        foreground: gpui::white(),
                        muted: gpui::black(),
                        muted_foreground: gpui::black(),
                        border: gpui::black(),
                        accent: gpui::black(),
                        accent_foreground: gpui::white(),
                    },
                    window,
                    cx,
                )
            });
            panel_slot = Some(panel.clone());
            Root::new(panel, window, cx)
        });

        let panel = panel_slot.expect("file manager fixture panel created");
        cx.update(|cx| {
            window
                .update(cx, |_, _, cx| {
                    panel.update(cx, |this, cx| {
                        this.current_path = "/remote".to_string();
                        cx.notify();
                    });
                })
                .expect("file manager fixture window remains open");
        });

        (provider, executor, panel, window)
    }

    fn submit_delete(
        cx: &mut TestAppContext,
        panel: &Entity<super::FileManagerPanel>,
    ) -> SftpTransferId {
        let targets = vec![
            delete_target("report.txt", false),
            delete_target("archive", true),
        ];
        panel.update(cx, |this, cx| {
            this.enqueue_delete(targets, "/remote".to_string(), cx);
            *this
                .pending_global_deletes
                .keys()
                .next()
                .expect("delete should be mirrored")
        })
    }

    fn submit_download(
        cx: &mut TestAppContext,
        panel: &Entity<super::FileManagerPanel>,
        local_path: PathBuf,
    ) -> SftpTransferId {
        panel.update(cx, |this, cx| {
            this.enqueue_download("/remote/report.txt".to_string(), local_path, false, cx);
        });
        panel.read_with(cx, |panel, cx| {
            panel
                .global_executor
                .read(cx)
                .active_for_connection(&panel.upload_connection_identity)
                .expect("download should be globally queued")
                .id
        })
    }

    fn executor_state(
        cx: &TestAppContext,
        panel: &Entity<super::FileManagerPanel>,
        id: SftpTransferId,
    ) -> Option<SftpTransferState> {
        panel.read_with(cx, |panel, cx| {
            panel
                .global_executor
                .read(cx)
                .snapshot(id)
                .map(|snapshot| snapshot.state)
        })
    }

    #[test]
    fn delete_refresh_decision_depends_on_remote_directory() {
        assert!(should_refresh_after_delete("/remote", "/remote"));
        assert!(!should_refresh_after_delete("/remote", "/other"));
    }

    #[gpui::test]
    fn delete_request_maps_targets_to_global_executor(mut cx: &mut TestAppContext) {
        let temp_dir = tempfile::TempDir::new().expect("create fixture temp dir");
        let (provider, executor, panel, _window) = file_manager_fixture(&mut cx, &temp_dir);
        let id = submit_delete(&mut cx, &panel);

        wait_until(&mut cx, |_| provider.delete_entries(id).is_some());

        let snapshot = executor.read_with(cx, |executor, _| executor.snapshot(id));
        snapshot.inspect(|snapshot| {
            assert_eq!(snapshot.operation, SftpTransferOperation::DeleteRemote);
            assert_eq!(snapshot.connection, SftpConnectionIdentity::Local(7));
            assert_eq!(snapshot.remote_path, "/remote");
            assert_eq!(
                provider.delete_entries(id),
                Some(vec![
                    SftpRemoteDeleteEntry {
                        remote_path: "/remote/report.txt".to_string(),
                        is_dir: false,
                    },
                    SftpRemoteDeleteEntry {
                        remote_path: "/remote/archive".to_string(),
                        is_dir: true,
                    },
                ])
            );
            assert_eq!(provider.remote_path(id), Some("/remote".to_string()));
        });
        assert_eq!(
            executor.read_with(cx, |executor, _| {
                executor.pending_count(&SftpConnectionIdentity::Local(7))
            }),
            0
        );
    }

    #[gpui::test]
    fn download_request_maps_to_global_executor(mut cx: &mut TestAppContext) {
        let temp_dir = tempfile::TempDir::new().expect("create fixture temp dir");
        let (provider, executor, panel, _window) = file_manager_fixture(&mut cx, &temp_dir);
        let local_path = temp_dir.path().join("report.txt");
        let id = submit_download(&mut cx, &panel, local_path.clone());

        wait_until(&mut cx, |_| {
            provider.operation(id) == Some(FileManagerTestOperation::Download)
        });

        let snapshot = executor
            .read_with(cx, |executor, _| executor.snapshot(id))
            .expect("download snapshot should exist");
        assert_eq!(snapshot.operation, SftpTransferOperation::Download);
        assert_eq!(snapshot.remote_path, "/remote/report.txt");
        assert_eq!(snapshot.local_path, local_path);
        panel.read_with(cx, |panel, _| {
            assert!(!panel.transfer_queue.has_active());
        });
    }

    #[gpui::test]
    fn delete_task_key_is_connection_scoped_and_target_sensitive() {
        let connection = SftpConnectionIdentity::Local(7);
        let base = delete_remote_task_key(
            &connection,
            "/remote",
            &[
                SftpRemoteDeleteEntry {
                    remote_path: "/remote/report.txt".to_string(),
                    is_dir: false,
                },
                SftpRemoteDeleteEntry {
                    remote_path: "/remote/archive".to_string(),
                    is_dir: true,
                },
            ],
        );

        assert_eq!(
            base,
            delete_remote_task_key(
                &connection,
                "/remote",
                &[
                    SftpRemoteDeleteEntry {
                        remote_path: "/remote/report.txt".to_string(),
                        is_dir: false,
                    },
                    SftpRemoteDeleteEntry {
                        remote_path: "/remote/archive".to_string(),
                        is_dir: true,
                    },
                ],
            )
        );
        assert_ne!(
            base,
            delete_remote_task_key(
                &SftpConnectionIdentity::Local(8),
                "/remote",
                &[
                    SftpRemoteDeleteEntry {
                        remote_path: "/remote/report.txt".to_string(),
                        is_dir: false,
                    },
                    SftpRemoteDeleteEntry {
                        remote_path: "/remote/archive".to_string(),
                        is_dir: true,
                    },
                ],
            )
        );
        assert_ne!(
            base,
            delete_remote_task_key(
                &connection,
                "/remote",
                &[SftpRemoteDeleteEntry {
                    remote_path: "/remote/report.txt".to_string(),
                    is_dir: true,
                }],
            )
        );
    }

    #[gpui::test]
    fn delete_finish_refreshes_once_and_ignores_duplicate_finished(mut cx: &mut TestAppContext) {
        let temp_dir = tempfile::TempDir::new().expect("create fixture temp dir");
        let (provider, _executor, panel, _window) = file_manager_fixture(&mut cx, &temp_dir);
        let id = submit_delete(&mut cx, &panel);
        wait_until(&mut cx, |_| {
            provider.operation(id) == Some(FileManagerTestOperation::DeleteRemote)
        });

        let refresh_count = Arc::new(AtomicU64::new(0));
        panel.update(cx, |this, _| {
            this.set_test_refresh_count(refresh_count.clone());
            this.selected_indices.insert(0);
        });

        provider.complete(id, Ok(()));
        cx.run_until_parked();

        panel.update(cx, |this, cx| {
            let executor = this.global_executor.clone();
            this.handle_global_transfer_event(&executor, &SftpTransferEvent::Finished(id), cx);
        });

        assert_eq!(refresh_count.load(Ordering::Relaxed), 1);
        panel.read_with(cx, |panel, _| {
            assert!(panel.pending_global_deletes.is_empty());
            assert!(panel.selected_indices.is_empty());
        });
    }

    #[gpui::test]
    fn delete_cancel_routes_to_global_executor_and_refreshes(mut cx: &mut TestAppContext) {
        let temp_dir = tempfile::TempDir::new().expect("create fixture temp dir");
        let (provider, _executor, panel, _window) = file_manager_fixture(&mut cx, &temp_dir);
        let id = submit_delete(&mut cx, &panel);
        wait_until(&mut cx, |_| {
            provider.operation(id) == Some(FileManagerTestOperation::DeleteRemote)
        });

        let refresh_count = Arc::new(AtomicU64::new(0));
        panel.update(cx, |this, cx| {
            this.set_test_refresh_count(refresh_count.clone());
            this.cancel_transfer_target(TransferCancelTarget::Global(id), cx);
        });
        wait_until(&mut cx, |_| provider.is_cancelled(id));
        provider.complete(id, Err(anyhow!(sftp::TransferCancelled)));
        wait_until(&mut cx, |cx| {
            executor_state(cx, &panel, id) == Some(SftpTransferState::Cancelled)
        });

        assert_eq!(refresh_count.load(Ordering::Relaxed), 1);
        assert_eq!(
            executor_state(cx, &panel, id),
            Some(SftpTransferState::Cancelled)
        );
    }

    #[gpui::test]
    fn delete_and_upload_share_the_global_connection_fifo(mut cx: &mut TestAppContext) {
        let temp_dir = tempfile::TempDir::new().expect("create fixture temp dir");
        let (provider, _executor, panel, _window) = file_manager_fixture(&mut cx, &temp_dir);

        panel.update(cx, |this, cx| {
            this.enqueue_pending_uploads(
                vec![PendingUpload {
                    name: "first.txt".to_string(),
                    local_path: PathBuf::from("/tmp/first.txt"),
                    remote_path: "/remote/first.txt".to_string(),
                    is_dir: false,
                    has_conflict: false,
                    directory_conflict_policy: DirectoryConflictPolicy::Merge,
                }],
                cx,
            );
        });
        let delete_id = submit_delete(&mut cx, &panel);
        wait_until(&mut cx, |_| provider.started().first().is_some());
        let upload_id = provider.started()[0];
        wait_until(&mut cx, |test_cx| {
            provider.operation(upload_id) == Some(FileManagerTestOperation::Upload)
                && panel.read_with(test_cx, |panel, cx| {
                    panel
                        .global_executor
                        .read(cx)
                        .pending_count(&SftpConnectionIdentity::Local(7))
                        == 1
                })
        });
        assert_eq!(provider.delete_entries(delete_id), None);

        provider.complete(upload_id, Ok(()));
        wait_until(&mut cx, |_| {
            provider.operation(delete_id) == Some(FileManagerTestOperation::DeleteRemote)
        });
        assert_eq!(
            executor_state(&cx, &panel, delete_id),
            Some(SftpTransferState::Running)
        );
    }

    #[test]
    fn overwrite_sets_conflicting_directories_to_replace() {
        let upload = resolve_upload_conflict(
            pending_directory(true),
            FileConflictChoice::Overwrite,
            &mut HashSet::new(),
        )
        .expect("overwrite keeps the upload");

        assert_eq!(
            upload.directory_conflict_policy,
            DirectoryConflictPolicy::Replace
        );
    }

    #[test]
    fn merge_keeps_directory_policy_merge() {
        let upload = resolve_upload_conflict(
            pending_directory(true),
            FileConflictChoice::Merge,
            &mut HashSet::new(),
        )
        .expect("merge keeps the upload");

        assert_eq!(
            upload.directory_conflict_policy,
            DirectoryConflictPolicy::Merge
        );
    }

    #[test]
    fn keep_both_keeps_directory_policy_merge() {
        let upload = resolve_upload_conflict(
            pending_directory(true),
            FileConflictChoice::KeepBoth,
            &mut HashSet::from(["folder".to_string()]),
        )
        .expect("keep both keeps the renamed upload");

        assert_eq!(
            upload.directory_conflict_policy,
            DirectoryConflictPolicy::Merge
        );
        assert_eq!(upload.name, "folder (copy)");
    }

    #[test]
    fn pending_uploads_mark_remote_name_conflicts() {
        let existing_names = HashSet::from(["archive.tar".to_string()]);
        let uploads = super::build_pending_uploads(
            vec![
                PathBuf::from("/tmp/archive.tar"),
                PathBuf::from("/tmp/readme.txt"),
            ],
            "/remote",
            Some(&existing_names),
        );

        assert_eq!(uploads.len(), 2);
        assert_eq!(uploads[0].name, "archive.tar");
        assert_eq!(uploads[0].remote_path, "/remote/archive.tar");
        assert!(uploads[0].has_conflict);
        assert_eq!(uploads[1].name, "readme.txt");
        assert_eq!(uploads[1].remote_path, "/remote/readme.txt");
        assert!(!uploads[1].has_conflict);
    }

    #[test]
    fn folder_upload_progress_includes_the_current_file() {
        assert_eq!(
            transfer_progress_display_label(
                "assets".to_string(),
                Some("images/banner-long-name.png".to_string())
            ),
            "assets - images/banner-long-name.png"
        );
        assert_eq!(
            transfer_progress_display_label("archive.tar".to_string(), None),
            "archive.tar"
        );
    }

    #[test]
    fn transfer_queue_starts_tasks_strictly_in_fifo_order() {
        let mut queue = TransferQueue::new();
        queue.enqueue(test_download_task(1, false));
        queue.enqueue(test_download_task(2, false));
        queue.enqueue(test_download_task(3, false));

        assert_eq!(Some(1), queue.next_startable().map(|task| task.id));
        assert!(queue.next_startable().is_none());

        queue
            .tasks
            .iter_mut()
            .find(|task| task.id == 1)
            .expect("first transfer task")
            .state = TransferTaskState::Completed;
        assert_eq!(Some(2), queue.next_startable().map(|task| task.id));

        queue
            .tasks
            .iter_mut()
            .find(|task| task.id == 2)
            .expect("second transfer task")
            .state = TransferTaskState::Completed;
        assert_eq!(Some(3), queue.next_startable().map(|task| task.id));
    }

    #[test]
    fn transfer_queue_removes_cancelled_pending_tasks_without_reordering() {
        let mut queue = TransferQueue::new();
        queue.enqueue(test_download_task(1, true));
        queue.enqueue(test_download_task(2, false));
        queue.enqueue(test_download_task(3, true));
        queue.enqueue(test_download_task(4, false));

        let cancelled_ids = queue
            .take_cancelled_pending()
            .into_iter()
            .map(|task| task.id)
            .collect::<Vec<_>>();

        assert_eq!(vec![1, 3], cancelled_ids);
        assert!(matches!(
            queue
                .tasks
                .iter()
                .find(|task| task.id == 1)
                .map(|task| &task.state),
            Some(TransferTaskState::Cancelled)
        ));
        assert_eq!(Some(2), queue.next_startable().map(|task| task.id));
        queue
            .tasks
            .iter_mut()
            .find(|task| task.id == 2)
            .expect("second transfer task")
            .state = TransferTaskState::Completed;
        assert_eq!(Some(4), queue.next_startable().map(|task| task.id));
    }

    #[test]
    fn toolbar_exposes_open_sftp_tab_action() {
        let source = include_str!("file_manager_panel.rs");
        let toolbar = source
            .split("fn render_toolbar")
            .nth(1)
            .and_then(|source| source.split("fn render_path_breadcrumb").next())
            .expect("file manager toolbar source");

        assert!(toolbar.contains(r#".id("fm-open-sftp")"#));
        assert!(toolbar.contains("FileManagerPanelEvent::OpenSftp("));
        assert!(toolbar.contains(r#"t!("FileManager.open_sftp")"#));
        assert!(toolbar.contains(".colors(foreground, muted_foreground)"));
        assert!(toolbar.contains(".text_color(muted_foreground)"));
    }

    #[test]
    fn toolbar_exposes_follow_terminal_cwd_toggle() {
        let source = include_str!("file_manager_panel.rs");
        let toolbar = source
            .split("fn render_toolbar")
            .nth(1)
            .and_then(|source| source.split("fn render_path_breadcrumb").next())
            .expect("file manager toolbar source");

        assert!(toolbar.contains(r#".id("fm-follow-terminal")"#));
        assert!(toolbar.contains("FileManagerPanelEvent::ToggleFollowTerminalCwd"));
        assert!(toolbar.contains(r#""FileManager.follow_terminal_dir_on""#));
        assert!(toolbar.contains(r#""FileManager.follow_terminal_dir_off""#));
    }

    #[test]
    fn sidebar_routes_follow_terminal_cwd_toggle_to_sync_path_setting() {
        let source = include_str!("mod.rs");
        let handler = source
            .split("FileManagerPanelEvent::ToggleFollowTerminalCwd =>")
            .nth(1)
            .and_then(|source| source.split("FileManagerPanelEvent::SyncWorkingDir").next())
            .or_else(|| {
                source
                    .split("FileManagerPanelEvent::ToggleFollowTerminalCwd =>")
                    .nth(1)
            })
            .expect("toggle handler branch");

        assert!(handler.contains("set_sync_path_enabled"));
        assert!(handler.contains("set_follow_terminal_cwd"));
        assert!(handler.contains("TerminalSidebarEvent::SyncPathChanged"));
    }

    #[test]
    fn paste_availability_does_not_depend_on_a_selected_file() {
        let clipboard = RemoteFileClipboard {
            kind: RemoteClipboardKind::Copy,
            entries: vec![RemoteClipboardEntry {
                name: "notes.txt".to_string(),
                source_path: "/srv/notes.txt".to_string(),
                is_dir: false,
                size: 10,
            }],
        };

        assert!(can_paste_remote_file_clipboard(Some(&clipboard), true));
        assert!(!can_paste_remote_file_clipboard(Some(&clipboard), false));
        assert!(!can_paste_remote_file_clipboard(None, true));

        let empty_clipboard = RemoteFileClipboard {
            kind: RemoteClipboardKind::Cut,
            entries: Vec::new(),
        };
        assert!(!can_paste_remote_file_clipboard(
            Some(&empty_clipboard),
            true
        ));
    }

    #[test]
    fn build_retry_reset_plan_prefers_explicit_working_dir() {
        let plan = build_retry_reset_plan("/srv/project", Some("/srv/override".to_string()));

        assert_eq!(plan.next_state, ConnectionState::Idle);
        assert_eq!(plan.initial_working_dir.as_deref(), Some("/srv/override"));
        assert!(plan.clear_listing);
    }

    #[test]
    fn build_navigation_recovery_plan_prefers_working_directory() {
        let plan = build_navigation_recovery_plan(
            "/srv/invalid",
            Some("/srv/workspace"),
            &["/srv/home".to_string(), "/srv/invalid".to_string()],
            1,
        );

        assert_eq!(
            plan,
            NavigationRecoveryPlan {
                fallback_path: "/srv/workspace".to_string(),
            }
        );
    }

    #[test]
    fn build_navigation_recovery_plan_falls_back_to_previous_history() {
        let plan = build_navigation_recovery_plan(
            "/srv/invalid",
            None,
            &["/srv/home".to_string(), "/srv/invalid".to_string()],
            1,
        );

        assert_eq!(
            plan,
            NavigationRecoveryPlan {
                fallback_path: "/srv/home".to_string(),
            }
        );
    }

    #[test]
    fn clear_remote_listing_state_clears_items_and_selection() {
        let mut items = vec![1, 2, 3];
        let mut filtered_indices = vec![0, 2];
        let mut selected_indices = HashSet::from([0usize, 1usize]);

        clear_remote_listing_state(&mut items, &mut filtered_indices, &mut selected_indices);

        assert!(items.is_empty());
        assert!(filtered_indices.is_empty());
        assert!(selected_indices.is_empty());
    }

    #[test]
    fn frame_move_options_disable_current_placement() {
        let options = frame_move_options(SidebarPlacement::Left);

        assert_eq!(
            vec![
                (SidebarPlacement::Left, true),
                (SidebarPlacement::Right, false),
                (SidebarPlacement::Bottom, false),
            ],
            options
                .iter()
                .map(|option| (option.placement, option.disabled))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn only_apply_directory_result_for_active_path() {
        assert!(should_apply_directory_result("/srv/app", "/srv/app"));
        assert!(!should_apply_directory_result("/srv/other", "/srv/app"));
    }

    #[test]
    fn only_apply_upload_preparation_for_current_connection_generation() {
        assert!(super::is_current_generation(7, 7));
        assert!(!super::is_current_generation(8, 7));
    }

    #[test]
    fn only_refresh_upload_target_directory_when_still_viewing_it() {
        assert!(should_refresh_after_upload("/srv/app", "/srv/app/file.txt"));
        assert!(!should_refresh_after_upload(
            "/srv/other",
            "/srv/app/file.txt"
        ));
    }

    #[test]
    fn zero_byte_files_display_zero_bytes() {
        assert_eq!("0 B", super::format_file_size(0));
    }

    #[test]
    fn size_sort_key_distinguishes_unknown_and_empty_directories() {
        let file = super::RemoteFileItem {
            name: "empty.txt".to_string(),
            size: 0,
            modified: std::time::UNIX_EPOCH,
            is_dir: false,
            permissions: String::new(),
            owner: None,
            directory_size: super::DirectorySizeState::Unknown,
        };
        let ready_directory = super::RemoteFileItem {
            name: "empty-dir".to_string(),
            size: 0,
            modified: std::time::UNIX_EPOCH,
            is_dir: true,
            permissions: String::new(),
            owner: None,
            directory_size: super::DirectorySizeState::Ready(0),
        };
        let calculating_directory = super::RemoteFileItem {
            directory_size: super::DirectorySizeState::Calculating,
            ..ready_directory.clone()
        };
        let unknown_directory = super::RemoteFileItem {
            directory_size: super::DirectorySizeState::Unknown,
            ..ready_directory.clone()
        };

        assert_eq!((0, 0), super::size_sort_key(&file));
        assert_eq!((0, 0), super::size_sort_key(&ready_directory));
        assert_eq!((1, 0), super::size_sort_key(&calculating_directory));
        assert_eq!((2, 0), super::size_sort_key(&unknown_directory));
        assert_eq!("0 B", super::size_label(&ready_directory));
    }

    #[test]
    fn delete_targets_follow_filtered_selection_order() {
        let items = vec![
            super::RemoteFileItem {
                name: "app.log".to_string(),
                size: 10,
                modified: std::time::UNIX_EPOCH,
                is_dir: false,
                permissions: String::new(),
                owner: None,
                directory_size: super::DirectorySizeState::Unknown,
            },
            super::RemoteFileItem {
                name: "conf".to_string(),
                size: 0,
                modified: std::time::UNIX_EPOCH,
                is_dir: true,
                permissions: String::new(),
                owner: None,
                directory_size: super::DirectorySizeState::Unknown,
            },
            super::RemoteFileItem {
                name: "data.db".to_string(),
                size: 20,
                modified: std::time::UNIX_EPOCH,
                is_dir: false,
                permissions: String::new(),
                owner: None,
                directory_size: super::DirectorySizeState::Unknown,
            },
        ];
        let filtered_indices = vec![1, 0, 2];
        let selected_indices = HashSet::from([0usize, 2usize]);

        let targets = super::delete_targets_for_selection(
            "/srv/app",
            &items,
            &filtered_indices,
            &selected_indices,
        );

        assert_eq!(2, targets.len());
        assert_eq!("conf", targets[0].name);
        assert_eq!("/srv/app/conf", targets[0].path);
        assert!(targets[0].is_dir);
        assert_eq!("data.db", targets[1].name);
        assert_eq!("/srv/app/data.db", targets[1].path);
        assert!(!targets[1].is_dir);
    }

    #[test]
    fn download_targets_follow_filtered_selection_order() {
        let items = vec![
            super::RemoteFileItem {
                name: "app.log".to_string(),
                size: 10,
                modified: std::time::UNIX_EPOCH,
                is_dir: false,
                permissions: String::new(),
                owner: None,
                directory_size: super::DirectorySizeState::Unknown,
            },
            super::RemoteFileItem {
                name: "conf".to_string(),
                size: 0,
                modified: std::time::UNIX_EPOCH,
                is_dir: true,
                permissions: String::new(),
                owner: None,
                directory_size: super::DirectorySizeState::Unknown,
            },
            super::RemoteFileItem {
                name: "data.db".to_string(),
                size: 20,
                modified: std::time::UNIX_EPOCH,
                is_dir: false,
                permissions: String::new(),
                owner: None,
                directory_size: super::DirectorySizeState::Unknown,
            },
        ];
        let filtered_indices = vec![1, 0, 2];
        let selected_indices = HashSet::from([0usize, 2usize]);

        let targets = super::download_targets_for_selection(
            "/srv/app",
            &items,
            &filtered_indices,
            &selected_indices,
        );

        assert_eq!(
            vec![
                super::DownloadTarget {
                    name: "conf".to_string(),
                    path: "/srv/app/conf".to_string(),
                    is_dir: true,
                },
                super::DownloadTarget {
                    name: "data.db".to_string(),
                    path: "/srv/app/data.db".to_string(),
                    is_dir: false,
                },
            ],
            targets
        );
    }

    #[test]
    fn context_menu_uses_selection_only_for_selected_multi_item() {
        let selected_indices = HashSet::from([0usize, 2usize]);

        assert!(super::should_use_context_selection(&selected_indices, 0));
        assert!(super::should_use_context_selection(&selected_indices, 2));
        assert!(!super::should_use_context_selection(&selected_indices, 1));

        let single_selection = HashSet::from([0usize]);
        assert!(!super::should_use_context_selection(&single_selection, 0));
    }

    #[test]
    fn clipboard_entries_follow_filtered_selection_order() {
        let items = vec![
            super::RemoteFileItem {
                name: "a.txt".to_string(),
                size: 10,
                modified: std::time::UNIX_EPOCH,
                is_dir: false,
                permissions: String::new(),
                owner: None,
                directory_size: super::DirectorySizeState::Unknown,
            },
            super::RemoteFileItem {
                name: "folder".to_string(),
                size: 0,
                modified: std::time::UNIX_EPOCH,
                is_dir: true,
                permissions: String::new(),
                owner: None,
                directory_size: super::DirectorySizeState::Unknown,
            },
            super::RemoteFileItem {
                name: "b.txt".to_string(),
                size: 20,
                modified: std::time::UNIX_EPOCH,
                is_dir: false,
                permissions: String::new(),
                owner: None,
                directory_size: super::DirectorySizeState::Unknown,
            },
        ];
        let entries = super::clipboard_entries_for_selection(
            "/srv",
            &items,
            &[1, 0, 2],
            &HashSet::from([0usize, 2usize]),
        );

        assert_eq!(
            vec![
                super::RemoteClipboardEntry {
                    name: "folder".to_string(),
                    source_path: "/srv/folder".to_string(),
                    is_dir: true,
                    size: 0,
                },
                super::RemoteClipboardEntry {
                    name: "b.txt".to_string(),
                    source_path: "/srv/b.txt".to_string(),
                    is_dir: false,
                    size: 20,
                },
            ],
            entries
        );
    }

    #[test]
    fn range_selection_selects_rows_between_anchor_and_clicked_row() {
        let mut selected_indices = HashSet::from([1usize]);
        let mut anchor_index = Some(1usize);

        super::apply_selection_mode(
            &mut selected_indices,
            &mut anchor_index,
            4,
            super::SelectionMode::Range,
        );

        assert_eq!(HashSet::from([1usize, 2, 3, 4]), selected_indices);
        assert_eq!(Some(1), anchor_index);
    }

    #[test]
    fn range_selection_without_anchor_selects_clicked_row() {
        let mut selected_indices = HashSet::new();
        let mut anchor_index = None;

        super::apply_selection_mode(
            &mut selected_indices,
            &mut anchor_index,
            3,
            super::SelectionMode::Range,
        );

        assert_eq!(HashSet::from([3usize]), selected_indices);
        assert_eq!(Some(3), anchor_index);
    }

    #[test]
    fn replace_selection_clears_previous_rows_and_updates_anchor() {
        let mut selected_indices = HashSet::from([0usize, 2]);
        let mut anchor_index = Some(0usize);

        super::apply_selection_mode(
            &mut selected_indices,
            &mut anchor_index,
            5,
            super::SelectionMode::Replace,
        );

        assert_eq!(HashSet::from([5usize]), selected_indices);
        assert_eq!(Some(5), anchor_index);
    }

    #[test]
    fn build_rename_target_path_keeps_parent_directory() {
        assert_eq!(
            "/srv/app/new.log",
            super::build_rename_target_path("/srv/app/old.log", "new.log")
        );
        assert_eq!(
            "/renamed.log",
            super::build_rename_target_path("/old.log", "renamed.log")
        );
    }

    #[test]
    fn build_new_file_target_path_keeps_current_directory() {
        assert_eq!(
            "/srv/app/new.log",
            super::build_new_file_target_path("/srv/app", "new.log")
        );
        assert_eq!(
            "/new.log",
            super::build_new_file_target_path("/", "new.log")
        );
    }

    #[test]
    fn new_file_names_reject_path_traversal_and_special_entries() {
        assert!(super::is_valid_entry_name("notes.txt"));
        assert!(!super::is_valid_entry_name("../notes.txt"));
        assert!(!super::is_valid_entry_name("."));
        assert!(!super::is_valid_entry_name(""));
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

    #[test]
    fn file_manager_keybindings_bind_backspace_to_navigate_parent() {
        let bindings = super::init_keybindings();

        let backspace = bindings
            .iter()
            .find(|binding| {
                binding
                    .keystrokes()
                    .iter()
                    .any(|keystroke| keystroke.key() == "backspace")
            })
            .expect("Backspace 绑定应存在");
        assert_eq!(
            "terminal_file_manager::NavigateParent",
            backspace.action().name()
        );
    }
}
