use std::path::Path;

use db::connection::DbConnection;
use db::executor::{ExecOptions, SqlResult};
use db::plugin::DatabasePlugin;
use db::sqlite::{SqliteDbConnection, SqlitePlugin};
use one_core::storage::{DatabaseType, DbConnectionConfig};

use crate::real_databases::common::assertions::{
    assert_binary, assert_cell, assert_columns, assert_no_sql_errors, assert_null,
};
use crate::real_databases::common::fixture::file_config;

const FIXTURE_SQL: &str = r#"
DROP TABLE IF EXISTS all_types;
CREATE TABLE all_types (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    integer_value INTEGER NOT NULL,
    real_value REAL NOT NULL,
    text_value TEXT COLLATE NOCASE NOT NULL,
    blob_value BLOB NOT NULL,
    numeric_value NUMERIC NOT NULL,
    boolean_value BOOLEAN NOT NULL,
    date_value DATE,
    time_value TIME,
    datetime_value DATETIME
);
INSERT INTO all_types VALUES (
    1, -9223372036854775808, 3.14159, '中文 🚀 O''Reilly\nline 2',
    X'000102FF', 12345.6789, 1, '2026-08-22', '12:34:56.789',
    '2026-08-22 12:34:56.789'
);
INSERT INTO all_types (id, integer_value, real_value, text_value, blob_value, numeric_value, boolean_value)
VALUES (2, 0, 0, '', X'', 0, 0);
INSERT INTO all_types (id, integer_value, real_value, text_value, blob_value, numeric_value, boolean_value)
VALUES (3, 9223372036854775807, -1.25, 'NULL', X'FF', 1, 0);
"#;

fn config(path: &Path) -> DbConnectionConfig {
    file_config("sqlite-real-core", DatabaseType::SQLite, path)
}

#[tokio::test]
async fn sqlite_real_script_query_error_transaction_and_metadata_flow() {
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let path = temp_dir.path().join("core.db");
    let plugin = SqlitePlugin::new();
    let mut connection = SqliteDbConnection::new(config(&path));
    connection.connect().await.expect("SQLite should connect");

    run_fixture(&plugin, &connection).await;
    assert_full_type_query(&plugin, &connection).await;
    assert_error_and_transaction(&plugin, &connection).await;
    assert_metadata(&plugin, &connection).await;
    connection
        .disconnect()
        .await
        .expect("disconnect should work");
}

async fn run_fixture(plugin: &SqlitePlugin, connection: &SqliteDbConnection) {
    let results = connection
        .execute(plugin, FIXTURE_SQL, ExecOptions::default())
        .await
        .expect("fixture should execute");
    assert_no_sql_errors(&results, "SQLite fixture");
}

async fn assert_full_type_query(plugin: &SqlitePlugin, connection: &SqliteDbConnection) {
    let result = connection
        .query("SELECT id, integer_value, real_value, text_value, blob_value, numeric_value, boolean_value, date_value, time_value, datetime_value FROM all_types ORDER BY id")
        .await
        .expect("query should execute");
    let SqlResult::Query(result) = result else {
        panic!("expected query result");
    };
    assert_columns(
        &result,
        &[
            "id",
            "integer_value",
            "real_value",
            "text_value",
            "blob_value",
            "numeric_value",
            "boolean_value",
            "date_value",
            "time_value",
            "datetime_value",
        ],
    );
    assert_eq!(result.rows.len(), 3);
    assert_cell(&result, 0, 1, "-9223372036854775808");
    assert_cell(&result, 0, 2, "3.14159");
    assert_cell(&result, 0, 3, "中文 🚀 O'Reilly\\nline 2");
    assert_cell(&result, 0, 5, "12345.6789");
    assert_cell(&result, 0, 6, "1");
    assert_null(&result, 1, 7);
    assert_cell(&result, 1, 3, "");
    assert_null(&result, 1, 7);
    assert_cell(&result, 2, 3, "NULL");
    assert_binary(&result, 0, 4, &[0, 1, 2, 255]);
    assert_binary(&result, 1, 4, &[]);
    assert_binary(&result, 2, 4, &[255]);
    let _ = plugin;
}

async fn assert_error_and_transaction(plugin: &SqlitePlugin, connection: &SqliteDbConnection) {
    let result = connection
        .query("SELECT * FROM missing_table")
        .await
        .expect("query should return");
    assert!(result.is_error(), "missing table should be an error result");

    let results = connection
        .execute(
            plugin,
            "INSERT INTO all_types (id) VALUES (4); SELECT broken",
            ExecOptions {
                stop_on_error: true,
                transactional: true,
                max_rows: Some(10),
                streaming: false,
            },
        )
        .await
        .expect("script execution should return");
    assert!(results.iter().any(|result| result.is_error()));
    let count = scalar_count(connection).await;
    assert_eq!(count, 3, "failed transactional script should roll back");
}

async fn scalar_count(connection: &SqliteDbConnection) -> usize {
    let result = connection
        .query("SELECT COUNT(*) FROM all_types")
        .await
        .expect("count query should run");
    let SqlResult::Query(result) = result else {
        panic!("count should be a query")
    };
    result.rows[0][0]
        .as_deref()
        .unwrap_or_default()
        .parse()
        .unwrap_or_default()
}

async fn assert_metadata(plugin: &SqlitePlugin, connection: &SqliteDbConnection) {
    let databases = plugin
        .list_databases(connection)
        .await
        .expect("databases should list");
    assert_eq!(databases, vec!["main".to_string()]);
    let tables = plugin
        .list_tables(connection, "main", None)
        .await
        .expect("tables should list");
    assert!(tables.iter().any(|table| table.name == "all_types"));
    let columns = plugin
        .list_columns(connection, "main", None, "all_types")
        .await
        .expect("columns should list");
    assert!(columns.len() >= 10);
    assert!(
        columns
            .iter()
            .any(|column| column.name == "blob_value" && column.data_type == "BLOB")
    );
    assert!(
        columns
            .iter()
            .any(|column| column.name == "id" && column.is_primary_key)
    );
}
