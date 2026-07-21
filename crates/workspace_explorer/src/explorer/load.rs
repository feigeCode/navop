use crate::file_system::{canonical_workspace_root, read_directory};
use crate::git::{GitChange, GitRepository, discover_repository, load_changes};
use crate::model::ExplorerEntry;
use anyhow::Result;
use std::path::PathBuf;

pub(super) struct WorkspaceSnapshot {
    pub(super) root: PathBuf,
    pub(super) entries: Vec<ExplorerEntry>,
    pub(super) repository: Option<GitRepository>,
    pub(super) changes: Vec<GitChange>,
}

pub(super) fn load_workspace(root: PathBuf) -> Result<WorkspaceSnapshot> {
    let initial_root = canonical_workspace_root(root)?;
    let repository = discover_repository(&initial_root)?;
    let root = repository
        .as_ref()
        .map(|repository| repository.root.clone())
        .unwrap_or(initial_root);
    let entries = read_directory(&root)?;
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
    })
}
