use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::cloud_sync::models::{CloudSyncData, data_type};
use crate::cloud_sync::personal::test_support::test_record;
use crate::cloud_sync::personal::{
    PersonalSyncConflictSink, PersonalSyncEvent, PersonalSyncItemSnapshot, PersonalSyncLocalSource,
    PersonalSyncRecordConflict, PersonalSyncStore, PersonalSyncWorker, SyncDeviceId,
    SyncStoreError, SyncStoreLock, SyncStoreStatus, WorkerConfig,
};

#[tokio::test]
async fn worker_coalesces_events_and_runs_single_sync_pass() {
    let store = FakePersonalSyncStore::default();
    let local = FakePersonalSyncLocalSource::default();
    let worker = PersonalSyncWorker::new(store.clone(), local.clone(), WorkerConfig::test());

    worker.enqueue(PersonalSyncEvent::LocalChanged {
        data_type: data_type::CONNECTION.to_string(),
        local_id: "1".to_string(),
    });
    worker.enqueue(PersonalSyncEvent::LocalChanged {
        data_type: data_type::CONNECTION.to_string(),
        local_id: "1".to_string(),
    });
    worker.drain_once().await.expect("drain succeeds");

    assert_eq!(1, store.list_calls());
}

#[tokio::test]
async fn worker_pauses_conflicting_record() {
    let store = FakePersonalSyncStore::with_records(vec![remote_record_conflicting()]);
    let local = FakePersonalSyncLocalSource::with_items(vec![local_record_conflicting()]);
    let conflicts = FakeConflictSink::default();
    let worker = PersonalSyncWorker::with_conflict_sink(
        store,
        local,
        conflicts.clone(),
        WorkerConfig::test(),
    );

    worker.enqueue(PersonalSyncEvent::FullScan);
    let error = worker
        .drain_once()
        .await
        .expect_err("conflict pauses sync pass");

    assert!(matches!(error, SyncStoreError::Conflict(_)));
    assert_eq!(vec!["cloud-1"], conflicts.paused_record_ids());
}

#[tokio::test]
async fn worker_tombstones_record_for_local_delete_event() {
    let remote = test_record("cloud-1", data_type::CONNECTION, 4, "remote");
    let store = FakePersonalSyncStore::with_records(vec![remote]);
    let local = FakePersonalSyncLocalSource::default();
    let worker = PersonalSyncWorker::new(store.clone(), local, WorkerConfig::test());

    worker.enqueue(PersonalSyncEvent::LocalDeleted {
        data_type: data_type::CONNECTION.to_string(),
        cloud_id: "cloud-1".to_string(),
    });
    worker.drain_once().await.expect("drain succeeds");

    assert_eq!(vec!["cloud-1"], store.tombstoned_ids());
}

#[tokio::test]
async fn worker_deletes_local_item_for_remote_tombstone() {
    let mut remote = test_record("cloud-1", data_type::CONNECTION, 4, "remote");
    remote.deleted_at = Some(400_000);
    let store = FakePersonalSyncStore::with_records(vec![remote]);
    let local = FakePersonalSyncLocalSource::with_items(vec![local_record_synced()]);
    let worker = PersonalSyncWorker::new(store, local.clone(), WorkerConfig::test());

    worker.enqueue(PersonalSyncEvent::RemoteChanged);
    worker.drain_once().await.expect("drain succeeds");

    assert_eq!(vec!["local-1"], local.deleted_local_ids());
}

#[derive(Clone, Default)]
struct FakePersonalSyncStore {
    records: Arc<Mutex<Vec<CloudSyncData>>>,
    list_calls: Arc<Mutex<usize>>,
    tombstoned: Arc<Mutex<Vec<String>>>,
}

impl FakePersonalSyncStore {
    fn with_records(records: Vec<CloudSyncData>) -> Self {
        Self {
            records: Arc::new(Mutex::new(records)),
            list_calls: Arc::new(Mutex::new(0)),
            tombstoned: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn list_calls(&self) -> usize {
        *self.list_calls.lock().expect("list_calls lock")
    }

    fn tombstoned_ids(&self) -> Vec<String> {
        self.tombstoned.lock().expect("tombstoned lock").clone()
    }
}

#[async_trait]
impl PersonalSyncStore for FakePersonalSyncStore {
    fn backend_id(&self) -> &'static str {
        "fake"
    }

    async fn probe(&self) -> Result<SyncStoreStatus, SyncStoreError> {
        Ok(SyncStoreStatus::ready())
    }

    async fn list_records(
        &self,
        _data_type: Option<&str>,
        _since: Option<i64>,
    ) -> Result<Vec<CloudSyncData>, SyncStoreError> {
        *self.list_calls.lock().expect("list_calls lock") += 1;
        Ok(self.records.lock().expect("records lock").clone())
    }

    async fn upsert_record(
        &self,
        record: &CloudSyncData,
        _expected_version: Option<u32>,
    ) -> Result<CloudSyncData, SyncStoreError> {
        self.records
            .lock()
            .expect("records lock")
            .push(record.clone());
        Ok(record.clone())
    }

    async fn tombstone_record(
        &self,
        id: &str,
        _expected_version: Option<u32>,
    ) -> Result<(), SyncStoreError> {
        self.tombstoned
            .lock()
            .expect("tombstoned lock")
            .push(id.to_string());
        Ok(())
    }

    async fn acquire_lock(&self, owner: &SyncDeviceId) -> Result<SyncStoreLock, SyncStoreError> {
        Ok(SyncStoreLock {
            owner: owner.clone(),
        })
    }
}

#[derive(Clone, Default)]
struct FakePersonalSyncLocalSource {
    items: Arc<Mutex<Vec<PersonalSyncItemSnapshot>>>,
    deleted: Arc<Mutex<Vec<String>>>,
}

impl FakePersonalSyncLocalSource {
    fn with_items(items: Vec<PersonalSyncItemSnapshot>) -> Self {
        Self {
            items: Arc::new(Mutex::new(items)),
            deleted: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn deleted_local_ids(&self) -> Vec<String> {
        self.deleted.lock().expect("deleted lock").clone()
    }
}

#[async_trait]
impl PersonalSyncLocalSource for FakePersonalSyncLocalSource {
    async fn list_items(&self) -> Result<Vec<PersonalSyncItemSnapshot>, SyncStoreError> {
        Ok(self.items.lock().expect("items lock").clone())
    }

    async fn export_item(
        &self,
        item: &PersonalSyncItemSnapshot,
    ) -> Result<CloudSyncData, SyncStoreError> {
        Ok(test_record(
            item.cloud_id.as_deref().unwrap_or(item.local_id.as_str()),
            item.data_type.as_str(),
            1,
            item.checksum.as_str(),
        ))
    }

    async fn apply_remote(
        &self,
        _record: &CloudSyncData,
        _local: Option<&PersonalSyncItemSnapshot>,
    ) -> Result<(), SyncStoreError> {
        Ok(())
    }

    async fn mark_synced(
        &self,
        _local_id: &str,
        _cloud_id: &str,
        _synced_at: i64,
    ) -> Result<(), SyncStoreError> {
        Ok(())
    }

    async fn delete_item(&self, item: &PersonalSyncItemSnapshot) -> Result<(), SyncStoreError> {
        self.deleted
            .lock()
            .expect("deleted lock")
            .push(item.local_id.clone());
        Ok(())
    }
}

#[derive(Clone, Default)]
struct FakeConflictSink {
    paused: Arc<Mutex<Vec<String>>>,
}

impl FakeConflictSink {
    fn paused_record_ids(&self) -> Vec<String> {
        self.paused.lock().expect("paused lock").clone()
    }
}

#[async_trait]
impl PersonalSyncConflictSink for FakeConflictSink {
    async fn pause_record(
        &self,
        conflict: &PersonalSyncRecordConflict,
        _local: Option<&PersonalSyncItemSnapshot>,
        _remote: Option<&CloudSyncData>,
    ) -> Result<(), SyncStoreError> {
        self.paused
            .lock()
            .expect("paused lock")
            .push(conflict.cloud_id.clone());
        Ok(())
    }
}

fn remote_record_conflicting() -> CloudSyncData {
    let mut record = test_record("cloud-1", data_type::CONNECTION, 2, "remote");
    record.updated_at = 300_000;
    record
}

fn local_record_conflicting() -> PersonalSyncItemSnapshot {
    PersonalSyncItemSnapshot {
        local_id: "local-1".to_string(),
        cloud_id: Some("cloud-1".to_string()),
        data_type: data_type::CONNECTION.to_string(),
        updated_at: 300,
        last_synced_at: Some(100),
        checksum: "local".to_string(),
        team_id: None,
    }
}

fn local_record_synced() -> PersonalSyncItemSnapshot {
    PersonalSyncItemSnapshot {
        local_id: "local-1".to_string(),
        cloud_id: Some("cloud-1".to_string()),
        data_type: data_type::CONNECTION.to_string(),
        updated_at: 100,
        last_synced_at: Some(100),
        checksum: "remote".to_string(),
        team_id: None,
    }
}
