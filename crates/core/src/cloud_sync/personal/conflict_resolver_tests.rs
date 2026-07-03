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
        vec![("local-1".to_string(), "cloud-1".to_string(), 400)],
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
        vec![("local-1".to_string(), "cloud-1".to_string(), 2)],
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

#[derive(Clone, Default)]
struct FakeStore {
    expected_versions: Arc<Mutex<Vec<Option<u32>>>>,
}

impl FakeStore {
    fn expected_versions(&self) -> Vec<Option<u32>> {
        self.expected_versions
            .lock()
            .expect("expected_versions lock")
            .clone()
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
        _id: &str,
        _expected_version: Option<u32>,
    ) -> Result<(), SyncStoreError> {
        Ok(())
    }

    async fn acquire_lock(&self, owner: &SyncDeviceId) -> Result<SyncStoreLock, SyncStoreError> {
        Ok(SyncStoreLock {
            owner: owner.clone(),
        })
    }
}

#[derive(Clone, Default)]
struct FakeLocalSource {
    items: Arc<Mutex<Vec<PersonalSyncItemSnapshot>>>,
    applied_remote_ids: Arc<Mutex<Vec<String>>>,
    marked_synced: Arc<Mutex<Vec<(String, String, i64)>>>,
}

impl FakeLocalSource {
    fn with_items(items: Vec<PersonalSyncItemSnapshot>) -> Self {
        Self {
            items: Arc::new(Mutex::new(items)),
            applied_remote_ids: Arc::new(Mutex::new(Vec::new())),
            marked_synced: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn applied_remote_ids(&self) -> Vec<String> {
        self.applied_remote_ids
            .lock()
            .expect("applied_remote_ids lock")
            .clone()
    }

    fn marked_synced(&self) -> Vec<(String, String, i64)> {
        self.marked_synced
            .lock()
            .expect("marked_synced lock")
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
        _local: Option<&PersonalSyncItemSnapshot>,
    ) -> Result<(), SyncStoreError> {
        self.applied_remote_ids
            .lock()
            .expect("applied_remote_ids lock")
            .push(record.id.clone());
        Ok(())
    }

    async fn mark_synced(
        &self,
        local_id: &str,
        cloud_id: &str,
        synced_at: i64,
    ) -> Result<(), SyncStoreError> {
        self.marked_synced
            .lock()
            .expect("marked_synced lock")
            .push((local_id.to_string(), cloud_id.to_string(), synced_at));
        Ok(())
    }

    async fn delete_item(&self, _item: &PersonalSyncItemSnapshot) -> Result<(), SyncStoreError> {
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
