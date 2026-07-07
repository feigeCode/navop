use anyhow::Result;

use crate::DatabasePlugin;
use crate::connection::DbConnection;
use crate::executor::{QueryResult, SqlResult};
use crate::import_export::{ExportConfig, ExportProgressEvent};

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
        let select_sql = export_page_select_sql(plugin, config, table, page_limit, offset);
        let query_result = query_export_page(connection, &select_sql).await?;
        let rows_count = query_result.rows.len() as u64;
        let data_output = sql_dump_page(
            plugin,
            &table_ident,
            table,
            &query_result,
            &mut wrote_header,
        );

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
) -> String {
    let table_ref =
        plugin.format_table_reference(&config.database, config.schema.as_deref(), table);
    let mut select_sql = format!("SELECT * FROM {}", table_ref);
    if let Some(where_c) = &config.where_clause {
        select_sql.push_str(" WHERE ");
        select_sql.push_str(where_c);
    }
    select_sql.push_str(&plugin.format_pagination(page_limit, offset, ""));
    select_sql
}

async fn query_export_page(connection: &dyn DbConnection, select_sql: &str) -> Result<QueryResult> {
    match connection
        .query(select_sql)
        .await
        .map_err(|e| anyhow::anyhow!("Query failed: {}", e))?
    {
        SqlResult::Query(query_result) => Ok(query_result),
        SqlResult::Exec(_) => Err(anyhow::anyhow!("Expected query result for SQL export")),
        SqlResult::Error(error) => Err(anyhow::anyhow!(error.message)),
    }
}

fn sql_dump_page(
    plugin: &dyn DatabasePlugin,
    table_ident: &str,
    table: &str,
    query_result: &QueryResult,
    wrote_header: &mut bool,
) -> String {
    if query_result.rows.is_empty() {
        return String::new();
    }

    let mut output = String::new();
    if !*wrote_header {
        output.push_str("-- Data for table ");
        output.push_str(table);
        output.push('\n');
        *wrote_header = true;
    }
    for row in &query_result.rows {
        push_insert_statement(plugin, &mut output, table_ident, &query_result.columns, row);
    }
    output
}

fn push_insert_statement(
    plugin: &dyn DatabasePlugin,
    output: &mut String,
    table_ident: &str,
    columns: &[String],
    row: &[Option<String>],
) {
    output.push_str("INSERT INTO ");
    output.push_str(table_ident);
    output.push_str(" (");
    output.push_str(
        &columns
            .iter()
            .map(|column| plugin.quote_identifier(column))
            .collect::<Vec<_>>()
            .join(", "),
    );
    output.push_str(") VALUES (");
    for (index, value) in row.iter().enumerate() {
        if index > 0 {
            output.push_str(", ");
        }
        push_sql_value(output, value.as_deref());
    }
    output.push_str(");\n");
}

fn push_sql_value(output: &mut String, value: Option<&str>) {
    match value {
        Some(value) => {
            output.push('\'');
            output.push_str(&value.replace('\'', "''"));
            output.push('\'');
        }
        None => output.push_str("NULL"),
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
