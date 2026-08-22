use db::connection::DbConnection;
use db::executor::{ExecOptions, SqlResult};
use db::plugin::DatabasePlugin;
use db::sqlite::{SqliteDbConnection, SqlitePlugin};
use db::types::{TableCellChange, TableCellValue, TableRowChange, TableSaveRequest};

use crate::real_databases::common::assertions::assert_no_sql_errors;
use crate::real_databases::common::fixture::file_config;
use one_core::storage::{DatabaseType, DbConnectionConfig};
use std::path::Path;

fn config(path: &Path) -> DbConnectionConfig {
    file_config("sqlite-real-data", DatabaseType::SQLite, path)
}

#[tokio::test]
async fn sqlite_real_table_data_crud_and_generated_sql() {
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let path = temp_dir.path().join("data.db");
    let plugin = SqlitePlugin::new();
    let mut connection = SqliteDbConnection::new(config(&path));
    connection.connect().await.expect("SQLite should connect");
    let setup = "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT NOT NULL, age INTEGER, payload BLOB);
        INSERT INTO people VALUES (1, 'Alice', 30, X'0102'), (2, 'Bob', 25, X''), (3, '中文', NULL, NULL);";
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

async fn assert_table_data(plugin: &SqlitePlugin, connection: &SqliteDbConnection) {
    let request = db::types::TableDataRequest::new("main", "people")
        .with_page(1, 2)
        .with_where_clause("id >= 1")
        .with_order_by_clause("id DESC");
    let response = plugin
        .query_table_data(connection, request)
        .await
        .expect("table data");
    assert_eq!(response.total_count, 3);
    assert_eq!(
        response.query_result.columns,
        vec!["__rowid__", "id", "name", "age", "payload"]
    );
    assert_eq!(response.query_result.rows.len(), 2);
    assert_eq!(response.query_result.rows[0][0].as_deref(), Some("3"));
    let second = db::types::TableDataRequest::new("main", "people")
        .with_offset(2)
        .with_page(2, 1)
        .with_known_total_count(3)
        .with_order_by_clause("id");
    let response = plugin
        .query_table_data(connection, second)
        .await
        .expect("second page");
    assert_eq!(response.total_count, 3);
    assert_eq!(response.query_result.rows.len(), 1);
    assert_eq!(response.query_result.rows[0][2].as_deref(), Some("中文"));

    let filtered = db::types::TableDataRequest::new("main", "people")
        .with_page(1, 100)
        .with_where_clause("name LIKE 'A%'")
        .with_order_by_clause("id");
    let response = plugin
        .query_table_data(connection, filtered)
        .await
        .expect("filtered");
    assert_eq!(response.total_count, 1);
    assert_eq!(response.query_result.rows[0][2].as_deref(), Some("Alice"));
}

async fn execute_generated_crud(plugin: &SqlitePlugin, connection: &SqliteDbConnection) {
    let columns = plugin
        .list_columns(connection, "main", None, "people")
        .await
        .expect("columns");
    let indexes = plugin
        .list_indexes(connection, "main", None, "people")
        .await
        .expect("indexes");
    let request = TableSaveRequest {
        database: "main".to_string(),
        schema: None,
        table: "people".to_string(),
        columns: columns.clone(),
        index_infos: indexes,
        changes: vec![
            TableRowChange::Added {
                data: vec![
                    TableCellValue::Text("4".into()),
                    TableCellValue::Text("O'Reilly 🚀".into()),
                    TableCellValue::Text("41".into()),
                    TableCellValue::Binary(vec![0, 255]),
                ],
            },
            TableRowChange::Updated {
                original_data: vec![
                    TableCellValue::Text("1".into()),
                    TableCellValue::Text("Alice".into()),
                    TableCellValue::Text("30".into()),
                    TableCellValue::Binary(vec![1, 2]),
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
                    TableCellValue::Binary(Vec::new()),
                ],
                rowid: None,
            },
        ],
    };
    let sql = plugin.generate_table_changes_sql(&request);
    assert!(sql.contains("INSERT INTO"));
    assert!(sql.contains("UPDATE"));
    assert!(sql.contains("DELETE FROM"));
    assert_no_sql_errors(
        &connection
            .execute(plugin, &sql, ExecOptions::default())
            .await
            .expect("CRUD"),
        "CRUD",
    );

    let result = connection
        .query("SELECT name, age, hex(payload) FROM people WHERE id IN (1,4) ORDER BY id")
        .await
        .expect("verify");
    let SqlResult::Query(result) = result else {
        panic!("verify query")
    };
    assert_eq!(result.rows[0][0].as_deref(), Some("Alice Renamed 🚀"));
    assert_eq!(result.rows[0][1].as_deref(), Some("30"));
    assert_eq!(result.rows[1][0].as_deref(), Some("O'Reilly 🚀"));
    assert_eq!(result.rows[1][2].as_deref(), Some("00FF"));
}
