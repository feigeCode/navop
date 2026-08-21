use std::collections::{HashMap, HashSet};

use encoding_rs::{BIG5, EUC_JP, EUC_KR, Encoding, GB18030, GBK, SHIFT_JIS, WINDOWS_1252};
use one_core::storage::DatabaseType;

use crate::{
    ColumnInfo, DatabasePlugin, FieldType, QueryColumnMeta, QueryResult, connection::DbConnection,
    executor::QueryResultError,
};

/// Errors raised while reconciling runtime query values with authoritative table schema metadata.
#[derive(Debug, thiserror::Error)]
pub enum QueryResultNormalizationError {
    #[error("invalid query result: {0}")]
    InvalidQueryResult(#[from] QueryResultError),
    #[error("query result contains duplicate column name `{column_name}`")]
    DuplicateResultColumn { column_name: String },
    #[error(
        "query result column {column_index} (`{column_name}`) does not exist in the table schema"
    )]
    SchemaColumnNotFound {
        column_index: usize,
        column_name: String,
    },
    #[error(
        "query result column {column_index} (`{column_name}`) matches multiple table schema columns"
    )]
    SchemaColumnAmbiguous {
        column_index: usize,
        column_name: String,
    },
    #[error(
        "MySQL text column `{column_name}` at row {row_index}, column {column_index} uses unsupported charset `{charset}`"
    )]
    UnsupportedTextCharset {
        row_index: usize,
        column_index: usize,
        column_name: String,
        charset: String,
    },
    #[error(
        "MySQL text column `{column_name}` at row {row_index}, column {column_index} contains bytes that are invalid for charset `{charset}`"
    )]
    InvalidTextEncoding {
        row_index: usize,
        column_index: usize,
        column_name: String,
        charset: String,
    },
}

/// Load authoritative schema metadata and normalize a direct table-query result.
///
/// The metadata lookup is deliberately skipped unless a built-in MySQL result
/// actually contains binary sidecars, so ordinary text-only pages and all other
/// database types keep their existing query cost.
pub async fn normalize_table_query_result<P>(
    plugin: &P,
    connection: &dyn DbConnection,
    database: &str,
    schema: Option<&str>,
    table: &str,
    query_result: &mut QueryResult,
) -> anyhow::Result<()>
where
    P: DatabasePlugin + ?Sized,
{
    if plugin.name() != DatabaseType::MySQL || query_result.binary_cells.is_empty() {
        return Ok(());
    }

    let schema_columns = plugin
        .list_columns(connection, database, schema.map(str::to_string), table)
        .await?;
    normalize_query_result_binary_semantics(query_result, &DatabaseType::MySQL, &schema_columns)
        .map_err(Into::into)
}

/// Reconcile table-query runtime values with authoritative schema semantics.
///
/// MySQL's wire protocol uses the BLOB type-code family for both BLOB and TEXT
/// variants. Most servers distinguish them through collation metadata, but
/// compatible servers and proxies can report a non-binary TEXT column as a
/// binary wire value. For a direct table query the table schema is the
/// authoritative source, so TEXT-family sidecars are decoded back to text while
/// real BLOB values remain byte-exact.
///
/// Arbitrary SQL results are intentionally not passed through this function:
/// joins, aliases, and expressions cannot be mapped safely to one table schema.
pub fn normalize_query_result_binary_semantics(
    query_result: &mut QueryResult,
    database_type: &DatabaseType,
    schema_columns: &[ColumnInfo],
) -> Result<(), QueryResultNormalizationError> {
    query_result.typed_view()?;

    if *database_type != DatabaseType::MySQL {
        return Ok(());
    }

    let schema_mapping = project_schema_columns(&query_result.columns, schema_columns)?;
    let text_column_indices = schema_mapping
        .iter()
        .enumerate()
        .filter_map(|(index, column)| {
            mysql_schema_column_is_character_text(column).then_some(index)
        })
        .collect::<HashSet<_>>();

    if text_column_indices.is_empty() || query_result.binary_cells.is_empty() {
        apply_schema_column_metadata(query_result, &schema_mapping);
        return Ok(());
    }

    let decoded_text_cells = query_result
        .binary_cells
        .iter()
        .filter(|cell| text_column_indices.contains(&cell.column_index))
        .map(|cell| {
            let column = &schema_mapping[cell.column_index];
            decode_mysql_text_bytes(column, &cell.bytes, cell.row_index, cell.column_index)
                .map(|text| (cell.row_index, cell.column_index, text))
        })
        .collect::<Result<Vec<_>, _>>()?;

    apply_schema_column_metadata(query_result, &schema_mapping);
    for (row_index, column_index, text) in decoded_text_cells {
        query_result.rows[row_index][column_index] = Some(text);
    }

    let mut retained_binary_cells = Vec::with_capacity(query_result.binary_cells.len());
    for cell in query_result.binary_cells.drain(..) {
        if !text_column_indices.contains(&cell.column_index) {
            retained_binary_cells.push(cell);
        }
    }
    query_result.binary_cells = retained_binary_cells;

    Ok(())
}

/// Project authoritative table schema columns into query-result column order.
///
/// Direct table queries may return a subset of the table columns and may order
/// them differently from the schema response. Matching is case-insensitive
/// after trimming surrounding whitespace, but every result column must map to
/// exactly one schema column.
pub fn project_schema_columns(
    result_columns: &[String],
    schema_columns: &[ColumnInfo],
) -> Result<Vec<ColumnInfo>, QueryResultNormalizationError> {
    let mut schema_indices_by_name = HashMap::<String, Vec<usize>>::new();
    for (index, column) in schema_columns.iter().enumerate() {
        schema_indices_by_name
            .entry(normalize_column_name(&column.name))
            .or_default()
            .push(index);
    }

    let mut seen_result_names = HashSet::new();
    result_columns
        .iter()
        .enumerate()
        .map(|(column_index, result_column)| {
            let normalized_name = normalize_column_name(result_column);
            if !seen_result_names.insert(normalized_name.clone()) {
                return Err(QueryResultNormalizationError::DuplicateResultColumn {
                    column_name: result_column.clone(),
                });
            }

            let Some(schema_indices) = schema_indices_by_name.get(&normalized_name) else {
                return Err(QueryResultNormalizationError::SchemaColumnNotFound {
                    column_index,
                    column_name: result_column.clone(),
                });
            };
            if schema_indices.len() != 1 {
                return Err(QueryResultNormalizationError::SchemaColumnAmbiguous {
                    column_index,
                    column_name: result_column.clone(),
                });
            }

            Ok(schema_columns[schema_indices[0]].clone())
        })
        .collect()
}

fn normalize_column_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn apply_schema_column_metadata(query_result: &mut QueryResult, schema_columns: &[ColumnInfo]) {
    query_result.column_meta = schema_columns
        .iter()
        .map(|column| {
            let mut metadata = QueryColumnMeta::new(&column.name, &column.data_type);
            metadata.nullable = column.is_nullable;
            metadata
        })
        .collect();
}

fn decode_mysql_text_bytes(
    column: &ColumnInfo,
    bytes: &[u8],
    row_index: usize,
    column_index: usize,
) -> Result<String, QueryResultNormalizationError> {
    let charset = mysql_text_charset(column).ok_or_else(|| {
        QueryResultNormalizationError::UnsupportedTextCharset {
            row_index,
            column_index,
            column_name: column.name.clone(),
            charset: "<missing>".to_string(),
        }
    })?;

    let invalid = || QueryResultNormalizationError::InvalidTextEncoding {
        row_index,
        column_index,
        column_name: column.name.clone(),
        charset: charset.clone(),
    };

    match charset.as_str() {
        "utf8" | "utf8mb3" | "utf8mb4" => std::str::from_utf8(bytes)
            .map(str::to_string)
            .map_err(|_| invalid()),
        "ascii" => {
            if bytes.iter().all(u8::is_ascii) {
                Ok(bytes.iter().map(|byte| char::from(*byte)).collect())
            } else {
                Err(invalid())
            }
        }
        "latin1" => decode_with_encoding(WINDOWS_1252, bytes).ok_or_else(invalid),
        "gbk" | "gb2312" => decode_with_encoding(GBK, bytes).ok_or_else(invalid),
        "gb18030" => decode_with_encoding(GB18030, bytes).ok_or_else(invalid),
        "big5" => decode_with_encoding(BIG5, bytes).ok_or_else(invalid),
        "sjis" | "cp932" => decode_with_encoding(SHIFT_JIS, bytes).ok_or_else(invalid),
        "ujis" | "eucjpms" => decode_with_encoding(EUC_JP, bytes).ok_or_else(invalid),
        "euckr" => decode_with_encoding(EUC_KR, bytes).ok_or_else(invalid),
        _ => Err(QueryResultNormalizationError::UnsupportedTextCharset {
            row_index,
            column_index,
            column_name: column.name.clone(),
            charset,
        }),
    }
}

fn mysql_text_charset(column: &ColumnInfo) -> Option<String> {
    column
        .charset
        .as_deref()
        .map(str::trim)
        .filter(|charset| !charset.is_empty())
        .map(str::to_ascii_lowercase)
        .or_else(|| {
            column
                .collation
                .as_deref()
                .map(str::trim)
                .filter(|collation| !collation.is_empty())
                .and_then(|collation| collation.split('_').next())
                .map(str::to_ascii_lowercase)
        })
}

fn decode_with_encoding(encoding: &'static Encoding, bytes: &[u8]) -> Option<String> {
    encoding
        .decode_without_bom_handling_and_without_replacement(bytes)
        .map(|text| text.into_owned())
}

fn mysql_schema_column_is_character_text(column: &ColumnInfo) -> bool {
    if mysql_schema_column_uses_binary_charset(column) {
        return false;
    }

    matches!(
        FieldType::from_db_type(&column.data_type),
        FieldType::Text | FieldType::LongText
    )
}

fn mysql_schema_column_uses_binary_charset(column: &ColumnInfo) -> bool {
    column
        .charset
        .as_deref()
        .is_some_and(|charset| charset.trim().eq_ignore_ascii_case("binary"))
        || column
            .collation
            .as_deref()
            .is_some_and(|collation| collation.trim().eq_ignore_ascii_case("binary"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BinaryCell, FieldType};
    use encoding_rs::{BIG5, GB18030, GBK};

    fn column(name: &str, data_type: &str) -> ColumnInfo {
        ColumnInfo {
            name: name.to_string(),
            data_type: data_type.to_string(),
            is_nullable: true,
            is_primary_key: false,
            default_value: None,
            comment: None,
            charset: Some("utf8mb4".to_string()),
            collation: Some("utf8mb4_0900_ai_ci".to_string()),
        }
    }

    fn result(
        columns: &[&str],
        rows: Vec<Vec<Option<&str>>>,
        binary_cells: Vec<BinaryCell>,
    ) -> QueryResult {
        QueryResult {
            sql: "SELECT * FROM example".to_string(),
            columns: columns.iter().map(|column| (*column).to_string()).collect(),
            column_meta: columns
                .iter()
                .map(|column| QueryColumnMeta::new(*column, "MYSQL_TYPE_LONG_BLOB"))
                .collect(),
            rows: rows
                .into_iter()
                .map(|row| {
                    row.into_iter()
                        .map(|value| value.map(str::to_string))
                        .collect()
                })
                .collect(),
            binary_cells,
            elapsed_ms: 0,
        }
    }

    fn encoded_bytes(encoding: &'static Encoding, text: &str) -> Vec<u8> {
        let (bytes, _, had_errors) = encoding.encode(text);
        assert!(!had_errors);
        bytes.into_owned()
    }

    fn assert_result_unchanged(actual: &QueryResult, expected: &QueryResult) {
        assert_eq!(actual.sql, expected.sql);
        assert_eq!(actual.columns, expected.columns);
        assert_eq!(actual.rows, expected.rows);
        assert_eq!(actual.binary_cells, expected.binary_cells);
        assert_eq!(actual.elapsed_ms, expected.elapsed_ms);
        assert_eq!(actual.column_meta.len(), expected.column_meta.len());
        for (actual, expected) in actual.column_meta.iter().zip(&expected.column_meta) {
            assert_eq!(actual.name, expected.name);
            assert_eq!(actual.db_type, expected.db_type);
            assert_eq!(actual.field_type, expected.field_type);
            assert_eq!(actual.nullable, expected.nullable);
        }
    }

    #[test]
    fn schema_projection_supports_subsets_and_result_order() {
        let result_columns = vec![" payload ".to_string(), "ID".to_string()];
        let schema_columns = vec![
            column("id", "BIGINT"),
            column("unused_blob", "LONGBLOB"),
            column("Payload", "LONGTEXT"),
        ];

        let projected = project_schema_columns(&result_columns, &schema_columns).unwrap();

        assert_eq!(
            projected
                .iter()
                .map(|column| column.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Payload", "id"]
        );
    }

    #[test]
    fn mysql_longtext_sidecar_becomes_authoritative_text() {
        let mut result = result(
            &["payload"],
            vec![vec![Some("incorrect display value")]],
            vec![BinaryCell {
                row_index: 0,
                column_index: 0,
                bytes: b"true".to_vec(),
            }],
        );

        normalize_query_result_binary_semantics(
            &mut result,
            &DatabaseType::MySQL,
            &[column("payload", "LONGTEXT")],
        )
        .unwrap();

        assert!(result.binary_cells.is_empty());
        assert_eq!(result.rows, vec![vec![Some("true".to_string())]]);
        assert_eq!(result.column_meta[0].db_type, "LONGTEXT");
        assert_eq!(result.column_meta[0].field_type, FieldType::LongText);
    }

    #[test]
    fn mysql_text_family_sidecars_become_text() {
        for data_type in ["TEXT", "TINYTEXT", "MEDIUMTEXT", "LONGTEXT"] {
            let mut result = result(
                &["payload"],
                vec![vec![Some("0x74727565")]],
                vec![BinaryCell {
                    row_index: 0,
                    column_index: 0,
                    bytes: b"true".to_vec(),
                }],
            );

            normalize_query_result_binary_semantics(
                &mut result,
                &DatabaseType::MySQL,
                &[column("payload", data_type)],
            )
            .unwrap();

            assert!(result.binary_cells.is_empty(), "{data_type}");
            assert_eq!(result.rows[0][0].as_deref(), Some("true"), "{data_type}");
        }
    }

    #[test]
    fn mysql_character_string_family_sidecars_become_text() {
        for data_type in [
            "CHAR(10)",
            "VARCHAR(255)",
            "NCHAR(10)",
            "NVARCHAR(255)",
            "ENUM('a','b')",
            "SET('x','y')",
        ] {
            let mut result = result(
                &["payload"],
                vec![vec![Some("wrong")]],
                vec![BinaryCell {
                    row_index: 0,
                    column_index: 0,
                    bytes: b"true".to_vec(),
                }],
            );

            normalize_query_result_binary_semantics(
                &mut result,
                &DatabaseType::MySQL,
                &[column("payload", data_type)],
            )
            .unwrap();

            assert!(result.binary_cells.is_empty(), "{data_type}");
            assert_eq!(result.rows[0][0].as_deref(), Some("true"), "{data_type}");
        }
    }

    #[test]
    fn mysql_binary_charset_or_collation_keeps_sidecar() {
        for (charset, collation) in [
            (Some("binary"), Some("binary")),
            (Some("utf8mb4"), Some("binary")),
        ] {
            let mut schema_column = column("payload", "LONGTEXT");
            schema_column.charset = charset.map(str::to_string);
            schema_column.collation = collation.map(str::to_string);
            let mut result = result(
                &["payload"],
                vec![vec![Some("true")]],
                vec![BinaryCell {
                    row_index: 0,
                    column_index: 0,
                    bytes: b"true".to_vec(),
                }],
            );

            normalize_query_result_binary_semantics(
                &mut result,
                &DatabaseType::MySQL,
                &[schema_column],
            )
            .unwrap();

            assert_eq!(result.binary_cells.len(), 1);
        }
    }

    #[test]
    fn mysql_bin_sort_collation_is_still_character_text() {
        let mut schema_column = column("payload", "LONGTEXT");
        schema_column.collation = Some("utf8mb4_bin".to_string());
        let mut result = result(
            &["payload"],
            vec![vec![Some("wrong")]],
            vec![BinaryCell {
                row_index: 0,
                column_index: 0,
                bytes: b"true".to_vec(),
            }],
        );

        normalize_query_result_binary_semantics(
            &mut result,
            &DatabaseType::MySQL,
            &[schema_column],
        )
        .unwrap();

        assert!(result.binary_cells.is_empty());
        assert_eq!(result.rows[0][0].as_deref(), Some("true"));
    }

    #[test]
    fn mysql_collation_supplies_charset_when_column_charset_is_missing() {
        for collation in ["utf8mb4_bin", "gbk_bin"] {
            let mut schema_column = column("payload", "LONGTEXT");
            schema_column.charset = None;
            schema_column.collation = Some(collation.to_string());
            let bytes = if collation.starts_with("gbk") {
                encoded_bytes(GBK, "中文")
            } else {
                "中文".as_bytes().to_vec()
            };
            let mut result = result(
                &["payload"],
                vec![vec![Some("wrong")]],
                vec![BinaryCell {
                    row_index: 0,
                    column_index: 0,
                    bytes,
                }],
            );

            normalize_query_result_binary_semantics(
                &mut result,
                &DatabaseType::MySQL,
                &[schema_column],
            )
            .unwrap();

            assert_eq!(result.rows[0][0].as_deref(), Some("中文"), "{collation}");
            assert!(result.binary_cells.is_empty(), "{collation}");
        }
    }

    #[test]
    fn mysql_supported_legacy_charsets_decode_strictly() {
        for (charset, encoding) in [("gbk", GBK), ("gb18030", GB18030), ("big5", BIG5)] {
            let mut schema_column = column("payload", "LONGTEXT");
            schema_column.charset = Some(charset.to_string());
            schema_column.collation = None;
            let mut result = result(
                &["payload"],
                vec![vec![Some("wrong")]],
                vec![BinaryCell {
                    row_index: 0,
                    column_index: 0,
                    bytes: encoded_bytes(encoding, "中文"),
                }],
            );

            normalize_query_result_binary_semantics(
                &mut result,
                &DatabaseType::MySQL,
                &[schema_column],
            )
            .unwrap();

            assert_eq!(result.rows[0][0].as_deref(), Some("中文"), "{charset}");
            assert!(result.binary_cells.is_empty(), "{charset}");
        }
    }

    #[test]
    fn mixed_projection_decodes_text_and_retains_blob_bytes() {
        let blob_bytes = vec![0, 1, 2, 255];
        let mut result = result(
            &["payload", "id", "raw"],
            vec![vec![Some("wrong"), Some("7"), Some("0x000102FF")]],
            vec![
                BinaryCell {
                    row_index: 0,
                    column_index: 0,
                    bytes: b"true".to_vec(),
                },
                BinaryCell {
                    row_index: 0,
                    column_index: 2,
                    bytes: blob_bytes.clone(),
                },
            ],
        );
        let mut id = column("id", "BIGINT");
        id.is_nullable = false;
        let schema = vec![
            id,
            column("raw", "LONGBLOB"),
            column("unused", "TEXT"),
            column("payload", "LONGTEXT"),
        ];

        normalize_query_result_binary_semantics(&mut result, &DatabaseType::MySQL, &schema)
            .unwrap();

        assert_eq!(result.rows[0][0].as_deref(), Some("true"));
        assert_eq!(
            result.binary_cells,
            vec![BinaryCell {
                row_index: 0,
                column_index: 2,
                bytes: blob_bytes,
            }]
        );
        assert_eq!(
            result
                .column_meta
                .iter()
                .map(|meta| (meta.name.as_str(), meta.db_type.as_str(), meta.nullable))
                .collect::<Vec<_>>(),
            vec![
                ("payload", "LONGTEXT", true),
                ("id", "BIGINT", false),
                ("raw", "LONGBLOB", true),
            ]
        );
    }

    #[test]
    fn schema_metadata_is_aligned_even_without_binary_sidecars() {
        let mut result = result(
            &["payload", "id"],
            vec![vec![Some("text"), Some("7")]],
            Vec::new(),
        );
        let mut id = column("id", "BIGINT");
        id.is_nullable = false;

        normalize_query_result_binary_semantics(
            &mut result,
            &DatabaseType::MySQL,
            &[id, column("payload", "LONGTEXT")],
        )
        .unwrap();

        assert_eq!(result.column_meta[0].name, "payload");
        assert_eq!(result.column_meta[0].db_type, "LONGTEXT");
        assert_eq!(result.column_meta[1].name, "id");
        assert_eq!(result.column_meta[1].db_type, "BIGINT");
        assert!(!result.column_meta[1].nullable);
    }

    #[test]
    fn mysql_blob_keeps_exact_bytes() {
        let bytes = vec![0, 1, 2, 255];
        let mut result = result(
            &["payload"],
            vec![vec![Some("0x000102FF")]],
            vec![BinaryCell {
                row_index: 0,
                column_index: 0,
                bytes: bytes.clone(),
            }],
        );

        normalize_query_result_binary_semantics(
            &mut result,
            &DatabaseType::MySQL,
            &[column("payload", "LONGBLOB")],
        )
        .unwrap();

        assert_eq!(result.binary_cells[0].bytes, bytes);
        assert_eq!(result.column_meta[0].field_type, FieldType::Binary);
    }

    #[test]
    fn non_mysql_runtime_binary_value_is_not_reclassified() {
        let mut result = result(
            &["payload"],
            vec![vec![Some("true")]],
            vec![BinaryCell {
                row_index: 0,
                column_index: 0,
                bytes: b"true".to_vec(),
            }],
        );
        let original_meta = result.column_meta.clone();

        normalize_query_result_binary_semantics(
            &mut result,
            &DatabaseType::SQLite,
            &[column("payload", "TEXT")],
        )
        .unwrap();

        assert_eq!(result.binary_cells.len(), 1);
        assert_eq!(result.column_meta[0].db_type, original_meta[0].db_type);
    }

    #[test]
    fn invalid_utf8_in_mysql_text_is_an_error_without_mutating_the_display_value() {
        let mut result = result(
            &["payload"],
            vec![vec![Some("0xFF")]],
            vec![BinaryCell {
                row_index: 0,
                column_index: 0,
                bytes: vec![0xff],
            }],
        );

        let error = normalize_query_result_binary_semantics(
            &mut result,
            &DatabaseType::MySQL,
            &[column("payload", "LONGTEXT")],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            QueryResultNormalizationError::InvalidTextEncoding {
                row_index: 0,
                column_index: 0,
                ..
            }
        ));
        assert_eq!(result.rows[0][0].as_deref(), Some("0xFF"));
        assert_eq!(result.binary_cells[0].bytes, vec![0xff]);
        assert_eq!(
            result.column_meta[0].db_type, "MYSQL_TYPE_LONG_BLOB",
            "normalization errors must not partially mutate metadata"
        );
    }

    #[test]
    fn a_later_decode_error_does_not_partially_mutate_the_result() {
        let mut result = result(
            &["first", "second"],
            vec![vec![Some("first old"), Some("second old")]],
            vec![
                BinaryCell {
                    row_index: 0,
                    column_index: 0,
                    bytes: b"first new".to_vec(),
                },
                BinaryCell {
                    row_index: 0,
                    column_index: 1,
                    bytes: vec![0xff],
                },
            ],
        );
        let before = result.clone();

        let error = normalize_query_result_binary_semantics(
            &mut result,
            &DatabaseType::MySQL,
            &[column("first", "LONGTEXT"), column("second", "LONGTEXT")],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            QueryResultNormalizationError::InvalidTextEncoding {
                row_index: 0,
                column_index: 1,
                ..
            }
        ));
        assert_result_unchanged(&result, &before);
    }

    #[test]
    fn unsupported_or_missing_text_charset_does_not_mutate_the_result() {
        for charset in [None, Some("utf16")] {
            let mut schema_column = column("payload", "LONGTEXT");
            schema_column.charset = charset.map(str::to_string);
            schema_column.collation = None;
            let mut result = result(
                &["payload"],
                vec![vec![Some("old")]],
                vec![BinaryCell {
                    row_index: 0,
                    column_index: 0,
                    bytes: b"text".to_vec(),
                }],
            );
            let before = result.clone();

            let error = normalize_query_result_binary_semantics(
                &mut result,
                &DatabaseType::MySQL,
                &[schema_column],
            )
            .unwrap_err();

            assert!(matches!(
                error,
                QueryResultNormalizationError::UnsupportedTextCharset { .. }
            ));
            assert_result_unchanged(&result, &before);
        }
    }

    #[test]
    fn empty_mysql_text_bytes_remain_empty_text_not_null() {
        let mut result = result(
            &["payload"],
            vec![vec![None]],
            vec![BinaryCell {
                row_index: 0,
                column_index: 0,
                bytes: Vec::new(),
            }],
        );

        normalize_query_result_binary_semantics(
            &mut result,
            &DatabaseType::MySQL,
            &[column("payload", "LONGTEXT")],
        )
        .unwrap();

        assert!(result.binary_cells.is_empty());
        assert_eq!(result.rows[0][0].as_deref(), Some(""));
    }

    #[test]
    fn schema_mismatch_is_an_explicit_error() {
        let mut result = result(&["payload"], vec![vec![Some("true")]], Vec::new());

        let error = normalize_query_result_binary_semantics(
            &mut result,
            &DatabaseType::MySQL,
            &[column("different", "LONGTEXT")],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            QueryResultNormalizationError::SchemaColumnNotFound {
                column_index: 0,
                ..
            }
        ));
    }

    #[test]
    fn duplicate_result_columns_are_rejected_after_name_normalization() {
        let mut result = result(
            &["payload", " PAYLOAD "],
            vec![vec![Some("a"), Some("b")]],
            Vec::new(),
        );
        let before = result.clone();

        let error = normalize_query_result_binary_semantics(
            &mut result,
            &DatabaseType::MySQL,
            &[column("payload", "LONGTEXT")],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            QueryResultNormalizationError::DuplicateResultColumn { .. }
        ));
        assert_result_unchanged(&result, &before);
    }

    #[test]
    fn ambiguous_schema_columns_are_rejected_after_name_normalization() {
        let mut result = result(&["payload"], vec![vec![Some("a")]], Vec::new());
        let before = result.clone();

        let error = normalize_query_result_binary_semantics(
            &mut result,
            &DatabaseType::MySQL,
            &[column("payload", "LONGTEXT"), column(" PAYLOAD ", "TEXT")],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            QueryResultNormalizationError::SchemaColumnAmbiguous { .. }
        ));
        assert_result_unchanged(&result, &before);
    }

    #[test]
    fn invalid_query_shape_is_rejected_without_mutation() {
        let mut result = result(
            &["id", "payload"],
            vec![vec![Some("7")]],
            vec![BinaryCell {
                row_index: 0,
                column_index: 1,
                bytes: b"text".to_vec(),
            }],
        );
        let before = result.clone();

        let error = normalize_query_result_binary_semantics(
            &mut result,
            &DatabaseType::MySQL,
            &[column("id", "BIGINT"), column("payload", "LONGTEXT")],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            QueryResultNormalizationError::InvalidQueryResult(_)
        ));
        assert_result_unchanged(&result, &before);
    }
}
