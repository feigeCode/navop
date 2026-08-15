use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;

use crate::cloud_sync::CloudSyncData;
use crate::storage::now;

use super::{
    PersonalSyncCloudKey, PersonalSyncConflict, PersonalSyncConflictRepository,
    PersonalSyncConflictSink, PersonalSyncItemSnapshot, PersonalSyncRecordConflict, SyncStoreError,
};

#[derive(Clone)]
pub struct SqlitePersonalSyncConflictSink {
    backend_profile_id: String,
    conflicts: Arc<PersonalSyncConflictRepository>,
}

impl SqlitePersonalSyncConflictSink {
    pub fn new(backend_profile_id: String, conflicts: Arc<PersonalSyncConflictRepository>) -> Self {
        Self {
            backend_profile_id,
            conflicts,
        }
    }
}

#[async_trait]
impl PersonalSyncConflictSink for SqlitePersonalSyncConflictSink {
    async fn paused_record_keys(&self) -> Result<HashSet<PersonalSyncCloudKey>, SyncStoreError> {
        let conflicts = self
            .conflicts
            .list(&self.backend_profile_id)
            .map_err(|error| SyncStoreError::Io(error.to_string()))?;
        Ok(conflicts
            .into_iter()
            .map(|conflict| PersonalSyncCloudKey {
                data_type: conflict.data_type,
                cloud_id: conflict.record_id,
            })
            .collect())
    }

    async fn pause_record(
        &self,
        conflict: &PersonalSyncRecordConflict,
        local: Option<&PersonalSyncItemSnapshot>,
        remote: Option<&CloudSyncData>,
    ) -> Result<(), SyncStoreError> {
        let stored = PersonalSyncConflict {
            backend_profile_id: self.backend_profile_id.clone(),
            record_id: conflict.cloud_id.clone(),
            data_type: conflict.data_type.clone(),
            conflict_type: conflict.conflict_type,
            local_snapshot: serialize_snapshot(local)?,
            remote_snapshot: serialize_snapshot(remote)?,
            detected_at: now(),
        };
        self.conflicts
            .upsert(&stored)
            .map_err(|error| SyncStoreError::Io(error.to_string()))
    }
}

fn serialize_snapshot<T: serde::Serialize>(
    snapshot: Option<&T>,
) -> Result<Option<String>, SyncStoreError> {
    snapshot
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| SyncStoreError::Parse(error.to_string()))
}
