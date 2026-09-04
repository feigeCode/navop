use db::connection::DbConnection;
use db::executor::ExecOptions;
use db::import_export::{CsvImportConfig, DataFormat, ExportConfig, ImportConfig};
use db::mysql::MySqlPlugin;
use db::plugin::DatabasePlugin;

use crate::real_databases::common::env::{mysql_config, skip_database};
use crate::real_databases::mysql::core_flow::{drop_database, execute, unique_database};

#[tokio::test]
async fn mysql_real_import_export_round_trips_sql_csv_and_json() {
    let Some(config) = mysql_config() else {
        skip_database("MySQL", "ONETCLI_TEST_MYSQL_PASSWORD");
        return;
    };
    let database = unique_database("io");
    let plugin = MySqlPlugin::new();
    let mut connection = plugin
        .create_connection(config)
        .await
        .expect("MySQL should connect");
    setup(&plugin, connection.as_ref(), &database).await;

    let sql = export(
        &plugin,
        connection.as_ref(),
        &database,
        DataFormat::Sql,
        None,
    )
    .await;
    assert_eq!(sql.rows_exported, 2);
    assert!(sql.output.to_uppercase().contains("INSERT INTO"));

    let csv = export(
        &plugin,
        connection.as_ref(),
        &database,
        DataFormat::Csv,
        Some(vec!["id".into(), "text_value".into()]),
    )
    .await;
    let json = export(
        &plugin,
        connection.as_ref(),
        &database,
        DataFormat::Json,
        Some(vec!["id".into(), "text_value".into()]),
    )
    .await;
    import(
        &plugin,
        connection.as_ref(),
        &database,
        DataFormat::Csv,
        &csv.output,
    )
    .await;
    import(
        &plugin,
        connection.as_ref(),
        &database,
        DataFormat::Json,
        &json.output,
    )
    .await;
    verify(&plugin, connection.as_ref(), &database).await;

    drop_database(&plugin, connection.as_ref(), &database).await;
    connection.disconnect().await.expect("disconnect");
}

#[tokio::test]
async fn mysql_real_sql_export_preserves_utf8mb3_bin_longtext_as_text() {
    let Some(config) = mysql_config() else {
        skip_database("MySQL", "ONETCLI_TEST_MYSQL_PASSWORD");
        return;
    };
    let database = unique_database("io_longtext");
    let plugin = MySqlPlugin::new();
    let mut connection = plugin
        .create_connection(config)
        .await
        .expect("MySQL should connect");

    execute(
        &plugin,
        connection.as_ref(),
        &format!(
            "CREATE DATABASE `{database}` CHARACTER SET utf8mb3 COLLATE utf8mb3_bin;\n\
             CREATE TABLE `{database}`.data (\
                 id VARCHAR(255) CHARACTER SET utf8mb3 COLLATE utf8mb3_bin PRIMARY KEY,\
                 entity_value LONGTEXT CHARACTER SET utf8mb3 COLLATE utf8mb3_bin,\
                 raw_value LONGBLOB\
             );\n\
             INSERT INTO `{database}`.data VALUES \
                 ('uuid_001', '{{\"metric\":\"sales\",\"label\":\"中文\"}}', UNHEX('000102FF'));"
        ),
    )
    .await;

    let sql = export(
        &plugin,
        connection.as_ref(),
        &database,
        DataFormat::Sql,
        None,
    )
    .await;

    assert_eq!(sql.rows_exported, 1);
    assert!(
        sql.output
            .contains(r#"'uuid_001', '{"metric":"sales","label":"中文"}'"#),
        "LONGTEXT should be exported as a quoted string:\n{}",
        sql.output
    );
    assert!(
        !sql.output
            .to_ascii_lowercase()
            .contains("x'7b226d657472696322"),
        "LONGTEXT JSON must not be re-encoded as a binary hex literal:\n{}",
        sql.output
    );
    assert!(
        sql.output.contains("X'000102ff'"),
        "real LONGBLOB bytes should remain a binary hex literal:\n{}",
        sql.output
    );

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
            "CREATE DATABASE `{database}` CHARACTER SET utf8mb4;\nCREATE TABLE `{database}`.data (id INT PRIMARY KEY, text_value TEXT);\nINSERT INTO `{database}`.data VALUES (1, '中文 🚀'), (2, NULL);"
        ),
    )
    .await;
}

async fn export(
    plugin: &MySqlPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    database: &str,
    format: DataFormat,
    columns: Option<Vec<String>>,
) -> db::import_export::ExportResult {
    plugin
        .export_data(
            connection,
            &ExportConfig {
                format,
                database: database.to_string(),
                tables: vec!["data".into()],
                columns,
                include_schema: true,
                include_data: true,
                ..Default::default()
            },
        )
        .await
        .expect("MySQL export should succeed")
}

async fn import(
    plugin: &MySqlPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    database: &str,
    format: DataFormat,
    data: &str,
) {
    let result = plugin
        .import_data(
            connection,
            &ImportConfig {
                format,
                database: database.to_string(),
                table: Some("data".into()),
                truncate_before_import: true,
                use_transaction: true,
                csv_config: if format == DataFormat::Csv {
                    Some(CsvImportConfig::default())
                } else {
                    None
                },
                ..Default::default()
            },
            data,
        )
        .await
        .expect("MySQL import should succeed");
    assert!(result.success, "{:?}", result.errors);
    assert_eq!(result.rows_imported, 2);
}

async fn verify(
    plugin: &MySqlPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    database: &str,
) {
    let result = connection
        .query(&format!(
            "SELECT id, text_value FROM `{database}`.data ORDER BY id"
        ))
        .await
        .expect("verify MySQL import");
    let db::executor::SqlResult::Query(result) = result else {
        panic!("verification should be a query");
    };
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][1].as_deref(), Some("中文 🚀"));
    assert_eq!(result.rows[1][1], None);
    assert_no_execute_errors(plugin, connection, "SELECT 1").await;
}

async fn assert_no_execute_errors(
    plugin: &MySqlPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    sql: &str,
) {
    let results = connection
        .execute(plugin, sql, ExecOptions::default())
        .await
        .expect("execute should return");
    crate::real_databases::common::assertions::assert_no_sql_errors(&results, sql);
}
