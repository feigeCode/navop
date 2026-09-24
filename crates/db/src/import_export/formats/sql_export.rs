use anyhow::Result;

use super::cells::{RenderCell, ResultCells};
use super::sync_typed_batch_from_legacy;
use crate::connection::DbConnection;
use crate::executor::{QueryResult, SqlResult};
use crate::import_export::{ExportConfig, ExportProgressEvent};
use crate::{
    ColumnInfo, DatabasePlugin, PaginatedQuery,
    query_result_normalization::normalize_query_result_binary_semantics,
};
use one_core::storage::DatabaseType;

const SQL_EXPORT_PAGE_SIZE: usize = 1000;

pub(super) async fn export_table_data_in_pages(
    plugin: &dyn DatabasePlugin,
    connection: &dyn DbConnection,
    config: &ExportConfig,
    table: &str,
    is_streaming: bool,
    output: &mut String,
    send_progress: &(dyn Fn(ExportProgressEvent) + Sync),
) -> Result<u64> {
    let table_ident =
        plugin.format_export_table_reference(&config.database, config.schema.as_deref(), table);
    let rows_per_statement = config.rows_per_statement.max(1);
    let mut offset = 0usize;
    let mut total_rows = 0u64;
    let mut remaining = config.limit;
    let mut wrote_header = false;
    let mut schema_columns: Option<Vec<ColumnInfo>> = None;

    loop {
        let Some(page_limit) = next_export_page_limit(remaining, rows_per_statement) else {
            break;
        };
        let paginated_query = export_page_select_sql(plugin, config, table, page_limit, offset);
        let mut query_result = query_export_page(connection, &paginated_query).await?;
        if plugin.name() == DatabaseType::MySQL {
            if schema_columns.is_none() {
                schema_columns = Some(
                    plugin
                        .list_columns(connection, &config.database, config.schema.clone(), table)
                        .await?,
                );
            }
            normalize_query_result_binary_semantics(
                &mut query_result,
                &DatabaseType::MySQL,
                schema_columns.as_deref().unwrap_or_default(),
            )?;
        }
        sync_typed_batch_from_legacy(&mut query_result)?;
        let rows_count = query_result.rows.len() as u64;
        let data_output = sql_dump_page(
            plugin,
            &table_ident,
            table,
            &query_result,
            &mut wrote_header,
            rows_per_statement,
        )?;

        append_or_send_export_page(
            output,
            is_streaming,
            send_progress,
            table,
            rows_count,
            data_output,
        );

        total_rows += rows_count;
        if rows_count < page_limit as u64 {
            break;
        }
        offset += page_limit;
        remaining = remaining.map(|limit| limit.saturating_sub(page_limit));
    }

    Ok(total_rows)
}

/// 计算下一页要取多少行。
///
/// 页大小向下取整成 `rows_per_statement` 的整数倍，这样除最后一页（不足一页，
/// 必然是表的尾部）以外的每一页都恰好由若干条完整语句组成，批量 INSERT 不会在
/// 分页边界被拆成两条语句。批量行数超过 [`SQL_EXPORT_PAGE_SIZE`] 时按批量行数
/// 取一页——一条语句本来就需要这么多行同时在手。
fn next_export_page_limit(remaining: Option<usize>, rows_per_statement: usize) -> Option<usize> {
    let rows_per_statement = rows_per_statement.max(1);
    let statements_per_page = (SQL_EXPORT_PAGE_SIZE / rows_per_statement).max(1);
    let page_size = rows_per_statement * statements_per_page;
    let page_limit = remaining
        .map(|limit| limit.min(page_size))
        .unwrap_or(page_size);
    (page_limit > 0).then_some(page_limit)
}

fn export_page_select_sql(
    plugin: &dyn DatabasePlugin,
    config: &ExportConfig,
    table: &str,
    page_limit: usize,
    offset: usize,
) -> PaginatedQuery {
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
    let mut select_sql = format!("SELECT {columns} FROM {table_ref}");
    if let Some(where_c) = &config.where_clause {
        select_sql.push_str(" WHERE ");
        select_sql.push_str(where_c);
    }
    plugin.build_paginated_query(&select_sql, page_limit, offset, "")
}

async fn query_export_page(
    connection: &dyn DbConnection,
    paginated_query: &PaginatedQuery,
) -> Result<QueryResult> {
    let mut query_result = match connection
        .query(&paginated_query.sql)
        .await
        .map_err(|e| anyhow::anyhow!("Query failed: {}", e))?
    {
        SqlResult::Query(query_result) => query_result,
        SqlResult::Exec(_) => {
            return Err(anyhow::anyhow!("Expected query result for SQL export"));
        }
        SqlResult::Error(error) => return Err(anyhow::anyhow!(error.message)),
    };
    paginated_query.strip_hidden_result_columns(&mut query_result)?;
    sync_typed_batch_from_legacy(&mut query_result)?;
    Ok(query_result)
}

fn sql_dump_page(
    plugin: &dyn DatabasePlugin,
    table_ident: &str,
    table: &str,
    query_result: &QueryResult,
    wrote_header: &mut bool,
    rows_per_statement: usize,
) -> Result<String> {
    if query_result.rows.is_empty() {
        ResultCells::new(query_result, "SQL")?;
        return Ok(String::new());
    }

    render_insert_statements_with_table_comment(
        plugin,
        table_ident,
        Some(table),
        query_result,
        wrote_header,
        rows_per_statement,
    )
}

/// 把结果集渲染成 INSERT 语句，`rows_per_statement` 行合并成一条。
pub(crate) fn render_insert_statements<P>(
    plugin: &P,
    table_ident: &str,
    query_result: &QueryResult,
    rows_per_statement: usize,
) -> Result<String>
where
    P: DatabasePlugin + ?Sized,
{
    let mut wrote_header = true;
    render_insert_statements_with_table_comment(
        plugin,
        table_ident,
        None,
        query_result,
        &mut wrote_header,
        rows_per_statement,
    )
}

fn render_insert_statements_with_table_comment<P>(
    plugin: &P,
    table_ident: &str,
    table_comment: Option<&str>,
    query_result: &QueryResult,
    wrote_header: &mut bool,
    rows_per_statement: usize,
) -> Result<String>
where
    P: DatabasePlugin + ?Sized,
{
    let cells = ResultCells::new(query_result, "SQL")?;
    let quoted_columns = query_result
        .columns
        .iter()
        .map(|column| plugin.quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    let context = InsertRenderContext {
        plugin,
        table_ident,
        columns: &query_result.columns,
        quoted_columns: &quoted_columns,
        column_meta: &query_result.column_meta,
        cells: &cells,
    };
    let rows_per_statement = rows_per_statement.max(1);
    let row_count = cells.row_count();
    let mut output = String::new();
    let mut batch_start = 0usize;
    while batch_start < row_count {
        let batch_end = row_count.min(batch_start + rows_per_statement);
        let mut values = String::new();
        for row_index in batch_start..batch_end {
            if row_index > batch_start {
                values.push_str(", ");
            }
            values.push_str(&format_row_values(&context, row_index));
        }
        push_insert_statement(&mut output, &context, table_comment, wrote_header, &values);
        batch_start = batch_end;
    }
    Ok(output)
}

struct InsertRenderContext<'a, 'b, P: DatabasePlugin + ?Sized> {
    plugin: &'a P,
    table_ident: &'a str,
    columns: &'a [String],
    /// 已按方言引用好的列清单，避免每一行重复拼接。
    quoted_columns: &'a str,
    column_meta: &'a [crate::executor::QueryColumnMeta],
    cells: &'b ResultCells<'a>,
}

fn push_insert_statement<P>(
    output: &mut String,
    context: &InsertRenderContext<'_, '_, P>,
    table_comment: Option<&str>,
    wrote_header: &mut bool,
    values: &str,
) where
    P: DatabasePlugin + ?Sized,
{
    if values.is_empty() {
        return;
    }
    if let Some(table) = table_comment {
        if !*wrote_header {
            output.push_str("-- Data for table ");
            output.push_str(table);
            output.push('\n');
            *wrote_header = true;
        }
    }
    output.push_str("INSERT INTO ");
    output.push_str(context.table_ident);
    output.push_str(" (");
    output.push_str(context.quoted_columns);
    output.push_str(") VALUES ");
    output.push_str(values);
    output.push_str(";\n");
}

fn format_row_values<P>(context: &InsertRenderContext<'_, '_, P>, row_index: usize) -> String
where
    P: DatabasePlugin + ?Sized,
{
    let mut values = String::from("(");
    for column_index in 0..context.columns.len() {
        if column_index > 0 {
            values.push_str(", ");
        }
        values.push_str(&format_export_value(context, row_index, column_index));
    }
    values.push(')');
    values
}

fn format_export_value<P>(
    context: &InsertRenderContext<'_, '_, P>,
    row_index: usize,
    column_index: usize,
) -> String
where
    P: DatabasePlugin + ?Sized,
{
    match context.cells.cell(row_index, column_index) {
        Some(RenderCell::Null) => "NULL".to_string(),
        Some(RenderCell::Binary(bytes)) => context.plugin.format_binary_literal(bytes),
        Some(RenderCell::Text(value)) => crate::sql_literal::format_query_text_value(
            context.plugin,
            Some(value),
            context.column_meta.get(column_index),
        ),
        None => unreachable!("result cells validated row and column bounds"),
    }
}

fn append_or_send_export_page(
    output: &mut String,
    is_streaming: bool,
    send_progress: &(dyn Fn(ExportProgressEvent) + Sync),
    table: &str,
    rows: u64,
    data_output: String,
) {
    let progress_data = if is_streaming {
        data_output
    } else {
        output.push_str(&data_output);
        data_output.clone()
    };
    send_progress(ExportProgressEvent::DataExported {
        table: table.to_string(),
        rows,
        data: progress_data,
    });
}

#[cfg(test)]
#[path = "sql_export_tests.rs"]
mod tests;
