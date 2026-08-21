use std::time::Instant;

use anyhow::{Result, anyhow};
use async_trait::async_trait;

use super::xml_codec::serialize_table;
use crate::DatabasePlugin;
use crate::connection::DbConnection;
use crate::executor::SqlResult;
use crate::import_export::{
    ExportConfig, ExportProgressEvent, ExportProgressSender, ExportResult, FormatHandler,
    ImportConfig, ImportProgressSender, ImportResult,
};

pub struct XmlFormatHandler;

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
        super::xml_import::import_xml(super::xml_import::XmlImportRequest {
            plugin,
            connection,
            config,
            data,
            file_name,
            progress_tx,
        })
        .await
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
    let mut base_sql = format!("SELECT {columns} FROM {table_ref}");
    if let Some(where_clause) = &config.where_clause {
        base_sql.push_str(" WHERE ");
        base_sql.push_str(where_clause);
    }
    let paginated_query = config
        .limit
        .map(|limit| plugin.build_paginated_query(&base_sql, limit, 0, ""));
    let sql = paginated_query
        .as_ref()
        .map(|query| query.sql.as_str())
        .unwrap_or(base_sql.as_str());
    match connection.query(sql).await? {
        SqlResult::Query(mut result) => {
            if let Some(paginated_query) = &paginated_query {
                paginated_query.strip_hidden_result_columns(&mut result)?;
            }
            crate::query_result_normalization::normalize_table_query_result(
                plugin,
                connection,
                &config.database,
                config.schema.as_deref(),
                table,
                &mut result,
            )
            .await?;
            Ok(result)
        }
        SqlResult::Error(error) => Err(anyhow!("Query failed: {}", error.message)),
        SqlResult::Exec(_) => Err(anyhow!("XML export query did not return rows")),
    }
}

fn xml_header() -> String {
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<data>\n".to_string()
}

fn send_export_progress(progress_tx: &Option<ExportProgressSender>, event: ExportProgressEvent) {
    if let Some(tx) = progress_tx {
        let _ = tx.send(event);
    }
}
