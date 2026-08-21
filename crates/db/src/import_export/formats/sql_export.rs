use anyhow::Result;

use crate::connection::DbConnection;
use crate::executor::{QueryCellRef, QueryResult, QueryResultView, SqlResult};
use crate::import_export::{ExportConfig, ExportProgressEvent};
use crate::{DatabasePlugin, PaginatedQuery};

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
    let mut offset = 0usize;
    let mut total_rows = 0u64;
    let mut remaining = config.limit;
    let mut wrote_header = false;

    loop {
        let Some(page_limit) = next_export_page_limit(remaining) else {
            break;
        };
        let paginated_query = export_page_select_sql(plugin, config, table, page_limit, offset);
        let query_result = query_export_page(connection, &paginated_query).await?;
        let rows_count = query_result.rows.len() as u64;
        let data_output = sql_dump_page(
            plugin,
            &table_ident,
            table,
            &query_result,
            &mut wrote_header,
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

fn next_export_page_limit(remaining: Option<usize>) -> Option<usize> {
    let page_limit = remaining
        .map(|limit| limit.min(SQL_EXPORT_PAGE_SIZE))
        .unwrap_or(SQL_EXPORT_PAGE_SIZE);
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
    let mut select_sql = format!("SELECT * FROM {}", table_ref);
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
    Ok(query_result)
}

fn sql_dump_page(
    plugin: &dyn DatabasePlugin,
    table_ident: &str,
    table: &str,
    query_result: &QueryResult,
    wrote_header: &mut bool,
) -> Result<String> {
    if query_result.rows.is_empty() {
        query_result
            .typed_view()
            .map_err(|error| anyhow::anyhow!("Invalid query result for SQL export: {error}"))?;
        return Ok(String::new());
    }

    let mut output = String::new();
    if !*wrote_header {
        output.push_str("-- Data for table ");
        output.push_str(table);
        output.push('\n');
        *wrote_header = true;
    }
    output.push_str(&render_insert_statements(
        plugin,
        table_ident,
        query_result,
    )?);
    Ok(output)
}

pub(crate) fn render_insert_statements<P>(
    plugin: &P,
    table_ident: &str,
    query_result: &QueryResult,
) -> Result<String>
where
    P: DatabasePlugin + ?Sized,
{
    let view = query_result
        .typed_view()
        .map_err(|error| anyhow::anyhow!("Invalid query result for SQL export: {error}"))?;
    let context = InsertRenderContext {
        plugin,
        table_ident,
        columns: &query_result.columns,
        column_meta: &query_result.column_meta,
        view: &view,
    };
    let mut output = String::new();
    for row_index in 0..query_result.rows.len() {
        push_insert_statement(&mut output, &context, row_index);
    }
    Ok(output)
}

struct InsertRenderContext<'a, P: DatabasePlugin + ?Sized> {
    plugin: &'a P,
    table_ident: &'a str,
    columns: &'a [String],
    column_meta: &'a [crate::executor::QueryColumnMeta],
    view: &'a QueryResultView<'a>,
}

fn push_insert_statement<P>(
    output: &mut String,
    context: &InsertRenderContext<'_, P>,
    row_index: usize,
) where
    P: DatabasePlugin + ?Sized,
{
    output.push_str("INSERT INTO ");
    output.push_str(context.table_ident);
    output.push_str(" (");
    output.push_str(
        &context
            .columns
            .iter()
            .map(|column| context.plugin.quote_identifier(column))
            .collect::<Vec<_>>()
            .join(", "),
    );
    output.push_str(") VALUES (");
    for column_index in 0..context.columns.len() {
        if column_index > 0 {
            output.push_str(", ");
        }
        output.push_str(&format_export_value(context, row_index, column_index));
    }
    output.push_str(");\n");
}

fn format_export_value<P>(
    context: &InsertRenderContext<'_, P>,
    row_index: usize,
    column_index: usize,
) -> String
where
    P: DatabasePlugin + ?Sized,
{
    match context.view.cell(row_index, column_index) {
        Some(QueryCellRef::Null) => "NULL".to_string(),
        Some(QueryCellRef::Binary(bytes)) => context.plugin.format_binary_literal(bytes),
        Some(QueryCellRef::Text(value)) => crate::sql_literal::format_query_text_value(
            context.plugin,
            Some(value),
            context.column_meta.get(column_index),
        ),
        None => unreachable!("typed view validated row and column bounds"),
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
