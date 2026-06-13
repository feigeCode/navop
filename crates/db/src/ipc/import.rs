use std::time::Instant;

use anyhow::{Result, anyhow};
use extension_protocol::data as wire_data;
use extension_protocol::method as wire_method;
use extension_protocol::row::Row;
use serde_json::Value;

use crate::connection::{DbConnection, DbError};
use crate::executor::{ExecOptions, SqlResult};
use crate::import_export::{
    DataFormat, ImportConfig, ImportProgressEvent, ImportProgressSender, ImportResult,
};
use crate::ipc::import_parse::{ParsedRows, parse_rows};
use crate::{DatabasePlugin, plugin};

const IMPORT_CHUNK_ROWS: usize = 1_000;

pub async fn import_data_with_progress(
    plugin: &dyn DatabasePlugin,
    connection: &dyn DbConnection,
    config: &ImportConfig,
    data: &str,
    file_name: &str,
    progress_tx: Option<ImportProgressSender>,
) -> Result<ImportResult> {
    if matches!(config.format, DataFormat::Sql | DataFormat::Xml) {
        return fallback_import(plugin, connection, config, data, file_name, progress_tx).await;
    }
    match import_rows_via_driver(
        plugin,
        connection,
        config,
        data,
        file_name,
        progress_tx.clone(),
    )
    .await
    {
        Ok(result) => Ok(result),
        Err(error) if is_not_supported(&error) => {
            fallback_import(plugin, connection, config, data, file_name, None).await
        }
        Err(error) => Err(error),
    }
}

async fn import_rows_via_driver(
    plugin: &dyn DatabasePlugin,
    connection: &dyn DbConnection,
    config: &ImportConfig,
    data: &str,
    file_name: &str,
    progress_tx: Option<ImportProgressSender>,
) -> Result<ImportResult> {
    let start = Instant::now();
    let table = config
        .table
        .as_ref()
        .ok_or_else(|| anyhow!("Table name required for external import"))?;
    send_progress(
        &progress_tx,
        ImportProgressEvent::FileStart {
            file: file_name.to_string(),
            file_index: 0,
            total_files: 1,
        },
    );
    send_progress(
        &progress_tx,
        ImportProgressEvent::ParsingFile {
            file: file_name.to_string(),
        },
    );
    let parsed = parse_rows(config, data)?;
    truncate_target_if_needed(plugin, connection, config, table).await?;
    if parsed.rows.is_empty() {
        return Ok(import_result(true, 0, Vec::new(), start));
    }
    let import_id = begin_import(connection, config, table, &parsed).await?;
    let import_outcome = send_import_chunks(connection, &import_id, &parsed.rows, config).await;
    match import_outcome {
        Ok((rows_imported, errors)) if errors.is_empty() || !config.stop_on_error => {
            commit_import(connection, &import_id).await?;
            send_progress(
                &progress_tx,
                ImportProgressEvent::FileFinished {
                    file: file_name.to_string(),
                    rows_imported,
                },
            );
            Ok(import_result(
                errors.is_empty(),
                rows_imported,
                errors,
                start,
            ))
        }
        Ok((_, errors)) => {
            abort_import(connection, &import_id).await?;
            send_progress_error(&progress_tx, file_name, &errors);
            Ok(import_result(false, 0, errors, start))
        }
        Err(error) => {
            let _ = abort_import(connection, &import_id).await;
            Err(error)
        }
    }
}

async fn begin_import(
    connection: &dyn DbConnection,
    config: &ImportConfig,
    table: &str,
    parsed: &ParsedRows,
) -> Result<String> {
    let value = connection
        .driver_request_value(
            wire_method::DATA_IMPORT_BEGIN,
            serde_json::json!({
                "table": table,
                "schema": config.schema,
                "database": empty_to_null(&config.database),
                "format": parsed.format,
                "columns": parsed.columns,
                "options": {"track_failed_rows": true}
            }),
        )
        .await?;
    let result: wire_data::ImportBeginResult = serde_json::from_value(value)?;
    Ok(result.import_id)
}

async fn send_import_chunks(
    connection: &dyn DbConnection,
    import_id: &str,
    rows: &[Row],
    config: &ImportConfig,
) -> Result<(u64, Vec<String>)> {
    let mut rows_imported = 0u64;
    let mut errors = Vec::new();
    for chunk in rows.chunks(IMPORT_CHUNK_ROWS) {
        let value = connection
            .driver_request_value(
                wire_method::DATA_IMPORT_CHUNK,
                serde_json::json!({ "import_id": import_id, "rows": chunk }),
            )
            .await?;
        let result: wire_data::ImportChunkResult = serde_json::from_value(value)?;
        rows_imported = rows_imported.saturating_add(result.inserted);
        errors.extend(result.failed.into_iter().map(|row| row.message));
        if config.stop_on_error && !errors.is_empty() {
            break;
        }
    }
    Ok((rows_imported, errors))
}

async fn commit_import(connection: &dyn DbConnection, import_id: &str) -> Result<()> {
    connection
        .driver_request_value(
            wire_method::DATA_IMPORT_COMMIT,
            serde_json::json!({ "import_id": import_id }),
        )
        .await?;
    Ok(())
}

async fn abort_import(connection: &dyn DbConnection, import_id: &str) -> Result<()> {
    connection
        .driver_request_value(
            wire_method::DATA_IMPORT_ABORT,
            serde_json::json!({ "import_id": import_id }),
        )
        .await?;
    Ok(())
}

async fn truncate_target_if_needed(
    plugin: &dyn DatabasePlugin,
    connection: &dyn DbConnection,
    config: &ImportConfig,
    table: &str,
) -> Result<()> {
    if !config.truncate_before_import {
        return Ok(());
    }
    let table_ref =
        plugin.format_table_reference(&config.database, config.schema.as_deref(), table);
    let results = connection
        .execute(
            plugin,
            &format!("TRUNCATE TABLE {table_ref}"),
            ExecOptions::default(),
        )
        .await?;
    for result in results {
        if let SqlResult::Error(error) = result {
            return Err(anyhow!("Truncate failed: {}", error.message));
        }
    }
    Ok(())
}

async fn fallback_import(
    plugin: &dyn DatabasePlugin,
    connection: &dyn DbConnection,
    config: &ImportConfig,
    data: &str,
    file_name: &str,
    progress_tx: Option<ImportProgressSender>,
) -> Result<ImportResult> {
    plugin::default_import_data_with_progress(
        plugin,
        connection,
        config,
        data,
        file_name,
        progress_tx,
    )
    .await
}

fn empty_to_null(value: &str) -> Value {
    if value.trim().is_empty() {
        Value::Null
    } else {
        Value::String(value.to_string())
    }
}

fn import_result(
    success: bool,
    rows_imported: u64,
    errors: Vec<String>,
    start: Instant,
) -> ImportResult {
    ImportResult {
        success,
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

fn send_progress_error(
    progress_tx: &Option<ImportProgressSender>,
    file_name: &str,
    errors: &[String],
) {
    for error in errors {
        send_progress(
            progress_tx,
            ImportProgressEvent::Error {
                file: file_name.to_string(),
                message: error.clone(),
            },
        );
    }
}

fn is_not_supported(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<DbError>()
        .is_some_and(|error| matches!(error, DbError::NotSupported(_)))
}
