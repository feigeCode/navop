use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::cloud_sync::models::{CloudSyncData, ConflictResolution, data_type};
use crate::cloud_sync::personal::test_support::test_record;
use crate::cloud_sync::personal::{
    PersonalConflictType, PersonalSyncConflict, PersonalSyncConflictRepository,
    PersonalSyncConflictResolver, PersonalSyncItemSnapshot, PersonalSyncLocalSource,
    PersonalSyncStore, SyncDeviceId, SyncStoreError, SyncStoreLock, SyncStoreStatus,
};
use crate::storage::connection::SqliteConnection;
use crate::storage::migration::run_migrations;

#[tokio::test]
async fn resolver_use_cloud_applies_remote_marks_synced_and_clears_conflict() {
    let repo = conflict_repo();
    let local_item = local_item("local-1", "cloud-1", "local");
    let mut remote = test_record("cloud-1", data_type::CONNECTION, 7, "remote");
    remote.updated_at = 400_000;
    let conflict = stored_conflict(&local_item, &remote);
    repo.upsert(&conflict).expect("conflict stored");
    let local = FakeLocalSource::with_items(vec![local_item.clone()]);
    let store = FakeStore::default();
    let resolver = PersonalSyncConflictResolver::new(store, local.clone(), repo.clone());

    resolver
        .resolve(&conflict, ConflictResolution::UseCloud)
        .await
        .expect("use cloud resolves");

    assert_eq!(vec!["cloud-1"], local.applied_remote_ids());
    assert_eq!(
        vec![(
            "local-1".to_string(),
            "cloud-1".to_string(),
            "remote".to_string(),
            400
        )],
        local.marked_synced()
    );
    assert!(repo.list("personal").expect("conflicts list").is_empty());
}

#[tokio::test]
async fn resolver_use_local_writes_expected_remote_version_and_clears_conflict() {
    let repo = conflict_repo();
    let local_item = local_item("local-1", "cloud-1", "local");
    let remote = test_record("cloud-1", data_type::CONNECTION, 7, "remote");
    let conflict = stored_conflict(&local_item, &remote);
    repo.upsert(&conflict).expect("conflict stored");
    let local = FakeLocalSource::with_items(vec![local_item]);
    let store = FakeStore::default();
    let resolver = PersonalSyncConflictResolver::new(store.clone(), local.clone(), repo.clone());

    resolver
        .resolve(&conflict, ConflictResolution::UseLocal)
        .await
        .expect("use local resolves");

    assert_eq!(vec![Some(7)], store.expected_versions());
    assert_eq!(
        vec![(
            "local-1".to_string(),
            "cloud-1".to_string(),
            "local".to_string(),
            2
        )],
        local.marked_synced()
    );
    assert!(repo.list("personal").expect("conflicts list").is_empty());
}

#[tokio::test]
async fn resolver_keep_both_is_explicitly_unsupported_and_keeps_conflict() {
    let repo = conflict_repo();
    let local_item = local_item("local-1", "cloud-1", "local");
    let remote = test_record("cloud-1", data_type::CONNECTION, 7, "remote");
    let conflict = stored_conflict(&local_item, &remote);
    repo.upsert(&conflict).expect("conflict stored");
    let resolver = PersonalSyncConflictResolver::new(
        FakeStore::default(),
        FakeLocalSource::with_items(vec![local_item]),
        repo.clone(),
    );

    let error = resolver
        .resolve(&conflict, ConflictResolution::KeepBoth)
        .await
        .expect_err("keep both needs a dedicated copy API");

    assert!(matches!(error, SyncStoreError::Conflict(_)));
    assert_eq!(1, repo.list("personal").expect("conflicts list").len());
}

/// issue #361：冲突存档里的那条本地行被删掉之后，点「使用远程版本」应该把远端条目
/// 重新落回本地，而不是报 `connection not found`。
#[tokio::test]
async fn resolver_use_cloud_reinserts_local_item_when_local_row_is_gone() {
    let repo = conflict_repo();
    let stored_item = local_item("connection:22", "cloud-1", "local");
    let mut remote = test_record("cloud-1", data_type::CONNECTION, 7, "remote");
    remote.updated_at = 400_000;
    let conflict = stored_conflict(&stored_item, &remote);
    repo.upsert(&conflict).expect("conflict stored");
    // 本地行已经不存在（用户在界面里删掉了这条连接）。
    let local = FakeLocalSource::with_items(Vec::new());
    let store = FakeStore::default();
    let resolver = PersonalSyncConflictResolver::new(store, local.clone(), repo.clone());

    resolver
        .resolve(&conflict, ConflictResolution::UseCloud)
        .await
        .expect("use cloud re-downloads the remote record");

    assert_eq!(vec!["cloud-1"], local.applied_remote_ids());
    // 关键：必须按「本地没有这条记录」下发，而不是把档存的过期 local_id 传下去。
    assert_eq!(vec![None], local.applied_remote_local_ids());
    assert!(local.marked_synced().is_empty());
    assert!(repo.list("personal").expect("conflicts list").is_empty());
}

/// issue #361：本地行没了时点「使用本地版本」，语义是「保留本地的删除」——
/// 应该把云端记录也标记为删除，而不是报错、把冲突永远留在表里。
#[tokio::test]
async fn resolver_use_local_tombstones_cloud_record_when_local_row_is_gone() {
    let repo = conflict_repo();
    let stored_item = local_item("connection:22", "cloud-1", "local");
    let remote = test_record("cloud-1", data_type::CONNECTION, 7, "remote");
    let conflict = stored_conflict(&stored_item, &remote);
    repo.upsert(&conflict).expect("conflict stored");
    let local = FakeLocalSource::with_items(Vec::new());
    let store = FakeStore::default();
    let resolver = PersonalSyncConflictResolver::new(store.clone(), local.clone(), repo.clone());

    resolver
        .resolve(&conflict, ConflictResolution::UseLocal)
        .await
        .expect("use local propagates the local deletion");

    assert_eq!(
        vec![(
            data_type::CONNECTION.to_string(),
            "cloud-1".to_string(),
            Some(7)
        )],
        store.tombstoned()
    );
    assert!(local.applied_remote_ids().is_empty());
    assert!(repo.list("personal").expect("conflicts list").is_empty());
}

/// 冲突存档里的本地行 id 已经过期（本地条目还在，但换了 id）时，也要按云端 id 找回来
/// 走「更新本地」，而不是当成「本地没有」再插一份。
#[tokio::test]
async fn resolver_use_cloud_re_resolves_local_item_by_cloud_id_with_stale_stored_id() {
    let repo = conflict_repo();
    let stored_item = local_item("connection:22", "cloud-1", "local");
    let mut remote = test_record("cloud-1", data_type::CONNECTION, 7, "remote");
    remote.updated_at = 400_000;
    let conflict = stored_conflict(&stored_item, &remote);
    repo.upsert(&conflict).expect("conflict stored");
    let live_item = local_item("connection:31", "cloud-1", "local");
    let local = FakeLocalSource::with_items(vec![live_item]);
    let resolver =
        PersonalSyncConflictResolver::new(FakeStore::default(), local.clone(), repo.clone());

    resolver
        .resolve(&conflict, ConflictResolution::UseCloud)
        .await
        .expect("use cloud resolves against the live local row");

    assert_eq!(vec!["cloud-1"], local.applied_remote_ids());
    assert_eq!(
        vec![Some("connection:31".to_string())],
        local.applied_remote_local_ids()
    );
    assert_eq!(
        vec![(
            "connection:31".to_string(),
            "cloud-1".to_string(),
            "remote".to_string(),
            400
        )],
        local.marked_synced()
    );
    assert!(repo.list("personal").expect("conflicts list").is_empty());
}

/// 冲突两边的记录都已经不在（本地行被删、云端也已是墓碑）时，点「使用远程版本」
/// 只应该清掉冲突，不产生任何写入。
#[tokio::test]
async fn resolver_use_cloud_only_clears_conflict_when_both_sides_are_gone() {
    let repo = conflict_repo();
    let stored_item = local_item("connection:22", "cloud-1", "local");
    let mut remote = test_record("cloud-1", data_type::CONNECTION, 7, "remote");
    remote.deleted_at = Some(400_000);
    let conflict = stored_conflict(&stored_item, &remote);
    repo.upsert(&conflict).expect("conflict stored");
    let local = FakeLocalSource::with_items(Vec::new());
    let store = FakeStore::default();
    let resolver = PersonalSyncConflictResolver::new(store.clone(), local.clone(), repo.clone());

    resolver
        .resolve(&conflict, ConflictResolution::UseCloud)
        .await
        .expect("use cloud is a no-op when both sides are gone");

    assert!(local.applied_remote_ids().is_empty());
    assert!(local.deleted_local_ids().is_empty());
    assert!(store.tombstoned().is_empty());
    assert!(repo.list("personal").expect("conflicts list").is_empty());
}

#[derive(Clone, Default)]
struct FakeStore {
    expected_versions: Arc<Mutex<Vec<Option<u32>>>>,
    tombstoned: Arc<Mutex<Vec<(String, String, Option<u32>)>>>,
}

impl FakeStore {
    fn expected_versions(&self) -> Vec<Option<u32>> {
        self.expected_versions
            .lock()
            .expect("expected_versions lock")
            .clone()
    }

    fn tombstoned(&self) -> Vec<(String, String, Option<u32>)> {
        self.tombstoned.lock().expect("tombstoned lock").clone()
    }
}

#[async_trait]
impl PersonalSyncStore for FakeStore {
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
        Ok(Vec::new())
    }

    async fn upsert_record(
        &self,
        record: &CloudSyncData,
        expected_version: Option<u32>,
    ) -> Result<CloudSyncData, SyncStoreError> {
        self.expected_versions
            .lock()
            .expect("expected_versions lock")
            .push(expected_version);
        let mut stored = record.clone();
        stored.version = expected_version.unwrap_or(1).saturating_add(1);
        stored.updated_at = 2_000;
        Ok(stored)
    }

    async fn tombstone_record(
        &self,
        data_type: &str,
        id: &str,
        expected_version: Option<u32>,
    ) -> Result<(), SyncStoreError> {
        self.tombstoned.lock().expect("tombstoned lock").push((
            data_type.to_string(),
            id.to_string(),
            expected_version,
        ));
        Ok(())
    }

    async fn acquire_lock(&self, owner: &SyncDeviceId) -> Result<SyncStoreLock, SyncStoreError> {
        Ok(SyncStoreLock::owned_by(owner.clone()))
    }
}

#[derive(Clone, Default)]
struct FakeLocalSource {
    items: Arc<Mutex<Vec<PersonalSyncItemSnapshot>>>,
    /// `(record_id, 传给 apply_remote 的本地条目 local_id)`
    applied_remote: Arc<Mutex<Vec<(String, Option<String>)>>>,
    marked_synced: Arc<Mutex<Vec<(String, String, String, i64)>>>,
    deleted_local_ids: Arc<Mutex<Vec<String>>>,
}

impl FakeLocalSource {
    fn with_items(items: Vec<PersonalSyncItemSnapshot>) -> Self {
        Self {
            items: Arc::new(Mutex::new(items)),
            applied_remote: Arc::new(Mutex::new(Vec::new())),
            marked_synced: Arc::new(Mutex::new(Vec::new())),
            deleted_local_ids: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn applied_remote_ids(&self) -> Vec<String> {
        self.applied_remote
            .lock()
            .expect("applied_remote lock")
            .iter()
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// `apply_remote` 收到的本地条目：`None` 表示「本地没有这条记录」。
    fn applied_remote_local_ids(&self) -> Vec<Option<String>> {
        self.applied_remote
            .lock()
            .expect("applied_remote lock")
            .iter()
            .map(|(_, local_id)| local_id.clone())
            .collect()
    }

    fn marked_synced(&self) -> Vec<(String, String, String, i64)> {
        self.marked_synced
            .lock()
            .expect("marked_synced lock")
            .clone()
    }

    fn deleted_local_ids(&self) -> Vec<String> {
        self.deleted_local_ids
            .lock()
            .expect("deleted_local_ids lock")
            .clone()
    }
}

#[async_trait]
impl PersonalSyncLocalSource for FakeLocalSource {
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
        local: Option<&PersonalSyncItemSnapshot>,
    ) -> Result<(), SyncStoreError> {
        self.applied_remote
            .lock()
            .expect("applied_remote lock")
            .push((record.id.clone(), local.map(|item| item.local_id.clone())));
        Ok(())
    }

    async fn mark_synced(
        &self,
        local_id: &str,
        cloud_id: &str,
        checksum: &str,
        synced_at: i64,
    ) -> Result<(), SyncStoreError> {
        self.marked_synced
            .lock()
            .expect("marked_synced lock")
            .push((
                local_id.to_string(),
                cloud_id.to_string(),
                checksum.to_string(),
                synced_at,
            ));
        Ok(())
    }

    async fn delete_item(&self, item: &PersonalSyncItemSnapshot) -> Result<(), SyncStoreError> {
        self.deleted_local_ids
            .lock()
            .expect("deleted_local_ids lock")
            .push(item.local_id.clone());
        Ok(())
    }
}

fn conflict_repo() -> PersonalSyncConflictRepository {
    let temp = tempfile::tempdir().expect("tempdir");
    let conn = SqliteConnection::open(temp.path().join("test.db")).expect("sqlite");
    conn.with_connection(|conn| run_migrations(conn))
        .expect("migrations run");
    PersonalSyncConflictRepository::new(conn)
}

fn stored_conflict(
    local: &PersonalSyncItemSnapshot,
    remote: &CloudSyncData,
) -> PersonalSyncConflict {
    PersonalSyncConflict {
        backend_profile_id: "personal".to_string(),
        record_id: remote.id.clone(),
        data_type: remote.data_type.clone(),
        conflict_type: PersonalConflictType::BothModified,
        local_snapshot: Some(serde_json::to_string(local).expect("local json")),
        remote_snapshot: Some(serde_json::to_string(remote).expect("remote json")),
        detected_at: 100,
    }
}

fn local_item(local_id: &str, cloud_id: &str, checksum: &str) -> PersonalSyncItemSnapshot {
    PersonalSyncItemSnapshot {
        local_id: local_id.to_string(),
        cloud_id: Some(cloud_id.to_string()),
        data_type: data_type::CONNECTION.to_string(),
        updated_at: 300,
        last_synced_at: Some(100),
        checksum: checksum.to_string(),
        team_id: None,
    }
}
