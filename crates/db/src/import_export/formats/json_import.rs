use std::time::Instant;

use anyhow::{Result, anyhow};
use serde_json::Value;

use super::import_execution::{ImportStatement, execute_import_statements};
use super::{format_import_table_reference, format_import_text_value, load_import_columns};
use crate::DatabasePlugin;
use crate::connection::DbConnection;
use crate::import_export::{ImportConfig, ImportResult};
use crate::types::ColumnInfo;

pub(super) struct JsonImportRequest<'a> {
    pub plugin: &'a dyn DatabasePlugin,
    pub connection: &'a dyn DbConnection,
    pub config: &'a ImportConfig,
    pub data: &'a str,
}

pub(super) async fn import_json(request: JsonImportRequest<'_>) -> Result<ImportResult> {
    let start = Instant::now();
    let table = request
        .config
        .table
        .as_deref()
        .ok_or_else(|| anyhow!("Table name required for JSON import"))?;
    let rows = parse_rows(request.data)?;
    if rows.is_empty() {
        return Ok(import_result(start, 0, Vec::new()));
    }

    let columns = first_row_columns(&rows)?;
    let table_columns =
        load_import_columns(request.plugin, request.connection, request.config, table).await?;
    let table_ref = format_import_table_reference(request.plugin, request.config, table);
    let context = StatementContext {
        plugin: request.plugin,
        config: request.config,
        table_ref: &table_ref,
        columns: &columns,
        table_columns: &table_columns,
    };
    let (statements, mut errors) = build_statements(&context, rows);
    if request.config.use_transaction && !errors.is_empty() {
        return Ok(import_result(start, 0, errors));
    }
    let (rows_imported, execution_errors) = execute_import_statements(
        request.plugin,
        request.connection,
        request.config,
        statements,
    )
    .await;
    errors.extend(execution_errors);
    Ok(import_result(start, rows_imported, errors))
}

fn parse_rows(data: &str) -> Result<Vec<Value>> {
    match serde_json::from_str(data)? {
        Value::Array(rows) => Ok(rows),
        value @ Value::Object(_) => Ok(vec![value]),
        _ => Err(anyhow!("JSON must be array or object")),
    }
}

fn first_row_columns(rows: &[Value]) -> Result<Vec<String>> {
    rows.first()
        .and_then(Value::as_object)
        .map(|row| row.keys().cloned().collect())
        .ok_or_else(|| anyhow!("JSON array must contain objects"))
}

struct StatementContext<'a> {
    plugin: &'a dyn DatabasePlugin,
    config: &'a ImportConfig,
    table_ref: &'a str,
    columns: &'a [String],
    table_columns: &'a [ColumnInfo],
}

fn build_statements(
    context: &StatementContext<'_>,
    rows: Vec<Value>,
) -> (Vec<ImportStatement>, Vec<String>) {
    let mut statements = Vec::new();
    let mut errors = Vec::new();
    if context.config.truncate_before_import {
        statements.push(ImportStatement::truncate(format!(
            "TRUNCATE TABLE {}",
            context.table_ref
        )));
    }
    for row in rows {
        let Some(object) = row.as_object() else {
            errors.push("Row is not an object".to_string());
            if context.config.stop_on_error {
                break;
            }
            continue;
        };
        statements.push(ImportStatement::row(
            insert_statement(context, object),
            "Insert failed",
        ));
    }
    (statements, errors)
}

fn insert_statement(
    context: &StatementContext<'_>,
    object: &serde_json::Map<String, Value>,
) -> String {
    let columns = context
        .columns
        .iter()
        .map(|column| context.plugin.quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    let value_context = JsonValueContext {
        plugin: context.plugin,
        table_columns: context.table_columns,
    };
    let values = context
        .columns
        .iter()
        .map(|column| format_json_value(&value_context, object.get(column), column))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "INSERT INTO {} ({columns}) VALUES ({values})",
        context.table_ref
    )
}

struct JsonValueContext<'a> {
    plugin: &'a dyn DatabasePlugin,
    table_columns: &'a [ColumnInfo],
}

fn format_json_value(
    context: &JsonValueContext<'_>,
    value: Option<&Value>,
    column: &str,
) -> String {
    match value {
        Some(Value::Null) | None => format_text_value(context, None, column),
        Some(Value::String(value)) => format_text_value(context, Some(value.clone()), column),
        Some(Value::Number(value)) if context.table_columns.is_empty() => value.to_string(),
        Some(Value::Bool(value)) if context.table_columns.is_empty() => {
            if *value { "1" } else { "0" }.to_string()
        }
        Some(Value::Number(value)) => format_text_value(context, Some(value.to_string()), column),
        Some(Value::Bool(value)) => format_text_value(context, Some(value.to_string()), column),
        Some(value) => context.plugin.escape_sql_value(&value.to_string()),
    }
}

fn format_text_value(
    context: &JsonValueContext<'_>,
    value: Option<String>,
    column: &str,
) -> String {
    format_import_text_value(context.plugin, &value, column, context.table_columns)
}

fn import_result(start: Instant, rows_imported: u64, errors: Vec<String>) -> ImportResult {
    ImportResult {
        success: errors.is_empty(),
        rows_imported,
        errors,
        elapsed_ms: start.elapsed().as_millis(),
    }
}
