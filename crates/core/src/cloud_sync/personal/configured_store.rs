use std::path::PathBuf;

use async_trait::async_trait;

use crate::cloud_sync::models::CloudSyncData;
use crate::settings::PersonalSyncBackendKind;

use super::{
    CommandGitRunner, DirectorySyncStore, GitRunner, GitSyncOptions, GitSyncStore,
    PersonalSyncStore, SyncDeviceId, SyncStoreError, SyncStoreLock, SyncStoreStatus,
};

#[derive(Clone)]
pub enum ConfiguredPersonalSyncStore<R = CommandGitRunner> {
    Folder(DirectorySyncStore),
    Git(GitSyncStore<R>),
}

impl ConfiguredPersonalSyncStore<CommandGitRunner> {
    pub fn new_folder(root: PathBuf) -> Self {
        Self::Folder(DirectorySyncStore::new(root))
    }

    pub fn from_runtime_config(
        config: &super::PersonalSyncRuntimeConfig,
    ) -> ConfiguredPersonalSyncStore<CommandGitRunner> {
        Self::from_backend(
            config.backend,
            config.root.clone(),
            CommandGitRunner,
            config.git_auto_push,
        )
    }
}

impl<R> ConfiguredPersonalSyncStore<R>
where
    R: GitRunner,
{
    pub fn from_backend(
        backend: PersonalSyncBackendKind,
        root: PathBuf,
        runner: R,
        git_auto_push: bool,
    ) -> Self {
        match backend {
            PersonalSyncBackendKind::Folder => Self::Folder(DirectorySyncStore::new(root)),
            PersonalSyncBackendKind::Git => Self::new_git(root, runner, git_auto_push),
        }
    }

    pub fn new_git(root: PathBuf, runner: R, auto_push: bool) -> Self {
        Self::Git(GitSyncStore::new(
            root,
            runner,
            GitSyncOptions { auto_push },
        ))
    }

    pub async fn flush(&self) -> Result<(), SyncStoreError> {
        match self {
            Self::Folder(_) => Ok(()),
            Self::Git(store) => store.flush().await,
        }
    }
}

#[async_trait]
impl<R> PersonalSyncStore for ConfiguredPersonalSyncStore<R>
where
    R: GitRunner,
{
    fn backend_id(&self) -> &'static str {
        match self {
            Self::Folder(store) => store.backend_id(),
            Self::Git(store) => store.backend_id(),
        }
    }

    async fn probe(&self) -> Result<SyncStoreStatus, SyncStoreError> {
        match self {
            Self::Folder(store) => store.probe().await,
            Self::Git(store) => store.probe().await,
        }
    }

    async fn list_records(
        &self,
        data_type: Option<&str>,
        since: Option<i64>,
    ) -> Result<Vec<CloudSyncData>, SyncStoreError> {
        match self {
            Self::Folder(store) => store.list_records(data_type, since).await,
            Self::Git(store) => store.list_records(data_type, since).await,
        }
    }

    async fn upsert_record(
        &self,
        record: &CloudSyncData,
        expected_version: Option<u32>,
    ) -> Result<CloudSyncData, SyncStoreError> {
        match self {
            Self::Folder(store) => store.upsert_record(record, expected_version).await,
            Self::Git(store) => store.upsert_record(record, expected_version).await,
        }
    }

    async fn tombstone_record(
        &self,
        id: &str,
        expected_version: Option<u32>,
    ) -> Result<(), SyncStoreError> {
        match self {
            Self::Folder(store) => store.tombstone_record(id, expected_version).await,
            Self::Git(store) => store.tombstone_record(id, expected_version).await,
        }
    }

    async fn acquire_lock(&self, owner: &SyncDeviceId) -> Result<SyncStoreLock, SyncStoreError> {
        match self {
            Self::Folder(store) => store.acquire_lock(owner).await,
            Self::Git(store) => store.acquire_lock(owner).await,
        }
    }
}
