use db::ipc::{ExternalDatabasePlugin, IpcDriverManifest};
use db::{BinaryCell, ColumnInfo, DatabasePlugin, DbManager};
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

fn binary_cell(row_index: usize, column_index: usize, bytes: &[u8]) -> BinaryCell {
    BinaryCell {
        row_index,
        column_index,
        bytes: bytes.to_vec(),
    }
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
fn json_escapes_column_names_and_only_emits_strict_json_numbers() {
    let data = vec![vec![
        Some("1".into()),
        Some("-1.5e2".into()),
        Some("+1".into()),
        Some("01".into()),
        Some("1.".into()),
        Some("NaN".into()),
        Some("inf".into()),
    ]];
    let columns = vec![
        "a\"b\\c\n".into(),
        "exponent".into(),
        "plus".into(),
        "leading_zero".into(),
        "trailing_dot".into(),
        "nan".into(),
        "infinity".into(),
    ];
    let metadata = TableMetadata::new("values_table");

    let json = CopyFormatter::format(
        CopyFormat::Json,
        CopyFormatContext::new(&data, &columns, &metadata),
    );
    let parsed: serde_json::Value =
        serde_json::from_str(&json).expect("copy output should be valid JSON");
    let row = parsed[0].as_object().expect("row should be a JSON object");

    assert_eq!(row["a\"b\\c\n"], serde_json::json!(1));
    assert_eq!(row["exponent"].as_f64(), Some(-1.5e2));
    assert_eq!(row["plus"], serde_json::json!("+1"));
    assert_eq!(row["leading_zero"], serde_json::json!("01"));
    assert_eq!(row["trailing_dot"], serde_json::json!("1."));
    assert_eq!(row["nan"], serde_json::json!("NaN"));
    assert_eq!(row["infinity"], serde_json::json!("inf"));
}

#[test]
fn plain_copy_formats_use_binary_sidecar_without_guessing_text() {
    let data = vec![
        vec![
            Some("wrong".into()),
            Some("true".into()),
            Some("8000".into()),
            Some("AQID".into()),
        ],
        vec![None, Some("base64:AQID".into()), None, None],
    ];
    let columns = vec![
        "binary".into(),
        "boolean_text".into(),
        "number_text".into(),
        "base64_shaped_text".into(),
    ];
    let metadata = TableMetadata::new("values_table");
    let binary_cells = vec![binary_cell(0, 0, &[1, 2, 3]), binary_cell(1, 0, &[])];
    let context =
        CopyFormatContext::new(&data, &columns, &metadata).with_binary_cells(&binary_cells);

    assert_eq!(
        CopyFormatter::format(CopyFormat::Tsv, context),
        "base64:AQID\ttrue\t8000\tAQID\nbase64:\tbase64:AQID\t\\N\t\\N"
    );
    assert_eq!(
        CopyFormatter::format(CopyFormat::Csv, context),
        "base64:AQID,true,8000,AQID\nbase64:,base64:AQID,\\N,\\N"
    );

    let json = CopyFormatter::format(CopyFormat::Json, context);
    assert!(json.contains("\"binary\": \"base64:AQID\""), "{json}");
    assert!(json.contains("\"binary\": \"base64:\""), "{json}");
    assert!(json.contains("\"base64_shaped_text\": \"AQID\""), "{json}");

    let markdown = CopyFormatter::format(CopyFormat::Markdown, context);
    assert!(
        markdown.contains("| base64:AQID | true | 8000 | AQID |"),
        "{markdown}"
    );
    assert!(
        markdown.contains("| base64: | base64:AQID | \\N | \\N |"),
        "{markdown}"
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
fn mysql_sql_formats_use_binary_sidecar_as_authoritative_bytes() {
    let manager = DbManager::default();
    let plugin = manager.get_plugin(&DatabaseType::MySQL).unwrap();
    let data = vec![vec![None, Some("wrong".into())]];
    let columns = vec!["binary_id".into(), "payload".into()];
    let metadata = TableMetadata::new("binary_values")
        .with_columns(columns.clone())
        .with_column_meta(vec![
            column("binary_id", "VARBINARY(8)"),
            column("payload", "BLOB"),
        ])
        .with_primary_keys(vec![0]);
    let binary_cells = vec![
        binary_cell(0, 0, &[1, 2, 3]),
        binary_cell(0, 1, &[0xde, 0xad]),
    ];
    let context =
        sql_context((&data, &columns, &metadata), plugin.as_ref()).with_binary_cells(&binary_cells);

    assert_eq!(
        CopyFormatter::format(CopyFormat::SqlInsert, context),
        "INSERT INTO `binary_values` (`binary_id`, `payload`) VALUES\n(X'010203', X'dead');"
    );
    assert_eq!(
        CopyFormatter::format(CopyFormat::SqlUpdate, context),
        "UPDATE `binary_values` SET `payload` = X'dead' WHERE `binary_id` = X'010203';"
    );
    assert_eq!(
        CopyFormatter::format(CopyFormat::SqlDelete, context),
        "DELETE FROM `binary_values` WHERE `binary_id` = X'010203';"
    );
}

#[test]
fn single_column_in_preserves_binary_rows_and_distinguishes_null() {
    let manager = DbManager::default();
    let plugin = manager.get_plugin(&DatabaseType::MySQL).unwrap();
    let data = vec![
        vec![None],
        vec![None],
        vec![Some("plain".into())],
        vec![Some("wrong".into())],
    ];
    let columns = vec!["value".into()];
    let metadata = TableMetadata::new("binary_values")
        .with_columns(columns.clone())
        .with_column_meta(vec![column("value", "BLOB")]);
    let binary_cells = vec![binary_cell(1, 0, &[]), binary_cell(3, 0, &[1, 2, 3])];
    let context =
        sql_context((&data, &columns, &metadata), plugin.as_ref()).with_binary_cells(&binary_cells);

    assert_eq!(
        CopyFormatter::format(CopyFormat::SqlIn, context),
        "(`value` IN (X'', X'706c61696e', X'010203') OR `value` IS NULL)"
    );
}

#[test]
fn fallback_sql_uses_standard_hex_literal_for_binary_sidecar() {
    let data = vec![vec![Some("wrong".into())]];
    let columns = vec!["payload".into()];
    let metadata = TableMetadata::new("binary_values").with_columns(columns.clone());
    let binary_cells = vec![binary_cell(0, 0, &[0, 0xff])];
    let context =
        CopyFormatContext::new(&data, &columns, &metadata).with_binary_cells(&binary_cells);

    assert_eq!(
        CopyFormatter::format(CopyFormat::SqlInsert, context),
        "INSERT INTO \"binary_values\" (\"payload\") VALUES\n(X'00ff');"
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

#[test]
fn sql_mutations_require_non_null_primary_keys() {
    let plugin = DbManager::default()
        .get_plugin(&DatabaseType::MySQL)
        .expect("MySQL plugin should exist");
    let columns = vec!["id".into(), "name".into()];
    let no_key_metadata = TableMetadata::new("items")
        .with_columns(columns.clone())
        .with_column_meta(vec![column("id", "INT"), column("name", "VARCHAR(20)")]);
    let data = vec![vec![Some("1".into()), Some("safe".into())]];
    let context = sql_context((&data, &columns, &no_key_metadata), plugin.as_ref());

    assert_eq!(CopyFormatter::format(CopyFormat::SqlUpdate, context), "");
    assert_eq!(CopyFormatter::format(CopyFormat::SqlDelete, context), "");

    let keyed_metadata = no_key_metadata.with_primary_keys(vec![0]);
    let null_key_data = vec![vec![None, Some("unsafe".into())]];
    let context = sql_context((&null_key_data, &columns, &keyed_metadata), plugin.as_ref());
    assert_eq!(CopyFormatter::format(CopyFormat::SqlUpdate, context), "");
    assert_eq!(CopyFormatter::format(CopyFormat::SqlDelete, context), "");
}

#[test]
fn sql_insert_and_multicolumn_in_reject_ragged_rows() {
    let data = vec![
        vec![Some("1".into()), Some("a".into())],
        vec![Some("2".into())],
    ];
    let columns = vec!["id".into(), "name".into()];
    let metadata = TableMetadata::new("items").with_columns(columns.clone());
    let context = CopyFormatContext::new(&data, &columns, &metadata);

    assert_eq!(CopyFormatter::format(CopyFormat::SqlInsert, context), "");
    assert_eq!(CopyFormatter::format(CopyFormat::SqlIn, context), "");
}
