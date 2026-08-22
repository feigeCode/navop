use std::path::Path;

use db::connection::DbConnection;
use db::duckdb::{DuckDbConnection, DuckDbPlugin};
use db::executor::{ExecOptions, SqlResult};
use db::plugin::DatabasePlugin;
use db::types::{TableCellChange, TableCellValue, TableRowChange, TableSaveRequest};
use one_core::storage::{DatabaseType, DbConnectionConfig};

use crate::real_databases::common::assertions::assert_no_sql_errors;
use crate::real_databases::common::fixture::file_config;

fn config(path: &Path) -> DbConnectionConfig {
    file_config("duckdb-real-data", DatabaseType::DuckDB, path)
}

#[tokio::test]
async fn duckdb_real_table_data_crud_and_generated_sql() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let plugin = DuckDbPlugin::new();
    let mut connection = DuckDbConnection::new(config(&temp_dir.path().join("data.duckdb")));
    connection.connect().await.expect("connect");
    let setup = "CREATE TABLE people (id INTEGER PRIMARY KEY, name VARCHAR NOT NULL, age INTEGER, payload BLOB);
        INSERT INTO people VALUES (1, 'Alice', 30, encode('\\x0102')), (2, 'Bob', 25, encode('\\x')), (3, '中文', NULL, NULL);";
    assert_no_sql_errors(
        &connection
            .execute(&plugin, setup, ExecOptions::default())
            .await
            .expect("setup"),
        "setup",
    );

    assert_table_data(&plugin, &connection).await;
    execute_generated_crud(&plugin, &connection).await;
    connection.disconnect().await.expect("disconnect");
}

async fn assert_table_data(plugin: &DuckDbPlugin, connection: &DuckDbConnection) {
    let request = db::types::TableDataRequest::new("main", "people")
        .with_page(1, 2)
        .with_where_clause("id >= 1")
        .with_order_by_clause("id DESC");
    let response = plugin
        .query_table_data(connection, request)
        .await
        .expect("table data");
    assert_eq!(response.total_count, 3);
    assert_eq!(response.query_result.rows.len(), 2);
    assert_eq!(response.query_result.rows[0][1].as_deref(), Some("3"));

    let second = db::types::TableDataRequest::new("main", "people")
        .with_offset(2)
        .with_page(2, 1)
        .with_known_total_count(3)
        .with_order_by_clause("id");
    let response = plugin
        .query_table_data(connection, second)
        .await
        .expect("page");
    assert_eq!(response.query_result.rows[0][2].as_deref(), Some("中文"));

    let filtered = db::types::TableDataRequest::new("main", "people")
        .with_page(1, 100)
        .with_where_clause("name LIKE 'A%'")
        .with_order_by_clause("id");
    let response = plugin
        .query_table_data(connection, filtered)
        .await
        .expect("filter");
    assert_eq!(response.total_count, 1);
    assert_eq!(response.query_result.rows[0][2].as_deref(), Some("Alice"));
}

async fn execute_generated_crud(plugin: &DuckDbPlugin, connection: &DuckDbConnection) {
    let columns = plugin
        .list_columns(connection, "main", None, "people")
        .await
        .expect("columns");
    let indexes = plugin
        .list_indexes(connection, "main", None, "people")
        .await
        .expect("indexes");
    let request = TableSaveRequest {
        database: "main".into(),
        schema: None,
        table: "people".into(),
        columns: columns.clone(),
        index_infos: indexes,
        changes: vec![
            TableRowChange::Added {
                data: vec![
                    TableCellValue::Text("4".into()),
                    TableCellValue::Text("O'Reilly 🚀".into()),
                    TableCellValue::Text("41".into()),
                    TableCellValue::Text("\\x00ff".into()),
                ],
            },
            TableRowChange::Updated {
                original_data: vec![
                    TableCellValue::Text("1".into()),
                    TableCellValue::Text("Alice".into()),
                    TableCellValue::Text("30".into()),
                    TableCellValue::Text("\\x0102".into()),
                ],
                changes: vec![TableCellChange {
                    column_index: 1,
                    column_name: "name".into(),
                    old_value: TableCellValue::Text("Alice".into()),
                    new_value: TableCellValue::Text("Alice Renamed 🚀".into()),
                }],
                rowid: None,
            },
            TableRowChange::Deleted {
                original_data: vec![
                    TableCellValue::Text("2".into()),
                    TableCellValue::Text("Bob".into()),
                    TableCellValue::Text("25".into()),
                    TableCellValue::Text("\\x".into()),
                ],
                rowid: None,
            },
        ],
    };
    let sql = plugin.generate_table_changes_sql(&request);
    assert_no_sql_errors(
        &connection
            .execute(plugin, &sql, ExecOptions::default())
            .await
            .expect("CRUD"),
        "CRUD",
    );

    let result = connection
        .query("SELECT name, age, payload FROM people WHERE id IN (1,4) ORDER BY id")
        .await
        .expect("verify");
    let SqlResult::Query(result) = result else {
        panic!("verify")
    };
    assert_eq!(result.rows[0][0].as_deref(), Some("Alice Renamed 🚀"));
    assert_eq!(result.rows[1][2].as_deref(), Some("\\x00ff"));
}
