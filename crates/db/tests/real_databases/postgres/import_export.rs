use db::connection::DbConnection;
use db::executor::SqlResult;
use db::import_export::{CsvImportConfig, DataFormat, ExportConfig, ImportConfig};
use db::plugin::DatabasePlugin;
use db::postgresql::PostgresPlugin;

use crate::real_databases::common::env::{optional_database, postgres_config, skip_database};
use crate::real_databases::postgres::core_flow::{
    drop_schema, execute, reset_schema, unique_schema,
};

#[tokio::test]
async fn postgres_real_import_export_round_trips_sql_csv_and_json() {
    let Some(config) = postgres_config() else {
        skip_database("PostgreSQL", "ONETCLI_TEST_POSTGRES_PASSWORD (empty string is valid)");
        return;
    };
    let config = optional_database(
        &config,
        &std::env::var("ONETCLI_TEST_POSTGRES_DATABASE").unwrap_or_else(|_| "postgres".to_string()),
    );
    let schema = unique_schema("io");
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
        .expect("switch to import/export schema");
    let sql = export(&plugin, connection.as_ref(), &schema, DataFormat::Sql, None).await;
    assert_eq!(sql.rows_exported, 2);
    assert!(sql.output.to_uppercase().contains("INSERT INTO"));

    let text_columns = Some(vec!["id".into(), "text_value".into()]);
    let csv = export(
        &plugin,
        connection.as_ref(),
        &schema,
        DataFormat::Csv,
        text_columns.clone(),
    )
    .await;
    let json = export(
        &plugin,
        connection.as_ref(),
        &schema,
        DataFormat::Json,
        text_columns,
    )
    .await;
    import(
        &plugin,
        connection.as_ref(),
        &schema,
        DataFormat::Csv,
        &csv.output,
    )
    .await;
    import(
        &plugin,
        connection.as_ref(),
        &schema,
        DataFormat::Json,
        &json.output,
    )
    .await;
    verify(connection.as_ref(), &schema).await;

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
            "CREATE SCHEMA IF NOT EXISTS \"{schema}\";\nCREATE TABLE \"{schema}\".data (\
             id INT PRIMARY KEY, text_value TEXT);\nINSERT INTO \"{schema}\".data VALUES \
             (1, '中文 🚀'), (2, NULL);"
        ),
    )
    .await;
}

async fn export(
    plugin: &PostgresPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    schema: &str,
    format: DataFormat,
    columns: Option<Vec<String>>,
) -> db::import_export::ExportResult {
    plugin
        .export_data(
            connection,
            &ExportConfig {
                format,
                database: connection
                    .current_database()
                    .await
                    .expect("current database")
                    .unwrap_or_default(),
                schema: Some(schema.to_string()),
                tables: vec!["data".into()],
                columns,
                include_schema: true,
                include_data: true,
                ..Default::default()
            },
        )
        .await
        .expect("PostgreSQL export should succeed")
}

async fn import(
    plugin: &PostgresPlugin,
    connection: &(dyn DbConnection + Send + Sync),
    schema: &str,
    format: DataFormat,
    data: &str,
) {
    let result = plugin
        .import_data(
            connection,
            &ImportConfig {
                format,
                database: connection
                    .current_database()
                    .await
                    .expect("current database")
                    .unwrap_or_default(),
                schema: Some(schema.to_string()),
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
        .expect("PostgreSQL import should succeed");
    assert!(result.success, "{:?}", result.errors);
    assert_eq!(result.rows_imported, 2);
}

async fn verify(connection: &(dyn DbConnection + Send + Sync), schema: &str) {
    let result = connection
        .query(&format!(
            "SELECT id, text_value FROM \"{schema}\".data ORDER BY id"
        ))
        .await
        .expect("verify PostgreSQL import");
    let SqlResult::Query(result) = result else {
        panic!("verification should be a query");
    };
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][1].as_deref(), Some("中文 🚀"));
    assert_eq!(result.rows[1][1], None);
}
