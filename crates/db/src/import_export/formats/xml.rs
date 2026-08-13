use std::collections::HashMap;
use std::time::Instant;

use anyhow::{Result, anyhow};
use async_trait::async_trait;

use super::format_import_table_reference;
use super::xml_codec::{ImportedRow, ImportedValue, parse_rows, serialize_table};
use crate::DatabasePlugin;
use crate::connection::DbConnection;
use crate::executor::{ExecOptions, SqlResult};
use crate::import_export::{
    ExportConfig, ExportProgressEvent, ExportProgressSender, ExportResult, FormatHandler,
    ImportConfig, ImportProgressEvent, ImportProgressSender, ImportResult,
};

pub struct XmlFormatHandler;

impl XmlFormatHandler {
    fn sanitize_legacy_tag_name(name: &str) -> String {
        let mut result = String::new();
        for (index, character) in name.chars().enumerate() {
            let valid = if index == 0 {
                character.is_ascii_alphabetic() || character == '_'
            } else {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
            };
            if valid {
                result.push(character);
            } else {
                if index == 0 {
                    result.push('_');
                }
                if index == 0 && character.is_ascii_alphanumeric() {
                    result.push(character);
                } else if index > 0 {
                    result.push('_');
                }
            }
        }
        if result.is_empty() {
            "field".to_string()
        } else {
            result
        }
    }
}

#[async_trait]
impl FormatHandler for XmlFormatHandler {
    async fn import(
        &self,
        plugin: &dyn DatabasePlugin,
        connection: &dyn DbConnection,
        config: &ImportConfig,
        data: &str,
    ) -> Result<ImportResult> {
        self.import_with_progress(plugin, connection, config, data, "", None)
            .await
    }

    async fn import_with_progress(
        &self,
        plugin: &dyn DatabasePlugin,
        connection: &dyn DbConnection,
        config: &ImportConfig,
        data: &str,
        file_name: &str,
        progress_tx: Option<ImportProgressSender>,
    ) -> Result<ImportResult> {
        let start = Instant::now();
        let table = config
            .table
            .as_deref()
            .ok_or_else(|| anyhow!("Table name required for XML import"))?;
        send_import_progress(
            &progress_tx,
            ImportProgressEvent::ParsingFile {
                file: file_name.to_string(),
            },
        );
        let rows = parse_rows(data, table, &Self::sanitize_legacy_tag_name(table))?;
        let (mut statements, errors) = build_insert_statements(plugin, config, table, rows);

        if statements.is_empty() && errors.is_empty() {
            return Ok(finished_import(
                start,
                0,
                Vec::new(),
                file_name,
                &progress_tx,
            ));
        }
        if config.use_transaction && !errors.is_empty() {
            return Ok(finished_import(start, 0, errors, file_name, &progress_tx));
        }
        let truncate_inserted = config.truncate_before_import && !statements.is_empty();
        if truncate_inserted {
            let table_ref = format_import_table_reference(plugin, config, table);
            statements.insert(0, format!("TRUNCATE TABLE {table_ref}"));
        }
        for index in 0..statements.len() {
            send_import_progress(
                &progress_tx,
                ImportProgressEvent::ExecutingStatement {
                    file: file_name.to_string(),
                    statement_index: index,
                    total_statements: statements.len(),
                },
            );
        }

        let mut rows_imported = 0;
        let mut all_errors = errors;
        if !statements.is_empty() {
            let options = ExecOptions {
                stop_on_error: config.stop_on_error,
                transactional: config.use_transaction,
                max_rows: None,
                streaming: false,
            };
            match connection
                .execute(plugin, &statements.join(";\n"), options)
                .await
            {
                Ok(results) => collect_execution_results(
                    results,
                    &mut rows_imported,
                    &mut all_errors,
                    file_name,
                    &progress_tx,
                    truncate_inserted,
                ),
                Err(error) => {
                    all_errors.push(error.to_string());
                    send_import_progress(
                        &progress_tx,
                        ImportProgressEvent::Error {
                            file: file_name.to_string(),
                            message: error.to_string(),
                        },
                    );
                }
            }
        }
        if config.use_transaction && !all_errors.is_empty() {
            rows_imported = 0;
        }

        Ok(finished_import(
            start,
            rows_imported,
            all_errors,
            file_name,
            &progress_tx,
        ))
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
        let mut output = xml_header();
        let mut total_rows = 0u64;
        let streaming = progress_tx.is_some();

        for (index, table) in config.tables.iter().enumerate() {
            send_export_progress(
                &progress_tx,
                ExportProgressEvent::TableStart {
                    table: table.clone(),
                    table_index: index,
                    total_tables: config.tables.len(),
                },
            );
            let query_result = query_table(plugin, connection, config, table).await?;
            let rows = query_result.rows.len() as u64;
            let mut table_output = serialize_table(table, &query_result)?;
            if streaming && index == 0 {
                table_output.insert_str(0, &xml_header());
            }
            if streaming && index + 1 == config.tables.len() {
                table_output.push_str("</data>\n");
            }
            send_export_progress(
                &progress_tx,
                ExportProgressEvent::DataExported {
                    table: table.clone(),
                    rows,
                    data: table_output.clone(),
                },
            );
            if !streaming {
                output.push_str(&table_output);
            }
            total_rows += rows;
            send_export_progress(
                &progress_tx,
                ExportProgressEvent::TableFinished {
                    table: table.clone(),
                },
            );
        }

        if !streaming {
            output.push_str("</data>\n");
        } else {
            output.clear();
        }
        let elapsed_ms = start.elapsed().as_millis();
        send_export_progress(
            &progress_tx,
            ExportProgressEvent::Finished {
                total_rows,
                elapsed_ms,
            },
        );
        Ok(ExportResult {
            success: true,
            output,
            rows_exported: total_rows,
            elapsed_ms,
        })
    }
}

fn build_insert_statements(
    plugin: &dyn DatabasePlugin,
    config: &ImportConfig,
    table: &str,
    rows: Vec<ImportedRow>,
) -> (Vec<String>, Vec<String>) {
    let Some(first_row) = rows.first() else {
        return (Vec::new(), Vec::new());
    };
    let columns = first_row
        .fields
        .iter()
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    let table_ref = format_import_table_reference(plugin, config, table);
    let column_sql = columns
        .iter()
        .map(|column| plugin.quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    let mut statements = Vec::new();
    let mut errors = Vec::new();

    for row in rows {
        let mut values = row.fields.into_iter().collect::<HashMap<_, _>>();
        let extras = values
            .keys()
            .filter(|column| !columns.contains(column))
            .cloned()
            .collect::<Vec<_>>();
        if !extras.is_empty() {
            errors.push(format!(
                "XML row contains unexpected columns: {}",
                extras.join(", ")
            ));
            if config.stop_on_error {
                break;
            }
            continue;
        }
        let value_sql = columns
            .iter()
            .map(|column| sql_value(plugin, values.remove(column)))
            .collect::<Vec<_>>()
            .join(", ");
        statements.push(format!(
            "INSERT INTO {table_ref} ({column_sql}) VALUES ({value_sql})"
        ));
    }
    (statements, errors)
}

fn sql_value(plugin: &dyn DatabasePlugin, value: Option<ImportedValue>) -> String {
    match value {
        None | Some(ImportedValue::Null) => "NULL".to_string(),
        Some(ImportedValue::Text(value)) => plugin.escape_sql_value(&value),
        Some(ImportedValue::Binary(bytes)) => plugin.format_binary_literal(&bytes),
    }
}

fn collect_execution_results(
    results: Vec<SqlResult>,
    rows_imported: &mut u64,
    errors: &mut Vec<String>,
    file_name: &str,
    progress_tx: &Option<ImportProgressSender>,
    truncate_inserted: bool,
) {
    for (index, result) in results.into_iter().enumerate() {
        match result {
            SqlResult::Exec(result) => {
                if !(truncate_inserted && index == 0) {
                    *rows_imported += result.rows_affected;
                }
                send_import_progress(
                    progress_tx,
                    ImportProgressEvent::StatementExecuted {
                        file: file_name.to_string(),
                        rows_affected: result.rows_affected,
                    },
                );
            }
            SqlResult::Error(error) => {
                errors.push(error.message.clone());
                send_import_progress(
                    progress_tx,
                    ImportProgressEvent::Error {
                        file: file_name.to_string(),
                        message: error.message,
                    },
                );
            }
            SqlResult::Query(_) => {}
        }
    }
}

fn finished_import(
    start: Instant,
    rows_imported: u64,
    errors: Vec<String>,
    file_name: &str,
    progress_tx: &Option<ImportProgressSender>,
) -> ImportResult {
    let elapsed_ms = start.elapsed().as_millis();
    send_import_progress(
        progress_tx,
        ImportProgressEvent::FileFinished {
            file: file_name.to_string(),
            rows_imported,
        },
    );
    ImportResult {
        success: errors.is_empty(),
        rows_imported,
        errors,
        elapsed_ms,
    }
}

async fn query_table(
    plugin: &dyn DatabasePlugin,
    connection: &dyn DbConnection,
    config: &ExportConfig,
    table: &str,
) -> Result<crate::executor::QueryResult> {
    let table_ref =
        plugin.format_table_reference(&config.database, config.schema.as_deref(), table);
    let columns = config
        .columns
        .as_ref()
        .map(|columns| {
            columns
                .iter()
                .map(|column| plugin.quote_identifier(column))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_else(|| "*".to_string());
    let mut sql = format!("SELECT {columns} FROM {table_ref}");
    if let Some(where_clause) = &config.where_clause {
        sql.push_str(" WHERE ");
        sql.push_str(where_clause);
    }
    if let Some(limit) = config.limit {
        sql.push_str(&plugin.format_pagination(limit, 0, ""));
    }
    match connection.query(&sql).await? {
        SqlResult::Query(result) => Ok(result),
        SqlResult::Error(error) => Err(anyhow!("Query failed: {}", error.message)),
        SqlResult::Exec(_) => Err(anyhow!("XML export query did not return rows")),
    }
}

fn xml_header() -> String {
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<data>\n".to_string()
}

fn send_import_progress(progress_tx: &Option<ImportProgressSender>, event: ImportProgressEvent) {
    if let Some(tx) = progress_tx {
        let _ = tx.send(event);
    }
}

fn send_export_progress(progress_tx: &Option<ExportProgressSender>, event: ExportProgressEvent) {
    if let Some(tx) = progress_tx {
        let _ = tx.send(event);
    }
}
