use crate::file_system::{canonical_workspace_root, read_directory, root_ignore_matcher};
use crate::git::{GitChange, GitRepository, discover_repository, load_changes};
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
    pub(super) ignore_matcher: Option<Arc<Gitignore>>,
}

pub(super) fn load_workspace(
    root: PathBuf,
    show_hidden: bool,
    show_ignored: bool,
) -> Result<WorkspaceSnapshot> {
    let initial_root = canonical_workspace_root(root)?;
    let repository = discover_repository(&initial_root)?;
    let root = repository
        .as_ref()
        .map(|repository| repository.root.clone())
        .unwrap_or(initial_root);
    let ignore_matcher = if show_ignored {
        None
    } else {
        root_ignore_matcher(&root)
    };
    let entries = read_directory(&root, ignore_matcher.as_deref(), show_hidden, show_ignored)?;
    let changes = repository
        .as_ref()
        .map(load_changes)
        .transpose()?
        .unwrap_or_default();
    Ok(WorkspaceSnapshot {
        root,
        entries,
        repository,
        changes,
        ignore_matcher,
    })
}
