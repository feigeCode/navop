use crate::connections::connection_tool_registry_with_workspaces;
use crate::workspaces::workspace_tool_registry;
use one_core::storage::connection::SqliteConnection;
use one_core::storage::migration::run_migrations;
use one_core::storage::traits::Repository;
use one_core::storage::{ConnectionRepository, DatabaseType, Workspace, WorkspaceRepository};
use serde_json::json;
use std::sync::Arc;
use tool_runtime::{ToolAdapter, ToolContext};

#[test]
fn connection_registry_lists_management_tools() {
    let (connection_repo, workspace_repo) = repos();
    let registry = connection_tool_registry_with_workspaces(connection_repo, Some(workspace_repo));
    let tool_ids = registry
        .list(ToolAdapter::Mcp)
        .into_iter()
        .map(|tool| tool.id)
        .collect::<Vec<_>>();

    assert!(tool_ids.contains(&"connections.find".to_string()));
    assert!(tool_ids.contains(&"connections.save".to_string()));
    assert!(tool_ids.contains(&"connections.delete".to_string()));
    assert!(tool_ids.contains(&"connections.test".to_string()));
    assert!(!tool_ids.contains(&"connections.update".to_string()));
    assert!(!tool_ids.contains(&"connections.move_workspace".to_string()));
    assert!(!tool_ids.contains(&"connections.set_sync_enabled".to_string()));
}

#[test]
fn list_saved_connections_supports_filters_pagination_and_workspace_names() {
    let (connection_repo, workspace_repo) = repos();
    let workspace_id = insert_workspace(&workspace_repo, "DB Ops");
    let registry =
        connection_tool_registry_with_workspaces(connection_repo.clone(), Some(workspace_repo));

    super::create_connection(
        &registry,
        json!({
            "kind": "database",
            "database_type": "MSSQL",
            "workspace_id": workspace_id,
            "values": {
                "name": "prod mssql",
                "host": "10.2.178.163",
                "username": "sa",
                "password": "secret",
                "connect_timeout": 30,
                "encrypt": "off",
                "trust_cert": true
            }
        }),
    );
    super::create_connection(
        &registry,
        json!({
            "kind": "database",
            "database_type": "MySQL",
            "workspace_id": workspace_id,
            "values": {
                "name": "other mysql",
                "host": "10.2.178.163",
                "username": "app"
            }
        }),
    );

    let result = call(
        &registry,
        "connections.list",
        json!({
            "kind": "database",
            "database_type": "MSSQL",
            "workspace_id": workspace_id,
            "host": "10.2.178.163",
            "limit": 1,
            "cursor": 0,
            "include_summary": true
        }),
    );
    let connection = &result["connections"][0];

    assert_eq!(json!(1), result["total_matched"]);
    assert_eq!(json!(null), result["next_cursor"]);
    assert_eq!("prod mssql", connection["name"]);
    assert_eq!("DB Ops", connection["workspace_name"]);
    assert_eq!(
        "30",
        connection["stored_extra_params"]["connect_timeout"]
            .as_str()
            .expect("connect_timeout should be stored")
    );
    assert_eq!(json!("off"), connection["effective_values"]["encrypt"]);
    assert_eq!(json!("true"), connection["effective_values"]["trust_cert"]);
    assert_eq!(json!("<redacted>"), connection["summary"]["password"]);
}

#[test]
fn list_saved_connections_omits_summary_by_default() {
    let (connection_repo, workspace_repo) = repos();
    let registry =
        connection_tool_registry_with_workspaces(connection_repo.clone(), Some(workspace_repo));

    super::create_connection(
        &registry,
        json!({
            "kind": "ssh_sftp",
            "values": {
                "name": "prod ssh",
                "host": "10.0.1.30",
                "username": "deploy",
                "password": "secret"
            }
        }),
    );

    let result = call(&registry, "connections.list", json!({}));
    let connection = &result["connections"][0];

    assert_eq!(json!(1), result["total_matched"]);
    assert!(connection.get("summary").is_none());
}

#[test]
fn find_returns_duplicate_name_candidates_and_show_rejects_ambiguous_name() {
    let (connection_repo, workspace_repo) = repos();
    let registry =
        connection_tool_registry_with_workspaces(connection_repo.clone(), Some(workspace_repo));
    let first_id = create_mysql(&registry, "Local MySQL", "127.0.0.1");
    let second_id = create_mysql(&registry, "Local MySQL", "127.0.0.2");

    let result = call(
        &registry,
        "connections.find",
        json!({ "name": "Local MySQL", "include_summary": false }),
    );
    let ids = result["connections"]
        .as_array()
        .expect("connections should be an array")
        .iter()
        .filter_map(|connection| connection["id"].as_i64())
        .collect::<Vec<_>>();

    assert_eq!(json!(2), result["total_matched"]);
    assert!(ids.contains(&first_id));
    assert!(ids.contains(&second_id));

    let error = futures::executor::block_on(registry.call(
        "connections.show",
        json!({ "connection": "Local MySQL" }),
        ToolContext::for_adapter(ToolAdapter::Mcp),
    ))
    .expect_err("ambiguous names should be rejected");

    assert!(
        error.to_string().contains("multiple connections named"),
        "unexpected error: {error}"
    );
}

#[test]
fn update_connection_changes_fields_and_values() {
    let (connection_repo, workspace_repo) = repos();
    let registry =
        connection_tool_registry_with_workspaces(connection_repo.clone(), Some(workspace_repo));
    let id = create_mysql(&registry, "prod mysql", "10.0.1.20");

    let result = call(
        &registry,
        "connections.save",
        json!({
            "id": id,
            "patch": {
                "name": "prod mysql renamed",
                "remark": "owned by app",
                "sync_enabled": false,
                "values": {
                    "host": "10.0.1.21",
                    "password": "rotated",
                    "connect_timeout": 45
                }
            }
        }),
    );
    let stored = connection_repo
        .get(id)
        .expect("connection should be readable")
        .expect("connection should exist");
    let config = stored.to_db_connection().expect("db config should parse");

    assert_eq!(json!(true), result["ok"]);
    assert_eq!("prod mysql renamed", stored.name);
    assert_eq!(Some("owned by app"), stored.remark.as_deref());
    assert!(!stored.sync_enabled);
    assert_eq!("10.0.1.21", config.host);
    assert_eq!("rotated", config.password);
    assert_eq!(
        Some(&"45".to_string()),
        config.extra_params.get("connect_timeout")
    );
}

#[test]
fn update_connection_rejects_unknown_database_type() {
    let (connection_repo, workspace_repo) = repos();
    let registry =
        connection_tool_registry_with_workspaces(connection_repo.clone(), Some(workspace_repo));
    let id = create_mysql(&registry, "prod mysql", "10.0.1.20");

    let error = futures::executor::block_on(registry.call(
        "connections.save",
        json!({
            "id": id,
            "patch": { "database_type": "UnknownDB" }
        }),
        ToolContext::for_adapter(ToolAdapter::Mcp),
    ))
    .expect_err("unknown database type should be rejected");
    let stored = connection_repo
        .get(id)
        .expect("connection should be readable")
        .expect("connection should exist");
    let config = stored.to_db_connection().expect("db config should parse");

    assert!(
        error.to_string().contains("unknown database type"),
        "unexpected error: {error}"
    );
    assert_eq!(DatabaseType::MySQL, config.database_type);
}

#[test]
fn delete_connection_removes_saved_connection() {
    let (connection_repo, workspace_repo) = repos();
    let registry =
        connection_tool_registry_with_workspaces(connection_repo.clone(), Some(workspace_repo));
    let id = create_mysql(&registry, "throwaway mysql", "10.0.1.20");

    let result = call(&registry, "connections.delete", json!({ "id": id }));

    assert_eq!(json!(true), result["ok"]);
    assert!(
        connection_repo
            .get(id)
            .expect("connection lookup should run")
            .is_none()
    );
}

#[test]
fn save_updates_workspace_id() {
    let (connection_repo, workspace_repo) = repos();
    let target_workspace_id = insert_workspace(&workspace_repo, "Target");
    let registry = connection_tool_registry_with_workspaces(
        connection_repo.clone(),
        Some(workspace_repo.clone()),
    );
    let id = create_mysql(&registry, "movable mysql", "10.0.1.20");

    let result = call(
        &registry,
        "connections.save",
        json!({
            "id": id,
            "patch": { "workspace_id": target_workspace_id }
        }),
    );
    let stored = connection_repo
        .get(id)
        .expect("connection should be readable")
        .expect("connection should exist");

    assert_eq!(json!(true), result["ok"]);
    assert_eq!(Some(target_workspace_id), stored.workspace_id);
    assert_eq!("Target", result["connection"]["workspace_name"]);
}

#[test]
fn save_updates_sync_enabled_flag() {
    let (connection_repo, workspace_repo) = repos();
    let registry =
        connection_tool_registry_with_workspaces(connection_repo.clone(), Some(workspace_repo));
    let id = create_mysql(&registry, "sync mysql", "10.0.1.20");

    let result = call(
        &registry,
        "connections.save",
        json!({
            "id": id,
            "patch": { "sync_enabled": false }
        }),
    );
    let stored = connection_repo
        .get(id)
        .expect("connection should be readable")
        .expect("connection should exist");

    assert_eq!(json!(true), result["ok"]);
    assert!(!stored.sync_enabled);
}

#[test]
fn test_saved_sqlite_database_connection_reports_ok() {
    let (connection_repo, workspace_repo) = repos();
    let registry = connection_tool_registry_with_workspaces(connection_repo, Some(workspace_repo));
    let dir = tempfile::tempdir().expect("temp dir should create");
    let db_path = dir.path().join("probe.sqlite");
    let id = super::create_connection(
        &registry,
        json!({
            "kind": "database",
            "database_type": "SQLite",
            "values": {
                "name": "local sqlite",
                "host": db_path.to_string_lossy()
            }
        }),
    );

    let result = tokio::runtime::Runtime::new()
        .expect("tokio runtime should build")
        .block_on(registry.call(
            "connections.test",
            json!({ "connection": id.to_string() }),
            ToolContext::for_adapter(ToolAdapter::Mcp),
        ))
        .expect("test tool should run")
        .structured_content;

    assert_eq!(json!(true), result["ok"]);
    assert_eq!(json!("database"), result["kind"]);
    assert_eq!(json!("SQLite"), result["database_type"]);
}

#[test]
fn test_non_database_connection_returns_structured_failure() {
    let (connection_repo, workspace_repo) = repos();
    let registry = connection_tool_registry_with_workspaces(connection_repo, Some(workspace_repo));
    let id = super::create_connection(
        &registry,
        json!({
            "kind": "ssh_sftp",
            "values": {
                "name": "prod ssh",
                "host": "10.0.1.30",
                "username": "deploy"
            }
        }),
    );

    let result = call(
        &registry,
        "connections.test",
        json!({ "connection": id.to_string() }),
    );

    assert_eq!(json!(false), result["ok"]);
    assert_eq!(json!("unsupported_kind"), result["code"]);
}

#[test]
fn workspace_registry_lists_and_shows_workspaces() {
    let (_connection_repo, workspace_repo) = repos();
    let first_id = insert_workspace(&workspace_repo, "Personal");
    insert_workspace(&workspace_repo, "Team");
    let registry = workspace_tool_registry(workspace_repo);

    let list = call(
        &registry,
        "workspaces.list",
        json!({ "name_contains": "son", "limit": 10 }),
    );
    let show = call(
        &registry,
        "workspaces.show",
        json!({ "workspace": first_id.to_string() }),
    );

    assert_eq!(json!(1), list["total_matched"]);
    assert_eq!("Personal", list["workspaces"][0]["name"]);
    assert_eq!(json!(first_id), show["workspace"]["id"]);
    assert_eq!("Personal", show["workspace"]["name"]);
}

fn call(
    registry: &tool_runtime::ToolRegistry,
    tool: &str,
    input: serde_json::Value,
) -> serde_json::Value {
    futures::executor::block_on(registry.call(
        tool,
        input,
        ToolContext::for_adapter(ToolAdapter::Mcp),
    ))
    .expect("tool call should run")
    .structured_content
}

fn create_mysql(registry: &tool_runtime::ToolRegistry, name: &str, host: &str) -> i64 {
    super::create_connection(
        registry,
        json!({
            "kind": "database",
            "database_type": "MySQL",
            "values": {
                "name": name,
                "host": host,
                "username": "app"
            }
        }),
    )
}

fn insert_workspace(repo: &Arc<WorkspaceRepository>, name: &str) -> i64 {
    let mut workspace = Workspace::new(name.to_string());
    repo.insert(&mut workspace)
        .expect("workspace should be inserted")
}

fn repos() -> (Arc<ConnectionRepository>, Arc<WorkspaceRepository>) {
    let conn = SqliteConnection::open_with_pool_size(":memory:", 1).expect("sqlite should open");
    conn.with_connection(|db| {
        run_migrations(db)?;
        Ok(())
    })
    .expect("migrations should run");
    (
        Arc::new(ConnectionRepository::new(conn.clone())),
        Arc::new(WorkspaceRepository::new(conn)),
    )
}
