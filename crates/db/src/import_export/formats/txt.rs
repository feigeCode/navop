use std::time::Instant;

use anyhow::{Result, anyhow};
use async_trait::async_trait;

use super::csv::render_delimited_query_result;
use super::import_execution::{ImportStatement, execute_import_statements};
use super::{
    CsvFormatHandler, format_import_table_reference, format_import_text_value, load_import_columns,
};
use crate::DatabasePlugin;
use crate::connection::DbConnection;
use crate::executor::SqlResult;
use crate::import_export::{
    CsvImportConfig, ExportConfig, ExportProgressEvent, ExportProgressSender, ExportResult,
    FormatHandler, ImportConfig, ImportResult,
};

pub struct TxtFormatHandler;

impl TxtFormatHandler {
    fn import_config(config: &ImportConfig) -> CsvImportConfig {
        config
            .csv_config
            .clone()
            .unwrap_or_else(|| CsvImportConfig {
                field_delimiter: '\t',
                text_qualifier: Some('"'),
                has_header: true,
                record_terminator: "\n".to_string(),
                null_string: "\\N".to_string(),
            })
    }
}

#[async_trait]
impl FormatHandler for TxtFormatHandler {
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
            .ok_or_else(|| anyhow!("Table name required for TXT import"))?;
        let table_ref = format_import_table_reference(plugin, config, table);

        let txt_config = Self::import_config(config);
        let records = CsvFormatHandler::parse_csv_data_with_null_string(
            data,
            txt_config.field_delimiter,
            txt_config.text_qualifier,
            Some(&txt_config.null_string),
        );
        if records.is_empty() {
            return Ok(ImportResult {
                success: true,
                rows_imported: 0,
                errors,
                elapsed_ms: start.elapsed().as_millis(),
            });
        }

        let (columns, data_start_record) = if txt_config.has_header {
            (
                records[0]
                    .iter()
                    .map(|value| value.clone().unwrap_or_default())
                    .collect::<Vec<_>>(),
                1,
            )
        } else {
            (
                (0..records[0].len())
                    .map(|index| format!("col{}", index + 1))
                    .collect(),
                0,
            )
        };
        if columns.is_empty() {
            return Err(anyhow!("TXT header is empty"));
        }
        if columns.iter().any(|column| column.trim().is_empty()) {
            return Err(anyhow!("TXT header contains empty column names"));
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

        let csv_config =
            config
                .csv_config
                .clone()
                .unwrap_or_else(|| crate::import_export::CsvExportConfig {
                    field_delimiter: '\t',
                    text_qualifier: Some('"'),
                    include_header: true,
                    record_terminator: "\n".to_string(),
                    null_string: "\\N".to_string(),
                });
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
                    "TXT",
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
    use super::render_delimited_query_result;
    use crate::executor::{BinaryCell, QueryColumnMeta, QueryResult};
    use crate::import_export::formats::csv::escape_delimited_field;

    #[test]
    fn escape_delimited_txt_quotes_empty_and_literal_null_marker() {
        assert_eq!(
            escape_delimited_field("TXT", "", '\t', Some('"'), "\\N").unwrap(),
            "\"\""
        );
        assert_eq!(
            escape_delimited_field("TXT", "\\N", '\t', Some('"'), "\\N").unwrap(),
            "\"\\N\""
        );
        assert_eq!(
            escape_delimited_field("TXT", "NULL", '\t', Some('"'), "\\N").unwrap(),
            "NULL"
        );
        assert!(escape_delimited_field("TXT", "", '\t', None, "\\N").is_err());
    }

    #[test]
    fn txt_render_preserves_null_and_empty_text() {
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
            render_delimited_query_result("TXT", &result, '\t', Some('"'), true, "\n", "\\N")
                .expect("text-only TXT should render");

        assert_eq!(output, "nullable\tempty\n\\N\t\"\"\n");
    }

    #[test]
    fn txt_render_rejects_binary_sidecar_even_with_display_text() {
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
            render_delimited_query_result("TXT", &result, '\t', Some('"'), true, "\n", "\\N")
                .expect_err("TXT has no binary wire encoding");

        assert!(
            error
                .to_string()
                .contains("TXT export does not support binary cell at row 1, column 1")
        );
    }
}
