use super::sftp_tool_registry;
use one_core::storage::connection::SqliteConnection;
use one_core::storage::migration::run_migrations;
use one_core::storage::traits::Repository;
use one_core::storage::{ConnectionRepository, DatabaseType, DbConnectionConfig, StoredConnection};
use serde_json::json;
use std::sync::Arc;
use tool_runtime::{ToolAdapter, ToolContext};

#[test]
fn sftp_registry_exposes_list_and_read_tools() {
    let registry = sftp_tool_registry(repo());
    let tools = registry.list(ToolAdapter::Mcp);
    let ids = tools.iter().map(|tool| tool.id.clone()).collect::<Vec<_>>();

    assert!(ids.contains(&"sftp.list".to_string()));
    assert!(ids.contains(&"sftp.read".to_string()));
    assert!(ids.contains(&"sftp.write".to_string()));
    let write = tools
        .iter()
        .find(|tool| tool.id == "sftp.write")
        .expect("write tool should be registered");
    assert_eq!(
        json!(["connection", "content_base64"]),
        write.input_schema["required"]
    );
    assert!(write.description.contains("instead of ssh.remote_exec"));
}

#[test]
fn sftp_tools_reject_non_sftp_connections_before_connecting() {
    let repo = repo();
    let registry = sftp_tool_registry(repo.clone());
    let mut connection =
        StoredConnection::new_database("prod mysql".to_string(), mysql_config(), None);
    repo.insert(&mut connection)
        .expect("database connection should insert");

    let error = futures::executor::block_on(registry.call(
        "sftp.list",
        json!({ "connection": "prod mysql", "path": "/" }),
        ToolContext::for_adapter(ToolAdapter::Mcp),
    ))
    .expect_err("non-sftp connection should be rejected");

    assert!(
        error
            .to_string()
            .contains("connection is not ssh_sftp: prod mysql")
    );
}

#[test]
fn sftp_tools_resolve_connections_by_id_before_type_check() {
    let repo = repo();
    let registry = sftp_tool_registry(repo.clone());
    let mut connection =
        StoredConnection::new_database("prod mysql".to_string(), mysql_config(), None);
    repo.insert(&mut connection)
        .expect("database connection should insert");

    let error = futures::executor::block_on(registry.call(
        "sftp.list",
        json!({ "connection": connection.id.unwrap().to_string(), "path": "/" }),
        ToolContext::for_adapter(ToolAdapter::Mcp),
    ))
    .expect_err("non-sftp connection should be rejected");

    assert!(!error.to_string().contains("unknown connection"));
    assert!(error.to_string().contains("connection is not ssh_sftp"));
}

fn repo() -> Arc<ConnectionRepository> {
    let conn = SqliteConnection::open_with_pool_size(":memory:", 1).expect("sqlite should open");
    conn.with_connection(|db| {
        run_migrations(db)?;
        Ok(())
    })
    .expect("migrations should run");
    Arc::new(ConnectionRepository::new(conn))
}

fn mysql_config() -> DbConnectionConfig {
    DbConnectionConfig {
        id: String::new(),
        database_type: DatabaseType::MySQL,
        name: "prod mysql".to_string(),
        host: "127.0.0.1".to_string(),
        port: 3306,
        username: "app".to_string(),
        password: String::new(),
        database: None,
        service_name: None,
        sid: None,
        workspace_id: None,
        extra_params: Default::default(),
    }
}
