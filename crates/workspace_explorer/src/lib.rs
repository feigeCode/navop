//! Local workspace file browsing, editing, and Git change inspection for GPUI hosts.
//!
//! The crate intentionally keeps filesystem and Git behavior out of host views. A host creates a
//! [`WorkspaceEditor`] and passes it to [`WorkspaceExplorer`]. Selecting a file or Git change in
//! the explorer opens the corresponding document in the editor.

rust_i18n::i18n!("locales", fallback = "en");

mod backend;
pub mod diff;
mod editor;
mod explorer;
mod file_system;
pub mod git;
mod model;
mod theme;

pub use backend::{
    ContainerBackend, LocalBackend, WorkspaceBackend, container_backend, local_backend,
};
pub use diff::{AlignedDiffSide, DiffLine, DiffLineKind, DiffRow, SideBySideDiff};
pub use editor::{GitDiffRequest, WorkspaceEditor, WorkspaceEditorEvent};
pub use explorer::{
    ExplorerFramePlacement, WorktreeReviewSnapshot, WorkspaceExplorer, WorkspaceExplorerConfig,
    WorkspaceExplorerEvent,
};
pub use git::{
    CreatedWorktree, GitBranch, GitBranchKind, GitChange, GitChangeKind, GitRepository,
    WorktreeEntry, anchor_checkpoint, anchored_checkpoint, capture_worktree_snapshot, commit_all,
    commit_context, create_worktree, diff_snapshots, discover_repository, discard_change,
    list_worktrees, load_branches, load_changes, load_diff, push_current_branch, remove_worktree,
    stage_change, unstage_change,
};
pub use theme::WorkspaceTheme;

/// Registers workspace explorer keyboard shortcuts.
pub fn init(cx: &mut gpui::App) {
    cx.bind_keys(explorer::keybindings());
    cx.bind_keys(editor::keybindings());
}
