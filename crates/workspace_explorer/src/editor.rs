mod load;
mod markdown;
mod render;
mod save;
mod tabs;

use crate::diff::{AlignedDiffSide, SideBySideDiff, aligned_side_by_side};
use crate::file_system::LoadedFile;
use crate::git::{GitChange, GitRepository};
use crate::theme::WorkspaceTheme;
use gpui::{App, Context, Entity, EventEmitter, ScrollHandle, Subscription};
use gpui_component::input::{InputLineDecoration, InputState};
use notes::NotesView;
use remote_file_editor::EditorMode;
use std::path::{Path, PathBuf};
use std::rc::Rc;

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
    diff_language: Option<String>,
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
            diff_language: None,
            file_size: file.file_size,
            policy,
            read_only: false,
        }
    }

    pub(super) fn from_diff(diff: String, language: String) -> Self {
        Self {
            file_size: diff.len(),
            text: diff,
            language: "diff".to_string(),
            diff_language: Some(language),
            policy: DocumentPolicy::Diff,
            read_only: true,
        }
    }
}

pub(super) struct DiffEditors {
    left: Entity<InputState>,
    right: Entity<InputState>,
}

pub(super) struct EditorTab {
    id: u64,
    key: DocumentKey,
    display_name: String,
    editor: Option<Entity<InputState>>,
    markdown: Option<Entity<NotesView>>,
    subscriptions: Vec<Subscription>,
    diff: Option<Rc<SideBySideDiff>>,
    diff_editors: Option<DiffEditors>,
    diff_side_by_side: bool,
    diff_scroll: ScrollHandle,
    diff_change_cursor: Option<usize>,
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
            markdown: None,
            subscriptions: Vec::new(),
            diff: None,
            diff_editors: None,
            diff_side_by_side: true,
            diff_scroll: ScrollHandle::new(),
            diff_change_cursor: None,
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
        for tab in &self.tabs {
            if let Some(markdown) = tab.markdown.as_ref() {
                let theme = markdown::markdown_editor_theme(theme);
                markdown.update(cx, |view, cx| view.set_editor_theme(theme, cx));
            }
            let (Some(diff), Some(editors)) = (&tab.diff, &tab.diff_editors) else {
                continue;
            };
            let (left, right) = aligned_side_by_side(diff);
            let left_decorations =
                diff_line_decorations(&left, theme.danger, theme.muted.opacity(0.35));
            let right_decorations =
                diff_line_decorations(&right, theme.success, theme.muted.opacity(0.35));
            editors.left.update(cx, |state, cx| {
                state.set_line_decorations(left_decorations, cx);
            });
            editors.right.update(cx, |state, cx| {
                state.set_line_decorations(right_decorations, cx);
            });
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

pub(super) fn diff_line_decorations(
    side: &AlignedDiffSide,
    changed_color: gpui::Hsla,
    placeholder_color: gpui::Hsla,
) -> Vec<InputLineDecoration> {
    side.line_numbers
        .iter()
        .zip(&side.changed)
        .zip(&side.placeholders)
        .map(
            |((&line_number, &changed), &placeholder)| InputLineDecoration {
                line_number,
                background: if changed {
                    Some(changed_color.opacity(0.15))
                } else if placeholder {
                    Some(placeholder_color)
                } else {
                    None
                },
            },
        )
        .collect()
}

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
