use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use super::GlobalDbState;
use crate::import_export::{
    ExportConfig, ExportProgressRequest, ImportConfig, ImportProgressRequest,
};
use crate::types::TableDataRequest;
use one_core::storage::{DatabaseType, DbConnectionConfig};

fn poll_once<T>(future: impl Future<Output = anyhow::Result<T>>) -> anyhow::Error {
    let mut future = pin!(future);
    let mut cx = Context::from_waker(Waker::noop());

    match future.as_mut().poll(&mut cx) {
        Poll::Ready(Err(error)) => error,
        Poll::Ready(Ok(_)) => panic!("runtime-bound database operation unexpectedly succeeded"),
        Poll::Pending => panic!("runtime contract must be checked before the first await"),
    }
}

#[test]
fn export_core_rejects_missing_tokio_runtime_before_connection_lookup() {
    let state = GlobalDbState::new();
    let error = poll_once(
        state.export_data_with_progress_on_tokio(ExportProgressRequest {
            connection_id: "missing-export-connection".to_string(),
            config: ExportConfig::default(),
            progress_tx: None,
        }),
    );

    assert_eq!(
        "database export requires the application Tokio runtime",
        error.to_string()
    );
}

#[test]
fn import_core_rejects_missing_tokio_runtime_before_connection_lookup() {
    let state = GlobalDbState::new();
    let error = poll_once(
        state.import_data_with_progress_on_tokio(ImportProgressRequest {
            connection_id: "missing-import-connection".to_string(),
            config: ImportConfig::default(),
            data: String::new(),
            file_name: String::new(),
            progress_tx: None,
        }),
    );

    assert_eq!(
        "database import requires the application Tokio runtime",
        error.to_string()
    );
}

#[test]
fn create_session_direct_rejects_missing_tokio_runtime_before_connection_lookup() {
    let state = GlobalDbState::new();
    let error =
        poll_once(state.create_session_direct("missing-session-connection".to_string(), None));

    assert_eq!(
        "database session creation requires the application Tokio runtime",
        error.to_string()
    );
}

#[test]
fn list_databases_direct_rejects_missing_tokio_runtime_before_connection_lookup() {
    let state = GlobalDbState::new();
    let error = poll_once(state.list_databases_direct("missing-database-connection".to_string()));

    assert_eq!(
        "database metadata query requires the application Tokio runtime",
        error.to_string()
    );
}

#[test]
fn list_schemas_direct_rejects_missing_tokio_runtime_before_connection_lookup() {
    let state = GlobalDbState::new();
    let error = poll_once(state.list_schemas_direct(
        "missing-schema-connection".to_string(),
        "missing-database".to_string(),
    ));

    assert_eq!(
        "database metadata query requires the application Tokio runtime",
        error.to_string()
    );
}

#[test]
fn execute_session_rejects_missing_tokio_runtime_before_session_lookup() {
    let state = GlobalDbState::new();
    let error = poll_once(state.execute_session(
        "missing-session".to_string(),
        "select 1".to_string(),
        None,
    ));

    assert_eq!(
        "database session execution requires the application Tokio runtime",
        error.to_string()
    );
}

#[test]
fn query_table_data_direct_rejects_missing_tokio_runtime_before_connection_lookup() {
    let state = GlobalDbState::new();
    let error = poll_once(
        state.query_table_data_direct(
            "missing-table-data-connection",
            TableDataRequest::new("main", "users"),
        ),
    );

    assert_eq!(
        "database table data query requires the application Tokio runtime",
        error.to_string()
    );
}

#[test]
fn close_session_direct_rejects_missing_tokio_runtime_before_session_lookup() {
    let state = GlobalDbState::new();
    let error = poll_once(state.close_session_direct("missing-session"));

    assert_eq!(
        "database session close requires the application Tokio runtime",
        error.to_string()
    );
}

#[tokio::test]
async fn query_table_data_direct_reads_paginated_sqlite_rows() {
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let database_path = temp_dir.path().join("nsh-table-data.db");
    let mut state = GlobalDbState::new();
    state.register_connection(DbConnectionConfig {
        id: "nsh-sqlite".to_string(),
        database_type: DatabaseType::SQLite,
        name: "nsh sqlite".to_string(),
        host: database_path.to_string_lossy().to_string(),
        port: 0,
        username: String::new(),
        password: String::new(),
        database: None,
        service_name: None,
        sid: None,
        workspace_id: None,
        proxy: None,
        credential_reference: None,
        extra_params: Default::default(),
    });

    let setup = state
        .execute_script_direct(
            "nsh-sqlite",
            "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
             INSERT INTO people VALUES (1, 'Ada'), (2, 'Grace'), (3, 'Linus');",
            Some("main".to_string()),
            None,
            None,
        )
        .await
        .expect("SQLite fixture should be created");
    assert!(!setup.iter().any(|result| result.is_error()));

    let response = state
        .query_table_data_direct(
            "nsh-sqlite",
            TableDataRequest::new("main", "people")
                .with_page(2, 2)
                .with_order_by_clause("id"),
        )
        .await
        .expect("direct table query should succeed");

    assert_eq!(response.total_count, 3);
    assert_eq!(response.page, 2);
    assert_eq!(response.page_size, 2);
    assert_eq!(response.query_result.columns, ["__rowid__", "id", "name"]);
    assert_eq!(response.query_result.rows.len(), 1);
    assert_eq!(response.query_result.rows[0][2].as_deref(), Some("Linus"));
}
