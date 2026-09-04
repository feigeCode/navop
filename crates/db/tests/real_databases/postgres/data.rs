use db::connection::DbConnection;
use db::executor::{ExecOptions, SqlResult};
use db::plugin::DatabasePlugin;
use db::postgresql::PostgresPlugin;
use db::types::{TableCellChange, TableCellValue, TableRowChange, TableSaveRequest};

use crate::real_databases::common::env::{optional_database, postgres_config, skip_database};
use crate::real_databases::postgres::core_flow::{
    drop_schema, execute, reset_schema, unique_schema,
};

#[tokio::test]
async fn postgres_real_table_data_pagination_filter_sort_and_crud() {
    let Some(config) = postgres_config() else {
        skip_database("PostgreSQL", "ONETCLI_TEST_POSTGRES_PASSWORD (empty string is valid)");
        return;
    };
    let config = optional_database(
        &config,
        &std::env::var("ONETCLI_TEST_POSTGRES_DATABASE").unwrap_or_else(|_| "postgres".to_string()),
    );
    let schema = unique_schema("data");
    let plugin = PostgresPlugin::new();
    let mut connection = plugin
        .create_connection(config)
        .await
        .expect("PostgreSQL should connect");
    reset_schema(&plugin, connection.as_ref(), &schema).await;
    setup(&plugin, connection.as_ref(), &schema).await;

    connection
        .switch_schema(&schema)
        .await
        .expect("switch to data schema");
    assert_table_data(&plugin, connection.as_ref(), &schema).await;
    execute_generated_crud(&plugin, connection.as_ref(), &schema).await;
    drop_schema(&plugin, connection.as_ref(), &schema).await;
    connection.disconnect().await.expect("disconnect");
}

async fn setup(
    plugin: &PostgresPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    schema: &str,
) {
    execute(
        plugin,
        connection,
        &format!(
            "CREATE SCHEMA IF NOT EXISTS \"{schema}\";\nCREATE TABLE \"{schema}\".people (\
             id INT PRIMARY KEY, name TEXT NOT NULL, age INT, payload BYTEA, score NUMERIC(8,2), \
             active BOOLEAN);\nINSERT INTO \"{schema}\".people VALUES \
             (1, 'Alice', 30, decode('0102', 'hex'), 91.50, true), \
             (2, 'Bob', 25, ''::bytea, NULL, false), \
             (3, '中文', NULL, NULL, NULL, NULL);"
        ),
    )
    .await;
}

async fn assert_table_data(
    plugin: &PostgresPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    schema: &str,
) {
    let first = db::types::TableDataRequest::new("current_database", "people")
        .with_schema(schema)
        .with_page(1, 2)
        .with_where_clause("id >= 1")
        .with_order_by_clause("id DESC");
    let response = plugin
        .query_table_data(connection, first)
        .await
        .expect("PostgreSQL table data");
    assert_eq!(response.total_count, 3);
    assert_eq!(
        response.query_result.columns,
        vec!["id", "name", "age", "payload", "score", "active"]
    );
    assert_eq!(response.query_result.rows.len(), 2);
    assert_eq!(response.query_result.rows[0][0].as_deref(), Some("3"));

    let second = db::types::TableDataRequest::new("current_database", "people")
        .with_schema(schema)
        .with_page(2, 1)
        .with_offset(2)
        .with_known_total_count(3)
        .with_order_by_clause("id");
    let response = plugin
        .query_table_data(connection, second)
        .await
        .expect("PostgreSQL second page");
    assert_eq!(response.total_count, 3);
    assert_eq!(response.query_result.rows.len(), 1);
    assert_eq!(response.query_result.rows[0][1].as_deref(), Some("中文"));

    let filtered = db::types::TableDataRequest::new("current_database", "people")
        .with_schema(schema)
        .with_page(1, 100)
        .with_where_clause("name LIKE 'A%'")
        .with_order_by_clause("id");
    let response = plugin
        .query_table_data(connection, filtered)
        .await
        .expect("PostgreSQL filtered table data");
    assert_eq!(response.total_count, 1);
    assert_eq!(response.query_result.rows[0][1].as_deref(), Some("Alice"));
}

async fn execute_generated_crud(
    plugin: &PostgresPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    schema: &str,
) {
    let columns = plugin
        .list_columns(connection, "postgres", Some(schema.to_string()), "people")
        .await
        .expect("PostgreSQL people columns");
    let indexes = plugin
        .list_indexes(connection, "postgres", Some(schema.to_string()), "people")
        .await
        .expect("PostgreSQL people indexes");
    let request = TableSaveRequest {
        database: std::env::var("ONETCLI_TEST_POSTGRES_DATABASE")
            .unwrap_or_else(|_| "postgres".to_string()),
        schema: Some(schema.to_string()),
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
                    TableCellValue::Text("true".into()),
                ],
            },
            TableRowChange::Updated {
                original_data: row(&columns, vec!["1", "Alice", "30", "0102", "91.50", "true"]),
                changes: vec![TableCellChange {
                    column_index: 1,
                    column_name: "name".into(),
                    old_value: TableCellValue::Text("Alice".into()),
                    new_value: TableCellValue::Text("Alice Renamed 🚀".into()),
                }],
                rowid: None,
            },
            TableRowChange::Deleted {
                original_data: row(&columns, vec!["2", "Bob", "25", "", "NULL", "false"]),
                rowid: None,
            },
        ],
    };
    let sql = plugin.generate_table_changes_sql(&request);
    assert!(sql.contains("INSERT INTO"));
    assert!(sql.contains("UPDATE"));
    assert!(sql.contains("DELETE FROM"));
    let results = connection
        .execute(plugin, &sql, ExecOptions::default())
        .await
        .expect("PostgreSQL generated CRUD should execute");
    crate::real_databases::common::assertions::assert_no_sql_errors(&results, &sql);

    let result = connection
        .query(&format!(
            "SELECT name, age, encode(payload, 'hex'), score FROM \"{schema}\".people \
             WHERE id IN (1,4) ORDER BY id"
        ))
        .await
        .expect("verify PostgreSQL CRUD");
    let SqlResult::Query(result) = result else {
        panic!("PostgreSQL CRUD verification should be a query");
    };
    assert_eq!(result.rows[0][0].as_deref(), Some("Alice Renamed 🚀"));
    assert_eq!(result.rows[0][1].as_deref(), Some("30"));
    assert_eq!(result.rows[1][0].as_deref(), Some("O'Reilly 🚀"));
    assert_eq!(result.rows[1][2].as_deref(), Some("00ff"));
}

fn row(columns: &[db::types::ColumnInfo], values: Vec<&str>) -> Vec<TableCellValue> {
    columns
        .iter()
        .zip(values)
        .map(|(column, value)| {
            if value == "NULL" {
                TableCellValue::Null
            } else if column.name == "payload" {
                TableCellValue::Binary(
                    (0..value.len())
                        .step_by(2)
                        .map(|index| {
                            u8::from_str_radix(&value[index..index + 2], 16)
                                .expect("valid binary hex")
                        })
                        .collect(),
                )
            } else {
                TableCellValue::Text(value.to_string())
            }
        })
        .collect()
}
