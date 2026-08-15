use crate::cloud_sync::models::data_type;
use crate::cloud_sync::personal::test_support::test_record;
use crate::cloud_sync::personal::{
    DirectorySyncStore, PersonalSyncStore, SyncStoreError, SyncStoreHealth,
};

#[tokio::test]
async fn probe_initializes_missing_sync_package() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = DirectorySyncStore::new(temp.path().to_path_buf());

    let status = store.probe().await.expect("probe succeeds");

    assert_eq!(SyncStoreHealth::Ready, status.health);
    assert!(temp.path().join(".onetcli-sync/manifest.json").exists());
}

#[tokio::test]
async fn upsert_record_writes_and_lists_by_type() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = DirectorySyncStore::new(temp.path().to_path_buf());
    store.probe().await.expect("probe succeeds");
    let connection = test_record(
        "shared-cloud-id",
        data_type::CONNECTION,
        1,
        "connection-checksum",
    );
    let credential = test_record(
        "shared-cloud-id",
        data_type::CREDENTIAL,
        1,
        "credential-checksum",
    );

    let stored_connection = store
        .upsert_record(&connection, None)
        .await
        .expect("connection upsert succeeds");
    let stored_credential = store
        .upsert_record(&credential, None)
        .await
        .expect("credential upsert succeeds");
    let connections = store
        .list_records(Some(data_type::CONNECTION), None)
        .await
        .expect("connection list succeeds");
    let credentials = store
        .list_records(Some(data_type::CREDENTIAL), None)
        .await
        .expect("credential list succeeds");

    assert_eq!(connection.id, stored_connection.id);
    assert_eq!(credential.id, stored_credential.id);
    assert_eq!(1, connections.len());
    assert_eq!(data_type::CONNECTION, connections[0].data_type);
    assert_eq!("shared-cloud-id", connections[0].id);
    assert_eq!(1, credentials.len());
    assert_eq!(data_type::CREDENTIAL, credentials[0].data_type);
    assert_eq!("shared-cloud-id", credentials[0].id);
}

#[tokio::test]
async fn upsert_rejects_stale_expected_version() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = DirectorySyncStore::new(temp.path().to_path_buf());
    store.probe().await.expect("probe succeeds");
    let record = test_record("connection-1", data_type::CONNECTION, 3, "checksum-1");
    store
        .upsert_record(&record, None)
        .await
        .expect("seed succeeds");

    let err = store
        .upsert_record(&record, Some(2))
        .await
        .expect_err("stale write conflicts");

    assert!(matches!(err, SyncStoreError::Conflict(_)));
}

#[tokio::test]
async fn upsert_advances_version_after_expected_write() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = DirectorySyncStore::new(temp.path().to_path_buf());
    let record = test_record("connection-1", data_type::CONNECTION, 1, "checksum-1");
    let first = store
        .upsert_record(&record, None)
        .await
        .expect("seed succeeds");

    let mut changed = first.clone();
    changed.checksum = "checksum-2".to_string();
    let second = store
        .upsert_record(&changed, Some(first.version))
        .await
        .expect("expected write succeeds");

    assert_eq!(first.version + 1, second.version);
    assert!(second.updated_at >= first.updated_at);
}

#[tokio::test]
async fn tombstone_marks_record_deleted() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = DirectorySyncStore::new(temp.path().to_path_buf());
    store.probe().await.expect("probe succeeds");
    let connection = test_record(
        "shared-cloud-id",
        data_type::CONNECTION,
        1,
        "connection-checksum",
    );
    let credential = test_record(
        "shared-cloud-id",
        data_type::CREDENTIAL,
        1,
        "credential-checksum",
    );
    store
        .upsert_record(&connection, None)
        .await
        .expect("connection upsert succeeds");
    store
        .upsert_record(&credential, None)
        .await
        .expect("credential upsert succeeds");

    store
        .tombstone_record(data_type::CONNECTION, "shared-cloud-id", Some(1))
        .await
        .expect("tombstone succeeds");
    let connections = store
        .list_records(Some(data_type::CONNECTION), None)
        .await
        .expect("connection list succeeds");
    let credentials = store
        .list_records(Some(data_type::CREDENTIAL), None)
        .await
        .expect("credential list succeeds");

    assert_eq!(1, connections.len());
    assert!(connections[0].deleted_at.is_some());
    assert_eq!(1, credentials.len());
    assert!(credentials[0].deleted_at.is_none());
    assert!(
        temp.path()
            .join(".onetcli-sync/tombstones/connection/shared-cloud-id.json")
            .exists()
    );
    assert!(
        !temp
            .path()
            .join(".onetcli-sync/tombstones/credential/shared-cloud-id.json")
            .exists()
    );
}

#[tokio::test]
async fn tombstone_advances_version() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = DirectorySyncStore::new(temp.path().to_path_buf());
    let record = test_record("connection-1", data_type::CONNECTION, 1, "checksum-1");
    let stored = store
        .upsert_record(&record, None)
        .await
        .expect("seed succeeds");

    store
        .tombstone_record(data_type::CONNECTION, "connection-1", Some(stored.version))
        .await
        .expect("tombstone succeeds");
    let records = store
        .list_records(Some(data_type::CONNECTION), None)
        .await
        .expect("list succeeds");

    assert_eq!(stored.version + 1, records[0].version);
    assert!(records[0].deleted_at.is_some());
}
