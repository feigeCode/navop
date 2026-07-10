use one_core::storage::connection::SqliteConnection;
use one_core::storage::migration::run_migrations;
use one_core::storage::traits::Repository;
use one_core::storage::{ConnectionRepository, DatabaseType, DbConnectionConfig, StoredConnection};
use onetcli_cli::{OutputFormat, ToolCommand};
use onetcli_runtime::cli_host::run_tool_command;
use serde_json::json;
use std::sync::Arc;
use tool_runtime::{ResourceKind, ToolAdapter};

#[test]
fn onetcli_tool_registry_exposes_redis_command_to_cli() {
    let registry = onetcli_runtime::tool_registry_with_version(repo(), "test")
        .expect("tool registry should build");

    let tool = registry
        .get("redis.command", ToolAdapter::FunctionCalling)
        .expect("redis.command should be CLI-callable");

    assert_eq!(
        json!(["connection", "command"]),
        tool.input_schema["required"]
    );
    assert_eq!("redis.command", tool.id);
    assert!(!tool.annotations.read_only);
    assert!(tool.annotations.destructive);
}

#[test]
fn onetcli_tool_registry_rejects_redis_execute_command_alias() {
    let registry = onetcli_runtime::tool_registry_with_version(repo(), "test")
        .expect("tool registry should build");

    assert!(
        registry
            .get("redis.execute_command", ToolAdapter::FunctionCalling)
            .is_none()
    );
}

#[test]
fn onetcli_tool_registry_exposes_redis_read_and_write_convenience_tools() {
    let registry = onetcli_runtime::tool_registry_with_version(repo(), "test")
        .expect("tool registry should build");

    let keys = registry
        .get("redis.keys", ToolAdapter::FunctionCalling)
        .expect("redis.keys should be CLI-callable");
    let get = registry
        .get("redis.get", ToolAdapter::FunctionCalling)
        .expect("redis.get should be CLI-callable");
    let set = registry
        .get("redis.set", ToolAdapter::FunctionCalling)
        .expect("redis.set should be CLI-callable");

    assert_eq!(
        json!(["connection", "pattern"]),
        keys.input_schema["required"]
    );
    assert!(keys.annotations.read_only);
    assert!(!keys.annotations.destructive);
    assert_eq!(json!(["connection", "key"]), get.input_schema["required"]);
    assert!(get.annotations.read_only);
    assert!(!get.annotations.destructive);
    assert_eq!(
        json!(["connection", "key", "value"]),
        set.input_schema["required"]
    );
    assert!(!set.annotations.read_only);
    assert!(set.annotations.destructive);
}

#[test]
fn onetcli_redis_tools_target_saved_redis_resources() {
    let registry = onetcli_runtime::tool_registry_with_version(repo(), "test")
        .expect("tool registry should build");

    for tool_id in ["redis.command", "redis.keys", "redis.get", "redis.set"] {
        let tool = registry
            .get_runtime(tool_id, ToolAdapter::FunctionCalling)
            .expect("redis tool should be registered");
        assert_eq!(vec![ResourceKind::Redis], tool.target.supported_kinds);
        assert!(tool.target.required, "{tool_id} should require target");
    }
}

#[test]
fn onetcli_tool_call_requires_allow_write_for_redis_commands() {
    let registry = onetcli_runtime::tool_registry_with_version(repo(), "test")
        .expect("tool registry should build");

    let error = run_tool_command(
        ToolCommand::Call {
            tool_id: "redis.command".to_string(),
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
fn onetcli_tool_call_allows_redis_read_convenience_tools_without_allow_write() {
    let registry = onetcli_runtime::tool_registry_with_version(repo(), "test")
        .expect("tool registry should build");

    let error = run_tool_command(
        ToolCommand::Call {
            tool_id: "redis.get".to_string(),
            input: Some(
                json!({
                    "connection": "missing redis",
                    "key": "user:1"
                })
                .to_string(),
            ),
            positional_input: None,
            allow_write: false,
            format: OutputFormat::Json,
        },
        registry,
    )
    .expect_err("read redis tool should reach connection resolution without allow-write");

    assert!(error.to_string().contains("unknown Redis connection"));
}

#[test]
fn onetcli_tool_call_requires_allow_write_for_redis_set() {
    let registry = onetcli_runtime::tool_registry_with_version(repo(), "test")
        .expect("tool registry should build");

    let error = run_tool_command(
        ToolCommand::Call {
            tool_id: "redis.set".to_string(),
            input: Some(
                json!({
                    "connection": "prod redis",
                    "key": "user:1",
                    "value": "Ada"
                })
                .to_string(),
            ),
            positional_input: None,
            allow_write: false,
            format: OutputFormat::Json,
        },
        registry,
    )
    .expect_err("redis.set should require explicit write permission");

    assert!(error.to_string().contains("write_not_allowed"));
}

#[test]
fn onetcli_tool_call_rejects_redis_execute_command_alias() {
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
            allow_write: true,
            format: OutputFormat::Json,
        },
        registry,
    )
    .expect_err("legacy redis alias should be unknown");

    assert!(error.to_string().contains("unknown_tool"));
}

#[test]
fn onetcli_tool_call_resolves_saved_connection_before_executing_redis_command() {
    let repo = repo();
    insert_mysql(&repo);
    let registry = onetcli_runtime::tool_registry_with_version(repo, "test")
        .expect("tool registry should build");

    let error = run_tool_command(
        ToolCommand::Call {
            tool_id: "redis.command".to_string(),
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
        proxy: None,
        extra_params: Default::default(),
    }
}
