mod load;
mod markdown;
mod render;
mod save;
mod tabs;

use crate::file_system::LoadedFile;
use crate::git::{GitChange, GitRepository};
use crate::theme::WorkspaceTheme;
use crate::{WorkspaceBackend, local_backend};
use gpui::{App, Context, Entity, EventEmitter, KeyBinding, Subscription, actions};
use gpui_component::input::EditorState;
use gpui_component::diff::DiffState;
use notes::NotesView;
use one_ui::StatusPresentation;
use remote_file_editor::EditorMode;
use std::path::{Path, PathBuf};
use std::sync::Arc;

actions!(workspace_editor, [SaveDocument]);

pub(crate) const WORKSPACE_EDITOR_KEY_CONTEXT: &str = "WorkspaceEditor";

/// 编辑器键盘快捷键。`secondary-s` 在 macOS 上为 Cmd+S,其他平台为 Ctrl+S。
pub(crate) fn keybindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new(
        "secondary-s",
        SaveDocument,
        Some(WORKSPACE_EDITOR_KEY_CONTEXT),
    )]
}

#[derive(Clone, Debug)]
pub enum WorkspaceEditorEvent {
    VisibilityChanged(bool),
    FileSaved(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum DocumentKey {
    File(PathBuf),
    Diff { repository: PathBuf, path: PathBuf },
    /// 稳定单例 key：同一会话的 last-turn review 复用同一标签页刷新。
    SnapshotDiff,
    /// 单个文件的审阅 diff（本轮快照里裁出来的那一段）；按文件分页。
    ReviewFile(PathBuf),
}

impl DocumentKey {
    pub(super) fn identity_path(&self) -> PathBuf {
        match self {
            Self::File(path) => path.clone(),
            Self::Diff { repository, path } => repository
                .join(".git")
                .join("workspace-explorer-diff")
                .join(path),
            Self::SnapshotDiff => PathBuf::from("navop://last-turn-review"),
            // 不能写成 `"navop://review".join(path)`：`join` 遇到绝对路径会整段
            // 替换，于是和 `Self::File` 撞成同一个身份、审阅页会把文件页顶掉。
            Self::ReviewFile(path) => PathBuf::from(format!("navop://review{}", path.display())),
        }
    }

    pub(super) fn display_path(&self) -> String {
        match self {
            Self::File(path) => path.display().to_string(),
            Self::Diff { repository, path } => {
                format!("{} · {}", repository.display(), path.display())
            }
            Self::SnapshotDiff => "last-turn".to_string(),
            Self::ReviewFile(path) => path.display().to_string(),
        }
    }
}

pub(super) enum LoadRequest {
    File(PathBuf),
    Diff {
        repository: GitRepository,
        change: GitChange,
    },
    /// 已就绪的 patch 原文（整轮 checkpoint diff、单文件审阅切片……），无需再查 git。
    SnapshotDiff { text: String },
}

impl LoadRequest {
    /// 是否为差异文档（只读快照）。
    ///
    /// 复用同名标签页时，文件与差异要分开对待：差异页每次打开都换成新的 patch，
    /// 文件页只能聚焦（重载会吞掉用户的未保存改动）。
    pub(super) fn is_diff(&self) -> bool {
        matches!(self, Self::Diff { .. } | Self::SnapshotDiff { .. })
    }
}

/// 请求编辑器展示某条 Git 变更的 diff。
///
/// 供工作台审阅面板等外部容器构造；`repository` 与 `change` 均来自
/// [`crate::git`] 的公开查询函数。
#[derive(Clone, Debug)]
pub struct GitDiffRequest {
    pub repository: GitRepository,
    pub change: GitChange,
}

pub(super) struct PendingDocument {
    pub(super) key: DocumentKey,
    pub(super) display_name: String,
    pub(super) load_request: LoadRequest,
}

pub(super) struct LoadedDocument {
    text: String,
    language: String,
    file_size: usize,
    policy: DocumentPolicy,
    read_only: bool,
}

#[derive(Clone, Copy)]
pub(super) enum DocumentPolicy {
    Code,
    PlainText,
    Markdown,
    Diff,
}

impl LoadedDocument {
    pub(super) fn from_file(path: &Path, file: LoadedFile) -> Self {
        let policy = if is_markdown_path(path) {
            DocumentPolicy::Markdown
        } else {
            match file.policy.mode {
                EditorMode::Code => DocumentPolicy::Code,
                EditorMode::PlainText => DocumentPolicy::PlainText,
            }
        };
        Self {
            text: file.text,
            language: file.language,
            file_size: file.file_size,
            policy,
            read_only: false,
        }
    }

    pub(super) fn from_diff(diff: String) -> Self {
        Self {
            file_size: diff.len(),
            text: diff,
            language: String::new(),
            policy: DocumentPolicy::Diff,
            read_only: true,
        }
    }

    /// 整轮快照 diff：多文件 patch,与单文件 diff 走同一条组件渲染路径。
    pub(super) fn from_snapshot_diff(diff: String) -> Self {
        Self {
            file_size: diff.len(),
            text: diff,
            language: String::new(),
            policy: DocumentPolicy::Diff,
            read_only: true,
        }
    }
}

pub(super) struct EditorTab {
    id: u64,
    key: DocumentKey,
    display_name: String,
    editor: Option<Entity<EditorState>>,
    markdown: Option<Entity<NotesView>>,
    subscriptions: Vec<Subscription>,
    /// Diff 组件状态:从 patch 原文解析,负责全部 diff 渲染(Split/Unified)。
    diff_state: Option<Entity<DiffState>>,
    /// `true` 为 Split 双栏,`false` 为 Unified 单栏。
    diff_side_by_side: bool,
    saved_text: String,
    file_size: usize,
    policy: DocumentPolicy,
    loading: bool,
    saving: bool,
    soft_wrap: bool,
    read_only: bool,
    status_message: String,
    status_presentation: StatusPresentation,
    load_error: Option<String>,
    load_request: LoadRequest,
}

impl EditorTab {
    pub(super) fn new(id: u64, document: PendingDocument) -> Self {
        Self {
            id,
            key: document.key,
            display_name: document.display_name,
            editor: None,
            markdown: None,
            subscriptions: Vec::new(),
            diff_state: None,
            // 默认单栏(Unified)。审阅面板是可调宽的侧栏,窄下来时 Split 的右栏
            // 会被父级 `overflow_hidden` 整块裁掉——宁可先给一栏看得全的,
            // 要并排再点工具栏切换(那里本来就有开关)。
            diff_side_by_side: false,
            saved_text: String::new(),
            file_size: 0,
            policy: DocumentPolicy::Code,
            loading: true,
            saving: false,
            soft_wrap: false,
            read_only: false,
            status_message: rust_i18n::t!("WorkspaceExplorer.status.loading").to_string(),
            status_presentation: StatusPresentation::Progress,
            load_error: None,
            load_request: document.load_request,
        }
    }

    pub(super) fn is_dirty(&self, cx: &App) -> bool {
        if let Some(markdown) = self.markdown.as_ref() {
            return markdown.read(cx).has_unsaved_changes(cx);
        }
        !self.read_only
            && self
                .editor
                .as_ref()
                .is_some_and(|editor| editor.read(cx).text() != self.saved_text.as_str())
    }
}

fn is_markdown_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
}

pub struct WorkspaceEditor {
    tabs: Vec<EditorTab>,
    active_tab: usize,
    next_tab_id: u64,
    close_prompt_open: bool,
    pending_close_tab: Option<usize>,
    theme: WorkspaceTheme,
    /// 文件读写后端(本机或容器);与 `WorkspaceExplorer` 共用同一个实例。
    backend: Arc<dyn WorkspaceBackend>,
}

impl WorkspaceEditor {
    pub fn new(theme: WorkspaceTheme) -> Self {
        Self::with_backend(theme, local_backend())
    }

    /// 用指定后端构造(容器会话传入容器后端)。
    pub fn with_backend(theme: WorkspaceTheme, backend: Arc<dyn WorkspaceBackend>) -> Self {
        Self {
            tabs: Vec::new(),
            active_tab: 0,
            next_tab_id: 1,
            close_prompt_open: false,
            pending_close_tab: None,
            theme,
            backend,
        }
    }

    pub fn set_theme(&mut self, theme: WorkspaceTheme, cx: &mut Context<Self>) {
        self.theme = theme;
        for tab in &self.tabs {
            if let Some(markdown) = tab.markdown.as_ref() {
                let theme = markdown::markdown_editor_theme(theme);
                markdown.update(cx, |view, cx| view.set_editor_theme(theme, cx));
            }
        }
        cx.notify();
    }

    pub fn has_open_tabs(&self) -> bool {
        !self.tabs.is_empty()
    }

    pub fn has_dirty_tabs(&self, cx: &App) -> bool {
        self.tabs.iter().any(|tab| tab.is_dirty(cx))
    }

    pub(super) fn active_tab(&self) -> Option<&EditorTab> {
        self.tabs.get(self.active_tab)
    }

    pub(super) fn active_tab_mut(&mut self) -> Option<&mut EditorTab> {
        self.tabs.get_mut(self.active_tab)
    }

    pub(super) fn tab_index(&self, tab_id: u64, key: &DocumentKey) -> Option<usize> {
        self.tabs
            .iter()
            .position(|tab| tab.id == tab_id && &tab.key == key)
    }
}

impl Default for WorkspaceEditor {
    fn default() -> Self {
        panic!("WorkspaceEditor requires an explicit WorkspaceTheme")
    }
}

impl EventEmitter<WorkspaceEditorEvent> for WorkspaceEditor {}

pub(super) fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

pub(super) fn format_size(size: usize) -> String {
    const KIB: usize = 1024;
    const MIB: usize = KIB * 1024;
    if size >= MIB {
        format!("{:.1} MiB", size as f64 / MIB as f64)
    } else if size >= KIB {
        format!("{:.1} KiB", size as f64 / KIB as f64)
    } else {
        format!("{size} B")
    }
}

#[cfg(test)]
mod tests;
