use db::connection::DbConnection;
use db::executor::{ExecOptions, SqlResult};
use db::plugin::DatabasePlugin;
use db::postgresql::PostgresPlugin;

use crate::real_databases::common::assertions::{
    assert_cell, assert_columns, assert_no_sql_errors, assert_null,
};
use crate::real_databases::common::env::{optional_database, postgres_config, skip_database};

const FIXTURE_SQL: &str = r#"
DROP TABLE IF EXISTS all_types;
CREATE TABLE all_types (
    id SERIAL PRIMARY KEY,
    smallint_value SMALLINT NOT NULL,
    integer_value INTEGER NOT NULL,
    bigint_value BIGINT NOT NULL,
    numeric_value NUMERIC(12, 4) NOT NULL,
    real_value REAL NOT NULL,
    double_value DOUBLE PRECISION NOT NULL,
    boolean_value BOOLEAN NOT NULL,
    char_value CHAR(3) NOT NULL,
    varchar_value VARCHAR(32) NOT NULL,
    text_value TEXT NOT NULL,
    bytea_value BYTEA NOT NULL,
    date_value DATE,
    time_value TIME,
    timestamp_value TIMESTAMP,
    timestamptz_value TIMESTAMPTZ,
    interval_value INTERVAL,
    uuid_value UUID NOT NULL,
    json_value JSON NOT NULL,
    jsonb_value JSONB NOT NULL,
    xml_value XML NOT NULL,
    inet_value INET NOT NULL,
    text_array_value TEXT[] NOT NULL,
    integer_array_value INTEGER[] NOT NULL
);
INSERT INTO all_types (
    smallint_value, integer_value, bigint_value, numeric_value, real_value, double_value,
    boolean_value, char_value, varchar_value, text_value, bytea_value, date_value, time_value,
    timestamp_value, timestamptz_value, interval_value, uuid_value, json_value, jsonb_value,
    xml_value, inet_value, text_array_value, integer_array_value
) VALUES (
    -32768, -2147483648, -9223372036854775808, 12345.6789, 1.25, -3.14159,
    true, '中', '中文 🚀 O''Reilly', 'line 1
line 2', '\x000102ff', '2026-08-22', '12:34:56.789',
    '2026-08-22 12:34:56.789', '2026-08-22 12:34:56.789+00', '1 year 2 months 3 days',
    '0f8f0a5a-71b6-4b93-ba99-5f9a94a36c99', '{"name":"中文","count":2}',
    '{"items":[1,"two",null]}', '<root><value>中文</value></root>', '192.168.1.1',
    ARRAY['中文', 'b'], ARRAY[1, 2, 3]
);
INSERT INTO all_types (
    smallint_value, integer_value, bigint_value, numeric_value, real_value, double_value,
    boolean_value, char_value, varchar_value, text_value, bytea_value, interval_value,
    uuid_value, json_value, jsonb_value, xml_value, inet_value, text_array_value,
    integer_array_value
) VALUES (
    32767, 2147483647, 9223372036854775807, -1.5, 0, 0,
    false, 'A', 'empty', '', '', '0',
    '0f8f0a5a-71b6-4b93-ba99-5f9a94a36c00', '[]', '{}', '<root/>', '127.0.0.1',
    ARRAY['x'], ARRAY[]::integer[]
);
"#;

#[tokio::test]
async fn postgres_real_script_query_error_transaction_and_metadata_flow() {
    let Some(config) = postgres_config() else {
        skip_database("PostgreSQL", "ONETCLI_TEST_POSTGRES_PASSWORD (empty string is valid)");
        return;
    };
    let config = optional_database(
        &config,
        &std::env::var("ONETCLI_TEST_POSTGRES_DATABASE").unwrap_or_else(|_| "postgres".to_string()),
    );
    let schema = unique_schema("core");
    let plugin = PostgresPlugin::new();
    let mut connection = plugin
        .create_connection(config)
        .await
        .expect("PostgreSQL should connect");

    reset_schema(&plugin, connection.as_ref(), &schema).await;
    run_fixture(&plugin, connection.as_ref(), &schema).await;
    assert_full_type_query(connection.as_ref(), &schema).await;
    assert_error_and_transaction(&plugin, connection.as_ref(), &schema).await;
    assert_metadata(&plugin, connection.as_ref(), &schema).await;
    drop_schema(&plugin, connection.as_ref(), &schema).await;
    connection
        .disconnect()
        .await
        .expect("PostgreSQL should disconnect");
}

pub(crate) fn unique_schema(slug: &str) -> String {
    format!("navop_real_pg_{}_{slug}", std::process::id())
}

pub(crate) async fn execute(
    plugin: &PostgresPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    sql: &str,
) -> Vec<SqlResult> {
    let results = connection
        .execute(plugin, sql, ExecOptions::default())
        .await
        .expect("PostgreSQL script should execute");
    assert_no_sql_errors(&results, sql);
    results
}

pub(crate) async fn reset_schema(
    plugin: &PostgresPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    schema: &str,
) {
    execute(
        plugin,
        connection,
        &format!("DROP SCHEMA IF EXISTS \"{schema}\" CASCADE;\nCREATE SCHEMA \"{schema}\";"),
    )
    .await;
}

async fn run_fixture(
    plugin: &PostgresPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    schema: &str,
) {
    connection
        .switch_schema(schema)
        .await
        .expect("switch to test schema");
    execute(plugin, connection, FIXTURE_SQL).await;
}

async fn assert_full_type_query(connection: &(dyn DbConnection + Send + Sync), schema: &str) {
    let result = connection
        .query(&format!(
            "SELECT id, smallint_value, integer_value, bigint_value, numeric_value, \
             real_value, double_value, boolean_value, char_value, varchar_value, text_value, \
             bytea_value, HOST(inet_value) AS inet_host, date_value, time_value, \
             timestamp_value, timestamptz_value, interval_value, uuid_value, json_value, \
             jsonb_value, xml_value, \
             text_array_value, integer_array_value FROM \"{schema}\".all_types ORDER BY id"
        ))
        .await
        .expect("all-types query should execute");
    let SqlResult::Query(result) = result else {
        panic!("all-types query should return rows");
    };
    assert_columns(
        &result,
        &[
            "id",
            "smallint_value",
            "integer_value",
            "bigint_value",
            "numeric_value",
            "real_value",
            "double_value",
            "boolean_value",
            "char_value",
            "varchar_value",
            "text_value",
            "bytea_value",
            "inet_host",
            "date_value",
            "time_value",
            "timestamp_value",
            "timestamptz_value",
            "interval_value",
            "uuid_value",
            "json_value",
            "jsonb_value",
            "xml_value",
            "text_array_value",
            "integer_array_value",
        ],
    );
    assert_eq!(result.rows.len(), 2);
    assert_cell(&result, 0, 1, "-32768");
    assert_cell(&result, 0, 3, "-9223372036854775808");
    assert_cell(&result, 0, 4, "12345.6789");
    assert_cell(&result, 0, 7, "true");
    assert_cell(&result, 0, 8, "中  ");
    assert_cell(&result, 0, 9, "中文 🚀 O'Reilly");
    assert_cell(&result, 0, 10, "line 1\nline 2");
    assert_cell(&result, 0, 12, "192.168.1.1");
    assert_cell(&result, 0, 13, "2026-08-22");
    assert_cell(&result, 0, 14, "12:34:56");
    assert_cell(&result, 0, 15, "2026-08-22 12:34:56");
    assert_cell(&result, 0, 15, "2026-08-22 12:34:56");
    assert_cell(&result, 0, 16, "2026-08-22 12:34:56 +0000");
    assert_cell(&result, 0, 22, "<_text>");
    assert_cell(&result, 0, 23, "<_int4>");
    assert!(
        result
            .binary_cells
            .iter()
            .any(|cell| cell.column_index == 11)
    );
    assert_null(&result, 1, 13);
    assert_null(&result, 1, 14);
    assert_null(&result, 1, 15);
    assert_cell(&result, 1, 7, "false");
    assert_cell(&result, 1, 10, "");
    assert_cell(&result, 1, 23, "<_int4>");
}

async fn assert_error_and_transaction(
    plugin: &PostgresPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    schema: &str,
) {
    let error = connection
        .query(&format!("SELECT * FROM \"{schema}\".missing_table"))
        .await
        .expect("error query should return a result");
    assert!(error.is_error(), "missing table should be an error");
    let results = connection
        .execute(
            plugin,
            "INSERT INTO all_types (id) VALUES (99); SELECT broken;",
            ExecOptions {
                stop_on_error: true,
                transactional: true,
                max_rows: Some(10),
                streaming: false,
            },
        )
        .await
        .expect("failed script should return results");
    assert!(results.iter().any(|result| result.is_error()));
    let count = connection
        .query("SELECT COUNT(*) FROM all_types")
        .await
        .expect("count query should run");
    let SqlResult::Query(count) = count else {
        panic!("count should be a query");
    };
    let count = count.rows[0][0]
        .as_deref()
        .unwrap_or_default()
        .parse::<usize>()
        .unwrap_or_default();
    assert_eq!(count, 2, "failed transactional script should roll back");
}

async fn assert_metadata(
    plugin: &PostgresPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    schema: &str,
) {
    let schemas = plugin
        .list_schemas(connection, "postgres")
        .await
        .expect("PostgreSQL schemas should list");
    assert!(schemas.iter().any(|name| name == schema));
    let tables = plugin
        .list_tables(connection, "postgres", Some(schema.to_string()))
        .await
        .expect("PostgreSQL tables should list");
    assert!(tables.iter().any(|table| table.name == "all_types"));
    let columns = plugin
        .list_columns(
            connection,
            "postgres",
            Some(schema.to_string()),
            "all_types",
        )
        .await
        .expect("PostgreSQL columns should list");
    assert!(columns.len() >= 24);
    assert!(
        columns
            .iter()
            .any(|column| column.name == "jsonb_value" && column.data_type == "jsonb")
    );
    assert!(
        columns
            .iter()
            .any(|column| column.name == "id" && column.is_primary_key)
    );
}

pub(crate) async fn drop_schema(
    plugin: &PostgresPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    schema: &str,
) {
    execute(
        plugin,
        connection,
        &format!("DROP SCHEMA IF EXISTS \"{schema}\" CASCADE;"),
    )
    .await;
}
