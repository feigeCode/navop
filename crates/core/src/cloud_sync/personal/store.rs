use async_trait::async_trait;

use crate::cloud_sync::models::CloudSyncData;

use super::{SyncDeviceId, SyncStoreError, SyncStoreLock, SyncStoreStatus};

#[async_trait]
pub trait PersonalSyncStore: Send + Sync {
    fn backend_id(&self) -> &'static str;

    async fn probe(&self) -> Result<SyncStoreStatus, SyncStoreError>;

    async fn list_records(
        &self,
        data_type: Option<&str>,
        since: Option<i64>,
    ) -> Result<Vec<CloudSyncData>, SyncStoreError>;

    async fn upsert_record(
        &self,
        record: &CloudSyncData,
        expected_version: Option<u32>,
    ) -> Result<CloudSyncData, SyncStoreError>;

    async fn tombstone_record(
        &self,
        id: &str,
        expected_version: Option<u32>,
    ) -> Result<(), SyncStoreError>;

    async fn acquire_lock(&self, owner: &SyncDeviceId) -> Result<SyncStoreLock, SyncStoreError>;
}
