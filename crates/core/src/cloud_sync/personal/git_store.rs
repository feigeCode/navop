use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::cloud_sync::models::CloudSyncData;

use super::{
    DirectorySyncStore, PersonalSyncStore, SyncDeviceId, SyncStoreError, SyncStoreLock,
    SyncStoreStatus,
};

const SYNC_COMMIT_MESSAGE: &str = "onetcli sync: update personal records";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GitSyncOptions {
    pub auto_push: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitRunnerError {
    AuthRequired,
    MergeConflict,
    NothingToCommit,
    CommandFailed(String),
}

#[async_trait]
pub trait GitRunner: Clone + Send + Sync {
    async fn pull_rebase(&self, repo: &Path) -> Result<(), GitRunnerError>;
    async fn add_sync_package(&self, repo: &Path) -> Result<(), GitRunnerError>;
    async fn commit_sync_package(&self, repo: &Path, message: &str) -> Result<(), GitRunnerError>;
    async fn push(&self, repo: &Path) -> Result<(), GitRunnerError>;
    async fn remote_url(&self, repo: &Path) -> Result<Option<String>, GitRunnerError>;
    async fn is_repo(&self, repo: &Path) -> Result<bool, GitRunnerError>;
    async fn is_clean_for_sync(&self, repo: &Path) -> Result<bool, GitRunnerError>;
}

#[derive(Debug, Clone, Default)]
pub struct CommandGitRunner;

#[async_trait]
impl GitRunner for CommandGitRunner {
    async fn pull_rebase(&self, repo: &Path) -> Result<(), GitRunnerError> {
        run_git(repo, &["pull", "--rebase"]).map(|_| ())
    }

    async fn add_sync_package(&self, repo: &Path) -> Result<(), GitRunnerError> {
        run_git(repo, &["add", ".onetcli-sync"]).map(|_| ())
    }

    async fn commit_sync_package(&self, repo: &Path, message: &str) -> Result<(), GitRunnerError> {
        match run_git(repo, &["commit", "-m", message]) {
            Err(GitRunnerError::NothingToCommit) => Ok(()),
            result => result.map(|_| ()),
        }
    }

    async fn push(&self, repo: &Path) -> Result<(), GitRunnerError> {
        run_git(repo, &["push"]).map(|_| ())
    }

    async fn remote_url(&self, repo: &Path) -> Result<Option<String>, GitRunnerError> {
        match run_git(repo, &["remote", "get-url", "origin"]) {
            Ok(output) => Ok(Some(output.trim().to_string()).filter(|value| !value.is_empty())),
            Err(_) => Ok(None),
        }
    }

    async fn is_repo(&self, repo: &Path) -> Result<bool, GitRunnerError> {
        match run_git(repo, &["rev-parse", "--is-inside-work-tree"]) {
            Ok(output) => Ok(output.trim() == "true"),
            Err(_) => Ok(false),
        }
    }

    async fn is_clean_for_sync(&self, repo: &Path) -> Result<bool, GitRunnerError> {
        let output = run_git(repo, &["status", "--porcelain", "--", ".onetcli-sync"])?;
        Ok(output.trim().is_empty())
    }
}

#[derive(Clone)]
pub struct GitSyncStore<R = CommandGitRunner> {
    root: PathBuf,
    directory: DirectorySyncStore,
    runner: R,
    options: GitSyncOptions,
    dirty: Arc<Mutex<bool>>,
}

impl<R> GitSyncStore<R>
where
    R: GitRunner,
{
    pub fn new(root: PathBuf, runner: R, options: GitSyncOptions) -> Self {
        Self {
            directory: DirectorySyncStore::new(root.clone()),
            root,
            runner,
            options,
            dirty: Arc::new(Mutex::new(false)),
        }
    }

    pub async fn flush(&self) -> Result<(), SyncStoreError> {
        if !self.take_dirty()? || self.runner.is_clean_for_sync(&self.root).await? {
            return Ok(());
        }

        self.runner.add_sync_package(&self.root).await?;
        self.runner
            .commit_sync_package(&self.root, SYNC_COMMIT_MESSAGE)
            .await?;
        if self.options.auto_push {
            self.runner.push(&self.root).await?;
        }
        Ok(())
    }

    fn mark_dirty(&self) -> Result<(), SyncStoreError> {
        *self
            .dirty
            .lock()
            .map_err(|_| SyncStoreError::Io("git dirty lock poisoned".to_string()))? = true;
        Ok(())
    }

    fn take_dirty(&self) -> Result<bool, SyncStoreError> {
        let mut dirty = self
            .dirty
            .lock()
            .map_err(|_| SyncStoreError::Io("git dirty lock poisoned".to_string()))?;
        let was_dirty = *dirty;
        *dirty = false;
        Ok(was_dirty)
    }
}

#[async_trait]
impl<R> PersonalSyncStore for GitSyncStore<R>
where
    R: GitRunner,
{
    fn backend_id(&self) -> &'static str {
        "git"
    }

    async fn probe(&self) -> Result<SyncStoreStatus, SyncStoreError> {
        if !self.runner.is_repo(&self.root).await? {
            return Err(SyncStoreError::NotConfigured);
        }
        self.runner.pull_rebase(&self.root).await?;
        self.directory.probe().await
    }

    async fn list_records(
        &self,
        data_type: Option<&str>,
        since: Option<i64>,
    ) -> Result<Vec<CloudSyncData>, SyncStoreError> {
        self.directory.list_records(data_type, since).await
    }

    async fn upsert_record(
        &self,
        record: &CloudSyncData,
        expected_version: Option<u32>,
    ) -> Result<CloudSyncData, SyncStoreError> {
        let stored = self
            .directory
            .upsert_record(record, expected_version)
            .await?;
        self.mark_dirty()?;
        Ok(stored)
    }

    async fn tombstone_record(
        &self,
        id: &str,
        expected_version: Option<u32>,
    ) -> Result<(), SyncStoreError> {
        self.directory
            .tombstone_record(id, expected_version)
            .await?;
        self.mark_dirty()
    }

    async fn acquire_lock(&self, owner: &SyncDeviceId) -> Result<SyncStoreLock, SyncStoreError> {
        self.directory.acquire_lock(owner).await
    }
}

impl From<GitRunnerError> for SyncStoreError {
    fn from(error: GitRunnerError) -> Self {
        match error {
            GitRunnerError::AuthRequired => Self::GitAuthRequired,
            GitRunnerError::MergeConflict => Self::GitMergeConflict,
            GitRunnerError::NothingToCommit => Self::Conflict("nothing to commit".to_string()),
            GitRunnerError::CommandFailed(message) => Self::Io(message),
        }
    }
}

fn run_git(repo: &Path, args: &[&str]) -> Result<String, GitRunnerError> {
    let output = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .map_err(|error| GitRunnerError::CommandFailed(error.to_string()))?;

    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).to_string());
    }

    Err(classify_failure(&output.stderr, &output.stdout))
}

fn classify_failure(stderr: &[u8], stdout: &[u8]) -> GitRunnerError {
    let message = format!(
        "{}{}",
        String::from_utf8_lossy(stderr),
        String::from_utf8_lossy(stdout)
    );
    let normalized = message.to_ascii_lowercase();
    if normalized.contains("authentication") || normalized.contains("permission denied") {
        return GitRunnerError::AuthRequired;
    }
    if normalized.contains("conflict") || normalized.contains("unmerged") {
        return GitRunnerError::MergeConflict;
    }
    if normalized.contains("nothing to commit") {
        return GitRunnerError::NothingToCommit;
    }
    GitRunnerError::CommandFailed(message)
}
