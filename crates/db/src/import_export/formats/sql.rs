use std::time::Instant;

use anyhow::Result;
use async_trait::async_trait;

use super::format_import_table_reference;
use super::sql_export::export_table_data_in_pages;
use crate::connection::DbConnection;
use crate::executor::{ExecOptions, SqlResult};
use crate::import_export::{
    ExportConfig, ExportProgressEvent, ExportProgressSender, ExportResult, FormatHandler,
    ImportConfig, ImportProgressEvent, ImportProgressSender, ImportResult,
};
use crate::{DatabasePlugin, DbError, SqlSource};

pub struct SqlFormatHandler;

#[async_trait]
impl FormatHandler for SqlFormatHandler {
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
        let mut errors = Vec::new();
        let mut total_rows = 0u64;

        let send_progress = |event: ImportProgressEvent| {
            if let Some(tx) = &progress_tx {
                let _ = tx.send(event);
            }
        };

        send_progress(ImportProgressEvent::ParsingFile {
            file: file_name.to_string(),
        });

        let parser = plugin
            .create_parser(SqlSource::Script(data.to_string()))
            .map_err(|e| DbError::query(format!("Failed to create parser: {}", e)))?;
        let mut statements = collect_sql_statements(parser)?;
        let mut truncate_inserted = false;
        if config.truncate_before_import {
            if let Some(table) = &config.table {
                let table_ref = format_import_table_reference(plugin, config, table);
                statements.insert(0, format!("TRUNCATE TABLE {}", table_ref));
                truncate_inserted = true;
            }
        }
        let total_statements = statements.len();

        for idx in 0..total_statements {
            send_progress(ImportProgressEvent::ExecutingStatement {
                file: file_name.to_string(),
                statement_index: idx,
                total_statements,
            });
        }

        if !statements.is_empty() {
            let script = statements.join(";\n");
            let exec_options = ExecOptions {
                stop_on_error: config.stop_on_error,
                transactional: config.use_transaction,
                max_rows: None,
                streaming: false,
            };

            match connection.execute(plugin, &script, exec_options).await {
                Ok(results) => {
                    for (index, result) in results.into_iter().enumerate() {
                        match result {
                            SqlResult::Exec(exec_result) => {
                                if !(truncate_inserted && index == 0) {
                                    total_rows += exec_result.rows_affected;
                                }
                                send_progress(ImportProgressEvent::StatementExecuted {
                                    file: file_name.to_string(),
                                    rows_affected: exec_result.rows_affected,
                                });
                            }
                            SqlResult::Error(err) => {
                                let error_msg = err.message.clone();
                                errors.push(error_msg.clone());
                                send_progress(ImportProgressEvent::Error {
                                    file: file_name.to_string(),
                                    message: error_msg,
                                });
                            }
                            _ => {}
                        }
                    }
                }
                Err(e) => {
                    let error_msg = e.to_string();
                    errors.push(error_msg.clone());
                    send_progress(ImportProgressEvent::Error {
                        file: file_name.to_string(),
                        message: error_msg,
                    });
                }
            }
        }
        if config.use_transaction && !errors.is_empty() {
            total_rows = 0;
        }

        let elapsed_ms = start.elapsed().as_millis();
        send_progress(ImportProgressEvent::FileFinished {
            file: file_name.to_string(),
            rows_imported: total_rows,
        });

        Ok(ImportResult {
            success: errors.is_empty(),
            rows_imported: total_rows,
            errors,
            elapsed_ms,
        })
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
        let mut output = String::new();
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

            if config.include_schema {
                send_progress(ExportProgressEvent::GettingStructure {
                    table: table.clone(),
                });

                match plugin
                    .export_table_create_sql(
                        connection,
                        &config.database,
                        config.schema.as_deref(),
                        table,
                    )
                    .await
                {
                    Ok(schema_sql) => {
                        let mut schema_output = String::new();
                        if !schema_sql.is_empty() {
                            schema_output.push_str("-- Table structure for ");
                            schema_output.push_str(table);
                            schema_output.push('\n');
                            schema_output.push_str(&schema_sql);
                            schema_output.push_str(";\n\n");
                        }
                        let progress_data = if is_streaming {
                            std::mem::take(&mut schema_output)
                        } else {
                            schema_output.clone()
                        };
                        send_progress(ExportProgressEvent::StructureExported {
                            table: table.clone(),
                            data: progress_data,
                        });
                        if !is_streaming {
                            output.push_str(&schema_output);
                        }
                    }
                    Err(e) => {
                        let error_output =
                            format!("-- Failed to export structure for {}: {}\n\n", table, e);
                        if !is_streaming {
                            output.push_str(&error_output);
                        }
                        send_progress(ExportProgressEvent::Error {
                            table: table.clone(),
                            message: format!("Failed to export structure: {}", e),
                        });
                    }
                }
            }

            if config.include_data {
                send_progress(ExportProgressEvent::FetchingData {
                    table: table.clone(),
                });

                match export_table_data_in_pages(
                    plugin,
                    connection,
                    config,
                    table,
                    is_streaming,
                    &mut output,
                    &send_progress,
                )
                .await
                {
                    Ok(rows_count) => {
                        total_rows += rows_count;
                    }
                    Err(e) => {
                        let error_output =
                            format!("-- Failed to export data for {}: {}\n\n", table, e);
                        if !is_streaming {
                            output.push_str(&error_output);
                        }
                        send_progress(ExportProgressEvent::Error {
                            table: table.clone(),
                            message: format!("Failed to export data: {}", e),
                        });
                    }
                }
            }

            send_progress(ExportProgressEvent::TableFinished {
                table: table.clone(),
            });
        }

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

fn collect_sql_statements<I>(parser: I) -> Result<Vec<String>, DbError>
where
    I: IntoIterator<Item = std::io::Result<String>>,
{
    Ok(parser
        .into_iter()
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(|e| DbError::query_with_source("failed to parse SQL import", e))?
        .into_iter()
        .map(|statement| statement.trim().to_string())
        .filter(|statement| !statement.is_empty())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::collect_sql_statements;
    use std::io;

    #[test]
    fn collect_sql_statements_propagates_iterator_error() {
        let parser = vec![
            Ok("INSERT INTO t VALUES (1);".to_string()),
            Err(io::Error::other("read failure")),
        ];

        let error = collect_sql_statements(parser).expect_err("parser error should be propagated");

        assert!(error.to_string().contains("read failure"));
    }
}
