use crate::backend::WorkspaceBackend;
use crate::git::{
    GitChange, GitRepository, WorktreeEntry, anchored_checkpoint, discover_repository,
    list_worktrees, load_changes,
};
use crate::model::ExplorerEntry;
use anyhow::Result;
use ignore::gitignore::Gitignore;
use std::path::PathBuf;
use std::sync::Arc;

pub(super) struct WorkspaceSnapshot {
    pub(super) root: PathBuf,
    pub(super) entries: Vec<ExplorerEntry>,
    pub(super) repository: Option<GitRepository>,
    pub(super) changes: Vec<GitChange>,
    /// 仓库已注册的 worktree；非 Git 后端为空。
    pub(super) worktrees: Vec<WorktreeEntry>,
    /// 上次锚定的 checkpoint（重启恢复的 last-turn 基线）。
    pub(super) anchored_checkpoint: Option<String>,
    pub(super) ignore_matcher: Option<Arc<Gitignore>>,
}

pub(super) fn load_workspace(
    root: PathBuf,
    show_hidden: bool,
    show_ignored: bool,
    backend: Arc<dyn WorkspaceBackend>,
) -> Result<WorkspaceSnapshot> {
    let initial_root = backend.canonical_root(root)?;
    // 容器后端没有本机 git 仓库,跳过仓库发现与变更视图。
    let repository = if backend.supports_git() {
        discover_repository(&initial_root)?
    } else {
        None
    };
    let root = repository
        .as_ref()
        .map(|repository| repository.root.clone())
        .unwrap_or(initial_root);
    let ignore_matcher = if show_ignored {
        None
    } else {
        backend.root_ignore_matcher(&root)
    };
    let entries =
        backend.read_directory(&root, ignore_matcher.as_deref(), show_hidden, show_ignored)?;
    let changes = repository
        .as_ref()
        .map(load_changes)
        .transpose()?
        .unwrap_or_default();
    let worktrees = repository
        .as_ref()
        .map(list_worktrees)
        .transpose()?
        .unwrap_or_default();
    let anchored_checkpoint = repository
        .as_ref()
        .map(anchored_checkpoint)
        .transpose()?
        .unwrap_or_default();
    Ok(WorkspaceSnapshot {
        root,
        entries,
        repository,
        changes,
        worktrees,
        anchored_checkpoint,
        ignore_matcher,
    })
}
