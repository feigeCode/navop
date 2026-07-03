use crate::cloud_sync::models::{CloudSyncData, ConflictResolution};

use super::{
    PersonalSyncConflict, PersonalSyncConflictRepository, PersonalSyncItemSnapshot,
    PersonalSyncLocalSource, PersonalSyncStore, SyncStoreError,
};

#[derive(Clone)]
pub struct PersonalSyncConflictResolver<S, L> {
    store: S,
    local: L,
    conflicts: PersonalSyncConflictRepository,
}

impl<S, L> PersonalSyncConflictResolver<S, L>
where
    S: PersonalSyncStore + Clone + Send + Sync,
    L: PersonalSyncLocalSource + Clone + Send + Sync,
{
    pub fn new(store: S, local: L, conflicts: PersonalSyncConflictRepository) -> Self {
        Self {
            store,
            local,
            conflicts,
        }
    }

    pub async fn resolve(
        &self,
        conflict: &PersonalSyncConflict,
        strategy: ConflictResolution,
    ) -> Result<(), SyncStoreError> {
        match strategy {
            ConflictResolution::UseCloud => self.use_cloud(conflict).await?,
            ConflictResolution::UseLocal => self.use_local(conflict).await?,
            ConflictResolution::KeepBoth => {
                return Err(SyncStoreError::Conflict(
                    "personal sync keep-both resolution needs a local copy API".to_string(),
                ));
            }
        }
        self.clear_conflict(conflict)
    }

    async fn use_cloud(&self, conflict: &PersonalSyncConflict) -> Result<(), SyncStoreError> {
        let remote = required_remote_snapshot(conflict)?;
        let local = optional_local_snapshot(conflict)?;
        if remote.deleted_at.is_some() {
            if let Some(local) = local.as_ref() {
                self.local.delete_item(local).await?;
            }
            return Ok(());
        }
        self.local.apply_remote(&remote, local.as_ref()).await?;
        if let Some(local) = local.as_ref() {
            self.local
                .mark_synced(&local.local_id, &remote.id, remote.updated_at / 1000)
                .await?;
        }
        Ok(())
    }

    async fn use_local(&self, conflict: &PersonalSyncConflict) -> Result<(), SyncStoreError> {
        let local = required_local_snapshot(conflict)?;
        let remote = required_remote_snapshot(conflict)?;
        let mut record = self.local.export_item(&local).await?;
        record.id = conflict.record_id.clone();
        let stored = self
            .store
            .upsert_record(&record, Some(remote.version))
            .await?;
        self.local
            .mark_synced(&local.local_id, &stored.id, stored.updated_at / 1000)
            .await
    }

    fn clear_conflict(&self, conflict: &PersonalSyncConflict) -> Result<(), SyncStoreError> {
        self.conflicts
            .delete(&conflict.backend_profile_id, &conflict.record_id)
            .map_err(|error| SyncStoreError::Io(error.to_string()))
    }
}

fn required_local_snapshot(
    conflict: &PersonalSyncConflict,
) -> Result<PersonalSyncItemSnapshot, SyncStoreError> {
    parse_snapshot(conflict.local_snapshot.as_deref(), "local")
}

fn optional_local_snapshot(
    conflict: &PersonalSyncConflict,
) -> Result<Option<PersonalSyncItemSnapshot>, SyncStoreError> {
    conflict
        .local_snapshot
        .as_deref()
        .map(|snapshot| parse_snapshot(Some(snapshot), "local"))
        .transpose()
}

fn required_remote_snapshot(
    conflict: &PersonalSyncConflict,
) -> Result<CloudSyncData, SyncStoreError> {
    parse_snapshot(conflict.remote_snapshot.as_deref(), "remote")
}

fn parse_snapshot<T: serde::de::DeserializeOwned>(
    snapshot: Option<&str>,
    label: &str,
) -> Result<T, SyncStoreError> {
    let snapshot = snapshot
        .ok_or_else(|| SyncStoreError::Parse(format!("missing {label} conflict snapshot")))?;
    serde_json::from_str(snapshot).map_err(|error| SyncStoreError::Parse(error.to_string()))
}
