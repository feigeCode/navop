use std::time::Instant;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde_json::Value;

use crate::DatabasePlugin;
use crate::connection::DbConnection;
use crate::executor::{QueryCellRef, QueryResult, SqlResult};
use crate::import_export::{
    ExportConfig, ExportProgressEvent, ExportProgressSender, ExportResult, FormatHandler,
    ImportConfig, ImportResult,
};

pub struct JsonFormatHandler;

fn query_result_to_json_rows(query_result: &QueryResult) -> Result<Vec<Value>> {
    let view = query_result
        .typed_view()
        .map_err(|error| anyhow!("Invalid query result for JSON export: {error}"))?;
    let mut rows = Vec::with_capacity(query_result.rows.len());

    for row_index in 0..query_result.rows.len() {
        let mut object = serde_json::Map::new();
        for (column_index, column_name) in query_result.columns.iter().enumerate() {
            let value = match view.cell(row_index, column_index) {
                Some(QueryCellRef::Null) => Value::Null,
                Some(QueryCellRef::Text(value)) => Value::String(value.to_string()),
                Some(QueryCellRef::Binary(_)) => {
                    return Err(anyhow!(
                        "JSON export does not support binary cell at row {}, column {} ({column_name:?}) without an explicit binary encoding",
                        row_index + 1,
                        column_index + 1,
                    ));
                }
                None => unreachable!("typed view validated row and column bounds"),
            };
            object.insert(column_name.clone(), value);
        }
        rows.push(Value::Object(object));
    }

    Ok(rows)
}

fn render_streaming_json_chunk(
    rows: &[Value],
    is_last_table: bool,
    stream_started: &mut bool,
    wrote_row: &mut bool,
) -> Result<String> {
    let mut output = String::new();

    if !*stream_started {
        output.push('[');
        *stream_started = true;
    }

    for row in rows {
        if *wrote_row {
            output.push_str(",\n");
        } else {
            output.push('\n');
        }

        let rendered = serde_json::to_string_pretty(row)?;
        for (line_index, line) in rendered.lines().enumerate() {
            if line_index > 0 {
                output.push('\n');
            }
            output.push_str("  ");
            output.push_str(line);
        }
        *wrote_row = true;
    }

    if is_last_table {
        if *wrote_row {
            output.push('\n');
        }
        output.push(']');
    }

    Ok(output)
}

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
        let mut json_stream_started = false;
        let mut wrote_json_row = false;

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

            let table_ref =
                plugin.format_table_reference(&config.database, config.schema.as_deref(), table);
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
                crate::query_result_normalization::normalize_table_query_result(
                    plugin,
                    connection,
                    &config.database,
                    config.schema.as_deref(),
                    table,
                    &mut query_result,
                )
                .await?;
                let rows_count = query_result.rows.len() as u64;
                let table_data = query_result_to_json_rows(&query_result)?;

                total_rows += rows_count;

                let table_output = if is_streaming {
                    render_streaming_json_chunk(
                        &table_data,
                        index == total_tables - 1,
                        &mut json_stream_started,
                        &mut wrote_json_row,
                    )?
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

#[cfg(test)]
mod tests {
    use super::{query_result_to_json_rows, render_streaming_json_chunk};
    use crate::executor::{BinaryCell, QueryColumnMeta, QueryResult};
    use serde_json::{Value, json};

    fn render_tables(tables: Vec<Vec<Value>>) -> String {
        let total_tables = tables.len();
        let mut stream_started = false;
        let mut wrote_row = false;
        let mut output = String::new();

        for (index, rows) in tables.iter().enumerate() {
            output.push_str(
                &render_streaming_json_chunk(
                    rows,
                    index == total_tables - 1,
                    &mut stream_started,
                    &mut wrote_row,
                )
                .unwrap(),
            );
        }

        output
    }

    #[test]
    fn streaming_json_closes_a_single_empty_table() {
        let output = render_tables(vec![vec![]]);

        assert_eq!(output, "[]");
        assert_eq!(serde_json::from_str::<Value>(&output).unwrap(), json!([]));
    }

    #[test]
    fn streaming_json_ignores_empty_tables_when_placing_separators() {
        let output = render_tables(vec![
            vec![],
            vec![json!({ "id": "1" })],
            vec![],
            vec![json!({ "id": "2" })],
            vec![],
        ]);

        assert_eq!(
            serde_json::from_str::<Value>(&output).unwrap(),
            json!([{ "id": "1" }, { "id": "2" }])
        );
        assert_eq!(output.matches(',').count(), 1);
    }

    #[test]
    fn streaming_json_closes_multiple_empty_tables() {
        let output = render_tables(vec![vec![], vec![], vec![]]);

        assert_eq!(output, "[]");
        assert_eq!(serde_json::from_str::<Value>(&output).unwrap(), json!([]));
    }

    #[test]
    fn json_rows_preserve_null_and_empty_text() {
        let result = QueryResult {
            sql: String::new(),
            columns: vec!["nullable".to_string(), "empty".to_string()],
            column_meta: vec![
                QueryColumnMeta::new("nullable", "TEXT"),
                QueryColumnMeta::new("empty", "TEXT"),
            ],
            rows: vec![vec![None, Some(String::new())]],
            binary_cells: vec![],
            elapsed_ms: 0,
        };

        let rows = query_result_to_json_rows(&result).expect("text-only JSON should render");

        assert_eq!(rows, vec![json!({ "nullable": null, "empty": "" })]);
    }

    #[test]
    fn json_rows_reject_binary_sidecar_even_with_display_text() {
        let result = QueryResult {
            sql: String::new(),
            columns: vec!["payload".to_string()],
            column_meta: vec![QueryColumnMeta::new("payload", "BLOB")],
            rows: vec![vec![Some("true".to_string())]],
            binary_cells: vec![BinaryCell {
                row_index: 0,
                column_index: 0,
                bytes: b"true".to_vec(),
            }],
            elapsed_ms: 0,
        };

        let error =
            query_result_to_json_rows(&result).expect_err("JSON has no binary wire encoding");

        assert!(
            error
                .to_string()
                .contains("JSON export does not support binary cell at row 1, column 1")
        );
    }

    #[test]
    fn json_rows_reject_short_rows_without_panicking() {
        let result = QueryResult {
            sql: String::new(),
            columns: vec!["a".to_string(), "b".to_string()],
            column_meta: vec![
                QueryColumnMeta::new("a", "TEXT"),
                QueryColumnMeta::new("b", "TEXT"),
            ],
            rows: vec![vec![Some("only one".to_string())]],
            binary_cells: vec![],
            elapsed_ms: 0,
        };

        let error = query_result_to_json_rows(&result)
            .expect_err("short rows must fail instead of indexing past the row");

        assert!(
            error
                .to_string()
                .contains("Invalid query result for JSON export")
        );
    }
}
