use std::time::Instant;

use anyhow::{Result, anyhow};
use async_trait::async_trait;

use super::import_execution::{ImportStatement, execute_import_statements};
use super::{format_import_table_reference, format_import_text_value, load_import_columns};
use crate::DatabasePlugin;
use crate::connection::DbConnection;
use crate::executor::{QueryCellRef, QueryResult, SqlResult};
use crate::import_export::{
    ExportConfig, ExportProgressEvent, ExportProgressSender, ExportResult, FormatHandler,
    ImportConfig, ImportResult,
};

pub struct CsvFormatHandler;

pub(super) fn render_delimited_query_result(
    format_name: &str,
    query_result: &QueryResult,
    delimiter: char,
    qualifier: Option<char>,
    include_header: bool,
    record_terminator: &str,
    null_string: &str,
) -> Result<String> {
    let view = query_result
        .typed_view()
        .map_err(|error| anyhow!("Invalid query result for {format_name} export: {error}"))?;
    let mut output = String::new();

    if include_header {
        for (column_index, column) in query_result.columns.iter().enumerate() {
            if column_index > 0 {
                output.push(delimiter);
            }
            output.push_str(&escape_delimited_field(
                format_name,
                column,
                delimiter,
                qualifier,
                null_string,
            )?);
        }
        output.push_str(record_terminator);
    }

    for row_index in 0..query_result.rows.len() {
        for column_index in 0..query_result.columns.len() {
            if column_index > 0 {
                output.push(delimiter);
            }
            match view.cell(row_index, column_index) {
                Some(QueryCellRef::Null) => output.push_str(null_string),
                Some(QueryCellRef::Text(value)) => output.push_str(&escape_delimited_field(
                    format_name,
                    value,
                    delimiter,
                    qualifier,
                    null_string,
                )?),
                Some(QueryCellRef::Binary(_)) => {
                    return Err(anyhow!(
                        "{format_name} export does not support binary cell at row {}, column {} ({:?}) without an explicit binary encoding",
                        row_index + 1,
                        column_index + 1,
                        query_result.columns[column_index],
                    ));
                }
                None => unreachable!("typed view validated row and column bounds"),
            }
        }
        output.push_str(record_terminator);
    }

    Ok(output)
}

pub(super) fn escape_delimited_field(
    format_name: &str,
    field: &str,
    delimiter: char,
    qualifier: Option<char>,
    null_string: &str,
) -> Result<String> {
    // 空字符串和与 NULL marker 相同的文本都需要用引号包裹，以保留三态语义。
    let needs_quote = field.is_empty()
        || field == null_string
        || field.contains(delimiter)
        || field.contains('\n')
        || field.contains('\r')
        || qualifier.map(|q| field.contains(q)).unwrap_or(false);

    if needs_quote {
        let q = qualifier.ok_or_else(|| {
            anyhow!(
                "{format_name} text qualifier is required to safely export empty, NULL-marker, delimited, or multiline text"
            )
        })?;
        let escaped = field.replace(q, &format!("{}{}", q, q));
        Ok(format!("{}{}{}", q, escaped, q))
    } else {
        Ok(field.to_string())
    }
}

impl CsvFormatHandler {
    pub(crate) fn parse_csv_data_with_null_string(
        data: &str,
        delimiter: char,
        qualifier: Option<char>,
        null_string: Option<&str>,
    ) -> Vec<Vec<Option<String>>> {
        let mut records = Vec::new();
        let mut current_record: Vec<Option<String>> = Vec::new();
        let mut current_field = String::new();
        let mut in_quotes = false;
        let mut was_quoted = false;
        let mut record_has_syntax = false;
        let mut chars = data.chars().peekable();

        let push_field =
            |record: &mut Vec<Option<String>>, field: &mut String, quoted: &mut bool| {
                let field_value = std::mem::take(field);
                let is_null_marker =
                    !*quoted && null_string.is_some_and(|marker| field_value == marker);
                let value = if !*quoted && (field_value.is_empty() || is_null_marker) {
                    None
                } else {
                    Some(field_value)
                };
                record.push(value);
                *quoted = false;
            };

        while let Some(ch) = chars.next() {
            if let Some(q) = qualifier {
                if ch == q {
                    record_has_syntax = true;
                    if in_quotes {
                        if chars.peek() == Some(&q) {
                            chars.next();
                            current_field.push(q);
                        } else {
                            in_quotes = false;
                        }
                    } else {
                        in_quotes = true;
                        was_quoted = true;
                    }
                    continue;
                }
            }

            if !in_quotes && ch == delimiter {
                record_has_syntax = true;
                push_field(&mut current_record, &mut current_field, &mut was_quoted);
                continue;
            }

            if !in_quotes && (ch == '\n' || ch == '\r') {
                if ch == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                push_field(&mut current_record, &mut current_field, &mut was_quoted);
                if record_has_syntax {
                    records.push(std::mem::take(&mut current_record));
                } else {
                    current_record.clear();
                }
                record_has_syntax = false;
                continue;
            }

            record_has_syntax = true;
            current_field.push(ch);
        }

        push_field(&mut current_record, &mut current_field, &mut was_quoted);
        if record_has_syntax {
            records.push(current_record);
        }

        records
    }
}

#[async_trait]
impl FormatHandler for CsvFormatHandler {
    async fn import(
        &self,
        plugin: &dyn DatabasePlugin,
        connection: &dyn DbConnection,
        config: &ImportConfig,
        data: &str,
    ) -> Result<ImportResult> {
        let start = Instant::now();
        let mut errors = Vec::new();

        let table = config
            .table
            .as_ref()
            .ok_or_else(|| anyhow!("Table name required for CSV import"))?;
        let table_ref = format_import_table_reference(plugin, config, table);

        let csv_config = config.csv_config.clone().unwrap_or_default();
        let delimiter = csv_config.field_delimiter;
        let qualifier = csv_config.text_qualifier;
        let has_header = csv_config.has_header;
        let null_string = csv_config.null_string;

        let records =
            Self::parse_csv_data_with_null_string(data, delimiter, qualifier, Some(&null_string));
        if records.is_empty() {
            return Ok(ImportResult {
                success: true,
                rows_imported: 0,
                errors,
                elapsed_ms: start.elapsed().as_millis(),
            });
        }

        let columns: Vec<String>;
        let data_start_record: usize;

        if has_header {
            columns = records[0]
                .clone()
                .into_iter()
                .map(|opt| opt.unwrap_or_default())
                .collect();
            data_start_record = 1;
        } else {
            let first_row = &records[0];
            columns = (0..first_row.len())
                .map(|i| format!("col{}", i + 1))
                .collect();
            data_start_record = 0;
        }

        if columns.is_empty() {
            return Err(anyhow!("CSV header is empty"));
        }
        if columns.iter().any(|column| column.trim().is_empty()) {
            return Err(anyhow!("CSV header contains empty column names"));
        }
        let table_columns = load_import_columns(plugin, connection, config, table).await?;

        let mut statements = Vec::new();
        if config.truncate_before_import {
            statements.push(ImportStatement::truncate(format!(
                "TRUNCATE TABLE {table_ref}"
            )));
        }

        for (record_num, values) in records.iter().skip(data_start_record).enumerate() {
            let record_number = record_num + data_start_record + 1;
            if values.len() != columns.len() {
                errors.push(format!("Record {}: column count mismatch", record_number));
                if config.stop_on_error {
                    break;
                }
                continue;
            }

            let mut insert_sql = format!("INSERT INTO {} (", table_ref);
            for (i, col) in columns.iter().enumerate() {
                if i > 0 {
                    insert_sql.push_str(", ");
                }
                insert_sql.push_str(&plugin.quote_identifier(col));
            }
            insert_sql.push_str(") VALUES (");

            for (i, (column, val)) in columns.iter().zip(values).enumerate() {
                if i > 0 {
                    insert_sql.push_str(", ");
                }
                insert_sql.push_str(&format_import_text_value(
                    plugin,
                    val,
                    column,
                    &table_columns,
                ));
            }
            insert_sql.push(')');
            statements.push(ImportStatement::row(
                insert_sql,
                format!("Record {record_number}"),
            ));
        }

        if config.use_transaction && !errors.is_empty() {
            return Ok(ImportResult {
                success: false,
                rows_imported: 0,
                errors,
                elapsed_ms: start.elapsed().as_millis(),
            });
        }
        let (total_rows, execution_errors) =
            execute_import_statements(plugin, connection, config, statements).await;
        errors.extend(execution_errors);

        Ok(ImportResult {
            success: errors.is_empty(),
            rows_imported: total_rows,
            errors,
            elapsed_ms: start.elapsed().as_millis(),
        })
    }

    async fn export(
        &self,
        plugin: &dyn DatabasePlugin,
        connection: &dyn DbConnection,
        config: &ExportConfig,
    ) -> Result<ExportResult> {
        self.export_with_progress(plugin, connection, config, None)
            .await
    }

    async fn export_with_progress(
        &self,
        plugin: &dyn DatabasePlugin,
        connection: &dyn DbConnection,
        config: &ExportConfig,
        progress_tx: Option<ExportProgressSender>,
    ) -> Result<ExportResult> {
        let start = Instant::now();
        let mut output = String::new();
        let mut total_rows = 0u64;
        let total_tables = config.tables.len();
        let is_streaming = progress_tx.is_some();

        let csv_config = config.csv_config.clone().unwrap_or_default();
        let delimiter = csv_config.field_delimiter;
        let qualifier = csv_config.text_qualifier;
        let include_header = csv_config.include_header;
        let record_terminator = csv_config.record_terminator;
        let null_string = csv_config.null_string;

        let send_progress = |event: ExportProgressEvent| {
            if let Some(tx) = &progress_tx {
                let _ = tx.send(event);
            }
        };

        for (index, table) in config.tables.iter().enumerate() {
            send_progress(ExportProgressEvent::TableStart {
                table: table.clone(),
                table_index: index,
                total_tables,
            });

            send_progress(ExportProgressEvent::FetchingData {
                table: table.clone(),
            });

            let table_ref =
                plugin.format_table_reference(&config.database, config.schema.as_deref(), table);
            let columns_str = if let Some(cols) = &config.columns {
                cols.iter()
                    .map(|c| plugin.quote_identifier(c))
                    .collect::<Vec<_>>()
                    .join(", ")
            } else {
                "*".to_string()
            };

            let mut base_sql = format!("SELECT {} FROM {}", columns_str, table_ref);
            if let Some(where_clause) = &config.where_clause {
                base_sql.push_str(" WHERE ");
                base_sql.push_str(where_clause);
            }
            let paginated_query = config
                .limit
                .map(|limit| plugin.build_paginated_query(&base_sql, limit, 0, ""));
            let select_sql = paginated_query
                .as_ref()
                .map(|query| query.sql.as_str())
                .unwrap_or(base_sql.as_str());

            let result = connection
                .query(select_sql)
                .await
                .map_err(|e| anyhow!("Query failed: {}", e))?;

            if let SqlResult::Query(mut query_result) = result {
                if let Some(paginated_query) = &paginated_query {
                    paginated_query.strip_hidden_result_columns(&mut query_result)?;
                }
                crate::query_result_normalization::normalize_table_query_result(
                    plugin,
                    connection,
                    &config.database,
                    config.schema.as_deref(),
                    table,
                    &mut query_result,
                )
                .await?;
                let rows_count = query_result.rows.len() as u64;
                let table_output = render_delimited_query_result(
                    "CSV",
                    &query_result,
                    delimiter,
                    qualifier,
                    include_header,
                    &record_terminator,
                    &null_string,
                )?;

                total_rows += rows_count;
                send_progress(ExportProgressEvent::DataExported {
                    table: table.clone(),
                    rows: rows_count,
                    data: table_output.clone(),
                });

                if !is_streaming {
                    if index > 0 {
                        output.push_str("\n\n");
                    }
                    output.push_str(&table_output);
                }
            }

            send_progress(ExportProgressEvent::TableFinished {
                table: table.clone(),
            });
        }

        let elapsed_ms = start.elapsed().as_millis();
        send_progress(ExportProgressEvent::Finished {
            total_rows,
            elapsed_ms,
        });

        Ok(ExportResult {
            success: true,
            output,
            rows_exported: total_rows,
            elapsed_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{CsvFormatHandler, escape_delimited_field, render_delimited_query_result};
    use crate::executor::{BinaryCell, QueryColumnMeta, QueryResult};

    #[test]
    fn test_parse_csv_data_supports_multiline_quoted_field() {
        let input = "id,content\n1,\"line1\nline2\"\n2,plain\n";
        let records =
            CsvFormatHandler::parse_csv_data_with_null_string(input, ',', Some('"'), None);
        assert_eq!(records.len(), 3);
        assert_eq!(
            records[0],
            vec![Some("id".to_string()), Some("content".to_string())]
        );
        assert_eq!(
            records[1],
            vec![Some("1".to_string()), Some("line1\nline2".to_string())]
        );
        assert_eq!(
            records[2],
            vec![Some("2".to_string()), Some("plain".to_string())]
        );
    }

    #[test]
    fn parse_csv_distinguishes_null_empty_and_literal_null_text() {
        let records = CsvFormatHandler::parse_csv_data_with_null_string(
            "\\N,\"\",NULL,\"\\N\"\n",
            ',',
            Some('"'),
            Some("\\N"),
        );
        assert_eq!(
            records,
            vec![vec![
                None,
                Some(String::new()),
                Some("NULL".to_string()),
                Some("\\N".to_string()),
            ]]
        );
    }

    #[test]
    fn parse_csv_preserves_a_row_containing_only_null_markers() {
        let records = CsvFormatHandler::parse_csv_data_with_null_string(
            "\\N,\\N\n\n",
            ',',
            Some('"'),
            Some("\\N"),
        );
        assert_eq!(records, vec![vec![None, None]]);
    }

    #[test]
    fn escape_delimited_csv_quotes_empty_and_literal_null_marker() {
        assert_eq!(
            escape_delimited_field("CSV", "", ',', Some('"'), "\\N").unwrap(),
            "\"\""
        );
        assert_eq!(
            escape_delimited_field("CSV", "\\N", ',', Some('"'), "\\N").unwrap(),
            "\"\\N\""
        );
        assert_eq!(
            escape_delimited_field("CSV", "NULL", ',', Some('"'), "\\N").unwrap(),
            "NULL"
        );
        assert!(escape_delimited_field("CSV", "", ',', None, "\\N").is_err());
    }

    #[test]
    fn csv_render_preserves_null_and_empty_text() {
        let result = QueryResult {
            sql: String::new(),
            columns: vec!["nullable".to_string(), "empty".to_string()],
            column_meta: vec![
                QueryColumnMeta::new("nullable", "TEXT"),
                QueryColumnMeta::new("empty", "TEXT"),
            ],
            rows: vec![vec![None, Some(String::new())]],
            binary_cells: vec![],
            elapsed_ms: 0,
        };

        let output =
            render_delimited_query_result("CSV", &result, ',', Some('"'), true, "\n", "\\N")
                .expect("text-only CSV should render");

        assert_eq!(output, "nullable,empty\n\\N,\"\"\n");
    }

    #[test]
    fn csv_render_rejects_binary_sidecar_even_with_display_text() {
        let result = QueryResult {
            sql: String::new(),
            columns: vec!["payload".to_string()],
            column_meta: vec![QueryColumnMeta::new("payload", "BLOB")],
            rows: vec![vec![Some("true".to_string())]],
            binary_cells: vec![BinaryCell {
                row_index: 0,
                column_index: 0,
                bytes: b"true".to_vec(),
            }],
            elapsed_ms: 0,
        };

        let error =
            render_delimited_query_result("CSV", &result, ',', Some('"'), true, "\n", "\\N")
                .expect_err("CSV has no binary wire encoding");

        assert!(
            error
                .to_string()
                .contains("CSV export does not support binary cell at row 1, column 1")
        );
    }
}
