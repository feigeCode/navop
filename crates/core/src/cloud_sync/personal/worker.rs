use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::cloud_sync::models::CloudSyncData;

use super::{
    PersonalSyncItemSnapshot, PersonalSyncPlan, PersonalSyncPlanner, PersonalSyncRecordConflict,
    PersonalSyncStore, SyncDeviceId, SyncStoreError,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PersonalSyncEvent {
    FullScan,
    LocalChanged { data_type: String, local_id: String },
    LocalDeleted { data_type: String, cloud_id: String },
    RemoteChanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerConfig {
    pub backend_profile_id: String,
    pub device_id: SyncDeviceId,
}

impl WorkerConfig {
    #[cfg(test)]
    pub fn test() -> Self {
        Self {
            backend_profile_id: "personal-test".to_string(),
            device_id: SyncDeviceId("test-device".to_string()),
        }
    }
}

#[async_trait]
pub trait PersonalSyncLocalSource: Send + Sync {
    async fn list_items(&self) -> Result<Vec<PersonalSyncItemSnapshot>, SyncStoreError>;

    async fn export_item(
        &self,
        item: &PersonalSyncItemSnapshot,
    ) -> Result<CloudSyncData, SyncStoreError>;

    async fn apply_remote(
        &self,
        record: &CloudSyncData,
        local: Option<&PersonalSyncItemSnapshot>,
    ) -> Result<(), SyncStoreError>;

    async fn mark_synced(
        &self,
        local_id: &str,
        cloud_id: &str,
        synced_at: i64,
    ) -> Result<(), SyncStoreError>;

    async fn delete_item(&self, item: &PersonalSyncItemSnapshot) -> Result<(), SyncStoreError>;
}

#[async_trait]
pub trait PersonalSyncConflictSink: Send + Sync {
    async fn paused_record_ids(&self) -> Result<HashSet<String>, SyncStoreError> {
        Ok(HashSet::new())
    }

    async fn pause_record(
        &self,
        conflict: &PersonalSyncRecordConflict,
        local: Option<&PersonalSyncItemSnapshot>,
        remote: Option<&CloudSyncData>,
    ) -> Result<(), SyncStoreError>;
}

#[derive(Debug, Clone, Default)]
pub struct NoopConflictSink;

#[async_trait]
impl PersonalSyncConflictSink for NoopConflictSink {
    async fn pause_record(
        &self,
        _conflict: &PersonalSyncRecordConflict,
        _local: Option<&PersonalSyncItemSnapshot>,
        _remote: Option<&CloudSyncData>,
    ) -> Result<(), SyncStoreError> {
        Ok(())
    }
}

#[derive(Debug, Default)]
struct WorkerState {
    pending: HashSet<PersonalSyncEvent>,
    active: bool,
    dirty: bool,
}

#[derive(Clone)]
pub struct PersonalSyncWorker<S, L, C = NoopConflictSink> {
    store: S,
    local: L,
    conflicts: C,
    config: WorkerConfig,
    planner: PersonalSyncPlanner,
    state: Arc<Mutex<WorkerState>>,
}

impl<S, L> PersonalSyncWorker<S, L, NoopConflictSink>
where
    S: PersonalSyncStore + Clone + Send + Sync,
    L: PersonalSyncLocalSource + Clone + Send + Sync,
{
    pub fn new(store: S, local: L, config: WorkerConfig) -> Self {
        Self::with_conflict_sink(store, local, NoopConflictSink, config)
    }
}

impl<S, L, C> PersonalSyncWorker<S, L, C>
where
    S: PersonalSyncStore + Clone + Send + Sync,
    L: PersonalSyncLocalSource + Clone + Send + Sync,
    C: PersonalSyncConflictSink + Clone + Send + Sync,
{
    pub fn with_conflict_sink(store: S, local: L, conflicts: C, config: WorkerConfig) -> Self {
        Self {
            store,
            local,
            conflicts,
            config,
            planner: PersonalSyncPlanner::new(),
            state: Arc::new(Mutex::new(WorkerState::default())),
        }
    }

    pub fn enqueue(&self, event: PersonalSyncEvent) {
        let mut state = self.state.lock().expect("personal sync worker state");
        if state.active {
            state.dirty = true;
        }
        state.pending.insert(event);
    }

    pub async fn drain_once(&self) -> Result<(), SyncStoreError> {
        let Some(events) = self.begin_drain() else {
            return Ok(());
        };

        let result = self.run_pass(events).await;
        self.finish_drain();
        result
    }

    fn begin_drain(&self) -> Option<HashSet<PersonalSyncEvent>> {
        let mut state = self.state.lock().expect("personal sync worker state");
        if state.active || state.pending.is_empty() {
            return None;
        }

        let events = std::mem::take(&mut state.pending);
        state.active = true;
        Some(events)
    }

    fn finish_drain(&self) {
        let mut state = self.state.lock().expect("personal sync worker state");
        state.active = false;
        if state.dirty {
            state.pending.insert(PersonalSyncEvent::FullScan);
            state.dirty = false;
        }
    }

    async fn run_pass(&self, events: HashSet<PersonalSyncEvent>) -> Result<(), SyncStoreError> {
        self.store.probe().await?;
        let _lock = self.store.acquire_lock(&self.config.device_id).await?;
        let local_items = self.local.list_items().await?;
        let remote_records = self.store.list_records(None, None).await?;
        self.apply_local_delete_events(&events, &remote_records)
            .await?;
        self.apply_remote_tombstones(&local_items, &remote_records)
            .await?;
        let deleted = local_deleted_cloud_ids(&events);
        let active_remote_records = remote_records
            .into_iter()
            .filter(|record| record.deleted_at.is_none() && !deleted.contains(&record.id))
            .collect::<Vec<_>>();
        let paused = self.conflicts.paused_record_ids().await?;
        let plan = self
            .planner
            .plan(&local_items, &active_remote_records, &paused);

        self.apply_plan(&plan, &local_items, &active_remote_records)
            .await
    }

    async fn apply_local_delete_events(
        &self,
        events: &HashSet<PersonalSyncEvent>,
        records: &[CloudSyncData],
    ) -> Result<(), SyncStoreError> {
        for cloud_id in local_deleted_cloud_ids(events) {
            let Some(record) = find_remote_by_id(records, &cloud_id) else {
                continue;
            };
            if record.deleted_at.is_none() {
                self.store
                    .tombstone_record(&cloud_id, Some(record.version))
                    .await?;
            }
        }
        Ok(())
    }

    async fn apply_remote_tombstones(
        &self,
        items: &[PersonalSyncItemSnapshot],
        records: &[CloudSyncData],
    ) -> Result<(), SyncStoreError> {
        for record in records.iter().filter(|record| record.deleted_at.is_some()) {
            if let Some(item) = find_local_by_cloud_id(items, &record.id) {
                self.local.delete_item(item).await?;
            }
        }
        Ok(())
    }

    async fn apply_plan(
        &self,
        plan: &PersonalSyncPlan,
        local_items: &[PersonalSyncItemSnapshot],
        remote_records: &[CloudSyncData],
    ) -> Result<(), SyncStoreError> {
        self.apply_uploads(plan).await?;
        self.apply_cloud_updates(plan).await?;
        self.apply_local_updates(plan).await?;
        self.apply_downloads(plan).await?;
        self.apply_synced_marks(plan, local_items).await?;
        self.apply_conflicts(plan, local_items, remote_records)
            .await
    }

    async fn apply_uploads(&self, plan: &PersonalSyncPlan) -> Result<(), SyncStoreError> {
        for item in &plan.to_upload {
            let record = self.local.export_item(item).await?;
            let stored = self.store.upsert_record(&record, None).await?;
            self.mark_synced(item, &stored).await?;
        }
        Ok(())
    }

    async fn apply_cloud_updates(&self, plan: &PersonalSyncPlan) -> Result<(), SyncStoreError> {
        for (item, remote) in &plan.to_update_cloud {
            let record = self.local.export_item(item).await?;
            let stored = self
                .store
                .upsert_record(&record, Some(remote.version))
                .await?;
            self.mark_synced(item, &stored).await?;
        }
        Ok(())
    }

    async fn apply_local_updates(&self, plan: &PersonalSyncPlan) -> Result<(), SyncStoreError> {
        for (record, item) in &plan.to_update_local {
            self.local.apply_remote(record, Some(item)).await?;
            self.mark_synced(item, record).await?;
        }
        Ok(())
    }

    async fn apply_downloads(&self, plan: &PersonalSyncPlan) -> Result<(), SyncStoreError> {
        for record in &plan.to_download {
            self.local.apply_remote(record, None).await?;
        }
        Ok(())
    }

    async fn apply_synced_marks(
        &self,
        plan: &PersonalSyncPlan,
        items: &[PersonalSyncItemSnapshot],
    ) -> Result<(), SyncStoreError> {
        for cloud_id in &plan.to_mark_synced {
            if let Some(item) = find_local_by_cloud_id(items, cloud_id) {
                self.local
                    .mark_synced(&item.local_id, cloud_id, item.updated_at)
                    .await?;
            }
        }
        Ok(())
    }

    async fn apply_conflicts(
        &self,
        plan: &PersonalSyncPlan,
        items: &[PersonalSyncItemSnapshot],
        records: &[CloudSyncData],
    ) -> Result<(), SyncStoreError> {
        for conflict in &plan.conflicts {
            let local = find_local_by_id(items, &conflict.local_id);
            let remote = find_remote_by_id(records, &conflict.cloud_id);
            self.conflicts.pause_record(conflict, local, remote).await?;
        }
        Ok(())
    }

    async fn mark_synced(
        &self,
        item: &PersonalSyncItemSnapshot,
        record: &CloudSyncData,
    ) -> Result<(), SyncStoreError> {
        self.local
            .mark_synced(&item.local_id, &record.id, record.updated_at / 1000)
            .await
    }
}

fn find_local_by_cloud_id<'a>(
    items: &'a [PersonalSyncItemSnapshot],
    cloud_id: &str,
) -> Option<&'a PersonalSyncItemSnapshot> {
    items
        .iter()
        .find(|item| item.cloud_id.as_deref() == Some(cloud_id))
}

fn find_local_by_id<'a>(
    items: &'a [PersonalSyncItemSnapshot],
    local_id: &str,
) -> Option<&'a PersonalSyncItemSnapshot> {
    items.iter().find(|item| item.local_id == local_id)
}

fn find_remote_by_id<'a>(records: &'a [CloudSyncData], id: &str) -> Option<&'a CloudSyncData> {
    records.iter().find(|record| record.id == id)
}

fn local_deleted_cloud_ids(events: &HashSet<PersonalSyncEvent>) -> HashSet<String> {
    events
        .iter()
        .filter_map(|event| match event {
            PersonalSyncEvent::LocalDeleted { cloud_id, .. } => Some(cloud_id.clone()),
            _ => None,
        })
        .collect()
}
