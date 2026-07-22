mod branches;
mod file_actions;
mod frame;
mod header;
mod load;
mod render;

use crate::WorkspaceEditor;
use crate::editor::{GitDiffRequest, WorkspaceEditorEvent};
use crate::file_system::read_directory;
use crate::git::{GitChange, GitRepository, load_changes};
use crate::model::ExplorerEntry;
use crate::theme::WorkspaceTheme;
use gpui::{
    AppContext as _, AsyncApp, Context, Entity, ScrollHandle, Subscription, WeakEntity, Window,
};
use ignore::gitignore::Gitignore;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use self::load::{WorkspaceSnapshot, load_workspace};
use branches::BranchManager;
use file_actions::{ExplorerConfirmation, FileActionEditor};

pub use frame::{ExplorerFramePlacement, WorkspaceExplorerEvent};

pub struct WorkspaceExplorer {
    root: PathBuf,
    listings: HashMap<PathBuf, Vec<ExplorerEntry>>,
    expanded: HashSet<PathBuf>,
    loading_directories: HashSet<PathBuf>,
    selected_path: Option<PathBuf>,
    selected_change_path: Option<PathBuf>,
    repository: Option<GitRepository>,
    branch_manager: Option<Entity<BranchManager>>,
    changes: Vec<GitChange>,
    changes_expanded: bool,
    files_expanded: bool,
    loading: bool,
    git_loading: bool,
    git_refresh_pending: bool,
    error: Option<String>,
    git_error: Option<String>,
    refresh_generation: u64,
    editor: Entity<WorkspaceEditor>,
    theme: WorkspaceTheme,
    scroll_handle: ScrollHandle,
    show_hidden: bool,
    show_ignored: bool,
    ignore_matcher: Option<Arc<Gitignore>>,
    show_frame_controls: bool,
    frame_placement: ExplorerFramePlacement,
    file_action_editor: Option<FileActionEditor>,
    file_confirmation: Option<ExplorerConfirmation>,
    file_operation_running: bool,
    file_action_subscription: Option<Subscription>,
    _subscriptions: Vec<Subscription>,
}

pub struct WorkspaceExplorerConfig {
    pub root: PathBuf,
    pub editor: Entity<WorkspaceEditor>,
    pub theme: WorkspaceTheme,
    pub show_frame_controls: bool,
}

impl WorkspaceExplorer {
    pub fn new(config: WorkspaceExplorerConfig, cx: &mut Context<Self>) -> Self {
        let WorkspaceExplorerConfig {
            root,
            editor,
            theme,
            show_frame_controls,
        } = config;
        let editor_subscription =
            cx.subscribe(&editor, |this, _, event: &WorkspaceEditorEvent, cx| {
                if matches!(event, WorkspaceEditorEvent::FileSaved(_)) {
                    this.refresh_git(cx);
                }
            });
        let mut this = Self {
            root,
            listings: HashMap::new(),
            expanded: HashSet::new(),
            loading_directories: HashSet::new(),
            selected_path: None,
            selected_change_path: None,
            repository: None,
            branch_manager: None,
            changes: Vec::new(),
            changes_expanded: true,
            files_expanded: true,
            loading: false,
            git_loading: false,
            git_refresh_pending: false,
            error: None,
            git_error: None,
            refresh_generation: 0,
            editor,
            theme,
            scroll_handle: ScrollHandle::new(),
            show_hidden: false,
            show_ignored: false,
            ignore_matcher: None,
            show_frame_controls,
            frame_placement: ExplorerFramePlacement::Right,
            file_action_editor: None,
            file_confirmation: None,
            file_operation_running: false,
            file_action_subscription: None,
            _subscriptions: vec![editor_subscription],
        };
        this.refresh(cx);
        this
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn set_root(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        if !should_update_root(&self.root, &root, self.repository.is_some()) {
            return;
        }
        self.root = root;
        self.reset_workspace_state();
        self.refresh(cx);
    }

    fn reset_workspace_state(&mut self) {
        self.listings.clear();
        self.expanded.clear();
        self.loading_directories.clear();
        self.selected_path = None;
        self.selected_change_path = None;
        self.repository = None;
        self.branch_manager = None;
        self.changes.clear();
        self.ignore_matcher = None;
        self.git_loading = false;
        self.git_refresh_pending = false;
        self.file_action_editor = None;
        self.file_confirmation = None;
        self.file_operation_running = false;
        self.file_action_subscription = None;
    }

    pub fn set_theme(&mut self, theme: WorkspaceTheme, cx: &mut Context<Self>) {
        self.theme = theme;
        if let Some(manager) = self.branch_manager.as_ref() {
            manager.update(cx, |manager, cx| manager.set_theme(theme, cx));
        }
        self.editor
            .update(cx, |editor, cx| editor.set_theme(theme, cx));
        cx.notify();
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if let Some(manager) = self.branch_manager.as_ref() {
            manager.update(cx, |manager, cx| manager.reload(cx));
        }
        self.refresh_workspace(cx);
    }

    /// Refreshes repository and file state after an operation initiated by the
    /// branch manager. The manager refreshes itself before calling this method,
    /// so updating it again here would recursively lease the same GPUI entity.
    pub(super) fn refresh_after_branch_operation(&mut self, cx: &mut Context<Self>) {
        self.refresh_workspace(cx);
    }

    fn refresh_workspace(&mut self, cx: &mut Context<Self>) {
        self.refresh_generation = self.refresh_generation.wrapping_add(1);
        let generation = self.refresh_generation;
        self.loading = true;
        self.git_loading = false;
        self.git_refresh_pending = false;
        self.error = None;
        self.git_error = None;
        let root = self.root.clone();
        let show_hidden = self.show_hidden;
        let show_ignored = self.show_ignored;
        let task =
            cx.background_spawn(async move { load_workspace(root, show_hidden, show_ignored) });
        let entity = cx.entity().downgrade();
        cx.spawn(async move |_: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = task.await;
            let _ = entity.update(cx, |this, cx| {
                if this.refresh_generation != generation {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(snapshot) => this.apply_snapshot(snapshot),
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn apply_snapshot(&mut self, snapshot: WorkspaceSnapshot) {
        if self.repository.as_ref().map(|repository| &repository.root) != Some(&snapshot.root) {
            self.branch_manager = None;
        }
        self.root = snapshot.root.clone();
        self.listings.clear();
        self.listings.insert(snapshot.root, snapshot.entries);
        self.expanded.clear();
        self.loading_directories.clear();
        self.selected_path = None;
        self.repository = snapshot.repository;
        self.changes = snapshot.changes;
        if self
            .selected_change_path
            .as_ref()
            .is_some_and(|selected| !self.changes.iter().any(|change| &change.path == selected))
        {
            self.selected_change_path = None;
        }
        self.ignore_matcher = snapshot.ignore_matcher;
        self.git_loading = false;
        self.git_refresh_pending = false;
    }

    fn refresh_git(&mut self, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.clone() else {
            return;
        };
        if self.git_loading {
            self.git_refresh_pending = true;
            return;
        }
        self.git_loading = true;
        self.git_refresh_pending = false;
        self.git_error = None;
        let generation = self.refresh_generation;
        let repository_root = repository.root.clone();
        let task = cx.background_spawn(async move { load_changes(&repository) });
        let entity = cx.entity().downgrade();
        cx.spawn(async move |_: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = task.await;
            let _ = entity.update(cx, |this, cx| {
                let current = GitResultIdentity {
                    generation: this.refresh_generation,
                    repository: this.repository.as_ref().map(|repo| repo.root.as_path()),
                };
                let result_identity = GitResultIdentity {
                    generation,
                    repository: Some(&repository_root),
                };
                if !accepts_git_result(current, result_identity) {
                    return;
                }
                this.git_loading = false;
                match result {
                    Ok(changes) => {
                        if this.selected_change_path.as_ref().is_some_and(|selected| {
                            !changes.iter().any(|change| &change.path == selected)
                        }) {
                            this.selected_change_path = None;
                        }
                        this.changes = changes;
                    }
                    Err(error) => this.git_error = Some(error.to_string()),
                }
                let refresh_again = this.git_refresh_pending;
                this.git_refresh_pending = false;
                cx.notify();
                if refresh_again {
                    this.refresh_git(cx);
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn toggle_directory(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.expanded.remove(&path) {
            cx.notify();
            return;
        }
        self.expanded.insert(path.clone());
        if self.listings.contains_key(&path) || self.loading_directories.contains(&path) {
            cx.notify();
            return;
        }
        self.loading_directories.insert(path.clone());
        let generation = self.refresh_generation;
        let task_path = path.clone();
        let show_hidden = self.show_hidden;
        let show_ignored = self.show_ignored;
        let matcher = self.ignore_matcher.clone();
        let task = cx.background_spawn(async move {
            read_directory(&task_path, matcher.as_deref(), show_hidden, show_ignored)
        });
        let entity = cx.entity().downgrade();
        cx.spawn(async move |_: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = task.await;
            let _ = entity.update(cx, |this, cx| {
                if this.refresh_generation != generation {
                    return;
                }
                this.loading_directories.remove(&path);
                match result {
                    Ok(entries) => {
                        this.listings.insert(path.clone(), entries);
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn open_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.selected_path = Some(path.clone());
        self.editor
            .update(cx, |editor, cx| editor.open_file(path, window, cx));
        cx.notify();
    }

    fn open_change(&mut self, change: GitChange, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.clone() else {
            return;
        };
        self.selected_change_path = Some(change.path.clone());
        self.editor.update(cx, |editor, cx| {
            editor.open_git_change(GitDiffRequest { repository, change }, window, cx);
        });
        cx.notify();
    }
}

#[derive(Clone, Copy)]
struct GitResultIdentity<'a> {
    generation: u64,
    repository: Option<&'a Path>,
}

fn accepts_git_result(current: GitResultIdentity<'_>, result: GitResultIdentity<'_>) -> bool {
    current.generation == result.generation && current.repository == result.repository
}

fn should_update_root(current: &Path, requested: &Path, in_repository: bool) -> bool {
    current != requested && !(in_repository && requested.starts_with(current))
}

#[cfg(test)]
mod tests;
