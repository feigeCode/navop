use db::connection::DbConnection;
use db::executor::{ExecOptions, SqlResult};
use db::mysql::MySqlPlugin;
use db::plugin::DatabasePlugin;

use crate::real_databases::common::assertions::{
    assert_cell, assert_columns, assert_no_sql_errors, assert_null,
};
use crate::real_databases::common::env::mysql_config;

const FIXTURE_SQL: &str = r#"
DROP TABLE IF EXISTS all_types;
CREATE TABLE all_types (
    id INT AUTO_INCREMENT PRIMARY KEY,
    tinyint_value TINYINT NOT NULL,
    smallint_value SMALLINT NOT NULL,
    mediumint_value MEDIUMINT NOT NULL,
    integer_value INT NOT NULL,
    bigint_value BIGINT NOT NULL,
    unsigned_value INT UNSIGNED NOT NULL,
    decimal_value DECIMAL(12, 4) NOT NULL,
    float_value FLOAT NOT NULL,
    double_value DOUBLE NOT NULL,
    bit_value BIT(8) NOT NULL,
    char_value CHAR(3) NOT NULL,
    varchar_value VARCHAR(32) NOT NULL,
    text_value TEXT NOT NULL,
    binary_value BINARY(4) NOT NULL,
    varbinary_value VARBINARY(16) NOT NULL,
    blob_value BLOB NOT NULL,
    date_value DATE,
    time_value TIME(3),
    datetime_value DATETIME(3),
    timestamp_value TIMESTAMP NULL,
    year_value YEAR NOT NULL,
    enum_value ENUM('alpha', 'beta') NOT NULL,
    set_value SET('a', 'b') NOT NULL,
    json_value JSON NOT NULL
);
INSERT INTO all_types (
    tinyint_value, smallint_value, mediumint_value, integer_value, bigint_value,
    unsigned_value, decimal_value, float_value, double_value, bit_value,
    char_value, varchar_value, text_value, binary_value, varbinary_value, blob_value,
    date_value, time_value, datetime_value, timestamp_value, year_value,
    enum_value, set_value, json_value
) VALUES (
    -128, -32768, -8388608, -2147483648, -9223372036854775808,
    4294967295, 12345.6789, 1.25, -3.14159, b'00000101',
    '中', '中文 🚀 O''Reilly', 'line 1\nline 2', X'000102FF', X'FF00', X'DEADBEEF',
    '2026-08-22', '12:34:56.789', '2026-08-22 12:34:56.789', '2026-08-22 12:34:56.789', 2026,
    'beta', 'a,b', JSON_OBJECT('name', '中文', 'count', 2)
);
INSERT INTO all_types (
    tinyint_value, smallint_value, mediumint_value, integer_value, bigint_value,
    unsigned_value, decimal_value, float_value, double_value, bit_value,
    char_value, varchar_value, text_value, binary_value, varbinary_value, blob_value,
    year_value, enum_value, set_value, json_value
) VALUES (
    127, 32767, 8388607, 2147483647, 9223372036854775807,
    0, -1.5, 0, 0, b'10000000',
    'A', 'empty', '', X'00000000', X'', X'',
    2026, 'alpha', '', JSON_ARRAY(1, 'two', NULL)
);
"#;

#[tokio::test]
async fn mysql_real_script_query_error_transaction_and_metadata_flow() {
    let config = mysql_config().expect(
        "set ONETCLI_TEST_MYSQL_PASSWORD (and optionally host/port/user) to run MySQL tests",
    );
    let database = unique_database("core");
    let plugin = MySqlPlugin::new();
    let mut connection = plugin
        .create_connection(config)
        .await
        .expect("MySQL should connect");

    reset_database(&plugin, connection.as_ref(), &database).await;
    run_fixture(&plugin, connection.as_ref(), &database).await;
    assert_full_type_query(connection.as_ref(), &database).await;
    assert_error_and_transaction(&plugin, connection.as_ref(), &database).await;
    assert_metadata(&plugin, connection.as_ref(), &database).await;
    drop_database(&plugin, connection.as_ref(), &database).await;
    connection
        .disconnect()
        .await
        .expect("MySQL should disconnect");
}

pub(crate) fn unique_database(slug: &str) -> String {
    format!("navop_real_mysql_{}_{slug}", std::process::id())
}

pub(crate) async fn execute(
    plugin: &MySqlPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    sql: &str,
) -> Vec<SqlResult> {
    let results = connection
        .execute(plugin, sql, ExecOptions::default())
        .await
        .expect("MySQL script should execute");
    assert_no_sql_errors(&results, sql);
    results
}

async fn reset_database(
    plugin: &MySqlPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    database: &str,
) {
    execute(
        plugin,
        connection,
        &format!(
            "DROP DATABASE IF EXISTS `{database}`; CREATE DATABASE `{database}` \
             CHARACTER SET utf8mb4 COLLATE utf8mb4_general_ci;"
        ),
    )
    .await;
}

async fn run_fixture(
    plugin: &MySqlPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    database: &str,
) {
    connection
        .switch_database(database)
        .await
        .expect("switch to test database");
    execute(plugin, connection, FIXTURE_SQL).await;
}

async fn assert_full_type_query(connection: &(dyn DbConnection + Send + Sync), database: &str) {
    let result = connection
        .query(&format!(
            "SELECT id, tinyint_value, smallint_value, mediumint_value, integer_value, \
             bigint_value, unsigned_value, decimal_value, float_value, double_value, \
             bit_value, char_value, varchar_value, text_value, binary_value, varbinary_value, \
             blob_value, date_value, time_value, datetime_value, timestamp_value, year_value, \
             enum_value, set_value, json_value FROM `{database}`.all_types ORDER BY id"
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
            "tinyint_value",
            "smallint_value",
            "mediumint_value",
            "integer_value",
            "bigint_value",
            "unsigned_value",
            "decimal_value",
            "float_value",
            "double_value",
            "bit_value",
            "char_value",
            "varchar_value",
            "text_value",
            "binary_value",
            "varbinary_value",
            "blob_value",
            "date_value",
            "time_value",
            "datetime_value",
            "timestamp_value",
            "year_value",
            "enum_value",
            "set_value",
            "json_value",
        ],
    );
    assert_eq!(result.rows.len(), 2);
    assert_cell(&result, 0, 1, "-128");
    assert_cell(&result, 0, 5, "-9223372036854775808");
    assert_cell(&result, 0, 6, "4294967295");
    assert_cell(&result, 0, 7, "12345.6789");
    assert_cell(&result, 0, 11, "中");
    assert_cell(&result, 0, 12, "中文 🚀 O'Reilly");
    assert_cell(&result, 0, 13, "line 1\nline 2");
    assert_cell(&result, 0, 17, "2026-08-22");
    assert_cell(&result, 0, 18, "12:34:56.789");
    assert_cell(&result, 0, 19, "2026-08-22 12:34:56.789");
    assert_cell(&result, 0, 20, "2026-08-22 12:34:57");
    assert_cell(&result, 0, 21, "2026");
    assert_cell(&result, 0, 22, "beta");
    assert_cell(&result, 0, 23, "a,b");
    assert!(
        result
            .binary_cells
            .iter()
            .any(|cell| cell.column_index == 14)
    );
    assert_null(&result, 1, 17);
    assert_null(&result, 1, 18);
    assert_cell(&result, 1, 13, "");
    assert_cell(&result, 1, 22, "alpha");
}

async fn assert_error_and_transaction(
    plugin: &MySqlPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    database: &str,
) {
    let error = connection
        .query(&format!("SELECT * FROM `{database}`.missing_table"))
        .await
        .expect("error query should return a result");
    assert!(error.is_error(), "missing table should be an error");

    let results = connection
        .execute(
            plugin,
            &format!("INSERT INTO `{database}`.all_types (id) VALUES (99); SELECT broken;"),
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
    let count = scalar_count(connection, database).await;
    assert_eq!(count, 2, "failed transactional script should roll back");
}

async fn scalar_count(connection: &(dyn DbConnection + Send + Sync), database: &str) -> usize {
    let result = connection
        .query(&format!("SELECT COUNT(*) FROM `{database}`.all_types"))
        .await
        .expect("count query should run");
    let SqlResult::Query(result) = result else {
        panic!("count should be a query");
    };
    result.rows[0][0]
        .as_deref()
        .unwrap_or_default()
        .parse()
        .unwrap_or_default()
}

async fn assert_metadata(
    plugin: &MySqlPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    database: &str,
) {
    let databases = plugin
        .list_databases(connection)
        .await
        .expect("MySQL databases should list");
    assert!(databases.iter().any(|name| name == database));
    let tables = plugin
        .list_tables(connection, database, None)
        .await
        .expect("MySQL tables should list");
    assert!(tables.iter().any(|table| table.name == "all_types"));
    let columns = plugin
        .list_columns(connection, database, None, "all_types")
        .await
        .expect("MySQL columns should list");
    assert!(columns.len() >= 25);
    assert!(
        columns
            .iter()
            .any(|column| column.name == "json_value" && column.data_type == "json")
    );
    assert!(
        columns
            .iter()
            .any(|column| column.name == "id" && column.is_primary_key)
    );
}

pub(crate) async fn drop_database(
    plugin: &MySqlPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    database: &str,
) {
    execute(
        plugin,
        connection,
        &format!("DROP DATABASE IF EXISTS `{database}`;"),
    )
    .await;
}
