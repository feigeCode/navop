//! 终端侧边栏文件管理器面板
//!
//! 仅针对 SSH 终端，通过独立的 SFTP 连接浏览远程文件系统。
//! UI 参考 `sftp_view` 的 `FileListPanel`，但为侧边栏场景做了精简和适配。
//! 支持文件传输（上传/下载/拖拽），使用独立的传输连接避免阻塞浏览。

use crate::theme::TerminalColors;
use chrono::{DateTime, Local};
use gpui::{
    Anchor, App, ClipboardItem, Context, Entity, EventEmitter, ExternalPaths, FocusHandle,
    Focusable, IntoElement, KeyBinding, ListSizingBehavior, MouseButton, MouseDownEvent,
    ParentElement, PathPromptOptions, Render, SharedString, Styled, UniformListScrollHandle,
    Window, actions, div, prelude::*, px, uniform_list,
};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, InteractiveElementExt, Sizable, Size, WindowExt,
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
use one_core::gpui_tokio::Tokio;
use one_core::sidebar_contribution::SidebarPlacement;
use one_core::storage::models::StoredConnection;
use one_core::storage::{
    GlobalStorageState, SftpFavoritePathRepository, normalize_sftp_favorite_path,
    sftp_favorite_connection_key,
};
use remote_file_editor::{
    ExternalEditorOpenRequest, RemoteMutationCallback, external_editor_menu_label,
    external_editors_for_file, open_remote_file_editor, open_remote_file_external_editor,
};
use remote_image_preview::{
    clipboard_upload_paths, image_format_for_path, open_remote_image_preview,
};
use rust_i18n::t;
use sftp::{RusshSftpClient, SftpClient, TransferCancelled, TransferProgress};
use ssh::{ChannelEvent, SshChannel, SshSessionManager};
use std::collections::{HashSet, VecDeque};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime};
use tokio::sync::Mutex;

actions!(terminal_file_manager, [PasteUpload]);

pub const FILE_MANAGER_CONTEXT: &str = "TerminalFileManager";

pub fn init_keybindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new(
        file_manager_paste_shortcut(),
        PasteUpload,
        Some(FILE_MANAGER_CONTEXT),
    )]
}

fn file_manager_paste_shortcut() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-v"
    } else {
        "ctrl-v"
    }
}

// ── 传输相关类型 ──────────────────────────────────────────────

/// 传输操作类型
#[derive(Clone)]
enum TransferOperation {
    Upload {
        local_path: PathBuf,
        remote_path: String,
        is_dir: bool,
    },
    Download {
        remote_path: String,
        local_path: PathBuf,
        is_dir: bool,
    },
    Delete {
        targets: Vec<DeleteTarget>,
        remote_dir: String,
    },
}

/// 传输任务状态
#[derive(Clone, PartialEq)]
enum TransferTaskState {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// 跨线程共享的进度数据（原子操作，无需加锁）
struct SharedProgress {
    transferred: AtomicU64,
    total: AtomicU64,
    /// 存储 f64::to_bits() 的速度值
    speed: AtomicU64,
    cancelled: Arc<AtomicBool>,
    current_file: std::sync::RwLock<Option<String>>,
}

impl SharedProgress {
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

/// 传输任务
#[derive(Clone)]
struct TransferTask {
    id: usize,
    operation: TransferOperation,
    state: TransferTaskState,
    shared_progress: Arc<SharedProgress>,
    error: Option<String>,
}

#[derive(Clone)]
struct PendingUpload {
    name: String,
    local_path: PathBuf,
    remote_path: String,
    is_dir: bool,
    has_conflict: bool,
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

/// 传输队列（单任务串行执行）
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

    fn enqueue(&mut self, task: TransferTask) {
        self.pending.push_back(task.id);
        self.tasks.push(task);
    }

    /// 取出下一个可执行的任务（串行：仅当没有 Running 时才启动）
    fn next_startable(&mut self) -> Option<TransferTask> {
        if self.running_count() > 0 {
            return None;
        }

        while let Some(task_id) = self.pending.pop_front() {
            let Some(task) = self.tasks.iter_mut().find(|t| t.id == task_id) else {
                continue;
            };

            if task.state != TransferTaskState::Pending {
                continue;
            }

            task.state = TransferTaskState::Running;
            return Some(task.clone());
        }

        None
    }

    /// 获取当前活跃任务（用于进度显示）
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

    /// 排队中的任务数（不含正在执行的）
    fn pending_count(&self) -> usize {
        self.tasks
            .iter()
            .filter(|t| t.state == TransferTaskState::Pending)
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
}

/// 文件管理器面板事件
#[derive(Clone, Debug)]
pub enum FileManagerPanelEvent {
    /// 关闭面板
    Close,
    /// 请求宿主把面板移动到指定位置
    MoveTo(SidebarPlacement),
    /// 在终端中 cd 到指定路径
    CdToTerminal(String),
    /// 请求将终端当前工作目录同步到文件管理器
    SyncWorkingDir,
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
        return "-".to_string();
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

/// 拼接远程路径
fn join_remote_path(base: &str, name: &str) -> String {
    if base == "/" {
        format!("/{}", name)
    } else {
        format!("{}/{}", base, name)
    }
}

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

fn should_refresh_after_upload(current_path: &str, remote_path: &str) -> bool {
    current_path == remote_path_parent(remote_path)
}

fn should_refresh_after_delete(current_path: &str, remote_dir: &str) -> bool {
    current_path == remote_dir
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
        .flex_shrink(1.0)
        .min_w(px(0.))
        .max_w(px(BREADCRUMB_ITEM_MAX_WIDTH))
        .overflow_hidden()
        .text_ellipsis()
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

fn rename_conflicting_uploads(
    mut uploads: Vec<PendingUpload>,
    existing_names: HashSet<String>,
) -> Vec<PendingUpload> {
    let mut used_names = existing_names;

    for upload in &mut uploads {
        if upload.has_conflict {
            let new_name = generate_unique_name(&upload.name, &used_names);
            used_names.insert(new_name.clone());

            let dir_part = if let Some(slash_pos) = upload.remote_path.rfind('/') {
                Some(upload.remote_path[..=slash_pos].to_string())
            } else {
                None
            };

            upload.remote_path = if let Some(dir) = dir_part {
                format!("{}{}", dir, new_name)
            } else {
                new_name.clone()
            };
            upload.name = new_name;
        }
    }

    uploads
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

fn update_delete_progress(progress: &SharedProgress, name: &str, transferred: u64, total: u64) {
    if let Ok(mut guard) = progress.current_file.write() {
        *guard = Some(name.to_string());
    }
    progress.total.store(total, Ordering::Relaxed);
    progress.transferred.store(transferred, Ordering::Relaxed);
}

async fn delete_remote_target(
    client: &mut RusshSftpClient,
    target: &DeleteTarget,
    cancelled: Arc<AtomicBool>,
    progress: Arc<SharedProgress>,
) -> anyhow::Result<()> {
    if target.is_dir {
        let callback_progress = progress;
        client
            .delete_recursive(
                &target.path,
                cancelled,
                Box::new(move |progress: TransferProgress| {
                    callback_progress
                        .transferred
                        .store(progress.transferred, Ordering::Relaxed);
                    callback_progress
                        .total
                        .store(progress.total, Ordering::Relaxed);
                    if let Some(file) = progress.current_file {
                        if let Ok(mut guard) = callback_progress.current_file.write() {
                            *guard = Some(file);
                        }
                    }
                }),
            )
            .await
    } else {
        client.delete(&target.path, false).await
    }
}

async fn delete_targets_with_progress(
    client: Arc<Mutex<RusshSftpClient>>,
    targets: Vec<DeleteTarget>,
    progress: Arc<SharedProgress>,
) -> anyhow::Result<()> {
    let cancelled = progress.cancelled.clone();
    let total = targets.len() as u64;
    let mut client = client.lock().await;
    let mut errors = Vec::new();

    for (index, target) in targets.iter().enumerate() {
        if cancelled.load(Ordering::Relaxed) {
            return Err(anyhow::Error::new(TransferCancelled));
        }

        update_delete_progress(&progress, &target.name, index as u64, total);
        match delete_remote_target(&mut client, target, cancelled.clone(), progress.clone()).await {
            Ok(()) => {}
            Err(error) if is_transfer_cancelled(&error) => return Err(error),
            Err(error) => errors.push(format!("{}: {}", target.name, error)),
        }
        update_delete_progress(&progress, &target.name, (index + 1) as u64, total);
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("{}", errors.join("; ")))
    }
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

/// 从 StoredConnection 构建 SshConnectConfig
// ── FileManagerPanel ──────────────────────────────────────────

/// 终端侧边栏文件管理器面板
pub struct FileManagerPanel {
    /// 共享 SSH 会话管理器
    session_manager: Arc<SshSessionManager>,
    /// SFTP 客户端（浏览用）
    sftp_client: Option<Arc<Mutex<RusshSftpClient>>>,
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
    /// 独立的传输 SFTP 连接（懒创建）
    transfer_client: Option<Arc<Mutex<RusshSftpClient>>>,
    /// 传输队列
    transfer_queue: TransferQueue,
    /// 下一个任务 ID
    next_task_id: usize,
    /// 进度刷新定时器
    progress_refresh_task: Option<gpui::Task<()>>,
    active_extract: Option<ActiveExtract>,
    /// 是否有外部文件拖入
    is_dragging_over: bool,
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

        Self {
            session_manager,
            sftp_client: None,
            connection_state: ConnectionState::Idle,
            current_path: "/".to_string(),
            items: Vec::new(),
            filtered_indices: Vec::new(),
            selected_indices: HashSet::new(),
            selection_anchor_index: None,
            sort_column: SortColumn::Name,
            sort_order: SortOrder::Ascending,
            show_hidden: false,
            search_input,
            path_input,
            search_query: String::new(),
            path_editing: false,
            history: vec!["/".to_string()],
            history_index: 0,
            scroll_handle: UniformListScrollHandle::new(),
            focus_handle,
            loading: false,
            favorite_paths,
            favorite_connection_id,
            favorite_connection_key,
            favorite_popover_open: false,
            favorite_search_input,
            favorite_edit_input,
            favorite_editing_path: None,
            _subscriptions: subscriptions,
            transfer_client: None,
            transfer_queue: TransferQueue::new(),
            next_task_id: 0,
            progress_refresh_task: None,
            active_extract: None,
            is_dragging_over: false,
            working_dir_hint: None,
            colors,
            frame_placement: SidebarPlacement::Right,
        }
    }

    pub fn set_colors(&mut self, colors: TerminalColors, cx: &mut Context<Self>) {
        self.colors = colors;
        cx.notify();
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
                    this.sftp_client = Some(Arc::new(Mutex::new(client)));
                    this.connection_state = ConnectionState::Connected;
                    this.current_path = real_path.clone();
                    this.working_dir_hint = Some(real_path.clone());
                    this.history = vec![real_path];
                    this.history_index = 0;
                    this.refresh_dir(cx);
                });
            }
            Ok(Err(e)) => {
                let _ = this.update(cx, |this, cx| {
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
        self.transfer_client = None;
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
        self.working_dir_hint = Some(path.clone());
        if self.connection_state == ConnectionState::Idle {
            self.current_path = path;
        }
    }

    /// 从终端 OSC 7 同步导航到指定路径
    ///
    /// 仅在已连接且路径不同时才导航，避免不必要的刷新。
    pub fn sync_navigate_to(&mut self, path: String, cx: &mut Context<Self>) {
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
        let foreground = self.colors.foreground;
        let muted_foreground = self.colors.muted_foreground;
        let mut breadcrumb = Breadcrumb::new().colors(foreground, muted_foreground);
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
                            .map(|e| RemoteFileItem {
                                name: e.name,
                                size: e.size,
                                modified: e.modified,
                                is_dir: e.is_dir,
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
                SortColumn::Size => a.size.cmp(&b.size),
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

    /// 更新选中状态
    fn select_row(&mut self, row_ix: usize, mode: SelectionMode) {
        apply_selection_mode(
            &mut self.selected_indices,
            &mut self.selection_anchor_index,
            row_ix,
            mode,
        );
    }

    // ── 传输调度 ──────────────────────────────────────────────

    /// 分配下一个任务 ID
    fn alloc_task_id(&mut self) -> usize {
        let id = self.next_task_id;
        self.next_task_id += 1;
        id
    }

    /// 创建传输专用连接（首次传输时懒创建），然后执行排队任务
    fn ensure_transfer_client_and_schedule(&mut self, cx: &mut Context<Self>) {
        if self.transfer_client.is_some() {
            self.schedule_transfers(cx);
            return;
        }

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

            match result {
                Ok(client) => {
                    let _ = this.update(cx, |this, cx| {
                        this.transfer_client = Some(Arc::new(Mutex::new(client)));
                        this.schedule_transfers(cx);
                    });
                }
                Err(e) => {
                    let _ = this.update(cx, |this, cx| {
                        let error_msg =
                            format!("{}: {}", t!("FileManager.transfer_connect_failed"), e);
                        tracing::error!("{}", error_msg);
                        for task in &mut this.transfer_queue.tasks {
                            if task.state == TransferTaskState::Pending {
                                task.state = TransferTaskState::Failed;
                                task.error = Some(error_msg.clone());
                            }
                        }
                        this.transfer_queue.pending.clear();
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// 调度下一个待执行的传输任务
    fn schedule_transfers(&mut self, cx: &mut Context<Self>) {
        let Some(task) = self.transfer_queue.next_startable() else {
            return;
        };

        match task.operation.clone() {
            TransferOperation::Upload {
                local_path,
                remote_path,
                is_dir,
            } => {
                self.start_upload_task(
                    task.id,
                    local_path,
                    remote_path,
                    is_dir,
                    task.shared_progress,
                    cx,
                );
            }
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
            TransferOperation::Delete {
                targets,
                remote_dir,
            } => {
                self.start_delete_task(task.id, targets, remote_dir, task.shared_progress, cx);
            }
        }

        self.start_progress_refresh(cx);
        cx.notify();
    }

    /// 执行上传任务
    fn start_upload_task(
        &mut self,
        task_id: usize,
        local_path: PathBuf,
        remote_path: String,
        is_dir: bool,
        shared_progress: Arc<SharedProgress>,
        cx: &mut Context<Self>,
    ) {
        let Some(client) = self.transfer_client.clone() else {
            return;
        };

        let cancelled = shared_progress.cancelled.clone();
        let progress_for_callback = shared_progress.clone();
        let remote_path_for_refresh = remote_path.clone();

        let upload_task = Tokio::spawn(cx, async move {
            let mut client_guard = client.lock().await;
            if is_dir {
                client_guard
                    .upload_dir_with_progress(
                        local_path.to_string_lossy().as_ref(),
                        &remote_path,
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
                    .upload_with_progress(
                        local_path.to_string_lossy().as_ref(),
                        &remote_path,
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
            let result = match upload_task.await {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => Err(e),
                Err(e) => Err(anyhow::Error::new(e)),
            };

            let should_refresh = result.is_ok();

            let _ = this.update(cx, |this, cx| {
                this.update_task_state(task_id, result);
                this.schedule_transfers(cx);

                if should_refresh
                    && should_refresh_after_upload(&this.current_path, &remote_path_for_refresh)
                {
                    this.refresh_dir(cx);
                }
                cx.notify();
            });
        })
        .detach();
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

    fn start_delete_task(
        &mut self,
        task_id: usize,
        targets: Vec<DeleteTarget>,
        remote_dir: String,
        shared_progress: Arc<SharedProgress>,
        cx: &mut Context<Self>,
    ) {
        let Some(client) = self.transfer_client.clone() else {
            return;
        };

        let delete_task = Tokio::spawn(
            cx,
            delete_targets_with_progress(client, targets, shared_progress),
        );

        cx.spawn(async move |this, cx| {
            let result = match delete_task.await {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => Err(e),
                Err(e) => Err(anyhow::Error::new(e)),
            };
            let should_refresh = match &result {
                Ok(()) => true,
                Err(error) => !is_transfer_cancelled(error),
            };

            let _ = this.update(cx, |this, cx| {
                this.update_task_state(task_id, result);
                this.schedule_transfers(cx);
                if should_refresh && should_refresh_after_delete(&this.current_path, &remote_dir) {
                    this.clear_selection();
                    this.refresh_dir(cx);
                }
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
                        task.state = TransferTaskState::Failed;
                        task.error = Some(error.to_string());
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
        for upload in uploads {
            let task = TransferTask {
                id: self.alloc_task_id(),
                operation: TransferOperation::Upload {
                    local_path: upload.local_path,
                    remote_path: upload.remote_path,
                    is_dir: upload.is_dir,
                },
                state: TransferTaskState::Pending,
                shared_progress: SharedProgress::new(),
                error: None,
            };
            self.transfer_queue.enqueue(task);
        }

        self.ensure_transfer_client_and_schedule(cx);
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

        let Some(client) = self.sftp_client.clone() else {
            let uploads: Vec<_> = paths
                .into_iter()
                .map(|path| {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    PendingUpload {
                        remote_path: join_remote_path(remote_dir, &name),
                        is_dir: path.is_dir(),
                        local_path: path,
                        name,
                        has_conflict: false,
                    }
                })
                .collect();
            self.enqueue_pending_uploads(uploads, cx);
            return;
        };

        let remote_dir = remote_dir.to_string();
        let view = cx.entity().clone();
        let list_task = Tokio::spawn(cx, {
            let remote_dir = remote_dir.clone();
            async move {
                let mut client_guard = client.lock().await;
                client_guard.list_dir(&remote_dir).await
            }
        });

        window
            .spawn(cx, async move |cx| {
                let remote_names: HashSet<String> = match list_task.await {
                    Ok(Ok(entries)) => entries
                        .into_iter()
                        .filter(|entry| entry.name != "." && entry.name != "..")
                        .map(|entry| entry.name)
                        .collect(),
                    Ok(Err(e)) => {
                        tracing::error!("读取远程目录失败: {}", e);
                        let error_msg = t!("FileManager.read_dir_failed", error = e).to_string();
                        let _ = view.update_in(cx, |_this, window, cx| {
                            window.push_notification(Notification::error(error_msg.clone()), cx);
                        });
                        return;
                    }
                    Err(e) => {
                        tracing::error!("远程目录检查任务失败: {}", e);
                        let error_msg = t!("FileManager.read_dir_failed", error = e).to_string();
                        let _ = view.update_in(cx, |_this, window, cx| {
                            window.push_notification(Notification::error(error_msg.clone()), cx);
                        });
                        return;
                    }
                };

                let _ = view.update_in(cx, |this, window, cx| {
                    let mut pending_uploads = Vec::new();
                    let mut has_conflict = false;

                    for path in paths {
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let has_name_conflict = remote_names.contains(&name);
                        if has_name_conflict {
                            has_conflict = true;
                        }
                        pending_uploads.push(PendingUpload {
                            remote_path: join_remote_path(&remote_dir, &name),
                            is_dir: path.is_dir(),
                            local_path: path,
                            name,
                            has_conflict: has_name_conflict,
                        });
                    }

                    if pending_uploads.is_empty() {
                        return;
                    }

                    if has_conflict {
                        let conflict_names = pending_uploads
                            .iter()
                            .filter(|upload| upload.has_conflict)
                            .map(|upload| upload.name.clone())
                            .collect();
                        this.show_upload_conflict_dialog(
                            conflict_names,
                            pending_uploads,
                            remote_names,
                            window,
                            cx,
                        );
                    } else {
                        this.enqueue_pending_uploads(pending_uploads, cx);
                    }
                });
            })
            .detach();
    }

    fn show_upload_conflict_dialog(
        &mut self,
        conflict_names: Vec<String>,
        pending_uploads: Vec<PendingUpload>,
        existing_names: HashSet<String>,
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
        let has_dir_conflict = pending_uploads
            .iter()
            .any(|upload| upload.has_conflict && upload.is_dir);

        window.open_dialog(cx, move |dialog, _window, cx| {
            let view_overwrite = view.clone();
            let view_keep = view.clone();
            let view_skip = view.clone();
            let view_merge = view.clone();

            let uploads_overwrite = pending_uploads.clone();
            let uploads_keep = pending_uploads.clone();
            let uploads_skip = pending_uploads.clone();
            let uploads_merge = pending_uploads.clone();
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
                                let uploads = uploads_skip.clone();
                                move |_, window, cx| {
                                    window.close_dialog(cx);
                                    let uploads: Vec<_> = uploads
                                        .iter()
                                        .filter(|upload| !upload.has_conflict)
                                        .cloned()
                                        .collect();
                                    if !uploads.is_empty() {
                                        view.update(cx, |this, cx| {
                                            this.enqueue_pending_uploads(uploads, cx);
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
                                let uploads = uploads_keep.clone();
                                let existing = existing_names_keep.clone();
                                move |_, window, cx| {
                                    window.close_dialog(cx);
                                    let uploads = rename_conflicting_uploads(
                                        uploads.clone(),
                                        existing.clone(),
                                    );
                                    view.update(cx, |this, cx| {
                                        this.enqueue_pending_uploads(uploads, cx);
                                    });
                                }
                            })
                            .into_any_element(),
                    ];

                    if has_dir_conflict {
                        buttons.push(
                            Button::new("merge")
                                .label(t!("Conflict.merge").to_string())
                                .ghost()
                                .on_click({
                                    let view = view_merge.clone();
                                    let uploads = uploads_merge.clone();
                                    move |_, window, cx| {
                                        window.close_dialog(cx);
                                        let uploads: Vec<_> = uploads
                                            .iter()
                                            .filter(|upload| !upload.has_conflict || upload.is_dir)
                                            .cloned()
                                            .collect();
                                        if !uploads.is_empty() {
                                            view.update(cx, |this, cx| {
                                                this.enqueue_pending_uploads(uploads, cx);
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
                                let uploads = uploads_overwrite.clone();
                                move |_, window, cx| {
                                    window.close_dialog(cx);
                                    view.update(cx, |this, cx| {
                                        this.enqueue_pending_uploads(uploads.clone(), cx);
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

    /// 入队下载任务
    fn enqueue_download(
        &mut self,
        remote_path: String,
        local_path: PathBuf,
        is_dir: bool,
        cx: &mut Context<Self>,
    ) {
        let task = TransferTask {
            id: self.alloc_task_id(),
            operation: TransferOperation::Download {
                remote_path,
                local_path,
                is_dir,
            },
            state: TransferTaskState::Pending,
            shared_progress: SharedProgress::new(),
            error: None,
        };
        self.transfer_queue.enqueue(task);
        self.ensure_transfer_client_and_schedule(cx);
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

        let first_file = targets.first().map(|target| target.name.clone());
        let shared_progress = Arc::new(SharedProgress {
            transferred: AtomicU64::new(0),
            total: AtomicU64::new(targets.len() as u64),
            speed: AtomicU64::new(0),
            cancelled: Arc::new(AtomicBool::new(false)),
            current_file: std::sync::RwLock::new(first_file),
        });

        let task = TransferTask {
            id: self.alloc_task_id(),
            operation: TransferOperation::Delete {
                targets,
                remote_dir,
            },
            state: TransferTaskState::Pending,
            shared_progress,
            error: None,
        };
        self.transfer_queue.enqueue(task);
        self.ensure_transfer_client_and_schedule(cx);
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
                .confirm()
                .button_props(
                    DialogButtonProps::default()
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
                .confirm()
                .button_props(
                    DialogButtonProps::default()
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
                .confirm()
                .button_props(
                    DialogButtonProps::default()
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
        path: String,
        command: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_extract.is_some() {
            window.push_notification(Notification::info(t!("FileManager.extract_running")), cx);
            return;
        }

        self.active_extract = Some(ActiveExtract {
            name: name.clone(),
            path: path.clone(),
        });
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
                        this.active_extract = None;
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
                        this.active_extract = None;
                        window.push_notification(Notification::error(message), cx);
                    });
                }
                Err(error) => {
                    let message = t!("FileManager.extract_failed", error = error).to_string();
                    let _ = view.update_in(cx, |this, window, cx| {
                        this.active_extract = None;
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
                .confirm()
                .button_props(
                    DialogButtonProps::default()
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
        let breadcrumb = self.render_path_breadcrumb(cx);
        let has_selection = !self.selected_indices.is_empty();
        let is_connected = self.connection_state == ConnectionState::Connected;
        let is_favorite = self.is_current_path_favorite();
        let favorite_paths = self.favorite_paths.clone();
        let border = self.colors.border;
        let panel = self.colors.muted;
        let hover = self.colors.muted.opacity(0.72);
        let field_bg = self.colors.background;
        let foreground = self.colors.foreground;
        let muted_foreground = self.colors.muted_foreground;
        let accent = self.colors.accent;

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
                        Button::new("fm-upload-file")
                            .ghost()
                            .small()
                            .icon(IconName::Upload)
                            .tooltip(t!("FileManager.upload_file"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.select_and_upload_files(window, cx);
                            })),
                    )
                    .child(
                        Button::new("fm-new-file")
                            .ghost()
                            .small()
                            .icon(IconName::File)
                            .tooltip(t!("FileManager.new_file"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.show_new_file_dialog(window, cx);
                            })),
                    )
                    .child(
                        Button::new("fm-new-folder")
                            .ghost()
                            .small()
                            .icon(IconName::NewFolder)
                            .tooltip(t!("FileManager.new_folder"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.show_new_folder_dialog(window, cx);
                            })),
                    )
                    .child(
                        Button::new("fm-download")
                            .ghost()
                            .small()
                            .icon(IconName::ArrowDown)
                            .tooltip(t!("FileManager.download"))
                            .disabled(!has_selection)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.download_selected(window, cx);
                            })),
                    )
                    .child(
                        Button::new("fm-delete")
                            .ghost()
                            .small()
                            .danger()
                            .icon(IconName::Remove)
                            .tooltip(t!("FileManager.delete"))
                            .disabled(!has_selection)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.delete_selected(window, cx);
                            })),
                    )
                    .child(div().flex_1())
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
                                    .caret_color(accent)
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
                            .ghost()
                            .small()
                            .icon(if is_favorite {
                                IconName::StarFill
                            } else {
                                IconName::Star
                            })
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
            .ghost()
            .small()
            .icon(IconName::Ellipsis)
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
        let accent = self.colors.accent;

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
                        .caret_color(accent)
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
            base.w(px(70.))
        }
    }

    /// 渲染单行文件项
    fn render_file_row(
        &self,
        item: &RemoteFileItem,
        is_selected: bool,
        _cx: &App,
    ) -> impl IntoElement {
        let name = item.name.clone();
        let is_dir = item.is_dir;
        let foreground = self.colors.foreground;
        let muted_foreground = self.colors.muted_foreground;
        let selection = self.colors.accent.opacity(0.24);

        h_flex()
            .h(px(36.))
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
                        .with_size(Size::Small)
                        .color(),
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
                    .w(px(50.))
                    .text_xs()
                    .text_color(muted_foreground)
                    .child(if is_dir {
                        "-".to_string()
                    } else {
                        format_file_size(item.size)
                    }),
            )
            // 时间列
            .child(
                div()
                    .w(px(70.))
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
            .h(px(36.))
            .px_2()
            .items_center()
            .text_color(foreground)
            .child(
                h_flex()
                    .flex_1()
                    .gap_1()
                    .items_center()
                    .child(Icon::new(IconName::Folder1).with_size(Size::Small).color())
                    .child(div().text_sm().child("..")),
            )
            .child(div().w(px(50.)))
            .child(div().w(px(70.)))
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

        let mut menu = menu;

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
        menu = menu
            .separator()
            .item(
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
            )
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
        let Some(task) = self.transfer_queue.active_task() else {
            return div().into_any_element();
        };
        let border = self.colors.border;
        let panel = self.colors.muted;
        let hover = self.colors.muted.opacity(0.72);
        let muted_foreground = self.colors.muted_foreground;

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
            TransferOperation::Delete { targets, .. } => (
                IconName::Remove,
                t!("FileManager.delete_n_items", count = targets.len()).to_string(),
            ),
        };

        let transferred = task.shared_progress.transferred.load(Ordering::Relaxed);
        let total = task.shared_progress.total.load(Ordering::Relaxed);
        let speed_bits = task.shared_progress.speed.load(Ordering::Relaxed);
        let speed = f64::from_bits(speed_bits);

        let progress_pct = if total > 0 {
            (transferred as f64 / total as f64 * 100.0) as u32
        } else {
            0
        };

        let task_id = task.id;
        let is_running = task.state == TransferTaskState::Running;
        let pending_count = self.transfer_queue.pending_count();

        let status_text = match task.state {
            TransferTaskState::Pending => t!("FileManager.transfer_pending").to_string(),
            TransferTaskState::Running => {
                if is_running && speed > 0.0 {
                    format!("{}% {}", progress_pct, format_speed(speed))
                } else {
                    format!("{}%", progress_pct)
                }
            }
            TransferTaskState::Completed => t!("FileManager.transfer_done").to_string(),
            TransferTaskState::Failed => t!("FileManager.transfer_failed").to_string(),
            TransferTaskState::Cancelled => t!("FileManager.transfer_cancelled").to_string(),
        };

        let tooltip_label = label.clone();

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
                            .text_xs()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(label)
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
                    .when(
                        is_running || task.state == TransferTaskState::Pending,
                        |el| {
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
                                            this.cancel_transfer(task_id, cx);
                                        }),
                                    )
                                    .child(
                                        Icon::new(IconName::Close)
                                            .xsmall()
                                            .text_color(muted_foreground),
                                    ),
                            )
                        },
                    ),
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

    fn render_extract_progress(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        let Some(extract) = self.active_extract.clone() else {
            return div().into_any_element();
        };
        let tooltip_label = extract.path.clone();
        let border = self.colors.border;
        let panel = self.colors.muted;
        let muted_foreground = self.colors.muted_foreground;

        h_flex()
            .border_t_1()
            .border_color(border)
            .bg(panel)
            .px_2()
            .py_1()
            .gap_2()
            .items_center()
            .child(Spinner::new().small())
            .child(
                Icon::new(IconName::Unarchive)
                    .xsmall()
                    .text_color(muted_foreground),
            )
            .child(
                div()
                    .id("fm-extract-name")
                    .flex_1()
                    .text_xs()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(extract.name)
                    .tooltip(move |window, cx| {
                        Tooltip::new(tooltip_label.clone()).build(window, cx)
                    }),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted_foreground)
                    .child(t!("FileManager.extract_running")),
            )
            .into_any_element()
    }

    /// 渲染拖拽覆盖层
    fn render_drop_overlay(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        let foreground = self.colors.foreground;

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .bg(gpui::rgba(0x3b82f630))
            .border_2()
            .border_color(gpui::rgba(0x3b82f6ff))
            .rounded_md()
            .flex()
            .items_center()
            .justify_center()
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
        let has_active_transfer = self.transfer_queue.has_active();
        let has_active_extract = self.active_extract.is_some();
        let is_dragging = self.is_dragging_over;
        let background = self.colors.background;
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
                        .drag_over::<ExternalPaths>(|el, _, _, _cx| el.bg(gpui::rgba(0x3b82f620)))
                        .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                            this.is_dragging_over = false;
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
                                                let item = &state.items[real_ix];
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
                                                        item,
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
                        .when(is_dragging, |el| el.child(self.render_drop_overlay(cx))),
                )
            })
            // 底部传输进度条
            .when(has_active_transfer, |el| {
                el.child(self.render_transfer_progress(cx))
            })
            .when(!has_active_transfer && has_active_extract, |el| {
                el.child(self.render_extract_progress(cx))
            })
    }
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
        ConnectionState, NavigationRecoveryPlan, build_navigation_recovery_plan,
        build_retry_reset_plan, clear_remote_listing_state, frame_move_options,
        should_apply_directory_result, should_refresh_after_upload,
    };
    use one_core::sidebar_contribution::SidebarPlacement;
    use std::collections::HashSet;

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
    fn only_refresh_upload_target_directory_when_still_viewing_it() {
        assert!(should_refresh_after_upload("/srv/app", "/srv/app/file.txt"));
        assert!(!should_refresh_after_upload(
            "/srv/other",
            "/srv/app/file.txt"
        ));
    }

    #[test]
    fn delete_targets_follow_filtered_selection_order() {
        let items = vec![
            super::RemoteFileItem {
                name: "app.log".to_string(),
                size: 10,
                modified: std::time::UNIX_EPOCH,
                is_dir: false,
            },
            super::RemoteFileItem {
                name: "conf".to_string(),
                size: 0,
                modified: std::time::UNIX_EPOCH,
                is_dir: true,
            },
            super::RemoteFileItem {
                name: "data.db".to_string(),
                size: 20,
                modified: std::time::UNIX_EPOCH,
                is_dir: false,
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
            },
            super::RemoteFileItem {
                name: "conf".to_string(),
                size: 0,
                modified: std::time::UNIX_EPOCH,
                is_dir: true,
            },
            super::RemoteFileItem {
                name: "data.db".to_string(),
                size: 20,
                modified: std::time::UNIX_EPOCH,
                is_dir: false,
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
}
