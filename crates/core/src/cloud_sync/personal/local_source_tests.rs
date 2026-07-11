use std::sync::{Arc, RwLock};

use crate::cloud_sync::models::data_type;
use crate::cloud_sync::personal::{PersonalSyncLocalRepositorySource, PersonalSyncLocalSource};
use crate::cloud_sync::service::CloudSyncService;
use crate::storage::connection::SqliteConnection;
use crate::storage::migration::run_migrations;
use crate::storage::traits::Repository;
use crate::storage::{
    ConnectionRepository, DatabaseType, DbConnectionConfig, StoredConnection, Workspace,
    WorkspaceRepository,
};

#[tokio::test]
async fn local_source_lists_only_personal_syncable_records() {
    let fixture = Fixture::new();
    let personal = fixture.insert_connection("personal", None, true);
    fixture.insert_connection("team", Some("team-1"), true);
    fixture.insert_connection("disabled", None, false);
    let workspace = fixture.insert_workspace("workspace");

    let items = fixture.source.list_items().await.expect("items list");

    let local_ids = items
        .iter()
        .map(|item| item.local_id.as_str())
        .collect::<Vec<_>>();
    assert!(local_ids.contains(&format!("connection:{personal}").as_str()));
    assert!(local_ids.contains(&format!("workspace:{workspace}").as_str()));
    assert_eq!(2, items.len());
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

struct Fixture {
    source: PersonalSyncLocalRepositorySource,
    connections: ConnectionRepository,
    workspaces: WorkspaceRepository,
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
        let workspaces = WorkspaceRepository::new(db);
        let service = Arc::new(RwLock::new(CloudSyncService::new()));
        {
            let mut service = service.write().expect("service write lock");
            service.set_logged_in("personal-user".to_string());
            service.set_master_key_directly("test-master-key".to_string());
        }
        let source = PersonalSyncLocalRepositorySource::new(
            connections.clone(),
            workspaces.clone(),
            service,
        );
        Self {
            source,
            connections,
            workspaces,
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
        };
        let mut conn = StoredConnection::new_database(name.to_string(), params, None);
        conn.team_id = team_id.map(str::to_string);
        conn.sync_enabled = sync_enabled;
        self.connections
            .insert(&mut conn)
            .expect("connection insert")
    }

    fn insert_workspace(&self, name: &str) -> i64 {
        let mut workspace = Workspace::new(name.to_string());
        self.workspaces
            .insert(&mut workspace)
            .expect("workspace insert")
    }
}
