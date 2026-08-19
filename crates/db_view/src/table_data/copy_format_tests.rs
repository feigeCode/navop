use db::ipc::{ExternalDatabasePlugin, IpcDriverManifest};
use db::{ColumnInfo, DatabasePlugin, DbManager};
use one_core::storage::DatabaseType;
use std::path::PathBuf;

use super::*;

fn column(name: &str, data_type: &str) -> ColumnInfo {
    ColumnInfo {
        name: name.to_string(),
        data_type: data_type.to_string(),
        is_nullable: true,
        is_primary_key: name == "id",
        default_value: None,
        comment: None,
        charset: None,
        collation: None,
    }
}

fn sql_context<'a>(
    input: (
        &'a [Vec<Option<String>>],
        &'a [SharedString],
        &'a TableMetadata,
    ),
    plugin: &'a dyn DatabasePlugin,
) -> CopyFormatContext<'a> {
    let (data, columns, metadata) = input;
    CopyFormatContext::new(data, columns, metadata).with_plugin(plugin)
}

fn compatible_driver_plugin(database_type: DatabaseType) -> ExternalDatabasePlugin {
    let mut manifest: IpcDriverManifest = serde_json::from_value(serde_json::json!({
        "id": "test-compatible",
        "name": "Test Compatible",
        "entry": { "command": "driver" },
        "transport": { "name": "test-compatible.sock" }
    }))
    .expect("compatible driver manifest should parse");
    manifest.manifest_dir = PathBuf::from("/drivers/test-compatible");
    manifest.dialect.compatible_database_type = Some(database_type);
    ExternalDatabasePlugin::for_driver(manifest)
}

#[test]
fn formats_csv_and_structured_null_values() {
    let data = vec![vec![None, Some(String::new()), Some("NULL".into())]];
    let columns = vec!["nullable".into(), "empty".into(), "literal".into()];
    let metadata = TableMetadata::new("values_table");

    assert_eq!(
        CopyFormatter::format(
            CopyFormat::Csv,
            CopyFormatContext::new(&data, &columns, &metadata)
        ),
        "\\N,\"\",NULL"
    );
    assert_eq!(
        CopyFormatter::format(
            CopyFormat::Tsv,
            CopyFormatContext::new(&data, &columns, &metadata)
        ),
        "\\N\t\tNULL"
    );
    assert!(
        CopyFormatter::format(
            CopyFormat::Json,
            CopyFormatContext::new(&data, &columns, &metadata)
        )
        .contains("\"nullable\": null")
    );
}

#[test]
fn mysql_insert_uses_typed_bit_and_string_literals() {
    let manager = DbManager::default();
    let plugin = manager.get_plugin(&DatabaseType::MySQL).unwrap();
    let data = vec![vec![Some("1".into()), Some("1".into()), Some("1".into())]];
    let columns = vec!["id".into(), "bit_name".into(), "text_name".into()];
    let metadata = TableMetadata::new("test_bit")
        .with_columns(columns.clone())
        .with_column_meta(vec![
            column("id", "INT"),
            column("bit_name", "BIT(1)"),
            column("text_name", "VARCHAR(20)"),
        ])
        .with_primary_keys(vec![0]);

    let sql = CopyFormatter::format(
        CopyFormat::SqlInsert,
        sql_context((&data, &columns, &metadata), plugin.as_ref()),
    );

    assert_eq!(
        sql,
        "INSERT INTO `test_bit` (`id`, `bit_name`, `text_name`) VALUES\n(1, 1, '1');"
    );
}

#[test]
fn every_builtin_database_uses_its_typed_literals() {
    let cases = [
        (DatabaseType::PostgreSQL, "BOOLEAN", "TRUE"),
        (DatabaseType::SQLite, "BOOLEAN", "1"),
        (DatabaseType::MSSQL, "BIT", "1"),
        (DatabaseType::Oracle, "BOOLEAN", "TRUE"),
        (DatabaseType::ClickHouse, "Boolean", "true"),
    ];
    for (database_type, data_type, expected) in cases {
        assert_typed_insert(database_type, data_type, expected);
    }
}

#[test]
fn compatible_external_database_uses_duckdb_typed_literals() {
    let plugin = compatible_driver_plugin(DatabaseType::DuckDB);
    let data = vec![vec![Some("true".into())]];
    let columns = vec!["enabled".into()];
    let metadata = TableMetadata::new("flags")
        .with_columns(columns.clone())
        .with_column_meta(vec![column("enabled", "BOOLEAN")]);

    let sql = CopyFormatter::format(
        CopyFormat::SqlInsert,
        sql_context((&data, &columns, &metadata), &plugin),
    );

    assert!(sql.contains("VALUES\n(TRUE);"), "{sql}");
}

fn assert_typed_insert(database_type: DatabaseType, data_type: &str, expected: &str) {
    let manager = DbManager::default();
    let plugin = manager.get_plugin(&database_type).unwrap();
    let data = vec![vec![Some("true".into())]];
    let columns = vec!["enabled".into()];
    let metadata = TableMetadata::new("flags")
        .with_columns(columns.clone())
        .with_column_meta(vec![column("enabled", data_type)]);
    let sql = CopyFormatter::format(
        CopyFormat::SqlInsert,
        sql_context((&data, &columns, &metadata), plugin.as_ref()),
    );

    assert!(
        sql.contains(&format!("VALUES\n({expected});")),
        "{database_type:?}: {sql}"
    );
}

#[test]
fn selected_metadata_remaps_primary_keys_and_types() {
    let metadata = TableMetadata::new("items")
        .with_columns(vec!["id", "enabled", "name"])
        .with_column_meta(vec![
            column("id", "INT"),
            column("enabled", "BIT"),
            column("name", "VARCHAR(20)"),
        ])
        .with_primary_keys(vec![0, 2]);

    let selected = metadata.select_columns(&[2, 1]);

    assert_eq!(selected.column_names, vec!["name", "enabled"]);
    assert_eq!(selected.primary_key_indices, vec![0]);
    assert_eq!(
        selected.column_meta[1]
            .as_ref()
            .map(|column| column.data_type.as_str()),
        Some("BIT")
    );
}

#[test]
fn export_metadata_preserves_known_types_when_one_column_is_unknown() {
    let metadata = TableMetadata::new("items")
        .with_columns(vec!["id", "enabled"])
        .with_column_meta(vec![column("id", "INT"), column("enabled", "BIT")])
        .with_primary_keys(vec![0]);

    let selected = metadata.for_columns(&["enabled".into(), "computed".into()]);

    assert_eq!(
        selected.column_meta[0]
            .as_ref()
            .map(|column| column.data_type.as_str()),
        Some("BIT")
    );
    assert!(selected.column_meta[1].is_none());
    assert!(selected.primary_key_indices.is_empty());
}

#[test]
fn typed_numeric_rejects_malicious_literal() {
    let manager = DbManager::default();
    let plugin = manager.get_plugin(&DatabaseType::MySQL).unwrap();
    let data = vec![vec![Some("1); DROP TABLE users; --".into())]];
    let columns = vec!["id".into()];
    let metadata = TableMetadata::new("users")
        .with_columns(columns.clone())
        .with_column_meta(vec![column("id", "INT")]);
    let sql = CopyFormatter::format(
        CopyFormat::SqlInsert,
        sql_context((&data, &columns, &metadata), plugin.as_ref()),
    );

    assert!(sql.contains("'1); DROP TABLE users; --'"));
}

#[test]
fn fallback_quotes_identifiers_and_non_finite_numbers() {
    let data = vec![
        vec![Some("NaN".into())],
        vec![Some("inf".into())],
        vec![Some("1.5".into())],
    ];
    let columns = vec!["value name".into()];
    let metadata = TableMetadata::new("odd\" table").with_columns(columns.clone());

    let sql = CopyFormatter::format(
        CopyFormat::SqlInsert,
        CopyFormatContext::new(&data, &columns, &metadata),
    );

    assert_eq!(
        sql,
        concat!(
            "INSERT INTO \"odd\"\" table\" (\"value name\") VALUES\n",
            "('NaN'),\n",
            "('inf'),\n",
            "(1.5);",
        )
    );
}

#[test]
fn sql_mutations_preserve_typed_values_and_null_predicates() {
    let plugin = DbManager::default()
        .get_plugin(&DatabaseType::MySQL)
        .expect("MySQL plugin should exist");
    let data = vec![vec![Some("1".into()), Some("1".into()), None]];
    let columns = vec!["id".into(), "enabled".into(), "note".into()];
    let metadata = TableMetadata::new("test_bit")
        .with_columns(columns.clone())
        .with_column_meta(vec![
            column("id", "INT"),
            column("enabled", "BIT(1)"),
            column("note", "VARCHAR(20)"),
        ])
        .with_primary_keys(vec![0]);
    let context = sql_context((&data, &columns, &metadata), plugin.as_ref());

    assert_eq!(
        CopyFormatter::format(CopyFormat::SqlUpdate, context),
        "UPDATE `test_bit` SET `enabled` = 1, `note` = NULL WHERE `id` = 1;"
    );

    let context = sql_context((&data, &columns, &metadata), plugin.as_ref());
    assert_eq!(
        CopyFormatter::format(CopyFormat::SqlDelete, context),
        "DELETE FROM `test_bit` WHERE `id` = 1;"
    );

    let in_data = vec![vec![Some("1".into())], vec![Some("0".into())], vec![None]];
    let in_columns = vec!["enabled".into()];
    let in_metadata = TableMetadata::new("test_bit")
        .with_columns(in_columns.clone())
        .with_column_meta(vec![column("enabled", "BIT(1)")]);
    let context = sql_context((&in_data, &in_columns, &in_metadata), plugin.as_ref());
    assert_eq!(
        CopyFormatter::format(CopyFormat::SqlIn, context),
        "(`enabled` IN (1, 0) OR `enabled` IS NULL)"
    );
}
