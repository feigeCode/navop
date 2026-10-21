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
        // 冲突里存档的 `local_snapshot` 只是「检测到冲突那一刻」的快照：本地条目可能
        // 已经被删掉（用户在界面里删了它），也可能换了一行。所以解决冲突前必须按
        // `(data_type, 云端 record_id)` 现场重解析本地条目 —— 与 planner / worker
        // 判定「本地是否有这条记录」的口径完全一致。
        let local = self.current_local_item(conflict).await?;
        match strategy {
            ConflictResolution::UseCloud => self.use_cloud(conflict, local.as_ref()).await?,
            ConflictResolution::UseLocal => self.use_local(conflict, local.as_ref()).await?,
            ConflictResolution::KeepBoth => {
                return Err(SyncStoreError::Conflict(
                    "personal sync keep-both resolution needs a local copy API".to_string(),
                ));
            }
        }
        self.clear_conflict(conflict)
    }

    /// 按云端记录现场找回本地条目；本地已不存在时返回 `None`。
    async fn current_local_item(
        &self,
        conflict: &PersonalSyncConflict,
    ) -> Result<Option<PersonalSyncItemSnapshot>, SyncStoreError> {
        Ok(self
            .local
            .list_items()
            .await?
            .into_iter()
            .find(|item| is_same_cloud_record(item, conflict)))
    }

    async fn use_cloud(
        &self,
        conflict: &PersonalSyncConflict,
        local: Option<&PersonalSyncItemSnapshot>,
    ) -> Result<(), SyncStoreError> {
        let remote = required_remote_snapshot(conflict)?;
        if remote.deleted_at.is_some() {
            // 云端也已经是墓碑：本地有就跟着删，没有就只剩清冲突。
            if let Some(local) = local {
                self.local.delete_item(local).await?;
            }
            return Ok(());
        }
        // 本地条目已不存在时 `apply_remote` 会走插入（远端胜出），这正是「使用远程版本」
        // 应有的语义，以前因为硬解 `local_snapshot` 里的本地行 id 而直接报错。
        self.local.apply_remote(&remote, local).await?;
        if let Some(local) = local {
            self.local
                .mark_synced(
                    &local.local_id,
                    &remote.id,
                    &remote.checksum,
                    remote.updated_at / 1000,
                )
                .await?;
        }
        Ok(())
    }

    async fn use_local(
        &self,
        conflict: &PersonalSyncConflict,
        local: Option<&PersonalSyncItemSnapshot>,
    ) -> Result<(), SyncStoreError> {
        let remote = required_remote_snapshot(conflict)?;
        let Some(local) = local else {
            // 本地条目已经不在了 ⇒「使用本地版本」= 保留本地的删除意图：把云端记录也
            // 标记为删除。以前这里会直接报 "connection not found"，用户点哪个按钮都出不来。
            return self
                .store
                .tombstone_record(
                    &conflict.data_type,
                    &conflict.record_id,
                    Some(remote.version),
                )
                .await;
        };
        let mut record = self.local.export_item(local).await?;
        record.id = conflict.record_id.clone();
        let stored = self
            .store
            .upsert_record(&record, Some(remote.version))
            .await?;
        self.local
            .mark_synced(
                &local.local_id,
                &stored.id,
                &stored.checksum,
                stored.updated_at / 1000,
            )
            .await
    }

    fn clear_conflict(&self, conflict: &PersonalSyncConflict) -> Result<(), SyncStoreError> {
        self.conflicts
            .delete(
                &conflict.backend_profile_id,
                &conflict.data_type,
                &conflict.record_id,
            )
            .map_err(|error| SyncStoreError::Io(error.to_string()))
    }
}

fn is_same_cloud_record(item: &PersonalSyncItemSnapshot, conflict: &PersonalSyncConflict) -> bool {
    item.data_type == conflict.data_type
        && item.cloud_id.as_deref() == Some(conflict.record_id.as_str())
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
