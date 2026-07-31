use std::time::Instant;

use anyhow::{Result, anyhow};
use async_trait::async_trait;

use super::{CsvFormatHandler, format_import_table_reference};
use crate::DatabasePlugin;
use crate::connection::DbConnection;
use crate::executor::{ExecOptions, SqlResult};
use crate::import_export::{
    CsvImportConfig, ExportConfig, ExportProgressEvent, ExportProgressSender, ExportResult,
    FormatHandler, ImportConfig, ImportResult,
};

pub struct TxtFormatHandler;

impl TxtFormatHandler {
    fn escape_txt_field(
        field: &str,
        delimiter: char,
        qualifier: Option<char>,
        null_string: &str,
    ) -> Result<String> {
        let needs_quote = field.is_empty()
            || field == null_string
            || field.contains(delimiter)
            || field.contains('\n')
            || field.contains('\r')
            || qualifier.map(|q| field.contains(q)).unwrap_or(false);

        if needs_quote {
            let q = qualifier.ok_or_else(|| {
                anyhow!(
                    "TXT text qualifier is required to safely export empty, NULL-marker, delimited, or multiline text"
                )
            })?;
            let escaped = field.replace(q, &format!("{}{}", q, q));
            Ok(format!("{}{}{}", q, escaped, q))
        } else {
            Ok(field.to_string())
        }
    }

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
        let mut total_rows = 0u64;

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

        if config.truncate_before_import {
            let truncate_sql = format!("TRUNCATE TABLE {}", table_ref);
            let results = connection
                .execute(plugin, &truncate_sql, ExecOptions::default())
                .await
                .map_err(|e| anyhow!("Truncate failed: {}", e))?;

            for result in results {
                if let SqlResult::Error(err) = result {
                    errors.push(format!("Truncate failed: {}", err.message));
                    if config.stop_on_error {
                        return Ok(ImportResult {
                            success: false,
                            rows_imported: 0,
                            errors,
                            elapsed_ms: start.elapsed().as_millis(),
                        });
                    }
                }
            }
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

            for (i, val) in values.iter().enumerate() {
                if i > 0 {
                    insert_sql.push_str(", ");
                }
                CsvFormatHandler::append_sql_value(&mut insert_sql, val);
            }
            insert_sql.push(')');

            match connection
                .execute(plugin, &insert_sql, ExecOptions::default())
                .await
            {
                Ok(results) => {
                    for result in results {
                        match result {
                            SqlResult::Exec(exec_result) => {
                                total_rows += exec_result.rows_affected;
                            }
                            SqlResult::Error(err) => {
                                errors.push(format!("Record {}: {}", record_number, err.message));
                                if config.stop_on_error {
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Err(e) => {
                    errors.push(format!("Record {}: {}", record_number, e));
                    if config.stop_on_error {
                        break;
                    }
                }
            }
        }

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

            let table_ref = plugin.format_table_reference(&config.database, None, table);
            let columns_str = if let Some(cols) = &config.columns {
                cols.iter()
                    .map(|c| plugin.quote_identifier(c))
                    .collect::<Vec<_>>()
                    .join(", ")
            } else {
                "*".to_string()
            };

            let mut select_sql = format!("SELECT {} FROM {}", columns_str, table_ref);
            if let Some(where_clause) = &config.where_clause {
                select_sql.push_str(" WHERE ");
                select_sql.push_str(where_clause);
            }
            if let Some(limit) = config.limit {
                let pagination = plugin.format_pagination(limit, 0, "");
                select_sql.push_str(&pagination);
            }

            let result = connection
                .query(&select_sql)
                .await
                .map_err(|e| anyhow!("Query failed: {}", e))?;

            if let SqlResult::Query(query_result) = result {
                let mut table_output = String::new();

                if include_header {
                    for (i, col) in query_result.columns.iter().enumerate() {
                        if i > 0 {
                            table_output.push(delimiter);
                        }
                        table_output.push_str(&Self::escape_txt_field(
                            col,
                            delimiter,
                            qualifier,
                            &null_string,
                        )?);
                    }
                    table_output.push_str(&record_terminator);
                }

                let rows_count = query_result.rows.len() as u64;
                for row in &query_result.rows {
                    for (i, val) in row.iter().enumerate() {
                        if i > 0 {
                            table_output.push(delimiter);
                        }
                        match val {
                            Some(v) => table_output.push_str(&Self::escape_txt_field(
                                v,
                                delimiter,
                                qualifier,
                                &null_string,
                            )?),
                            None => table_output.push_str(&null_string),
                        }
                    }
                    table_output.push_str(&record_terminator);
                }

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
    use super::TxtFormatHandler;

    #[test]
    fn escape_txt_quotes_empty_and_literal_null_marker() {
        assert_eq!(
            TxtFormatHandler::escape_txt_field("", '\t', Some('"'), "\\N").unwrap(),
            "\"\""
        );
        assert_eq!(
            TxtFormatHandler::escape_txt_field("\\N", '\t', Some('"'), "\\N").unwrap(),
            "\"\\N\""
        );
        assert_eq!(
            TxtFormatHandler::escape_txt_field("NULL", '\t', Some('"'), "\\N").unwrap(),
            "NULL"
        );
        assert!(TxtFormatHandler::escape_txt_field("", '\t', None, "\\N").is_err());
    }
}
