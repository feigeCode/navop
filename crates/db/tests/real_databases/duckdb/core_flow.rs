use std::path::Path;

use db::connection::DbConnection;
use db::duckdb::{DuckDbConnection, DuckDbPlugin};
use db::executor::{ExecOptions, SqlResult};
use db::plugin::DatabasePlugin;
use one_core::storage::{DatabaseType, DbConnectionConfig};

use crate::real_databases::common::assertions::{
    assert_binary, assert_cell, assert_columns, assert_no_sql_errors, assert_null,
};
use crate::real_databases::common::fixture::file_config;

const FIXTURE_SQL: &str = r#"
CREATE TABLE all_types (
    id INTEGER PRIMARY KEY,
    huge_value HUGEINT NOT NULL,
    decimal_value DECIMAL(18, 4) NOT NULL,
    double_value DOUBLE NOT NULL,
    boolean_value BOOLEAN NOT NULL,
    text_value VARCHAR NOT NULL,
    blob_value BLOB NOT NULL,
    date_value DATE,
    time_value TIME,
    timestamp_value TIMESTAMP,
    json_value JSON
);
INSERT INTO all_types VALUES (
    1, -170141183460469231731687303715884105727, 123456.7890, 3.14159, true,
    '中文 🚀 O''Reilly', '\x00\x01\x02\xff'::BLOB, '2026-08-22', '12:34:56.789',
    '2026-08-22 12:34:56.789', '{"key":"value","n":1}'
);
INSERT INTO all_types (id, huge_value, decimal_value, double_value, boolean_value, text_value, blob_value)
VALUES (2, 0, 0, 0, false, '', ''::BLOB);
"#;

fn config(path: &Path) -> DbConnectionConfig {
    file_config("duckdb-real-core", DatabaseType::DuckDB, path)
}

#[tokio::test]
async fn duckdb_real_script_query_error_transaction_and_metadata_flow() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let path = temp_dir.path().join("core.duckdb");
    let plugin = DuckDbPlugin::new();
    let mut connection = DuckDbConnection::new(config(&path));
    connection.connect().await.expect("DuckDB should connect");

    run_fixture(&plugin, &connection).await;
    assert_full_type_query(&connection).await;
    assert_error_and_transaction(&plugin, &connection).await;
    assert_metadata(&plugin, &connection).await;
    connection.disconnect().await.expect("disconnect");
}

async fn run_fixture(plugin: &DuckDbPlugin, connection: &DuckDbConnection) {
    let results = connection
        .execute(plugin, FIXTURE_SQL, ExecOptions::default())
        .await
        .expect("fixture");
    assert_no_sql_errors(&results, "DuckDB fixture");
}

async fn assert_full_type_query(connection: &DuckDbConnection) {
    let result = connection
        .query("SELECT * FROM all_types ORDER BY id")
        .await
        .expect("query");
    let SqlResult::Query(result) = result else {
        panic!("query result")
    };
    assert_columns(
        &result,
        &[
            "id",
            "huge_value",
            "decimal_value",
            "double_value",
            "boolean_value",
            "text_value",
            "blob_value",
            "date_value",
            "time_value",
            "timestamp_value",
            "json_value",
        ],
    );
    assert_eq!(result.rows.len(), 2);
    assert_cell(&result, 0, 1, "-170141183460469231731687303715884105727");
    assert_cell(&result, 0, 2, "123456.7890");
    assert_cell(&result, 0, 3, "3.14159");
    assert_cell(&result, 0, 4, "true");
    assert_cell(&result, 0, 5, "中文 🚀 O'Reilly");
    assert!(matches!(
        result.typed_view().expect("view").cell(0, 7),
        Some(db::executor::QueryCellRef::Text(_))
    ));
    assert_null(&result, 1, 7);
    assert_binary(&result, 0, 6, &[0, 1, 2, 255]);
    assert_binary(&result, 1, 6, &[]);
}

async fn assert_error_and_transaction(plugin: &DuckDbPlugin, connection: &DuckDbConnection) {
    let error = connection
        .query("SELECT * FROM missing")
        .await
        .expect("error query");
    assert!(error.is_error());

    let results = connection
        .execute(
            plugin,
            "INSERT INTO all_types (id) VALUES (3); SELECT broken",
            ExecOptions {
                stop_on_error: true,
                transactional: true,
                max_rows: Some(10),
                streaming: false,
            },
        )
        .await
        .expect("script");
    assert!(results.iter().any(|result| result.is_error()));
    let count = scalar_count(connection).await;
    assert_eq!(count, 2, "failed transaction should roll back");
}

async fn scalar_count(connection: &DuckDbConnection) -> usize {
    let result = connection
        .query("SELECT COUNT(*) FROM all_types")
        .await
        .expect("count");
    let SqlResult::Query(result) = result else {
        panic!("count result")
    };
    result.rows[0][0]
        .as_deref()
        .unwrap_or_default()
        .parse()
        .unwrap_or_default()
}

async fn assert_metadata(plugin: &DuckDbPlugin, connection: &DuckDbConnection) {
    let databases = plugin.list_databases(connection).await.expect("databases");
    assert_eq!(databases, vec!["main".to_string()]);
    let tables = plugin
        .list_tables(connection, "main", None)
        .await
        .expect("tables");
    assert!(tables.iter().any(|table| table.name == "all_types"));
    let columns = plugin
        .list_columns(connection, "main", None, "all_types")
        .await
        .expect("columns");
    assert!(columns.len() >= 11);
    assert!(
        columns
            .iter()
            .any(|column| column.name == "json_value" && column.data_type == "JSON")
    );
    assert!(
        columns
            .iter()
            .any(|column| column.name == "id" && column.is_primary_key)
    );
}
