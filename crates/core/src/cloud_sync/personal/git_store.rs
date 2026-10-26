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
    /// 把同步包目录的未提交改动暂存进 git stash（issue #457 的脏工作区自愈）。
    async fn stash_sync_package(&self, repo: &Path) -> Result<(), GitRunnerError>;
    /// 恢复最近一次暂存的同步包改动。
    async fn stash_pop(&self, repo: &Path) -> Result<(), GitRunnerError>;
    /// 中止进行中的 rebase；没有 rebase 在进行时失败是预期内的。
    async fn rebase_abort(&self, repo: &Path) -> Result<(), GitRunnerError>;
    /// 把同步包目录重置回 HEAD（丢弃未提交改动，含 unmerged 状态）。
    async fn reset_sync_package(&self, repo: &Path) -> Result<(), GitRunnerError>;
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

    async fn stash_sync_package(&self, repo: &Path) -> Result<(), GitRunnerError> {
        run_git(
            repo,
            &[
                "stash",
                "push",
                "-m",
                "onetcli-sync-self-heal",
                "--",
                ".onetcli-sync",
            ],
        )
        .map(|_| ())
    }

    async fn stash_pop(&self, repo: &Path) -> Result<(), GitRunnerError> {
        run_git(repo, &["stash", "pop"]).map(|_| ())
    }

    async fn rebase_abort(&self, repo: &Path) -> Result<(), GitRunnerError> {
        run_git(repo, &["rebase", "--abort"]).map(|_| ())
    }

    async fn reset_sync_package(&self, repo: &Path) -> Result<(), GitRunnerError> {
        run_git(repo, &["checkout", "--", ".onetcli-sync"]).map(|_| ())
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
        match self
            .runner
            .commit_sync_package(&self.root, SYNC_COMMIT_MESSAGE)
            .await
        {
            Err(GitRunnerError::NothingToCommit) => Ok(()),
            result => result,
        }?;
        if self.options.auto_push {
            self.push_with_self_heal().await?;
        }
        Ok(())
    }

    /// 带自愈的 push：被拒（远端有新提交）时先 pull --rebase 再重试一次。
    async fn push_with_self_heal(&self) -> Result<(), SyncStoreError> {
        match self.runner.push(&self.root).await {
            Ok(()) => Ok(()),
            Err(GitRunnerError::MergeConflict) | Err(GitRunnerError::CommandFailed(_)) => {
                tracing::warn!("git push rejected; pulling with rebase before retrying");
                self.pull_with_self_heal().await?;
                self.runner
                    .push(&self.root)
                    .await
                    .map_err(SyncStoreError::from)
            }
            Err(error) => Err(error.into()),
        }
    }

    /// 带自愈的 pull --rebase（issue #457）：
    /// - 脏工作区（上一轮有冲突、flush 被跳过留下未提交改动）会让 pull 直接拒绝；
    ///   先 stash 再 pull，成功后 pop 回来，恢复失败也不阻塞同步。
    /// - rebase 冲突会让仓库卡死在 unmerged 状态；先 abort 再重试一次，
    ///    仍冲突则如实上抛，交给上层挂冲突。
    async fn pull_with_self_heal(&self) -> Result<(), SyncStoreError> {
        match self.runner.pull_rebase(&self.root).await {
            Ok(()) => return Ok(()),
            Err(GitRunnerError::MergeConflict) => {
                // 仓库可能卡在上一轮 rebase 冲突：abort 后重试一次。
                let _ = self.runner.rebase_abort(&self.root).await;
                match self.runner.pull_rebase(&self.root).await {
                    Ok(()) => Ok(()),
                    Err(GitRunnerError::MergeConflict) => {
                        // 重试仍冲突：先退出 rebase 状态避免仓库卡死，再如实上抛。
                        let _ = self.runner.rebase_abort(&self.root).await;
                        Err(SyncStoreError::GitMergeConflict)
                    }
                    Err(error) => Err(error.into()),
                }
            }
            Err(GitRunnerError::CommandFailed(_)) => {
                // 典型场景：上一轮 pass 有冲突提前结束、flush 被跳过，工作区
                // 留着未提交的 .onetcli-sync 改动，git 拒绝 pull --rebase。
                tracing::warn!(
                    "git pull --rebase rejected by dirty worktree; stashing sync package"
                );
                if self.runner.stash_sync_package(&self.root).await.is_err() {
                    // stash 失败通常意味着 unmerged 残留：把同步包目录重置后再试。
                    // 本地数据库仍是数据源头，重置后 worker 会重新上传。
                    let _ = self.runner.reset_sync_package(&self.root).await;
                }
                match self.runner.pull_rebase(&self.root).await {
                    Ok(()) => {
                        if let Err(error) = self.runner.stash_pop(&self.root).await {
                            // stash pop 失败不阻塞同步：改动还在 stash 里，下一轮
                            // flush 会把整个同步包重新提交上去。
                            tracing::warn!(error = ?error, "git stash pop failed after pull");
                        }
                        Ok(())
                    }
                    Err(error) => {
                        // pull 本身失败：尝试把 stash 恢复回去，别把用户的改动弄丢。
                        if let Err(pop_error) = self.runner.stash_pop(&self.root).await {
                            tracing::warn!(error = ?pop_error, "git stash pop failed after failed pull");
                        }
                        Err(error.into())
                    }
                }
            }
            Err(error) => return Err(error.into()),
        }
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
        self.pull_with_self_heal().await?;
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
        data_type: &str,
        id: &str,
        expected_version: Option<u32>,
    ) -> Result<(), SyncStoreError> {
        self.directory
            .tombstone_record(data_type, id, expected_version)
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
    let mut command = Command::new("git");
    command.current_dir(repo).args(args);
    process_util::configure_background_child(&mut command);
    let output = command
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
