use std::time::Instant;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde_json::Value;

use crate::DatabasePlugin;
use crate::connection::DbConnection;
use crate::executor::SqlResult;
use crate::import_export::{
    ExportConfig, ExportProgressEvent, ExportProgressSender, ExportResult, FormatHandler,
    ImportConfig, ImportResult,
};

pub struct JsonFormatHandler;

#[async_trait]
impl FormatHandler for JsonFormatHandler {
    async fn import(
        &self,
        plugin: &dyn DatabasePlugin,
        connection: &dyn DbConnection,
        config: &ImportConfig,
        data: &str,
    ) -> Result<ImportResult> {
        super::json_import::import_json(super::json_import::JsonImportRequest {
            plugin,
            connection,
            config,
            data,
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
        let mut all_data = Vec::new();
        let mut total_rows = 0u64;
        let total_tables = config.tables.len();
        let is_streaming = progress_tx.is_some();

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
                let mut table_data = Vec::new();
                let rows_count = query_result.rows.len() as u64;

                for row in &query_result.rows {
                    let mut obj = serde_json::Map::new();
                    for (i, col_name) in query_result.columns.iter().enumerate() {
                        let value = match &row[i] {
                            Some(v) => Value::String(v.clone()),
                            None => Value::Null,
                        };
                        obj.insert(col_name.clone(), value);
                    }
                    table_data.push(Value::Object(obj));
                }

                total_rows += rows_count;

                let table_output = if is_streaming {
                    if index == 0 {
                        format!(
                            "[\n{}",
                            serde_json::to_string_pretty(&table_data)?
                                .trim_start_matches('[')
                                .trim_end_matches(']')
                        )
                    } else if index == total_tables - 1 {
                        format!(
                            ",\n{}\n]",
                            serde_json::to_string_pretty(&table_data)?
                                .trim_start_matches('[')
                                .trim_end_matches(']')
                        )
                    } else {
                        format!(
                            ",\n{}",
                            serde_json::to_string_pretty(&table_data)?
                                .trim_start_matches('[')
                                .trim_end_matches(']')
                        )
                    }
                } else {
                    String::new()
                };

                send_progress(ExportProgressEvent::DataExported {
                    table: table.clone(),
                    rows: rows_count,
                    data: table_output,
                });

                if !is_streaming {
                    all_data.extend(table_data);
                }
            }

            send_progress(ExportProgressEvent::TableFinished {
                table: table.clone(),
            });
        }

        let output = if !is_streaming {
            serde_json::to_string_pretty(&all_data)?
        } else {
            String::new()
        };

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
