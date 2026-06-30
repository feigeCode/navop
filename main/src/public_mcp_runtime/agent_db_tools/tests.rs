use super::*;
use agent_runtime::tools::{ObservationData, ToolInvocation};
use agent_runtime::{ResourceKind, ResourceRef, ResourceScope};
use one_core::storage::connection::SqliteConnection;
use one_core::storage::migration::run_migrations;
use one_core::storage::traits::Repository;
use one_core::storage::{ConnectionRepository, DatabaseType, DbConnectionConfig, StoredConnection};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[test]
fn registry_exposes_agent_native_db_tools() {
    let mut registry = ToolRegistry::new();
    register_agent_db_tool_handlers(&mut registry, repo());

    let names = registry
        .names()
        .into_iter()
        .map(|name| name.to_string())
        .collect::<Vec<_>>();

    assert!(names.contains(&"db_query".to_string()));
    assert!(names.contains(&"db_execute_sql".to_string()));
    assert!(names.contains(&"db_list_tables".to_string()));
    let exec = registry
        .get(&ToolName::new("db_execute_sql"))
        .expect("execute tool");
    assert_eq!(
        RiskLevel::High,
        exec.spec(&ResourceContext::new()).risk,
        "dangerous SQL must require approval through high risk"
    );
}

#[tokio::test]
async fn db_query_uses_current_resource_and_database_scope() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("agent.sqlite");
    let sqlite = rusqlite::Connection::open(&db_path).expect("fixture open");
    sqlite
        .execute_batch(
            "create table users(id integer primary key, name text);
             insert into users(name) values ('Ada');",
        )
        .expect("fixture seed");
    drop(sqlite);

    let repo = repo();
    let mut connection = StoredConnection::new_database(
        "agent sqlite".to_string(),
        sqlite_config(db_path.to_string_lossy().to_string()),
        None,
    );
    repo.insert(&mut connection).expect("insert connection");
    let id = connection.id.expect("connection id").to_string();
    let tool = AgentDbTool {
        repo,
        kind: AgentDbToolKind::Query,
    };
    let observation = tool
        .execute(invocation(
            json!({"sql": "select name from users order by id"}),
            resource_context(&id),
        ))
        .await
        .expect("query succeeds");

    assert!(observation.success);
    let ObservationData::Json(value) = observation.data else {
        panic!("expected json observation")
    };
    assert_eq!(id, value["connection"]);
    assert_eq!("main", value["database"]);
    assert_eq!("Ada", value["results"][0]["rows"][0][0]);
}

fn invocation(arguments: Value, resources: ResourceContext) -> ToolInvocation {
    ToolInvocation {
        session_id: agent_runtime::SessionId::from_string("session"),
        turn_id: agent_runtime::TurnId::from_string("turn"),
        call_id: agent_runtime::ToolCallId::from_string("call"),
        tool_name: ToolName::new("db_query"),
        arguments,
        resource_id: None,
        resources,
        cancellation: CancellationToken::new(),
    }
}

fn resource_context(id: &str) -> ResourceContext {
    ResourceContext::new().with_resource(
        ResourceRef::new(id, ResourceKind::Sqlite, "agent sqlite")
            .with_scope(ResourceScope::new("database", "Database", "main")),
    )
}

fn repo() -> Arc<ConnectionRepository> {
    let conn = SqliteConnection::open_with_pool_size(":memory:", 1).expect("sqlite open");
    conn.with_connection(|db| {
        run_migrations(db)?;
        Ok(())
    })
    .expect("migrations");
    Arc::new(ConnectionRepository::new(conn))
}

fn sqlite_config(path: impl Into<String>) -> DbConnectionConfig {
    DbConnectionConfig {
        id: String::new(),
        database_type: DatabaseType::SQLite,
        name: "agent sqlite".to_string(),
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
