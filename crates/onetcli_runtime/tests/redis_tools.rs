use one_core::storage::connection::SqliteConnection;
use one_core::storage::migration::run_migrations;
use one_core::storage::traits::Repository;
use one_core::storage::{ConnectionRepository, DatabaseType, DbConnectionConfig, StoredConnection};
use onetcli_cli::{OutputFormat, ToolCommand};
use onetcli_runtime::cli_host::run_tool_command;
use serde_json::json;
use std::sync::Arc;
use tool_runtime::ToolAdapter;

#[test]
fn onetcli_tool_registry_exposes_redis_execute_command_to_cli() {
    let registry = onetcli_runtime::tool_registry_with_version(repo(), "test")
        .expect("tool registry should build");

    let tool = registry
        .get("redis.execute_command", ToolAdapter::FunctionCalling)
        .expect("redis execute command should be CLI-callable");

    assert_eq!(
        json!(["connection", "command"]),
        tool.input_schema["required"]
    );
    assert!(!tool.annotations.read_only);
    assert!(tool.annotations.destructive);
}

#[test]
fn onetcli_tool_call_requires_allow_write_for_redis_commands() {
    let registry = onetcli_runtime::tool_registry_with_version(repo(), "test")
        .expect("tool registry should build");

    let error = run_tool_command(
        ToolCommand::Call {
            tool_id: "redis.execute_command".to_string(),
            input: Some(
                json!({
                    "connection": "prod mysql",
                    "command": "PING"
                })
                .to_string(),
            ),
            positional_input: None,
            allow_write: false,
            format: OutputFormat::Json,
        },
        registry,
    )
    .expect_err("redis command should require explicit write permission");

    assert!(error.to_string().contains("write_not_allowed"));
}

#[test]
fn onetcli_tool_call_resolves_saved_connection_before_executing_redis_command() {
    let repo = repo();
    insert_mysql(&repo);
    let registry = onetcli_runtime::tool_registry_with_version(repo, "test")
        .expect("tool registry should build");

    let error = run_tool_command(
        ToolCommand::Call {
            tool_id: "redis.execute_command".to_string(),
            input: Some(
                json!({
                    "connection": "prod mysql",
                    "command": "PING"
                })
                .to_string(),
            ),
            positional_input: None,
            allow_write: true,
            format: OutputFormat::Json,
        },
        registry,
    )
    .expect_err("non-redis connection should be rejected before connecting");

    assert!(error.to_string().contains("connection is not redis"));
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

fn insert_mysql(repo: &ConnectionRepository) {
    let mut connection = StoredConnection::new_database("prod mysql".to_string(), mysql(), None);
    repo.insert(&mut connection)
        .expect("database connection should insert");
}

fn mysql() -> DbConnectionConfig {
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
