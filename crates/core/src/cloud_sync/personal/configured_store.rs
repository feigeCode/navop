use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use gpui::http_client::HttpClient;

use crate::cloud_sync::models::CloudSyncData;
use crate::settings::PersonalSyncBackendKind;

use super::{
    CommandGitRunner, DirectorySyncStore, GitRunner, GitSyncOptions, GitSyncStore,
    PersonalSyncStore, PersonalWebDavRuntimeSettings, SyncDeviceId, SyncStoreError, SyncStoreLock,
    SyncStoreStatus, WebDavCredentials, WebDavSyncStore,
};

#[derive(Clone)]
pub enum ConfiguredPersonalSyncStore<R = CommandGitRunner> {
    Folder(DirectorySyncStore),
    Git(GitSyncStore<R>),
    Webdav(WebDavSyncStore),
}

impl ConfiguredPersonalSyncStore<CommandGitRunner> {
    pub fn new_folder(root: PathBuf) -> Self {
        Self::Folder(DirectorySyncStore::new(root))
    }

    pub fn from_runtime_config(
        config: &super::PersonalSyncRuntimeConfig,
        http: Arc<dyn HttpClient>,
    ) -> Result<ConfiguredPersonalSyncStore<CommandGitRunner>, SyncStoreError> {
        Self::from_backend(
            config.backend,
            config.root.clone(),
            CommandGitRunner,
            config.git_auto_push,
            config.webdav.clone(),
            http,
        )
    }
}

impl<R> ConfiguredPersonalSyncStore<R>
where
    R: GitRunner,
{
    #[allow(clippy::too_many_arguments)]
    pub fn from_backend(
        backend: PersonalSyncBackendKind,
        root: PathBuf,
        runner: R,
        git_auto_push: bool,
        webdav: Option<PersonalWebDavRuntimeSettings>,
        http: Arc<dyn HttpClient>,
    ) -> Result<Self, SyncStoreError> {
        match backend {
            PersonalSyncBackendKind::Folder => Ok(Self::Folder(DirectorySyncStore::new(root))),
            PersonalSyncBackendKind::Git => {
                Ok(Self::new_git(root, runner, git_auto_push))
            }
            PersonalSyncBackendKind::Webdav => {
                let Some(settings) = webdav else {
                    return Err(SyncStoreError::NotConfigured);
                };
                Ok(Self::Webdav(WebDavSyncStore::new(
                    http,
                    WebDavCredentials {
                        url: settings.url().to_string(),
                        username: settings.username().to_string(),
                        password: settings.password().to_string(),
                    },
                )?))
            }
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
            Self::Webdav(_) => Ok(()),
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
            Self::Webdav(store) => store.backend_id(),
        }
    }

    async fn probe(&self) -> Result<SyncStoreStatus, SyncStoreError> {
        match self {
            Self::Folder(store) => store.probe().await,
            Self::Git(store) => store.probe().await,
            Self::Webdav(store) => store.probe().await,
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
            Self::Webdav(store) => store.list_records(data_type, since).await,
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
            Self::Webdav(store) => store.upsert_record(record, expected_version).await,
        }
    }

    async fn tombstone_record(
        &self,
        data_type: &str,
        id: &str,
        expected_version: Option<u32>,
    ) -> Result<(), SyncStoreError> {
        match self {
            Self::Folder(store) => {
                store
                    .tombstone_record(data_type, id, expected_version)
                    .await
            }
            Self::Git(store) => {
                store
                    .tombstone_record(data_type, id, expected_version)
                    .await
            }
            Self::Webdav(store) => {
                store
                    .tombstone_record(data_type, id, expected_version)
                    .await
            }
        }
    }

    async fn acquire_lock(&self, owner: &SyncDeviceId) -> Result<SyncStoreLock, SyncStoreError> {
        match self {
            Self::Folder(store) => store.acquire_lock(owner).await,
            Self::Git(store) => store.acquire_lock(owner).await,
            Self::Webdav(store) => store.acquire_lock(owner).await,
        }
    }
}
