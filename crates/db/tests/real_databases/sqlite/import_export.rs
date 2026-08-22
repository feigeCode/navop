use std::path::{Path, PathBuf};

use db::connection::DbConnection;
use db::executor::{ExecOptions, SqlResult};
use db::import_export::{CsvImportConfig, DataFormat, ExportConfig, ImportConfig};
use db::plugin::DatabasePlugin;
use db::sqlite::{SqliteDbConnection, SqlitePlugin};
use one_core::storage::{DatabaseType, DbConnectionConfig};

use crate::real_databases::common::assertions::{
    assert_binary, assert_cell, assert_columns, assert_no_sql_errors as assert_no_errors,
    assert_null,
};
use crate::real_databases::common::fixture::file_config;

fn config(id: &str, path: &Path) -> DbConnectionConfig {
    file_config(id, DatabaseType::SQLite, path)
}

#[tokio::test]
async fn sqlite_real_import_export_round_trips_sql_xml_csv_and_json() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let plugin = SqlitePlugin::new();
    let mut source = create_source(&plugin, temp_dir.path().join("source.db")).await;
    assert_sql_export(&plugin, &source).await;
    assert_xml_round_trip(&plugin, temp_dir.path().join("xml-target.db"), &source).await;
    assert_csv_and_json(&plugin, temp_dir.path().join("table-target.db"), &source).await;
    source.disconnect().await.expect("disconnect source");
}

async fn create_source(plugin: &SqlitePlugin, path: PathBuf) -> SqliteDbConnection {
    let mut connection = SqliteDbConnection::new(config("sqlite-export-source", &path));
    connection.connect().await.expect("connect source");
    assert_no_errors(&connection.execute(plugin, "CREATE TABLE data (id INTEGER PRIMARY KEY, text_value TEXT, payload BLOB); INSERT INTO data VALUES (1, '中文 🚀', X'0001FF'), (2, NULL, X'');", ExecOptions::default()).await.expect("fixture"), "fixture");
    connection
}

async fn export(
    plugin: &SqlitePlugin,
    connection: &SqliteDbConnection,
    format: DataFormat,
) -> db::import_export::ExportResult {
    plugin
        .export_data(
            connection,
            &ExportConfig {
                format,
                database: "main".into(),
                tables: vec!["data".into()],
                include_schema: true,
                include_data: true,
                ..Default::default()
            },
        )
        .await
        .expect("export should succeed")
}

async fn export_text(
    plugin: &SqlitePlugin,
    connection: &SqliteDbConnection,
    format: DataFormat,
) -> db::import_export::ExportResult {
    plugin
        .export_data(
            connection,
            &ExportConfig {
                format,
                database: "main".into(),
                tables: vec!["data".into()],
                columns: Some(vec!["id".into(), "text_value".into()]),
                include_schema: true,
                include_data: true,
                ..Default::default()
            },
        )
        .await
        .expect("text export should succeed")
}

async fn assert_sql_export(plugin: &SqlitePlugin, connection: &SqliteDbConnection) {
    let result = export(plugin, connection, DataFormat::Sql).await;
    assert_eq!(result.rows_exported, 2);
    assert!(result.output.to_uppercase().contains("INSERT INTO"));
}

async fn assert_xml_round_trip(plugin: &SqlitePlugin, path: PathBuf, source: &SqliteDbConnection) {
    let exported = export(plugin, source, DataFormat::Xml).await;
    let mut target = SqliteDbConnection::new(config("sqlite-xml-target", &path));
    target.connect().await.expect("connect target");
    assert_no_errors(
        &target
            .execute(
                plugin,
                "CREATE TABLE data (id INTEGER PRIMARY KEY, text_value TEXT, payload BLOB);",
                ExecOptions::default(),
            )
            .await
            .expect("target schema"),
        "target schema",
    );
    let imported = plugin
        .import_data(
            &target,
            &ImportConfig {
                format: DataFormat::Xml,
                database: "main".into(),
                table: Some("data".into()),
                truncate_before_import: false,
                use_transaction: true,
                ..Default::default()
            },
            &exported.output,
        )
        .await
        .expect("import");
    assert!(imported.success, "{:?}", imported.errors);
    verify_data(plugin, &target).await;
    target.disconnect().await.expect("disconnect target");
}

async fn assert_csv_and_json(plugin: &SqlitePlugin, path: PathBuf, source: &SqliteDbConnection) {
    let csv = export_text(plugin, source, DataFormat::Csv).await;
    let json = export_text(plugin, source, DataFormat::Json).await;
    let mut target = SqliteDbConnection::new(config("sqlite-table-target", &path));
    target.connect().await.expect("connect target");
    assert_no_errors(
        &target
            .execute(
                plugin,
                "CREATE TABLE data (id INTEGER PRIMARY KEY, text_value TEXT);",
                ExecOptions::default(),
            )
            .await
            .expect("target schema"),
        "target schema",
    );
    let imported_csv = plugin
        .import_data(
            &target,
            &ImportConfig {
                format: DataFormat::Csv,
                database: "main".into(),
                table: Some("data".into()),
                truncate_before_import: false,
                csv_config: Some(CsvImportConfig::default()),
                ..Default::default()
            },
            &csv.output,
        )
        .await
        .expect("CSV import");
    assert!(imported_csv.success, "{:?}", imported_csv.errors);
    assert_eq!(imported_csv.rows_imported, 2);

    assert_no_errors(
        &target
            .execute(plugin, "DELETE FROM data", ExecOptions::default())
            .await
            .expect("clear"),
        "clear",
    );
    let imported_json = plugin
        .import_data(
            &target,
            &ImportConfig {
                format: DataFormat::Json,
                database: "main".into(),
                table: Some("data".into()),
                ..Default::default()
            },
            &json.output,
        )
        .await
        .expect("JSON import");
    assert!(imported_json.success, "{:?}", imported_json.errors);
    assert_eq!(imported_json.rows_imported, 2);
    let result = target
        .query("SELECT id, text_value FROM data ORDER BY id")
        .await
        .expect("verify JSON");
    let SqlResult::Query(result) = result else {
        panic!("query")
    };
    assert_columns(&result, &["id", "text_value"]);
    assert_cell(&result, 0, 1, "中文 🚀");
    assert_null(&result, 1, 1);
    target.disconnect().await.expect("disconnect target");
}

async fn verify_data(plugin: &SqlitePlugin, connection: &SqliteDbConnection) {
    let result = connection
        .query("SELECT id, text_value, payload FROM data ORDER BY id")
        .await
        .expect("verify");
    let SqlResult::Query(result) = result else {
        panic!("query")
    };
    assert_eq!(result.rows.len(), 2);
    assert_cell(&result, 0, 1, "中文 🚀");
    assert_null(&result, 1, 1);
    assert_binary(&result, 0, 2, &[0, 1, 255]);
    assert_binary(&result, 1, 2, &[]);
    let _ = plugin;
}
