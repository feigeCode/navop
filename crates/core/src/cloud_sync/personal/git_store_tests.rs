use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::cloud_sync::models::data_type;
use crate::cloud_sync::personal::test_support::test_record;
use crate::cloud_sync::personal::{
    GitRunner, GitRunnerError, GitSyncOptions, GitSyncStore, PersonalSyncStore, SyncStoreError,
};

#[tokio::test]
async fn git_store_pulls_before_probe_and_pushes_after_write() {
    let temp = tempfile::tempdir().expect("tempdir");
    let runner = FakeGitRunner::new()
        .with_success("pull")
        .with_success("add")
        .with_success("commit")
        .with_success("push");
    let store = GitSyncStore::new(
        temp.path().to_path_buf(),
        runner.clone(),
        GitSyncOptions { auto_push: true },
    );
    let record = test_record("connection-1", data_type::CONNECTION, 1, "checksum-1");

    store.probe().await.expect("probe succeeds");
    store
        .upsert_record(&record, None)
        .await
        .expect("upsert succeeds");
    store.flush().await.expect("flush succeeds");

    assert_eq!(
        vec![
            "pull --rebase",
            "add .onetcli-sync",
            "commit onetcli sync: update personal records",
            "push"
        ],
        runner.commands()
    );
}

#[tokio::test]
async fn git_store_maps_auth_failure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let runner = FakeGitRunner::new().with_error("pull", GitRunnerError::AuthRequired);
    let store = GitSyncStore::new(
        temp.path().to_path_buf(),
        runner,
        GitSyncOptions { auto_push: true },
    );

    let err = store.probe().await.expect_err("auth failure maps");

    assert_eq!(SyncStoreError::GitAuthRequired, err);
}

#[tokio::test]
async fn git_store_maps_merge_conflict_after_self_heal() {
    let temp = tempfile::tempdir().expect("tempdir");
    // pull 连续两次冲突：自愈路径 abort 后重试，仍冲突则如实上抛
    //（但仓库已退出 rebase 中间态，不会卡死）。
    let runner = FakeGitRunner::new().with_error("pull", GitRunnerError::MergeConflict);
    let store = GitSyncStore::new(
        temp.path().to_path_buf(),
        runner.clone(),
        GitSyncOptions { auto_push: true },
    );

    let err = store.probe().await.expect_err("merge failure maps");

    assert_eq!(SyncStoreError::GitMergeConflict, err);
    // 自愈路径必须先 abort 再重试，最后再 abort 退出中间态。
    assert_eq!(
        vec![
            "pull --rebase",
            "rebase --abort",
            "pull --rebase",
            "rebase --abort"
        ],
        runner.commands()
    );
}

#[tokio::test]
async fn git_store_recovers_from_stuck_rebase_conflict() {
    let temp = tempfile::tempdir().expect("tempdir");
    // 第一次 pull 撞上上一轮残留的 rebase 冲突，abort 后重试成功。
    let runner = FakeGitRunner::new().with_error_once("pull", GitRunnerError::MergeConflict);
    let store = GitSyncStore::new(
        temp.path().to_path_buf(),
        runner.clone(),
        GitSyncOptions { auto_push: true },
    );

    store.probe().await.expect("probe succeeds after self-heal");

    assert_eq!(
        vec!["pull --rebase", "rebase --abort", "pull --rebase"],
        runner.commands()
    );
}

#[tokio::test]
async fn git_store_recovers_from_dirty_worktree_rejection() {
    let temp = tempfile::tempdir().expect("tempdir");
    // 上一轮 flush 被跳过留下脏工作区：pull 被拒（CommandFailed），先 stash
    // 再 pull，成功后 pop 恢复。
    let runner = FakeGitRunner::new().with_error_once(
        "pull",
        GitRunnerError::CommandFailed(
            "cannot pull with rebase: Your index contains uncommitted changes.".to_string(),
        ),
    );
    let store = GitSyncStore::new(
        temp.path().to_path_buf(),
        runner.clone(),
        GitSyncOptions { auto_push: true },
    );

    store
        .probe()
        .await
        .expect("probe succeeds after stash self-heal");

    assert_eq!(
        vec!["pull --rebase", "stash push", "pull --rebase", "stash pop"],
        runner.commands()
    );
}

#[tokio::test]
async fn git_store_retries_push_after_rejection() {
    let temp = tempfile::tempdir().expect("tempdir");
    // push 被拒（远端有新提交）：先 pull --rebase 再重试一次成功。
    let runner = FakeGitRunner::new()
        .with_success("pull")
        .with_success("add")
        .with_success("commit")
        .with_error_once(
            "push",
            GitRunnerError::CommandFailed("non-fast-forward".to_string()),
        );
    let store = GitSyncStore::new(
        temp.path().to_path_buf(),
        runner.clone(),
        GitSyncOptions { auto_push: true },
    );
    let record = test_record("connection-1", data_type::CONNECTION, 1, "checksum-1");

    store.probe().await.expect("probe succeeds");
    store
        .upsert_record(&record, None)
        .await
        .expect("upsert succeeds");
    store.flush().await.expect("flush succeeds");

    assert_eq!(
        vec![
            "pull --rebase",
            "add .onetcli-sync",
            "commit onetcli sync: update personal records",
            "push",
            "pull --rebase",
            "push"
        ],
        runner.commands()
    );
}

#[test]
fn git_commands_run_as_hidden_background_children() {
    let source = include_str!("git_store.rs");
    let run_git = source
        .split("fn run_git(")
        .nth(1)
        .expect("run_git body must exist");

    assert!(
        run_git.contains("process_util::configure_background_child(&mut command)"),
        "run_git must hide the console window on Windows (issue #145)"
    );
    assert!(
        run_git.find("configure_background_child") < run_git.find(".output()"),
        "background-child console hiding must be applied before git is spawned"
    );
}

#[derive(Clone, Default)]
struct FakeGitRunner {
    commands: Arc<Mutex<Vec<String>>>,
    errors: Arc<Mutex<HashMap<&'static str, GitRunnerError>>>,
    one_shot_errors: Arc<Mutex<HashMap<&'static str, GitRunnerError>>>,
}

impl FakeGitRunner {
    fn new() -> Self {
        Self::default()
    }

    fn with_success(self, _name: &'static str) -> Self {
        self
    }

    fn with_error(self, name: &'static str, error: GitRunnerError) -> Self {
        self.errors.lock().expect("errors lock").insert(name, error);
        self
    }

    /// 只在第一次调用时报错，之后恢复成功（模拟自愈后重试成功）。
    fn with_error_once(self, name: &'static str, error: GitRunnerError) -> Self {
        self.one_shot_errors
            .lock()
            .expect("one-shot errors lock")
            .insert(name, error);
        self
    }

    fn commands(&self) -> Vec<String> {
        self.commands.lock().expect("commands lock").clone()
    }

    fn maybe_fail(&self, name: &'static str) -> Result<(), GitRunnerError> {
        if let Some(error) = self
            .one_shot_errors
            .lock()
            .expect("one-shot errors lock")
            .remove(name)
        {
            return Err(error);
        }
        match self.errors.lock().expect("errors lock").get(name) {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }
}

#[async_trait]
impl GitRunner for FakeGitRunner {
    async fn pull_rebase(&self, _repo: &Path) -> Result<(), GitRunnerError> {
        self.commands
            .lock()
            .expect("commands lock")
            .push("pull --rebase".to_string());
        self.maybe_fail("pull")
    }

    async fn add_sync_package(&self, _repo: &Path) -> Result<(), GitRunnerError> {
        self.commands
            .lock()
            .expect("commands lock")
            .push("add .onetcli-sync".to_string());
        self.maybe_fail("add")
    }

    async fn commit_sync_package(&self, _repo: &Path, message: &str) -> Result<(), GitRunnerError> {
        self.commands
            .lock()
            .expect("commands lock")
            .push(format!("commit {message}"));
        self.maybe_fail("commit")
    }

    async fn push(&self, _repo: &Path) -> Result<(), GitRunnerError> {
        self.commands
            .lock()
            .expect("commands lock")
            .push("push".to_string());
        self.maybe_fail("push")
    }

    async fn is_repo(&self, _repo: &Path) -> Result<bool, GitRunnerError> {
        Ok(true)
    }

    async fn is_clean_for_sync(&self, _repo: &Path) -> Result<bool, GitRunnerError> {
        Ok(false)
    }

    async fn remote_url(&self, _repo: &Path) -> Result<Option<String>, GitRunnerError> {
        Ok(Some("git@example.test:sync.git".to_string()))
    }

    async fn stash_sync_package(&self, _repo: &Path) -> Result<(), GitRunnerError> {
        self.commands
            .lock()
            .expect("commands lock")
            .push("stash push".to_string());
        self.maybe_fail("stash push")
    }

    async fn stash_pop(&self, _repo: &Path) -> Result<(), GitRunnerError> {
        self.commands
            .lock()
            .expect("commands lock")
            .push("stash pop".to_string());
        self.maybe_fail("stash pop")
    }

    async fn rebase_abort(&self, _repo: &Path) -> Result<(), GitRunnerError> {
        self.commands
            .lock()
            .expect("commands lock")
            .push("rebase --abort".to_string());
        self.maybe_fail("rebase abort")
    }

    async fn reset_sync_package(&self, _repo: &Path) -> Result<(), GitRunnerError> {
        self.commands
            .lock()
            .expect("commands lock")
            .push("checkout -- .onetcli-sync".to_string());
        self.maybe_fail("checkout")
    }
}
