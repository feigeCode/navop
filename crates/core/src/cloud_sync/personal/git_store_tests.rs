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
async fn git_store_maps_merge_conflict() {
    let temp = tempfile::tempdir().expect("tempdir");
    let runner = FakeGitRunner::new().with_error("pull", GitRunnerError::MergeConflict);
    let store = GitSyncStore::new(
        temp.path().to_path_buf(),
        runner,
        GitSyncOptions { auto_push: true },
    );

    let err = store.probe().await.expect_err("merge failure maps");

    assert_eq!(SyncStoreError::GitMergeConflict, err);
}

#[derive(Clone, Default)]
struct FakeGitRunner {
    commands: Arc<Mutex<Vec<String>>>,
    errors: Arc<Mutex<HashMap<&'static str, GitRunnerError>>>,
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

    fn commands(&self) -> Vec<String> {
        self.commands.lock().expect("commands lock").clone()
    }

    fn maybe_fail(&self, name: &'static str) -> Result<(), GitRunnerError> {
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
}
