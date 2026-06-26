use std::path::Path;

use async_trait::async_trait;

use crate::cloud_sync::models::data_type;
use crate::cloud_sync::personal::test_support::test_record;
use crate::cloud_sync::personal::{
    CommandGitRunner, ConfiguredPersonalSyncStore, GitRunner, GitRunnerError, PersonalSyncStore,
};
use crate::settings::PersonalSyncBackendKind;

#[test]
fn configured_store_builds_folder_store() {
    let temp = tempfile::tempdir().expect("tempdir");

    let store = ConfiguredPersonalSyncStore::new_folder(temp.path().to_path_buf());

    assert_eq!("folder", store.backend_id());
}

#[test]
fn configured_store_builds_git_store() {
    let temp = tempfile::tempdir().expect("tempdir");

    let store =
        ConfiguredPersonalSyncStore::new_git(temp.path().to_path_buf(), CommandGitRunner, true);

    assert_eq!("git", store.backend_id());
}

#[test]
fn configured_store_maps_backend_kind() {
    let temp = tempfile::tempdir().expect("tempdir");

    let folder = ConfiguredPersonalSyncStore::from_backend(
        PersonalSyncBackendKind::Folder,
        temp.path().to_path_buf(),
        CommandGitRunner,
        true,
    );
    let git = ConfiguredPersonalSyncStore::from_backend(
        PersonalSyncBackendKind::Git,
        temp.path().to_path_buf(),
        CommandGitRunner,
        false,
    );

    assert_eq!("folder", folder.backend_id());
    assert_eq!("git", git.backend_id());
}

#[tokio::test]
async fn configured_store_flushes_git_store_after_writes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let runner = FlushRunner::default();
    let store =
        ConfiguredPersonalSyncStore::new_git(temp.path().to_path_buf(), runner.clone(), true);
    store
        .upsert_record(
            &test_record("record-1", data_type::CONNECTION, 1, "checksum"),
            None,
        )
        .await
        .expect("record written");

    store.flush().await.expect("flush");

    assert!(runner.did_check_clean());
}

#[derive(Clone, Default)]
struct FlushRunner {
    checked_clean: std::sync::Arc<std::sync::Mutex<bool>>,
}

impl FlushRunner {
    fn did_check_clean(&self) -> bool {
        *self.checked_clean.lock().expect("checked clean lock")
    }
}

#[async_trait]
impl GitRunner for FlushRunner {
    async fn pull_rebase(&self, _repo: &Path) -> Result<(), GitRunnerError> {
        Ok(())
    }

    async fn add_sync_package(&self, _repo: &Path) -> Result<(), GitRunnerError> {
        Ok(())
    }

    async fn commit_sync_package(
        &self,
        _repo: &Path,
        _message: &str,
    ) -> Result<(), GitRunnerError> {
        Ok(())
    }

    async fn push(&self, _repo: &Path) -> Result<(), GitRunnerError> {
        Ok(())
    }

    async fn remote_url(&self, _repo: &Path) -> Result<Option<String>, GitRunnerError> {
        Ok(None)
    }

    async fn is_repo(&self, _repo: &Path) -> Result<bool, GitRunnerError> {
        Ok(true)
    }

    async fn is_clean_for_sync(&self, _repo: &Path) -> Result<bool, GitRunnerError> {
        *self.checked_clean.lock().expect("checked clean lock") = true;
        Ok(false)
    }
}
