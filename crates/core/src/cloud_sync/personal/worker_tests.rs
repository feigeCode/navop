use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::cloud_sync::models::{CloudSyncData, data_type};
use crate::cloud_sync::personal::test_support::test_record;
use crate::cloud_sync::personal::{
    PersonalConflictType, PersonalSyncConflictSink, PersonalSyncEvent, PersonalSyncItemSnapshot,
    PersonalSyncLocalSource, PersonalSyncRecordConflict, PersonalSyncStore, PersonalSyncWorker,
    SyncDeviceId, SyncStoreError, SyncStoreLock, SyncStoreStatus, WorkerConfig,
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

    assert_eq!(
        vec![(data_type::CONNECTION.to_string(), "cloud-1".to_string())],
        store.tombstoned_keys()
    );
}

#[tokio::test]
async fn worker_local_delete_isolates_same_cloud_id_by_data_type() {
    let connection = test_record("shared-cloud-id", data_type::CONNECTION, 4, "connection");
    let credential = test_record("shared-cloud-id", data_type::CREDENTIAL, 7, "credential");
    let store = FakePersonalSyncStore::with_records(vec![connection, credential]);
    let worker = PersonalSyncWorker::new(
        store.clone(),
        FakePersonalSyncLocalSource::default(),
        WorkerConfig::test(),
    );

    worker.enqueue(PersonalSyncEvent::LocalDeleted {
        data_type: data_type::CONNECTION.to_string(),
        cloud_id: "shared-cloud-id".to_string(),
    });
    worker.drain_once().await.expect("drain succeeds");

    assert_eq!(
        vec![(
            data_type::CONNECTION.to_string(),
            "shared-cloud-id".to_string()
        )],
        store.tombstoned_keys()
    );
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

#[tokio::test]
async fn worker_remote_tombstone_isolates_same_cloud_id_by_data_type() {
    let mut remote = test_record("shared-cloud-id", data_type::CREDENTIAL, 4, "credential");
    remote.deleted_at = Some(400_000);
    let store = FakePersonalSyncStore::with_records(vec![remote]);
    let local = FakePersonalSyncLocalSource::with_items(vec![
        local_record(
            "local-connection",
            "shared-cloud-id",
            data_type::CONNECTION,
            "connection",
        ),
        local_record(
            "local-credential",
            "shared-cloud-id",
            data_type::CREDENTIAL,
            "credential",
        ),
    ]);
    let worker = PersonalSyncWorker::new(store, local.clone(), WorkerConfig::test());

    worker.enqueue(PersonalSyncEvent::RemoteChanged);
    worker.drain_once().await.expect("drain succeeds");

    assert_eq!(vec!["local-credential"], local.deleted_local_ids());
}

#[tokio::test]
async fn worker_pauses_remote_delete_conflict_and_continues_other_records() {
    let mut tombstone = test_record("credential-cloud-1", data_type::CREDENTIAL, 4, "credential");
    tombstone.deleted_at = Some(400_000);
    let download = test_record("connection-cloud-2", data_type::CONNECTION, 1, "connection");
    let store = FakePersonalSyncStore::with_records(vec![tombstone, download]);
    let local = FakePersonalSyncLocalSource::with_items(vec![local_record(
        "local-credential",
        "credential-cloud-1",
        data_type::CREDENTIAL,
        "credential",
    )]);
    local.reject_delete_for("local-credential");
    let conflicts = FakeConflictSink::default();
    let worker = PersonalSyncWorker::with_conflict_sink(
        store,
        local.clone(),
        conflicts.clone(),
        WorkerConfig::test(),
    );

    worker.enqueue(PersonalSyncEvent::RemoteChanged);
    let error = worker
        .drain_once()
        .await
        .expect_err("remote deletion conflict pauses only that record");

    assert!(matches!(error, SyncStoreError::Conflict(_)));
    assert_eq!(
        vec![(
            data_type::CONNECTION.to_string(),
            "connection-cloud-2".to_string()
        )],
        local.applied_remote_keys()
    );
    assert!(local.deleted_local_ids().is_empty());
    let paused = conflicts.paused_conflicts();
    assert_eq!(1, paused.len());
    assert_eq!("credential-cloud-1", paused[0].cloud_id);
    assert_eq!(data_type::CREDENTIAL, paused[0].data_type);
    assert_eq!(
        PersonalConflictType::LocalModifiedRemoteDeleted,
        paused[0].conflict_type
    );
}

#[derive(Clone, Default)]
struct FakePersonalSyncStore {
    records: Arc<Mutex<Vec<CloudSyncData>>>,
    list_calls: Arc<Mutex<usize>>,
    tombstoned: Arc<Mutex<Vec<(String, String)>>>,
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

    fn tombstoned_keys(&self) -> Vec<(String, String)> {
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
        data_type: &str,
        id: &str,
        _expected_version: Option<u32>,
    ) -> Result<(), SyncStoreError> {
        self.tombstoned
            .lock()
            .expect("tombstoned lock")
            .push((data_type.to_string(), id.to_string()));
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
    rejected_deletes: Arc<Mutex<Vec<String>>>,
    applied_remote: Arc<Mutex<Vec<(String, String)>>>,
}

impl FakePersonalSyncLocalSource {
    fn with_items(items: Vec<PersonalSyncItemSnapshot>) -> Self {
        Self {
            items: Arc::new(Mutex::new(items)),
            deleted: Arc::new(Mutex::new(Vec::new())),
            rejected_deletes: Arc::new(Mutex::new(Vec::new())),
            applied_remote: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn reject_delete_for(&self, local_id: &str) {
        self.rejected_deletes
            .lock()
            .expect("rejected_deletes lock")
            .push(local_id.to_string());
    }

    fn deleted_local_ids(&self) -> Vec<String> {
        self.deleted.lock().expect("deleted lock").clone()
    }

    fn applied_remote_keys(&self) -> Vec<(String, String)> {
        self.applied_remote
            .lock()
            .expect("applied_remote lock")
            .clone()
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
        record: &CloudSyncData,
        _local: Option<&PersonalSyncItemSnapshot>,
    ) -> Result<(), SyncStoreError> {
        self.applied_remote
            .lock()
            .expect("applied_remote lock")
            .push((record.data_type.clone(), record.id.clone()));
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
        if self
            .rejected_deletes
            .lock()
            .expect("rejected_deletes lock")
            .contains(&item.local_id)
        {
            return Err(SyncStoreError::Conflict(format!(
                "{} is still referenced",
                item.local_id
            )));
        }
        self.deleted
            .lock()
            .expect("deleted lock")
            .push(item.local_id.clone());
        Ok(())
    }
}

#[derive(Clone, Default)]
struct FakeConflictSink {
    paused: Arc<Mutex<Vec<PersonalSyncRecordConflict>>>,
}

impl FakeConflictSink {
    fn paused_record_ids(&self) -> Vec<String> {
        self.paused
            .lock()
            .expect("paused lock")
            .iter()
            .map(|conflict| conflict.cloud_id.clone())
            .collect()
    }

    fn paused_conflicts(&self) -> Vec<PersonalSyncRecordConflict> {
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
            .push(conflict.clone());
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
    local_record("local-1", "cloud-1", data_type::CONNECTION, "remote")
}

fn local_record(
    local_id: &str,
    cloud_id: &str,
    item_type: &str,
    checksum: &str,
) -> PersonalSyncItemSnapshot {
    PersonalSyncItemSnapshot {
        local_id: local_id.to_string(),
        cloud_id: Some(cloud_id.to_string()),
        data_type: item_type.to_string(),
        updated_at: 100,
        last_synced_at: Some(100),
        checksum: checksum.to_string(),
        team_id: None,
    }
}
