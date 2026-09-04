use db::connection::DbConnection;
use db::executor::{ExecOptions, SqlResult};
use db::mysql::MySqlPlugin;
use db::plugin::DatabasePlugin;
use db::types::{ColumnInfo, TableCellValue, TableRowChange, TableSaveRequest};

use crate::real_databases::common::env::{mysql_config, skip_database};
use crate::real_databases::mysql::core_flow::{drop_database, execute, unique_database};

#[tokio::test]
async fn mysql_real_table_data_pagination_filter_sort_and_crud() {
    let Some(config) = mysql_config() else {
        skip_database("MySQL", "ONETCLI_TEST_MYSQL_PASSWORD");
        return;
    };
    let database = unique_database("data");
    let plugin = MySqlPlugin::new();
    let mut connection = plugin
        .create_connection(config)
        .await
        .expect("MySQL should connect");
    setup(&plugin, connection.as_ref(), &database).await;

    assert_table_data(&plugin, connection.as_ref(), &database).await;
    execute_generated_crud(&plugin, connection.as_ref(), &database).await;
    drop_database(&plugin, connection.as_ref(), &database).await;
    connection.disconnect().await.expect("disconnect");
}

async fn setup(
    plugin: &MySqlPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    database: &str,
) {
    execute(
        plugin,
        connection,
        &format!(
            "DROP DATABASE IF EXISTS `{database}`; CREATE DATABASE `{database}`;\nCREATE TABLE `{database}`.people (\
             id INT PRIMARY KEY, name VARCHAR(64) NOT NULL, age INT, payload VARBINARY(32), \
             score DECIMAL(8,2), active BIT(1));\nINSERT INTO `{database}`.people VALUES \
             (1, 'Alice', 30, X'0102', 91.50, b'1'), \
             (2, 'Bob', 25, X'', NULL, b'0'), \
             (3, '中文', NULL, NULL, NULL, NULL);"
        ),
    )
    .await;
}

async fn assert_table_data(
    plugin: &MySqlPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    database: &str,
) {
    let first = db::types::TableDataRequest::new(database, "people")
        .with_page(1, 2)
        .with_where_clause("id >= 1")
        .with_order_by_clause("id DESC");
    let response = plugin
        .query_table_data(connection, first)
        .await
        .expect("MySQL table data");
    assert_eq!(response.total_count, 3);
    assert_eq!(
        response.query_result.columns,
        vec!["id", "name", "age", "payload", "score", "active"]
    );
    assert_eq!(response.query_result.rows.len(), 2);
    assert_eq!(response.query_result.rows[0][0].as_deref(), Some("3"));

    let second = db::types::TableDataRequest::new(database, "people")
        .with_page(2, 1)
        .with_offset(2)
        .with_known_total_count(3)
        .with_order_by_clause("id");
    let response = plugin
        .query_table_data(connection, second)
        .await
        .expect("MySQL second page");
    assert_eq!(response.total_count, 3);
    assert_eq!(response.query_result.rows.len(), 1);
    assert_eq!(response.query_result.rows[0][1].as_deref(), Some("中文"));

    let filtered = db::types::TableDataRequest::new(database, "people")
        .with_page(1, 100)
        .with_where_clause("name LIKE 'A%'")
        .with_order_by_clause("id");
    let response = plugin
        .query_table_data(connection, filtered)
        .await
        .expect("MySQL filtered table data");
    assert_eq!(response.total_count, 1);
    assert_eq!(response.query_result.rows[0][1].as_deref(), Some("Alice"));
}

async fn execute_generated_crud(
    plugin: &MySqlPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    database: &str,
) {
    let columns = plugin
        .list_columns(connection, database, None, "people")
        .await
        .expect("MySQL people columns");
    let indexes = plugin
        .list_indexes(connection, database, None, "people")
        .await
        .expect("MySQL people indexes");
    let request = TableSaveRequest {
        database: database.to_string(),
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
                    TableCellValue::Text("77.25".into()),
                    TableCellValue::Text("1".into()),
                ],
            },
            TableRowChange::Updated {
                original_data: row(&columns, vec!["1", "Alice", "30", "0102", "91.50", "1"]),
                changes: vec![db::types::TableCellChange {
                    column_index: 1,
                    column_name: "name".into(),
                    old_value: TableCellValue::Text("Alice".into()),
                    new_value: TableCellValue::Text("Alice Renamed 🚀".into()),
                }],
                rowid: None,
            },
            TableRowChange::Deleted {
                original_data: row(&columns, vec!["2", "Bob", "25", "", "NULL", "0"]),
                rowid: None,
            },
        ],
    };
    let sql = plugin.generate_table_changes_sql(&request);
    assert!(sql.contains("INSERT INTO"));
    assert!(sql.contains("UPDATE"));
    assert!(sql.contains("DELETE FROM"));
    assert!(sql.contains("LIMIT 1"));
    assert_no_errors(plugin, connection, &sql).await;

    let result = connection
        .query(&format!(
            "SELECT name, age, HEX(payload), score FROM `{database}`.people \
             WHERE id IN (1,4) ORDER BY id"
        ))
        .await
        .expect("verify MySQL CRUD");
    let SqlResult::Query(result) = result else {
        panic!("MySQL CRUD verification should be a query");
    };
    assert_eq!(result.rows[0][0].as_deref(), Some("Alice Renamed 🚀"));
    assert_eq!(result.rows[0][1].as_deref(), Some("30"));
    assert_eq!(result.rows[1][0].as_deref(), Some("O'Reilly 🚀"));
    assert_eq!(result.rows[1][2].as_deref(), Some("00FF"));
}

fn row(columns: &[ColumnInfo], values: Vec<&str>) -> Vec<TableCellValue> {
    columns
        .iter()
        .zip(values)
        .map(|(_column, value)| {
            if value == "NULL" {
                TableCellValue::Null
            } else {
                TableCellValue::Text(value.to_string())
            }
        })
        .collect()
}

async fn assert_no_errors(
    plugin: &MySqlPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    sql: &str,
) {
    let results = connection
        .execute(plugin, sql, ExecOptions::default())
        .await
        .expect("MySQL generated CRUD should execute");
    crate::real_databases::common::assertions::assert_no_sql_errors(&results, sql);
}
