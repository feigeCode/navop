use super::*;
use db::{QueryColumnMeta, SqlErrorInfo};
use one_core::storage::{DatabaseType, DbConnectionConfig};

#[test]
fn list_connections_requires_explicit_permission() {
    let gateway = ExtensionDbGateway::new("ext", PermissionSet::new(["db:read:conn1"]), db_state());

    let error = gateway.list_connections().unwrap_err();

    assert_eq!("permission_denied", error.code);
}

#[test]
fn list_connections_returns_sanitized_protocol_info() {
    let mut state = db_state();
    state.register_connection(config("conn1"));
    let gateway =
        ExtensionDbGateway::new("ext", PermissionSet::new(["db:connections:list"]), state);

    let connections = gateway.list_connections().unwrap();

    assert_eq!(1, connections.len());
    assert_eq!("conn1", connections[0].id);
    assert_eq!("test", connections[0].name);
    assert_eq!("PostgreSQL", connections[0].driver);
    assert_eq!(Some("postgres".to_string()), connections[0].database);
}

#[test]
fn execute_request_denies_unpermitted_sql_access() {
    let gateway = ExtensionDbGateway::new("ext", PermissionSet::new(["db:read:conn1"]), db_state());
    let request = ExecuteSqlRequest {
        session_id: "session".to_string(),
        connection_id: "conn1".to_string(),
        sql: "delete from users".to_string(),
        options: Default::default(),
    };

    let error = futures::executor::block_on(gateway.execute(request)).unwrap_err();

    assert_eq!("permission_denied", error.code);
}

#[test]
fn schema_metadata_requires_schema_permission() {
    let gateway = ExtensionDbGateway::new("ext", PermissionSet::new(["db:read:conn1"]), db_state());

    let databases_error =
        futures::executor::block_on(gateway.list_databases("conn1".to_string())).unwrap_err();
    let schemas_error = futures::executor::block_on(
        gateway.list_schemas("conn1".to_string(), "postgres".to_string()),
    )
    .unwrap_err();

    assert_eq!("permission_denied", databases_error.code);
    assert_eq!("permission_denied", schemas_error.code);
}

#[test]
fn query_result_converts_to_row_batch() {
    let batch = sql_results_to_row_batch(vec![SqlResult::Query(QueryResult {
        sql: "select id".to_string(),
        columns: vec!["id".to_string()],
        column_meta: vec![QueryColumnMeta::new("id", "int").with_nullable(false)],
        rows: vec![vec![Some("1".to_string())], vec![None]],
        elapsed_ms: 1,
    })]);

    assert_eq!("id", batch.columns[0].name);
    assert_eq!("int", batch.columns[0].type_name);
    assert!(!batch.columns[0].nullable);
    assert_eq!(DbValue::Text("1".to_string()), batch.rows[0][0]);
    assert_eq!(DbValue::Null, batch.rows[1][0]);
}

#[test]
fn exec_and_error_results_convert_to_status_rows() {
    let batch = sql_results_to_row_batch(vec![
        SqlResult::Exec(ExecResult {
            sql: "update t set id = 1".to_string(),
            rows_affected: 2,
            elapsed_ms: 1,
            message: None,
        }),
        SqlResult::Error(SqlErrorInfo {
            sql: "bad".to_string(),
            message: "syntax error".to_string(),
        }),
    ]);

    assert_eq!(2, batch.rows.len());
    assert_eq!(DbValue::Text("ok".to_string()), batch.rows[0][1]);
    assert_eq!(DbValue::Text("error".to_string()), batch.rows[1][1]);
}

#[test]
fn gateway_implements_extension_db_host_trait() {
    fn assert_host<T: ExtensionDbHost>() {}

    assert_host::<ExtensionDbGateway>();
}

#[test]
fn host_trait_execute_rejects_foreign_session_resource() {
    let gateway =
        ExtensionDbGateway::new("ext", PermissionSet::new(["db:admin:conn1"]), db_state());
    let foreign_session = DbSessionResource::new("other-ext", "conn1", "session1");

    let error = futures::executor::block_on(ExtensionDbHost::execute(
        &gateway,
        &foreign_session,
        "select 1".to_string(),
        Default::default(),
    ))
    .unwrap_err();

    assert_eq!("permission_denied", error.code);
}

#[test]
fn host_trait_execute_rejects_closed_session_resource() {
    let gateway =
        ExtensionDbGateway::new("ext", PermissionSet::new(["db:admin:conn1"]), db_state());
    let mut session = DbSessionResource::new("ext", "conn1", "session1");
    session.close();

    let error = futures::executor::block_on(ExtensionDbHost::execute(
        &gateway,
        &session,
        "select 1".to_string(),
        Default::default(),
    ))
    .unwrap_err();

    assert_eq!("invalid_resource", error.code);
}

fn db_state() -> GlobalDbState {
    GlobalDbState::new()
}

fn config(id: &str) -> DbConnectionConfig {
    DbConnectionConfig {
        id: id.to_string(),
        database_type: DatabaseType::PostgreSQL,
        name: "test".to_string(),
        host: "localhost".to_string(),
        port: 5432,
        username: "user".to_string(),
        password: "secret".to_string(),
        database: Some("postgres".to_string()),
        service_name: None,
        sid: None,
        workspace_id: None,
        extra_params: Default::default(),
    }
}
