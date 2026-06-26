use one_core::storage::connection::SqliteConnection;
use one_core::storage::migration::run_migrations;
use one_core::storage::traits::Repository;
use one_core::storage::{
    ConnectionRepository, DatabaseType, DbConnectionConfig, SshAuthMethod, SshParams,
    StoredConnection,
};
use serde_json::json;
use std::sync::Arc;
use tool_runtime::{ToolAdapter, ToolContext};

#[test]
fn database_tool_registry_exposes_schema_query_and_exec_tools() {
    let registry = onetcli_runtime::database_tools::database_tool_registry(repo());
    let tools = registry.list(ToolAdapter::Mcp);
    let ids = tools.iter().map(|tool| tool.id.clone()).collect::<Vec<_>>();

    assert!(ids.contains(&"db.schema".to_string()));
    assert!(ids.contains(&"db.query".to_string()));
    assert!(ids.contains(&"db.exec".to_string()));

    let query = tools
        .iter()
        .find(|tool| tool.id == "db.query")
        .expect("query tool should be registered");
    assert_eq!(json!(["connection", "sql"]), query.input_schema["required"]);
    assert!(query.annotations.read_only);
    assert!(!query.annotations.destructive);

    let exec = tools
        .iter()
        .find(|tool| tool.id == "db.exec")
        .expect("exec tool should be registered");
    assert_eq!(json!(["connection"]), exec.input_schema["required"]);
    assert!(!exec.annotations.read_only);
    assert!(exec.annotations.destructive);
}

#[test]
fn database_tools_reject_non_database_connections_before_connecting() {
    let repo = repo();
    let registry = onetcli_runtime::database_tools::database_tool_registry(repo.clone());
    let mut connection = StoredConnection::new_ssh("prod ssh".to_string(), ssh_params(), None);
    repo.insert(&mut connection)
        .expect("ssh connection should insert");

    let error = futures::executor::block_on(registry.call(
        "db.query",
        json!({ "connection": "prod ssh", "sql": "select 1" }),
        ToolContext::for_adapter(ToolAdapter::Mcp),
    ))
    .expect_err("non-database connection should be rejected");

    assert!(
        error
            .to_string()
            .contains("connection is not database: prod ssh")
    );
}

#[test]
fn database_query_executes_saved_sqlite_connection() {
    let dir = tempfile::tempdir().expect("tempdir should be created");
    let db_path = dir.path().join("query.sqlite");
    let sqlite = rusqlite::Connection::open(&db_path).expect("sqlite fixture should open");
    sqlite
        .execute_batch("create table users(id integer primary key, name text); insert into users(name) values ('Ada');")
        .expect("sqlite fixture should be seeded");
    drop(sqlite);

    let repo = repo();
    let mut connection = StoredConnection::new_database(
        "local sqlite".to_string(),
        sqlite_config(db_path.to_string_lossy().to_string()),
        None,
    );
    repo.insert(&mut connection)
        .expect("sqlite connection should insert");
    let registry = onetcli_runtime::database_tools::database_tool_registry(repo);

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime should start");
    let result = runtime
        .block_on(registry.call(
            "db.query",
            json!({
                "connection": "local sqlite",
                "sql": "select name from users order by id"
            }),
            ToolContext::for_adapter(ToolAdapter::Mcp),
        ))
        .expect("sqlite query should execute");

    assert_eq!("local sqlite", result.structured_content["connection"]);
    assert_eq!(
        json!(["name"]),
        result.structured_content["results"][0]["columns"]
    );
    assert_eq!(
        json!([[Some("Ada".to_string())]]),
        result.structured_content["results"][0]["rows"]
    );
}

#[test]
fn database_query_rejects_write_sql_before_connecting() {
    let repo = repo();
    let mut connection = StoredConnection::new_database(
        "local sqlite".to_string(),
        sqlite_config("/tmp/nonexistent-onetcli-query-test.sqlite".to_string()),
        None,
    );
    repo.insert(&mut connection)
        .expect("sqlite connection should insert");
    let registry = onetcli_runtime::database_tools::database_tool_registry(repo);

    let error = futures::executor::block_on(registry.call(
        "db.query",
        json!({
            "connection": "local sqlite",
            "sql": "create table unsafe_write(id integer)"
        }),
        ToolContext::for_adapter(ToolAdapter::Mcp),
    ))
    .expect_err("db.query should reject write SQL before connecting");

    assert!(
        error
            .to_string()
            .contains("db.query only accepts query statements")
    );
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

fn ssh_params() -> SshParams {
    SshParams {
        host: "127.0.0.1".to_string(),
        port: 22,
        username: "app".to_string(),
        auth_method: SshAuthMethod::AutoPublicKey,
        connect_timeout: None,
        keepalive_interval: None,
        keepalive_max: None,
        default_directory: None,
        init_script: None,
        disable_shell_integration: None,
        jump_server: None,
        proxy: None,
    }
}

#[allow(dead_code)]
fn sqlite_config(path: impl Into<String>) -> DbConnectionConfig {
    DbConnectionConfig {
        id: String::new(),
        database_type: DatabaseType::SQLite,
        name: "local sqlite".to_string(),
        host: path.into(),
        port: 0,
        username: String::new(),
        password: String::new(),
        database: None,
        service_name: None,
        sid: None,
        workspace_id: None,
        extra_params: Default::default(),
    }
}
