mod load;
mod render;
mod save;
mod tabs;

use crate::file_system::LoadedFile;
use crate::git::{GitChange, GitRepository};
use crate::theme::WorkspaceTheme;
use gpui::{App, Context, Entity, EventEmitter, Subscription};
use gpui_component::input::InputState;
use remote_file_editor::EditorMode;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub enum WorkspaceEditorEvent {
    VisibilityChanged(bool),
    FileSaved(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum DocumentKey {
    File(PathBuf),
    Diff { repository: PathBuf, path: PathBuf },
}

impl DocumentKey {
    pub(super) fn identity_path(&self) -> PathBuf {
        match self {
            Self::File(path) => path.clone(),
            Self::Diff { repository, path } => repository
                .join(".git")
                .join("workspace-explorer-diff")
                .join(path),
        }
    }

    pub(super) fn display_path(&self) -> String {
        match self {
            Self::File(path) => path.display().to_string(),
            Self::Diff { repository, path } => {
                format!("{} · {}", repository.display(), path.display())
            }
        }
    }
}

pub(super) enum LoadRequest {
    File(PathBuf),
    Diff {
        repository: GitRepository,
        change: GitChange,
    },
}

pub(crate) struct GitDiffRequest {
    pub(crate) repository: GitRepository,
    pub(crate) change: GitChange,
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
    Diff,
}

impl LoadedDocument {
    pub(super) fn from_file(file: LoadedFile) -> Self {
        let policy = match file.policy.mode {
            EditorMode::Code => DocumentPolicy::Code,
            EditorMode::PlainText => DocumentPolicy::PlainText,
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
            language: "diff".to_string(),
            policy: DocumentPolicy::Diff,
            read_only: true,
        }
    }
}

pub(super) struct EditorTab {
    id: u64,
    key: DocumentKey,
    display_name: String,
    editor: Option<Entity<InputState>>,
    subscriptions: Vec<Subscription>,
    saved_text: String,
    file_size: usize,
    policy: DocumentPolicy,
    loading: bool,
    saving: bool,
    soft_wrap: bool,
    read_only: bool,
    status_message: String,
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
            subscriptions: Vec::new(),
            saved_text: String::new(),
            file_size: 0,
            policy: DocumentPolicy::Code,
            loading: true,
            saving: false,
            soft_wrap: false,
            read_only: false,
            status_message: rust_i18n::t!("WorkspaceExplorer.status.loading").to_string(),
            load_error: None,
            load_request: document.load_request,
        }
    }

    pub(super) fn is_dirty(&self, cx: &App) -> bool {
        !self.read_only
            && self
                .editor
                .as_ref()
                .is_some_and(|editor| editor.read(cx).text() != self.saved_text.as_str())
    }
}

pub struct WorkspaceEditor {
    tabs: Vec<EditorTab>,
    active_tab: usize,
    next_tab_id: u64,
    close_prompt_open: bool,
    pending_close_tab: Option<usize>,
    theme: WorkspaceTheme,
}

impl WorkspaceEditor {
    pub fn new(theme: WorkspaceTheme) -> Self {
        Self {
            tabs: Vec::new(),
            active_tab: 0,
            next_tab_id: 1,
            close_prompt_open: false,
            pending_close_tab: None,
            theme,
        }
    }

    pub fn set_theme(&mut self, theme: WorkspaceTheme, cx: &mut Context<Self>) {
        self.theme = theme;
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
