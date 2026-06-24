use super::connection_tool_registry;
use one_core::storage::connection::SqliteConnection;
use one_core::storage::migration::run_migrations;
use one_core::storage::traits::Repository;
use one_core::storage::{ConnectionRepository, DatabaseType};
use serde_json::json;
use std::sync::Arc;
use tool_runtime::{ToolAdapter, ToolContext};

mod create_extended;

#[test]
fn connection_registry_lists_creation_tools() {
    let registry = connection_tool_registry(repo());
    let tool_ids = registry
        .list(ToolAdapter::Mcp)
        .into_iter()
        .map(|tool| tool.id)
        .collect::<Vec<_>>();

    assert!(tool_ids.contains(&"onetcli.connections.list_kinds".to_string()));
    assert!(tool_ids.contains(&"onetcli.connections.get_schema".to_string()));
    assert!(tool_ids.contains(&"onetcli.connections.validate".to_string()));
    assert!(tool_ids.contains(&"onetcli.connections.create".to_string()));
}

#[test]
fn connection_registry_exposes_creation_tools_to_cli() {
    let registry = connection_tool_registry(repo());
    let tool_ids = registry
        .list(ToolAdapter::Cli)
        .into_iter()
        .map(|tool| tool.id)
        .collect::<Vec<_>>();

    assert!(tool_ids.contains(&"onetcli.connections.list".to_string()));
    assert!(tool_ids.contains(&"onetcli.connections.show".to_string()));
    assert!(tool_ids.contains(&"onetcli.connections.list_kinds".to_string()));
    assert!(tool_ids.contains(&"onetcli.connections.create".to_string()));
}

#[test]
fn list_saved_connections_returns_redacted_summaries() {
    let repo = repo();
    let registry = connection_tool_registry(repo);
    create_connection(
        &registry,
        json!({
            "kind": "database",
            "database_type": "MySQL",
            "values": {
                "name": "prod mysql",
                "host": "10.0.1.20",
                "username": "app",
                "password": "secret"
            }
        }),
    );

    let result = futures::executor::block_on(registry.call(
        "onetcli.connections.list",
        json!({}),
        ToolContext::for_adapter(ToolAdapter::Cli),
    ))
    .expect("list saved connections should run");

    assert_eq!(
        "prod mysql",
        result.structured_content["connections"][0]["name"]
    );
    assert_eq!(
        "<redacted>",
        result.structured_content["connections"][0]["summary"]["password"]
    );
}

#[test]
fn show_saved_connection_supports_id_and_name() {
    let repo = repo();
    let registry = connection_tool_registry(repo);
    let id = create_connection(
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

    let by_id = futures::executor::block_on(registry.call(
        "onetcli.connections.show",
        json!({ "connection": id.to_string() }),
        ToolContext::for_adapter(ToolAdapter::Cli),
    ))
    .expect("show by id should run");
    let by_name = futures::executor::block_on(registry.call(
        "onetcli.connections.show",
        json!({ "connection": "prod ssh" }),
        ToolContext::for_adapter(ToolAdapter::Cli),
    ))
    .expect("show by name should run");

    assert_eq!(id, by_id.structured_content["connection"]["id"]);
    assert_eq!(by_id.structured_content, by_name.structured_content);
}

#[test]
fn list_kinds_includes_all_creatable_connection_types() {
    let registry = connection_tool_registry(repo());

    let result = futures::executor::block_on(registry.call(
        "onetcli.connections.list_kinds",
        json!({}),
        ToolContext::for_adapter(ToolAdapter::Mcp),
    ))
    .expect("list kinds should run");

    let kinds = result.structured_content["kinds"]
        .as_array()
        .expect("kinds should be an array")
        .iter()
        .filter_map(|kind| kind["kind"].as_str())
        .collect::<Vec<_>>();

    assert_eq!(creatable_kinds(), kinds);
}

#[test]
fn get_schema_supports_all_creatable_connection_types() {
    let registry = connection_tool_registry(repo());

    for kind in creatable_kinds() {
        let result = futures::executor::block_on(registry.call(
            "onetcli.connections.get_schema",
            json!({ "kind": kind }),
            ToolContext::for_adapter(ToolAdapter::Mcp),
        ))
        .expect("schema tool should run");

        assert_eq!(kind, result.structured_content["kind"]);
        assert_eq!(json!(1), result.structured_content["schema_version"]);
        assert!(
            result.structured_content["fields"]
                .as_array()
                .is_some_and(|fields| !fields.is_empty())
        );
    }
}

#[test]
fn create_database_connection_persists_mysql_config() {
    let repo = repo();
    let registry = connection_tool_registry(repo.clone());

    let result = futures::executor::block_on(registry.call(
        "onetcli.connections.create",
        json!({
            "kind": "database",
            "database_type": "MySQL",
            "values": {
                "name": "prod mysql",
                "host": "10.0.1.20",
                "port": 3306,
                "username": "app",
                "password": "secret",
                "database": "ai_app"
            }
        }),
        ToolContext::for_adapter(ToolAdapter::Mcp),
    ))
    .expect("create tool should run");

    assert_eq!(json!(true), result.structured_content["ok"]);
    assert_eq!("database", result.structured_content["connection"]["kind"]);
    let id = result.structured_content["connection"]["id"]
        .as_i64()
        .expect("created id should be returned");
    let stored = repo
        .get(id)
        .expect("connection should be readable")
        .expect("connection should exist");
    let db = stored.to_db_connection().expect("params should parse");

    assert_eq!("prod mysql", stored.name);
    assert_eq!(DatabaseType::MySQL, db.database_type);
    assert_eq!("10.0.1.20", db.host);
    assert_eq!(3306, db.port);
    assert_eq!("app", db.username);
    assert_eq!("secret", db.password);
    assert_eq!(Some("ai_app"), db.database.as_deref());
    assert_eq!(
        "<redacted>",
        result.structured_content["connection"]["summary"]["password"]
    );
}

#[test]
fn validate_reports_missing_required_fields_without_writing() {
    let repo = repo();
    let registry = connection_tool_registry(repo.clone());

    let result = futures::executor::block_on(registry.call(
        "onetcli.connections.validate",
        json!({
            "kind": "database",
            "database_type": "MySQL",
            "values": {
                "host": "10.0.1.20"
            }
        }),
        ToolContext::for_adapter(ToolAdapter::Mcp),
    ))
    .expect("validate tool should run");

    assert_eq!(json!(false), result.structured_content["ok"]);
    assert_eq!(json!(false), result.structured_content["can_apply"]);
    assert_eq!(json!(0), repo.count().expect("count should run"));
    assert_eq!(
        json!(["name", "username"]),
        result.structured_content["missing_required"]
    );
}

#[test]
fn validate_rejects_invalid_numeric_fields_without_writing() {
    let repo = repo();
    let registry = connection_tool_registry(repo.clone());

    let result = futures::executor::block_on(registry.call(
        "onetcli.connections.create",
        json!({
            "kind": "database",
            "database_type": "MySQL",
            "values": {
                "name": "bad mysql",
                "host": "10.0.1.20",
                "port": 70000,
                "username": "app"
            }
        }),
        ToolContext::for_adapter(ToolAdapter::Mcp),
    ))
    .expect("create tool should return validation output");

    assert_eq!(json!(false), result.structured_content["ok"]);
    assert_eq!(json!(false), result.structured_content["can_apply"]);
    assert_eq!(json!(0), repo.count().expect("count should run"));
    assert_eq!(
        json!([{
            "field": "port",
            "message": "must be an integer between 0 and 65535"
        }]),
        result.structured_content["invalid_fields"]
    );
}

pub(super) fn create_connection(
    registry: &tool_runtime::ToolRegistry,
    input: serde_json::Value,
) -> i64 {
    let result = futures::executor::block_on(registry.call(
        "onetcli.connections.create",
        input,
        ToolContext::for_adapter(ToolAdapter::Mcp),
    ))
    .expect("create tool should run");

    assert_eq!(json!(true), result.structured_content["ok"]);
    result.structured_content["connection"]["id"]
        .as_i64()
        .expect("created id should be returned")
}

fn creatable_kinds() -> Vec<&'static str> {
    vec![
        "database",
        "ssh_sftp",
        "redis",
        "mongodb",
        "serial",
        "port_forwarding",
        "rdp",
        "vnc",
    ]
}

pub(super) fn repo() -> Arc<ConnectionRepository> {
    let conn = SqliteConnection::open_with_pool_size(":memory:", 1).expect("sqlite should open");
    conn.with_connection(|db| {
        run_migrations(db)?;
        Ok(())
    })
    .expect("migrations should run");
    Arc::new(ConnectionRepository::new(conn))
}
