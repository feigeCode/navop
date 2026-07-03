use serde_json::{Value, json};
use tool_runtime::{ToolError, ToolResult};

use super::{DatabaseToolHandler, OpenedDatabase, execute_sql, required_str, tool_error};

const DEFAULT_SAMPLE_ROWS: usize = 20;
const MAX_SAMPLE_ROWS: usize = 100;
type TableMetadata = (
    Vec<db::ColumnInfo>,
    Vec<db::IndexInfo>,
    Vec<db::ForeignKeyDefinition>,
);

pub(super) async fn tables(
    handler: &DatabaseToolHandler,
    input: Value,
) -> Result<ToolResult, ToolError> {
    let mut opened = handler.open_database(&input).await?;
    let tables = opened
        .plugin
        .list_tables(
            &*opened.db_connection,
            &opened.database,
            opened.schema.clone(),
        )
        .await
        .map_err(tool_error);
    let _ = opened.db_connection.disconnect().await;
    Ok(ToolResult::structured(json!({
        "connection": opened.connection,
        "database": opened.database,
        "schema": opened.schema,
        "tables": tables?
    })))
}

pub(super) async fn describe_table(
    handler: &DatabaseToolHandler,
    input: Value,
) -> Result<ToolResult, ToolError> {
    let table = required_str(&input, "table")?;
    let mut opened = handler.open_database(&input).await?;
    let metadata = collect_table_metadata(&opened, &table).await;
    let _ = opened.db_connection.disconnect().await;
    let (columns, indexes, foreign_keys) = metadata?;
    Ok(ToolResult::structured(json!({
        "connection": opened.connection,
        "database": opened.database,
        "schema": opened.schema,
        "table": table,
        "columns": columns,
        "indexes": indexes,
        "foreign_keys": foreign_keys
    })))
}

pub(super) async fn sample_rows(
    handler: &DatabaseToolHandler,
    input: Value,
) -> Result<ToolResult, ToolError> {
    let table = required_str(&input, "table")?;
    let limit = row_limit(&input)?;
    let opened = handler.open_database(&input).await?;
    let OpenedDatabase {
        connection,
        database,
        schema,
        plugin,
        mut db_connection,
    } = opened;
    let sql = format!(
        "select * from {}",
        qualified_table_name(plugin.as_ref(), schema.as_deref(), &table)
    );
    let config = db_connection.config().clone();
    let _ = db_connection.disconnect().await;
    let results = execute_sql(
        plugin,
        config,
        &sql,
        db::ExecOptions {
            max_rows: Some(limit),
            ..Default::default()
        },
    )
    .await?;
    Ok(ToolResult::structured(json!({
        "connection": connection,
        "database": database,
        "schema": schema,
        "table": table,
        "limit": limit,
        "sql": sql,
        "result": results.into_iter().next()
    })))
}

async fn collect_table_metadata(
    opened: &OpenedDatabase,
    table: &str,
) -> Result<TableMetadata, ToolError> {
    let columns = opened
        .plugin
        .list_columns(
            &*opened.db_connection,
            &opened.database,
            opened.schema.clone(),
            table,
        )
        .await
        .map_err(tool_error)?;
    let indexes = opened
        .plugin
        .list_indexes(
            &*opened.db_connection,
            &opened.database,
            opened.schema.clone(),
            table,
        )
        .await
        .map_err(tool_error)?;
    let foreign_keys = opened
        .plugin
        .list_foreign_keys(
            &*opened.db_connection,
            &opened.database,
            opened.schema.clone(),
            table,
        )
        .await
        .map_err(tool_error)?;
    Ok((columns, indexes, foreign_keys))
}

fn row_limit(input: &Value) -> Result<usize, ToolError> {
    match input.get("limit") {
        None | Some(Value::Null) => Ok(DEFAULT_SAMPLE_ROWS),
        Some(Value::Number(value)) => value
            .as_u64()
            .filter(|limit| (1..=MAX_SAMPLE_ROWS as u64).contains(limit))
            .map(|limit| limit as usize)
            .ok_or_else(invalid_limit),
        _ => Err(invalid_limit()),
    }
}

fn invalid_limit() -> ToolError {
    ToolError::Failed {
        message: format!("field `limit` must be an integer from 1 to {MAX_SAMPLE_ROWS}"),
    }
}

fn qualified_table_name(
    plugin: &dyn db::DatabasePlugin,
    schema: Option<&str>,
    table: &str,
) -> String {
    let table = plugin.quote_identifier(table);
    match schema.filter(|value| !value.is_empty()) {
        Some(schema) => format!("{}.{}", plugin.quote_identifier(schema), table),
        None => table,
    }
}
