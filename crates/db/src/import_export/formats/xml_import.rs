use std::collections::HashMap;
use std::time::Instant;

use anyhow::{Result, anyhow};

use super::xml_codec::{ImportedRow, ImportedValue, parse_rows};
use super::{format_import_table_reference, load_import_columns};
use crate::DatabasePlugin;
use crate::connection::DbConnection;
use crate::executor::{ExecOptions, SqlResult};
use crate::import_export::{ImportConfig, ImportProgressEvent, ImportProgressSender, ImportResult};
use crate::types::{ColumnInfo, TableCellValue};

pub(super) struct XmlImportRequest<'a> {
    pub plugin: &'a dyn DatabasePlugin,
    pub connection: &'a dyn DbConnection,
    pub config: &'a ImportConfig,
    pub data: &'a str,
    pub file_name: &'a str,
    pub progress_tx: Option<ImportProgressSender>,
}

pub(super) async fn import_xml(request: XmlImportRequest<'_>) -> Result<ImportResult> {
    let start = Instant::now();
    let table = request
        .config
        .table
        .as_deref()
        .ok_or_else(|| anyhow!("Table name required for XML import"))?;
    send_progress(
        &request.progress_tx,
        ImportProgressEvent::ParsingFile {
            file: request.file_name.to_string(),
        },
    );
    let rows = parse_rows(request.data, table, &sanitize_legacy_tag_name(table))?;
    let columns =
        load_import_columns(request.plugin, request.connection, request.config, table).await?;
    let context = InsertBuildContext {
        plugin: request.plugin,
        config: request.config,
        table,
        table_columns: &columns,
    };
    let (mut statements, errors) = build_insert_statements(&context, rows);
    if statements.is_empty() && errors.is_empty() {
        return Ok(finished_import(&request, start, 0, Vec::new()));
    }
    if request.config.use_transaction && !errors.is_empty() {
        return Ok(finished_import(&request, start, 0, errors));
    }
    let truncate_inserted = prepend_truncate(&request, table, &mut statements);
    send_statement_progress(&request, statements.len());
    let (mut rows_imported, mut all_errors) =
        execute_statements(&request, statements, truncate_inserted).await;
    all_errors.splice(0..0, errors);
    if request.config.use_transaction && !all_errors.is_empty() {
        rows_imported = 0;
    }
    Ok(finished_import(&request, start, rows_imported, all_errors))
}

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
        } else if index == 0 {
            result.push('_');
            if character.is_ascii_alphanumeric() {
                result.push(character);
            }
        } else {
            result.push('_');
        }
    }
    if result.is_empty() {
        "field".to_string()
    } else {
        result
    }
}

struct InsertBuildContext<'a> {
    plugin: &'a dyn DatabasePlugin,
    config: &'a ImportConfig,
    table: &'a str,
    table_columns: &'a [ColumnInfo],
}

fn build_insert_statements(
    context: &InsertBuildContext<'_>,
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
    let mut statements = Vec::new();
    let mut errors = Vec::new();
    for row in rows {
        match insert_statement(context, &columns, row) {
            Ok(statement) => statements.push(statement),
            Err(error) => {
                errors.push(error);
                if context.config.stop_on_error {
                    break;
                }
            }
        }
    }
    (statements, errors)
}

fn insert_statement(
    context: &InsertBuildContext<'_>,
    columns: &[String],
    row: ImportedRow,
) -> std::result::Result<String, String> {
    let mut values = row.fields.into_iter().collect::<HashMap<_, _>>();
    let extras = values
        .keys()
        .filter(|column| !columns.contains(column))
        .cloned()
        .collect::<Vec<_>>();
    if !extras.is_empty() {
        return Err(format!(
            "XML row contains unexpected columns: {}",
            extras.join(", ")
        ));
    }
    let table_ref = format_import_table_reference(context.plugin, context.config, context.table);
    let column_sql = columns
        .iter()
        .map(|column| context.plugin.quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    let value_sql = columns
        .iter()
        .map(|column| sql_value(context, values.remove(column), column))
        .collect::<Vec<_>>()
        .join(", ");
    Ok(format!(
        "INSERT INTO {table_ref} ({column_sql}) VALUES ({value_sql})"
    ))
}

fn sql_value(
    context: &InsertBuildContext<'_>,
    value: Option<ImportedValue>,
    column_name: &str,
) -> String {
    match value {
        None | Some(ImportedValue::Null) => "NULL".to_string(),
        Some(ImportedValue::Text(value)) => context.plugin.format_table_change_value(
            &TableCellValue::Text(value),
            find_column(context.table_columns, column_name),
        ),
        Some(ImportedValue::Binary(bytes)) => context.plugin.format_binary_literal(&bytes),
    }
}

fn find_column<'a>(columns: &'a [ColumnInfo], name: &str) -> Option<&'a ColumnInfo> {
    columns
        .iter()
        .find(|column| column.name == name)
        .or_else(|| {
            columns
                .iter()
                .find(|column| column.name.eq_ignore_ascii_case(name))
        })
}

fn prepend_truncate(
    request: &XmlImportRequest<'_>,
    table: &str,
    statements: &mut Vec<String>,
) -> bool {
    let enabled = request.config.truncate_before_import && !statements.is_empty();
    if enabled {
        let table_ref = format_import_table_reference(request.plugin, request.config, table);
        statements.insert(0, format!("TRUNCATE TABLE {table_ref}"));
    }
    enabled
}

fn send_statement_progress(request: &XmlImportRequest<'_>, total: usize) {
    for statement_index in 0..total {
        send_progress(
            &request.progress_tx,
            ImportProgressEvent::ExecutingStatement {
                file: request.file_name.to_string(),
                statement_index,
                total_statements: total,
            },
        );
    }
}

async fn execute_statements(
    request: &XmlImportRequest<'_>,
    statements: Vec<String>,
    skip_first_result: bool,
) -> (u64, Vec<String>) {
    let options = ExecOptions {
        stop_on_error: request.config.stop_on_error,
        transactional: request.config.use_transaction,
        max_rows: None,
        streaming: false,
    };
    match request
        .connection
        .execute(request.plugin, &statements.join(";\n"), options)
        .await
    {
        Ok(results) => collect_results(request, results, skip_first_result),
        Err(error) => {
            send_error(request, &error.to_string());
            (0, vec![error.to_string()])
        }
    }
}

fn collect_results(
    request: &XmlImportRequest<'_>,
    results: Vec<SqlResult>,
    skip_first_result: bool,
) -> (u64, Vec<String>) {
    let mut rows_imported = 0;
    let mut errors = Vec::new();
    for (index, result) in results.into_iter().enumerate() {
        match result {
            SqlResult::Exec(result) => {
                if !(skip_first_result && index == 0) {
                    rows_imported += result.rows_affected;
                }
                send_progress(
                    &request.progress_tx,
                    ImportProgressEvent::StatementExecuted {
                        file: request.file_name.to_string(),
                        rows_affected: result.rows_affected,
                    },
                );
            }
            SqlResult::Error(error) => {
                errors.push(error.message.clone());
                send_error(request, &error.message);
            }
            SqlResult::Query(_) => {}
        }
    }
    (rows_imported, errors)
}

fn send_error(request: &XmlImportRequest<'_>, message: &str) {
    send_progress(
        &request.progress_tx,
        ImportProgressEvent::Error {
            file: request.file_name.to_string(),
            message: message.to_string(),
        },
    );
}

fn finished_import(
    request: &XmlImportRequest<'_>,
    start: Instant,
    rows_imported: u64,
    errors: Vec<String>,
) -> ImportResult {
    send_progress(
        &request.progress_tx,
        ImportProgressEvent::FileFinished {
            file: request.file_name.to_string(),
            rows_imported,
        },
    );
    ImportResult {
        success: errors.is_empty(),
        rows_imported,
        errors,
        elapsed_ms: start.elapsed().as_millis(),
    }
}

fn send_progress(progress_tx: &Option<ImportProgressSender>, event: ImportProgressEvent) {
    if let Some(tx) = progress_tx {
        let _ = tx.send(event);
    }
}
