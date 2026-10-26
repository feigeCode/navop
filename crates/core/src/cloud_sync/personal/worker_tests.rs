use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::cloud_sync::models::{CloudSyncData, data_type};
use crate::cloud_sync::personal::test_support::test_record;
use crate::cloud_sync::personal::{
    PersonalConflictType, PersonalSyncCloudKey, PersonalSyncConflictSink, PersonalSyncEvent,
    PersonalSyncItemSnapshot, PersonalSyncLocalSource, PersonalSyncRecordConflict,
    PersonalSyncStore, PersonalSyncWorker, SyncDeviceId, SyncStoreError, SyncStoreLock,
    SyncStoreStatus, WorkerConfig,
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

/// 远端锁（WebDAV）必须在 pass 结束后显式释放 —— `Drop` 里 await 不了。
/// 这里刻意让这一轮以冲突收尾：释放必须发生在**错误路径**上，否则一次失败的 pass
/// 会把锁一直挂到 TTL 过期，后续几轮同步全被拒掉。
#[tokio::test]
async fn worker_releases_store_lock_even_when_the_pass_fails() {
    let store = FakePersonalSyncStore::with_records(vec![remote_record_conflicting()]);
    let worker = PersonalSyncWorker::with_conflict_sink(
        store.clone(),
        FakePersonalSyncLocalSource::with_items(vec![local_record_conflicting()]),
        FakeConflictSink::default(),
        WorkerConfig::test(),
    );

    worker.enqueue(PersonalSyncEvent::FullScan);
    worker.drain_once().await.expect_err("这一轮以冲突结束");

    assert_eq!(1, store.acquire_calls());
    assert_eq!(1, store.release_calls());
}

#[tokio::test]
async fn worker_marks_matching_record_synced_at_remote_timestamp() {
    let mut remote = test_record("cloud-1", data_type::CONNECTION, 2, "same");
    remote.updated_at = 300_000;
    let store = FakePersonalSyncStore::with_records(vec![remote]);
    let local = FakePersonalSyncLocalSource::with_items(vec![local_record(
        "local-1",
        "cloud-1",
        data_type::CONNECTION,
        "same",
    )]);
    let worker = PersonalSyncWorker::new(store, local.clone(), WorkerConfig::test());

    worker.enqueue(PersonalSyncEvent::FullScan);
    worker.drain_once().await.expect("drain succeeds");

    assert_eq!(
        vec![(
            "local-1".to_string(),
            "cloud-1".to_string(),
            "same".to_string(),
            300
        )],
        local.synced_marks()
    );
}

#[tokio::test]
async fn worker_reports_touched_data_types_for_downloaded_records() {
    // 下载落库的数据类型必须回报给调用方：个人同步运行时靠它通知 UI 刷新
    // （同步直接写业务表，不通知的话已经打开的页面显示旧列表）。
    let store = FakePersonalSyncStore::with_records(vec![test_record(
        "cloud-1",
        data_type::CONNECTION,
        4,
        "remote",
    )]);
    let worker = PersonalSyncWorker::new(
        store,
        FakePersonalSyncLocalSource::default(),
        WorkerConfig::test(),
    );

    worker.enqueue(PersonalSyncEvent::FullScan);
    let touched = worker.drain_once().await.expect("drain succeeds");

    assert_eq!(
        HashSet::from([data_type::CONNECTION.to_string()]),
        touched,
        "下载落库的数据类型必须回报"
    );
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
        last_synced_at: None,
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
        last_synced_at: None,
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

/// 本地自上次同步之后改过，远端却把这条记录删了 ⇒ 不能静默丢掉用户的改动，
/// 应该挂起一条 `LocalModifiedRemoteDeleted` 冲突（这个分支以前根本不存在，
/// 连接会被无条件删掉，导致冲突悬挂）。
#[tokio::test]
async fn worker_pauses_remote_delete_when_local_item_was_modified_locally() {
    let mut tombstone = test_record("cloud-1", data_type::CONNECTION, 4, "remote");
    tombstone.deleted_at = Some(400_000);
    let store = FakePersonalSyncStore::with_records(vec![tombstone]);
    // 本地自上次同步（100）之后改过（300）。
    let local = FakePersonalSyncLocalSource::with_items(vec![local_record_conflicting()]);
    let conflicts = FakeConflictSink::default();
    let worker = PersonalSyncWorker::with_conflict_sink(
        store.clone(),
        local.clone(),
        conflicts.clone(),
        WorkerConfig::test(),
    );

    worker.enqueue(PersonalSyncEvent::RemoteChanged);
    let error = worker
        .drain_once()
        .await
        .expect_err("locally modified record must not be silently dropped");

    assert!(matches!(error, SyncStoreError::Conflict(_)));
    assert!(local.deleted_local_ids().is_empty());
    let paused = conflicts.paused_conflicts();
    assert_eq!(1, paused.len());
    assert_eq!("cloud-1", paused[0].cloud_id);
    assert_eq!(data_type::CONNECTION, paused[0].data_type);
    assert_eq!(
        PersonalConflictType::LocalModifiedRemoteDeleted,
        paused[0].conflict_type
    );
    // 同一次 pass 不能一边挂起冲突一边把这条记录又推回云端。
    assert_eq!(1, store.record_count());
}

/// 已经挂起冲突的记录，遇到远端墓碑时必须原样留着本地行：删掉它会让冲突存档里的
/// `local_snapshot` 指向不存在的本地条目，冲突就再也解不开了（issue #361）。
#[tokio::test]
async fn worker_keeps_paused_local_item_when_remote_record_is_tombstoned() {
    let mut tombstone = test_record("cloud-1", data_type::CONNECTION, 4, "remote");
    tombstone.deleted_at = Some(400_000);
    let store = FakePersonalSyncStore::with_records(vec![tombstone]);
    // 已同步、且本地没改过 —— 没有 `paused` 保护的话这条会被直接删掉。
    let local = FakePersonalSyncLocalSource::with_items(vec![local_record_synced()]);
    let conflicts = FakeConflictSink::default();
    conflicts.pause_key(data_type::CONNECTION, "cloud-1");
    let worker = PersonalSyncWorker::with_conflict_sink(
        store,
        local.clone(),
        conflicts.clone(),
        WorkerConfig::test(),
    );

    worker.enqueue(PersonalSyncEvent::RemoteChanged);
    worker
        .drain_once()
        .await
        .expect("a paused conflict is left untouched by remote tombstones");

    assert!(local.deleted_local_ids().is_empty());
    assert!(conflicts.paused_conflicts().is_empty());
}

/// 用户删掉本地条目时，这条记录上遗留的冲突要一起丢掉：本地条目没了，冲突就永远
/// 解不开（点「使用远程版本」还会把刚删掉的条目重新拉回来）。
#[tokio::test]
async fn worker_forgets_conflict_when_local_item_is_deleted() {
    let store = FakePersonalSyncStore::with_records(vec![test_record(
        "cloud-1",
        data_type::CONNECTION,
        4,
        "remote",
    )]);
    let conflicts = FakeConflictSink::default();
    let worker = PersonalSyncWorker::with_conflict_sink(
        store.clone(),
        FakePersonalSyncLocalSource::default(),
        conflicts.clone(),
        WorkerConfig::test(),
    );

    worker.enqueue(PersonalSyncEvent::LocalDeleted {
        data_type: data_type::CONNECTION.to_string(),
        cloud_id: "cloud-1".to_string(),
        last_synced_at: None,
    });
    worker.drain_once().await.expect("drain succeeds");

    assert_eq!(
        vec![(data_type::CONNECTION.to_string(), "cloud-1".to_string())],
        conflicts.forgotten()
    );
    assert_eq!(
        vec![(data_type::CONNECTION.to_string(), "cloud-1".to_string())],
        store.tombstoned_keys()
    );
}

/// 本地删除时，如果远端在删除之前那条基线之后又被别的设备改过，就不能直接推墓碑
/// —— 那会把对方刚做的改动一起抹掉。改为挂起 `LocalDeletedRemoteModified`（这个
/// 枚举以前从来没被产生过，UI 文案和默认策略早就写好了）。
#[tokio::test]
async fn worker_pauses_local_delete_conflict_when_remote_changed_since_baseline() {
    // 远端在 400 秒被改过；本地行删除前记录的基线是 100 秒。
    let store = FakePersonalSyncStore::with_records(vec![remote_updated_at("cloud-1", 400_000)]);
    let conflicts = FakeConflictSink::default();
    let worker = PersonalSyncWorker::with_conflict_sink(
        store.clone(),
        FakePersonalSyncLocalSource::default(),
        conflicts.clone(),
        WorkerConfig::test(),
    );

    worker.enqueue(PersonalSyncEvent::LocalDeleted {
        data_type: data_type::CONNECTION.to_string(),
        cloud_id: "cloud-1".to_string(),
        last_synced_at: Some(100),
    });
    let error = worker
        .drain_once()
        .await
        .expect_err("远端改过的删除不能静默推墓碑");

    assert!(matches!(error, SyncStoreError::Conflict(_)));
    assert!(store.tombstoned_keys().is_empty());
    let paused = conflicts.paused_conflicts();
    assert_eq!(1, paused.len());
    assert_eq!("cloud-1", paused[0].cloud_id);
    assert_eq!(data_type::CONNECTION, paused[0].data_type);
    assert_eq!(
        PersonalConflictType::LocalDeletedRemoteModified,
        paused[0].conflict_type
    );
    // 本地行已经不存在了：不能拿空串冒充本地 id，也不该存本地快照。
    assert_eq!(None, paused[0].local_id);
    assert_eq!(
        vec![(None, Some("cloud-1".to_string()))],
        conflicts.paused_snapshots()
    );
}

/// 远端自基线之后没动过 ⇒ 删除是安全的，照旧推墓碑，不能平白给用户弹冲突。
#[tokio::test]
async fn worker_tombstones_local_delete_when_remote_unchanged_since_baseline() {
    // 远端 300 秒的改动早于本地基线（400 秒）⇒ 我们删的是自己见过的那一版。
    let store = FakePersonalSyncStore::with_records(vec![remote_updated_at("cloud-1", 300_000)]);
    let conflicts = FakeConflictSink::default();
    let worker = PersonalSyncWorker::with_conflict_sink(
        store.clone(),
        FakePersonalSyncLocalSource::default(),
        conflicts.clone(),
        WorkerConfig::test(),
    );

    worker.enqueue(PersonalSyncEvent::LocalDeleted {
        data_type: data_type::CONNECTION.to_string(),
        cloud_id: "cloud-1".to_string(),
        last_synced_at: Some(400),
    });
    worker.drain_once().await.expect("drain succeeds");

    assert_eq!(
        vec![(data_type::CONNECTION.to_string(), "cloud-1".to_string())],
        store.tombstoned_keys()
    );
    assert!(conflicts.paused_conflicts().is_empty());
}

/// 基线是秒级的 `last_synced_at`：远端与基线同秒时不算「被改过」。
#[tokio::test]
async fn worker_tombstones_local_delete_when_remote_matches_baseline_second() {
    let store = FakePersonalSyncStore::with_records(vec![remote_updated_at("cloud-1", 400_000)]);
    let worker = PersonalSyncWorker::new(
        store.clone(),
        FakePersonalSyncLocalSource::default(),
        WorkerConfig::test(),
    );

    worker.enqueue(PersonalSyncEvent::LocalDeleted {
        data_type: data_type::CONNECTION.to_string(),
        cloud_id: "cloud-1".to_string(),
        last_synced_at: Some(400),
    });
    worker.drain_once().await.expect("drain succeeds");

    assert_eq!(1, store.tombstoned_keys().len());
}

/// 两边都已经是墓碑时什么都不用做（既不再推墓碑，也不挂冲突）。
#[tokio::test]
async fn worker_ignores_local_delete_when_remote_is_already_a_tombstone() {
    let mut tombstone = test_record("cloud-1", data_type::CONNECTION, 4, "remote");
    tombstone.deleted_at = Some(400_000);
    let store = FakePersonalSyncStore::with_records(vec![tombstone]);
    let conflicts = FakeConflictSink::default();
    let worker = PersonalSyncWorker::with_conflict_sink(
        store.clone(),
        FakePersonalSyncLocalSource::default(),
        conflicts.clone(),
        WorkerConfig::test(),
    );

    worker.enqueue(PersonalSyncEvent::LocalDeleted {
        data_type: data_type::CONNECTION.to_string(),
        cloud_id: "cloud-1".to_string(),
        last_synced_at: Some(100),
    });
    worker.drain_once().await.expect("drain succeeds");

    assert!(store.tombstoned_keys().is_empty());
    assert!(conflicts.paused_conflicts().is_empty());
}

fn remote_updated_at(cloud_id: &str, updated_at: i64) -> CloudSyncData {
    let mut record = test_record(cloud_id, data_type::CONNECTION, 4, "remote");
    record.updated_at = updated_at;
    record
}

#[derive(Clone, Default)]
struct FakePersonalSyncStore {
    records: Arc<Mutex<Vec<CloudSyncData>>>,
    list_calls: Arc<Mutex<usize>>,
    tombstoned: Arc<Mutex<Vec<(String, String)>>>,
    locked: Arc<AtomicUsize>,
    released: Arc<AtomicUsize>,
}

impl FakePersonalSyncStore {
    fn with_records(records: Vec<CloudSyncData>) -> Self {
        Self {
            records: Arc::new(Mutex::new(records)),
            list_calls: Arc::new(Mutex::new(0)),
            tombstoned: Arc::new(Mutex::new(Vec::new())),
            locked: Arc::new(AtomicUsize::new(0)),
            released: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn list_calls(&self) -> usize {
        *self.list_calls.lock().expect("list_calls lock")
    }

    fn acquire_calls(&self) -> usize {
        self.locked.load(Ordering::SeqCst)
    }

    fn release_calls(&self) -> usize {
        self.released.load(Ordering::SeqCst)
    }

    fn record_count(&self) -> usize {
        self.records.lock().expect("records lock").len()
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
        self.locked.fetch_add(1, Ordering::SeqCst);
        Ok(SyncStoreLock::owned_by(owner.clone()))
    }

    async fn release_lock(&self, _lock: &SyncStoreLock) -> Result<(), SyncStoreError> {
        self.released.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[derive(Clone, Default)]
struct FakePersonalSyncLocalSource {
    items: Arc<Mutex<Vec<PersonalSyncItemSnapshot>>>,
    deleted: Arc<Mutex<Vec<String>>>,
    rejected_deletes: Arc<Mutex<Vec<String>>>,
    applied_remote: Arc<Mutex<Vec<(String, String)>>>,
    synced: Arc<Mutex<Vec<(String, String, String, i64)>>>,
}

impl FakePersonalSyncLocalSource {
    fn with_items(items: Vec<PersonalSyncItemSnapshot>) -> Self {
        Self {
            items: Arc::new(Mutex::new(items)),
            deleted: Arc::new(Mutex::new(Vec::new())),
            rejected_deletes: Arc::new(Mutex::new(Vec::new())),
            applied_remote: Arc::new(Mutex::new(Vec::new())),
            synced: Arc::new(Mutex::new(Vec::new())),
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

    fn synced_marks(&self) -> Vec<(String, String, String, i64)> {
        self.synced.lock().expect("synced lock").clone()
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
        local_id: &str,
        cloud_id: &str,
        checksum: &str,
        synced_at: i64,
    ) -> Result<(), SyncStoreError> {
        self.synced.lock().expect("synced lock").push((
            local_id.to_string(),
            cloud_id.to_string(),
            checksum.to_string(),
            synced_at,
        ));
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
    paused_keys: Arc<Mutex<HashSet<PersonalSyncCloudKey>>>,
    forgotten: Arc<Mutex<Vec<(String, String)>>>,
    /// 每次 `pause_record` 传进来的 `(本地条目 id, 远端记录 id)`，用来确认
    /// 「本地行已经不存在的冲突」真的没有本地快照。
    paused_snapshots: Arc<Mutex<Vec<(Option<String>, Option<String>)>>>,
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

    fn paused_snapshots(&self) -> Vec<(Option<String>, Option<String>)> {
        self.paused_snapshots
            .lock()
            .expect("paused_snapshots lock")
            .clone()
    }

    fn pause_key(&self, data_type: &str, cloud_id: &str) {
        self.paused_keys
            .lock()
            .expect("paused_keys lock")
            .insert(PersonalSyncCloudKey {
                data_type: data_type.to_string(),
                cloud_id: cloud_id.to_string(),
            });
    }

    fn forgotten(&self) -> Vec<(String, String)> {
        self.forgotten.lock().expect("forgotten lock").clone()
    }
}

#[async_trait]
impl PersonalSyncConflictSink for FakeConflictSink {
    async fn paused_record_keys(&self) -> Result<HashSet<PersonalSyncCloudKey>, SyncStoreError> {
        Ok(self.paused_keys.lock().expect("paused_keys lock").clone())
    }

    async fn pause_record(
        &self,
        conflict: &PersonalSyncRecordConflict,
        local: Option<&PersonalSyncItemSnapshot>,
        remote: Option<&CloudSyncData>,
    ) -> Result<(), SyncStoreError> {
        self.paused
            .lock()
            .expect("paused lock")
            .push(conflict.clone());
        self.paused_snapshots
            .lock()
            .expect("paused_snapshots lock")
            .push((
                local.map(|item| item.local_id.clone()),
                remote.map(|record| record.id.clone()),
            ));
        Ok(())
    }

    async fn forget_record(&self, data_type: &str, cloud_id: &str) -> Result<(), SyncStoreError> {
        self.forgotten
            .lock()
            .expect("forgotten lock")
            .push((data_type.to_string(), cloud_id.to_string()));
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
