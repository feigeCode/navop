use std::ops::Deref;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, RwLock};

use crate::cloud_sync::models::data_type;
use crate::cloud_sync::personal::{PersonalSyncLocalRepositorySource, PersonalSyncLocalSource};
use crate::cloud_sync::service::CloudSyncService;
use crate::crypto;
use crate::storage::connection::SqliteConnection;
use crate::storage::migration::run_migrations;
use crate::storage::traits::Repository;
use crate::storage::{
    ConnectionRepository, CredentialEntry, CredentialReference, CredentialRepository, DatabaseType,
    DbConnectionConfig, StoredConnection, Workspace, WorkspaceRepository,
};

#[tokio::test]
async fn local_source_lists_only_personal_syncable_records() {
    let fixture = Fixture::new();
    let personal = fixture.insert_connection("personal", None, true);
    fixture.insert_connection("team", Some("team-1"), true);
    fixture.insert_connection("disabled", None, false);
    let personal_credential = fixture.insert_credential(credential("personal credential", true));
    let mut team_credential = credential("team credential", true);
    team_credential.team_id = Some("team-1".to_string());
    fixture.insert_credential(team_credential);
    fixture.insert_credential(credential("disabled credential", false));
    let workspace = fixture.insert_workspace("workspace");

    let items = fixture.source.list_items().await.expect("items list");

    let local_ids = items
        .iter()
        .map(|item| item.local_id.as_str())
        .collect::<Vec<_>>();
    assert!(local_ids.contains(&format!("connection:{personal}").as_str()));
    assert!(local_ids.contains(&format!("credential:{personal_credential}").as_str()));
    assert!(local_ids.contains(&format!("workspace:{workspace}").as_str()));
    assert_eq!(3, items.len());
}

#[tokio::test]
async fn local_source_exports_connection_as_encrypted_sync_record() {
    let fixture = Fixture::new();
    let local_id = fixture.insert_connection("personal", None, true);
    let item = fixture
        .source
        .list_items()
        .await
        .expect("items list")
        .into_iter()
        .find(|item| item.local_id == format!("connection:{local_id}"))
        .expect("connection snapshot exists");

    let record = fixture.source.export_item(&item).await.expect("export");

    assert_eq!(data_type::CONNECTION, record.data_type);
    assert_eq!("personal-user", record.owner_id);
    assert!(!record.encrypted_data.is_empty());
    assert!(!record.checksum.is_empty());
}

#[tokio::test]
async fn local_source_reads_and_updates_workspace_sync_timestamp() {
    let fixture = Fixture::new();
    let workspace_id = fixture.insert_workspace("workspace");

    fixture
        .source
        .mark_synced(
            &format!("workspace:{workspace_id}"),
            "cloud-workspace-1",
            10,
        )
        .await
        .expect("workspace marked synced");

    let item = fixture
        .source
        .list_items()
        .await
        .expect("items list")
        .into_iter()
        .find(|item| item.local_id == format!("workspace:{workspace_id}"))
        .expect("workspace snapshot exists");

    assert_eq!(Some("cloud-workspace-1".to_string()), item.cloud_id);
    assert_eq!(Some(10), item.last_synced_at);
}

#[tokio::test]
async fn local_source_downloads_remote_connection_and_marks_synced() {
    let fixture = Fixture::new();
    let local_id = fixture.insert_connection("personal", None, true);
    let item = fixture
        .source
        .list_items()
        .await
        .expect("items list")
        .into_iter()
        .find(|item| item.local_id == format!("connection:{local_id}"))
        .expect("connection snapshot exists");
    let mut record = fixture.source.export_item(&item).await.expect("export");
    record.id = "cloud-connection-1".to_string();
    record.updated_at = 10_000;

    fixture
        .source
        .apply_remote(&record, None)
        .await
        .expect("remote applied");

    let loaded = fixture
        .connections
        .get_by_cloud_id("cloud-connection-1")
        .expect("query by cloud id")
        .expect("downloaded connection exists");
    assert_eq!("personal", loaded.name);
    assert_eq!(Some("cloud-connection-1".to_string()), loaded.cloud_id);
    assert_eq!(Some(10), loaded.last_synced_at);
}

#[tokio::test]
async fn local_source_does_not_reenable_disabled_connection_from_remote_history() {
    let fixture = Fixture::new();
    let disabled_id = fixture.insert_connection("disabled local", None, false);
    let mut disabled = fixture
        .connections
        .get(disabled_id)
        .expect("connection query")
        .expect("disabled connection exists");
    disabled.cloud_id = Some("connection-cloud-disabled".to_string());
    fixture
        .connections
        .update(&disabled)
        .expect("disabled connection updated");

    let remote_id = fixture.insert_connection("remote historical", None, true);
    let remote_item = fixture
        .source
        .list_items()
        .await
        .expect("items list")
        .into_iter()
        .find(|item| item.local_id == format!("connection:{remote_id}"))
        .expect("remote connection snapshot exists");
    let mut record = fixture
        .source
        .export_item(&remote_item)
        .await
        .expect("remote connection export");
    record.id = "connection-cloud-disabled".to_string();

    fixture
        .source
        .apply_remote(&record, None)
        .await
        .expect("disabled connection skips remote history");

    let loaded = fixture
        .connections
        .get_by_cloud_id("connection-cloud-disabled")
        .expect("query by cloud id")
        .expect("disabled connection remains");
    assert_eq!(Some(disabled_id), loaded.id);
    assert_eq!("disabled local", loaded.name);
    assert!(!loaded.sync_enabled);
}

#[tokio::test]
async fn local_source_skips_remote_team_connection() {
    let fixture = Fixture::new();
    let local_id = fixture.insert_connection("personal", None, true);
    let item = fixture
        .source
        .list_items()
        .await
        .expect("items list")
        .into_iter()
        .find(|item| item.local_id == format!("connection:{local_id}"))
        .expect("connection snapshot exists");
    let mut record = fixture.source.export_item(&item).await.expect("export");
    record.id = "team-cloud-connection-1".to_string();
    record.team_id = Some("team-1".to_string());

    fixture
        .source
        .apply_remote(&record, None)
        .await
        .expect("team record ignored");

    let loaded = fixture
        .connections
        .get_by_cloud_id("team-cloud-connection-1")
        .expect("query by cloud id");
    assert!(loaded.is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn local_source_exports_credential_secrets_without_local_private_key_path() {
    let fixture = CredentialFixture::new();
    let mut entry = credential("deploy key", true);
    entry.username = Some("deploy".to_string());
    entry.password = Some("credential-password-unique".to_string());
    entry.private_key_path = Some("/private/local-only/id_ed25519".to_string());
    entry.private_key_content = Some("credential-private-key-unique".to_string());
    entry.passphrase = Some("credential-passphrase-unique".to_string());
    let credential_id = fixture.insert_credential(entry);
    let item = fixture
        .source
        .list_items()
        .await
        .expect("items list")
        .into_iter()
        .find(|item| item.local_id == format!("credential:{credential_id}"))
        .expect("credential snapshot exists");

    let record = fixture.source.export_item(&item).await.expect("export");
    let serialized = serde_json::to_string(&record).expect("record serializes");
    let decrypted = fixture
        .service
        .read()
        .expect("service read lock")
        .decrypt_sync_data_credential(&record)
        .expect("credential decrypts");

    assert_eq!(data_type::CREDENTIAL, record.data_type);
    assert_eq!(
        Some("credential-password-unique".to_string()),
        decrypted.password
    );
    assert_eq!(
        Some("credential-private-key-unique".to_string()),
        decrypted.private_key_content
    );
    assert_eq!(
        Some("credential-passphrase-unique".to_string()),
        decrypted.passphrase
    );
    assert_eq!(None, decrypted.private_key_path);
    for secret in [
        "credential-password-unique",
        "credential-private-key-unique",
        "credential-passphrase-unique",
        "/private/local-only/id_ed25519",
    ] {
        assert!(!serialized.contains(secret));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn local_source_downloads_credential_and_preserves_existing_local_key_path() {
    let fixture = CredentialFixture::new();
    let mut local = credential("local key", true);
    local.private_key_path = Some("/local/device/id_ed25519".to_string());
    local.password = Some("old-password".to_string());
    let credential_id = fixture.insert_credential(local);
    let local_item = fixture
        .source
        .list_items()
        .await
        .expect("items list")
        .into_iter()
        .find(|item| item.local_id == format!("credential:{credential_id}"))
        .expect("credential snapshot exists");

    let mut remote = credential("remote key", true);
    remote.username = Some("remote-user".to_string());
    remote.password = Some("new-password".to_string());
    remote.private_key_content = Some("new-private-key".to_string());
    remote.passphrase = Some("new-passphrase".to_string());
    let mut record = fixture
        .service
        .read()
        .expect("service read lock")
        .prepare_credential_sync_data_upload(&remote)
        .expect("remote record");
    record.id = "credential-cloud-update".to_string();
    record.updated_at = 42_000;

    fixture
        .source
        .apply_remote(&record, Some(&local_item))
        .await
        .expect("remote applied");

    let loaded = fixture
        .credentials
        .get(credential_id)
        .expect("credential query")
        .expect("credential exists");
    assert_eq!("remote key", loaded.name);
    assert_eq!(Some("remote-user".to_string()), loaded.username);
    assert_eq!(Some("new-password".to_string()), loaded.password);
    assert_eq!(
        Some("/local/device/id_ed25519".to_string()),
        loaded.private_key_path
    );
    assert_eq!(
        Some("new-private-key".to_string()),
        loaded.private_key_content
    );
    assert_eq!(Some("new-passphrase".to_string()), loaded.passphrase);
    assert_eq!(Some("credential-cloud-update".to_string()), loaded.cloud_id);
    assert_eq!(Some(42), loaded.last_synced_at);
}

#[tokio::test(flavor = "current_thread")]
async fn local_source_downloads_new_credential_without_local_private_key_path() {
    let fixture = CredentialFixture::new();
    let mut remote = credential("new device key", true);
    remote.username = Some("remote-user".to_string());
    remote.password = Some("remote-password".to_string());
    remote.private_key_path = Some("/source-device/id_ed25519".to_string());
    remote.private_key_content = Some("remote-private-key".to_string());
    remote.passphrase = Some("remote-passphrase".to_string());
    let mut record = fixture
        .service
        .read()
        .expect("service read lock")
        .prepare_credential_sync_data_upload(&remote)
        .expect("remote record");
    record.id = "credential-cloud-new-device".to_string();
    record.updated_at = 42_000;

    fixture
        .source
        .apply_remote(&record, None)
        .await
        .expect("remote applied");

    let loaded = fixture
        .credentials
        .get_by_cloud_id("credential-cloud-new-device")
        .expect("credential query")
        .expect("downloaded credential exists");
    assert_eq!("new device key", loaded.name);
    assert_eq!(Some("remote-user".to_string()), loaded.username);
    assert_eq!(Some("remote-password".to_string()), loaded.password);
    assert_eq!(None, loaded.private_key_path);
    assert_eq!(
        Some("remote-private-key".to_string()),
        loaded.private_key_content
    );
    assert_eq!(Some("remote-passphrase".to_string()), loaded.passphrase);
    assert!(loaded.sync_enabled);
    assert_eq!(
        Some("credential-cloud-new-device".to_string()),
        loaded.cloud_id
    );
    assert_eq!(Some(42), loaded.last_synced_at);
}

#[tokio::test(flavor = "current_thread")]
async fn local_source_does_not_reenable_disabled_credential_from_remote_history() {
    let fixture = CredentialFixture::new();
    let mut disabled = credential("disabled local", false);
    disabled.cloud_id = Some("credential-cloud-disabled".to_string());
    disabled.password = Some("keep-local-password".to_string());
    let credential_id = fixture.insert_credential(disabled);

    let mut remote = credential("remote historical", true);
    remote.password = Some("remote-password".to_string());
    let mut record = fixture
        .service
        .read()
        .expect("service read lock")
        .prepare_credential_sync_data_upload(&remote)
        .expect("remote record");
    record.id = "credential-cloud-disabled".to_string();

    fixture
        .source
        .apply_remote(&record, None)
        .await
        .expect("disabled credential skips remote history");

    let loaded = fixture
        .credentials
        .get(credential_id)
        .expect("credential query")
        .expect("credential exists");
    assert_eq!("disabled local", loaded.name);
    assert!(!loaded.sync_enabled);
    assert_eq!(Some("keep-local-password".to_string()), loaded.password);
}

#[tokio::test(flavor = "current_thread")]
async fn local_source_uses_stable_credential_cloud_ids_in_exported_connections() {
    let fixture = CredentialFixture::new();
    let mut synced = credential("synced credential", true);
    synced.cloud_id = Some("credential-cloud-stable".to_string());
    let synced_id = fixture.insert_credential(synced);
    let synced_connection =
        fixture.insert_connection_with_credential("synced connection", synced_id);

    let mut unsynced = credential("local-only credential", false);
    unsynced.cloud_id = None;
    let unsynced_id = fixture.insert_credential(unsynced);
    let unsynced_connection =
        fixture.insert_connection_with_credential("unsynced connection", unsynced_id);

    let items = fixture.source.list_items().await.expect("items list");
    let synced_record = fixture
        .source
        .export_item(
            items
                .iter()
                .find(|item| item.local_id == format!("connection:{synced_connection}"))
                .expect("synced connection snapshot"),
        )
        .await
        .expect("synced connection export");
    let unsynced_record = fixture
        .source
        .export_item(
            items
                .iter()
                .find(|item| item.local_id == format!("connection:{unsynced_connection}"))
                .expect("unsynced connection snapshot"),
        )
        .await
        .expect("unsynced connection export");

    let service = fixture.service.read().expect("service read lock");
    let synced_connection = service
        .decrypt_sync_data_connection(&synced_record)
        .expect("synced connection decrypts");
    let unsynced_connection = service
        .decrypt_sync_data_connection(&unsynced_record)
        .expect("unsynced connection decrypts");
    let synced_config: DbConnectionConfig =
        serde_json::from_str(&synced_connection.params).expect("synced params parse");
    let unsynced_config: DbConnectionConfig =
        serde_json::from_str(&unsynced_connection.params).expect("unsynced params parse");

    let synced_reference = synced_config
        .credential_reference
        .expect("synced credential reference");
    assert_eq!(synced_id, synced_reference.credential_id);
    assert_eq!(
        Some("credential-cloud-stable".to_string()),
        synced_reference.credential_cloud_id
    );
    let unsynced_reference = unsynced_config
        .credential_reference
        .expect("unsynced credential reference");
    assert_eq!(0, unsynced_reference.credential_id);
    assert_eq!(None, unsynced_reference.credential_cloud_id);
}

#[tokio::test(flavor = "current_thread")]
async fn local_source_replaces_stale_credential_cloud_id_during_connection_export() {
    let fixture = CredentialFixture::new();
    let mut credential = credential("synced credential", true);
    credential.cloud_id = Some("credential-cloud-current".to_string());
    let credential_id = fixture.insert_credential(credential);
    let connection_id =
        fixture.insert_connection_with_credential("connection with stale reference", credential_id);
    let mut connection = fixture
        .connections
        .get(connection_id)
        .expect("connection query")
        .expect("connection exists");
    let mut config = connection
        .to_db_connection()
        .expect("connection params parse");
    config
        .credential_reference
        .as_mut()
        .expect("credential reference")
        .credential_cloud_id = Some("credential-cloud-stale".to_string());
    connection.params = serde_json::to_string(&config).expect("connection params serialize");
    fixture
        .connections
        .update(&connection)
        .expect("connection updated");

    let item = fixture
        .source
        .list_items()
        .await
        .expect("items list")
        .into_iter()
        .find(|item| item.local_id == format!("connection:{connection_id}"))
        .expect("connection snapshot exists");
    let record = fixture.source.export_item(&item).await.expect("export");
    let exported_connection = fixture
        .service
        .read()
        .expect("service read lock")
        .decrypt_sync_data_connection(&record)
        .expect("connection decrypts");
    let exported_config: DbConnectionConfig =
        serde_json::from_str(&exported_connection.params).expect("exported params parse");
    let exported_reference = exported_config
        .credential_reference
        .expect("exported credential reference");

    assert_eq!(credential_id, exported_reference.credential_id);
    assert_eq!(
        Some("credential-cloud-current".to_string()),
        exported_reference.credential_cloud_id
    );
}

#[tokio::test(flavor = "current_thread")]
async fn local_source_rejects_deleting_referenced_remote_credential() {
    let fixture = CredentialFixture::new();
    let credential_id = fixture.insert_credential(credential("referenced credential", true));
    fixture.insert_connection_with_credential("referencing connection", credential_id);
    let item = fixture
        .source
        .list_items()
        .await
        .expect("items list")
        .into_iter()
        .find(|item| item.local_id == format!("credential:{credential_id}"))
        .expect("credential snapshot exists");

    let error = fixture
        .source
        .delete_item(&item)
        .await
        .expect_err("referenced credential deletion must conflict");

    assert!(error.to_string().contains("still referenced"));
    assert!(
        fixture
            .credentials
            .get_summary(credential_id)
            .expect("credential summary query")
            .is_some()
    );
}

struct Fixture {
    _temp: tempfile::TempDir,
    source: PersonalSyncLocalRepositorySource,
    connections: ConnectionRepository,
    credentials: CredentialRepository,
    workspaces: WorkspaceRepository,
    service: Arc<RwLock<CloudSyncService>>,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = SqliteConnection::open(temp.path().join("test.db")).expect("sqlite opens");
        db.with_connection(|conn| {
            run_migrations(conn)?;
            Ok(())
        })
        .expect("migrations run");
        let connections = ConnectionRepository::new(db.clone());
        let credentials = CredentialRepository::new(db.clone());
        let workspaces = WorkspaceRepository::new(db);
        let service = Arc::new(RwLock::new(CloudSyncService::new()));
        {
            let mut service = service.write().expect("service write lock");
            service.set_logged_in("personal-user".to_string());
            service.set_master_key_directly("test-master-key".to_string());
        }
        let source = PersonalSyncLocalRepositorySource::new(
            connections.clone(),
            credentials.clone(),
            workspaces.clone(),
            service.clone(),
        );
        Self {
            _temp: temp,
            source,
            connections,
            credentials,
            workspaces,
            service,
        }
    }

    fn insert_connection(&self, name: &str, team_id: Option<&str>, sync_enabled: bool) -> i64 {
        let params = DbConnectionConfig {
            id: String::new(),
            database_type: DatabaseType::SQLite,
            name: name.to_string(),
            host: ":memory:".to_string(),
            port: 0,
            username: String::new(),
            password: String::new(),
            database: Some(":memory:".to_string()),
            service_name: None,
            sid: None,
            workspace_id: None,
            proxy: None,
            extra_params: Default::default(),
            credential_reference: None,
        };
        let mut conn = StoredConnection::new_database(name.to_string(), params, None);
        conn.team_id = team_id.map(str::to_string);
        conn.sync_enabled = sync_enabled;
        self.connections
            .insert(&mut conn)
            .expect("connection insert")
    }

    fn insert_connection_with_credential(&self, name: &str, credential_id: i64) -> i64 {
        let params = DbConnectionConfig {
            id: String::new(),
            database_type: DatabaseType::SQLite,
            name: name.to_string(),
            host: ":memory:".to_string(),
            port: 0,
            username: String::new(),
            password: String::new(),
            database: Some(":memory:".to_string()),
            service_name: None,
            sid: None,
            workspace_id: None,
            proxy: None,
            extra_params: Default::default(),
            credential_reference: Some(CredentialReference::all(credential_id)),
        };
        let mut conn = StoredConnection::new_database(name.to_string(), params, None);
        self.connections
            .insert(&mut conn)
            .expect("connection insert")
    }

    fn insert_credential(&self, mut entry: CredentialEntry) -> i64 {
        self.credentials
            .insert(&mut entry)
            .expect("credential insert")
    }

    fn insert_workspace(&self, name: &str) -> i64 {
        let mut workspace = Workspace::new(name.to_string());
        self.workspaces
            .insert(&mut workspace)
            .expect("workspace insert")
    }
}

fn credential(name: &str, sync_enabled: bool) -> CredentialEntry {
    let mut entry = CredentialEntry::new(name);
    entry.sync_enabled = sync_enabled;
    entry
}

fn credential_crypto_mutex() -> &'static Mutex<()> {
    static MUTEX: OnceLock<Mutex<()>> = OnceLock::new();
    MUTEX.get_or_init(|| Mutex::new(()))
}

struct CredentialFixture {
    fixture: Fixture,
    _crypto_guard: MutexGuard<'static, ()>,
}

impl CredentialFixture {
    fn new() -> Self {
        let crypto_guard = credential_crypto_mutex()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        crypto::clear_master_key();
        crypto::set_master_key_for_session("personal-sync-local-source-test-key");
        Self {
            fixture: Fixture::new(),
            _crypto_guard: crypto_guard,
        }
    }
}

impl Deref for CredentialFixture {
    type Target = Fixture;

    fn deref(&self) -> &Self::Target {
        &self.fixture
    }
}

impl Drop for CredentialFixture {
    fn drop(&mut self) {
        crypto::clear_master_key();
    }
}
