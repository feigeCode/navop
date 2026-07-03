use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;

use crate::cloud_sync::CloudSyncData;
use crate::cloud_sync::models::data_type;
use crate::storage::now;

use super::{
    PersonalSyncConflict, PersonalSyncConflictRepository, PersonalSyncConflictSink,
    PersonalSyncItemSnapshot, PersonalSyncRecordConflict, SyncStoreError,
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
    async fn paused_record_ids(&self) -> Result<HashSet<String>, SyncStoreError> {
        let conflicts = self
            .conflicts
            .list(&self.backend_profile_id)
            .map_err(|error| SyncStoreError::Io(error.to_string()))?;
        Ok(conflicts
            .into_iter()
            .map(|conflict| conflict.record_id)
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
            data_type: conflict_data_type(local, remote),
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

fn conflict_data_type(
    local: Option<&PersonalSyncItemSnapshot>,
    remote: Option<&CloudSyncData>,
) -> String {
    remote
        .map(|record| record.data_type.clone())
        .or_else(|| local.map(|item| item.data_type.clone()))
        .unwrap_or_else(|| data_type::CONNECTION.to_string())
}

fn serialize_snapshot<T: serde::Serialize>(
    snapshot: Option<&T>,
) -> Result<Option<String>, SyncStoreError> {
    snapshot
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| SyncStoreError::Parse(error.to_string()))
}
