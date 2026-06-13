use std::time::Instant;

use anyhow::Result;
use base64::Engine;
use extension_protocol::data as wire_data;
use extension_protocol::method as wire_method;
use serde_json::Value;

use crate::connection::{DbConnection, DbError};
use crate::import_export::{
    DataFormat, ExportConfig, ExportProgressEvent, ExportProgressSender, ExportResult,
};
use crate::{DatabasePlugin, plugin};

const EXPORT_READ_BYTES: u32 = 256 * 1024;

pub async fn export_data_with_progress(
    plugin: &dyn DatabasePlugin,
    connection: &dyn DbConnection,
    config: &ExportConfig,
    progress_tx: Option<ExportProgressSender>,
) -> Result<ExportResult> {
    if matches!(config.format, DataFormat::Sql | DataFormat::Xml) {
        return fallback_export(plugin, connection, config, progress_tx).await;
    }
    match export_via_driver(plugin, connection, config, progress_tx.clone()).await {
        Ok(result) => Ok(result),
        Err(error) if is_not_supported(&error) => {
            fallback_export(plugin, connection, config, progress_tx).await
        }
        Err(error) => Err(error),
    }
}

async fn export_via_driver(
    _plugin: &dyn DatabasePlugin,
    connection: &dyn DbConnection,
    config: &ExportConfig,
    progress_tx: Option<ExportProgressSender>,
) -> Result<ExportResult> {
    let start = Instant::now();
    let mut parts = Vec::new();
    let mut json_rows = Vec::new();
    let mut total_rows = 0u64;
    for (index, table) in config.tables.iter().enumerate() {
        send_progress(
            &progress_tx,
            ExportProgressEvent::TableStart {
                table: table.clone(),
                table_index: index,
                total_tables: config.tables.len(),
            },
        );
        send_progress(
            &progress_tx,
            ExportProgressEvent::FetchingData {
                table: table.clone(),
            },
        );
        let bytes = export_table_stream(connection, config, table).await?;
        let text = String::from_utf8(bytes)?;
        let rows = exported_row_count(config, &text);
        total_rows = total_rows.saturating_add(rows);
        collect_output(config, &text, &mut parts, &mut json_rows)?;
        send_progress(
            &progress_tx,
            ExportProgressEvent::DataExported {
                table: table.clone(),
                rows,
                data: text,
            },
        );
        send_progress(
            &progress_tx,
            ExportProgressEvent::TableFinished {
                table: table.clone(),
            },
        );
    }
    let output = output_text(config, parts, json_rows)?;
    let elapsed_ms = start.elapsed().as_millis();
    send_progress(
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

async fn export_table_stream(
    connection: &dyn DbConnection,
    config: &ExportConfig,
    table: &str,
) -> Result<Vec<u8>> {
    let stream_id = format!("onetcli-export-{}", uuid::Uuid::new_v4());
    connection
        .driver_request_value(
            wire_method::DATA_EXPORT,
            export_params(config, table, &stream_id),
        )
        .await?;
    let read_result = read_stream(connection, &stream_id).await;
    let close_result = close_stream(connection, &stream_id).await;
    match (read_result, close_result) {
        (Ok(bytes), Ok(())) => Ok(bytes),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

async fn read_stream(connection: &dyn DbConnection, stream_id: &str) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let value = connection
            .driver_request_value(
                wire_method::STREAM_READ,
                serde_json::json!({ "stream_id": stream_id, "max_bytes": EXPORT_READ_BYTES }),
            )
            .await?;
        let chunk: wire_data::StreamReadResult = serde_json::from_value(value)?;
        out.extend(base64::engine::general_purpose::STANDARD.decode(chunk.data.as_bytes())?);
        if chunk.done {
            return Ok(out);
        }
    }
}

async fn close_stream(connection: &dyn DbConnection, stream_id: &str) -> Result<()> {
    connection
        .driver_request_value(
            wire_method::STREAM_CLOSE,
            serde_json::json!({ "stream_id": stream_id }),
        )
        .await?;
    Ok(())
}

fn export_params(config: &ExportConfig, table: &str, stream_id: &str) -> Value {
    let (format, options) = wire_format_and_options(config);
    serde_json::json!({
        "table": table,
        "schema": config.schema,
        "database": empty_to_null(&config.database),
        "format": format,
        "where": config.where_clause,
        "include_columns": config.columns.clone().unwrap_or_default(),
        "max_rows": config.limit.map(|limit| limit as u64),
        "options": options,
        "stream_id": stream_id
    })
}

fn wire_format_and_options(config: &ExportConfig) -> (wire_data::DataFormat, Value) {
    match config.format {
        DataFormat::Json => (wire_data::DataFormat::Ndjson, Value::Null),
        DataFormat::Txt => {
            let options = config.csv_config.clone().unwrap_or_default();
            (
                wire_data::DataFormat::Csv,
                csv_options('\t', options.include_header, options.text_qualifier),
            )
        }
        DataFormat::Csv => {
            let options = config.csv_config.clone().unwrap_or_default();
            (
                wire_data::DataFormat::Csv,
                csv_options(
                    options.field_delimiter,
                    options.include_header,
                    options.text_qualifier,
                ),
            )
        }
        DataFormat::Sql | DataFormat::Xml => unreachable!("handled by fallback"),
    }
}

fn csv_options(delimiter: char, header: bool, quote: Option<char>) -> Value {
    serde_json::json!({
        "delimiter": delimiter.to_string(),
        "header": header,
        "quote": quote.unwrap_or('"').to_string(),
        "encoding": "utf-8"
    })
}

fn collect_output(
    config: &ExportConfig,
    text: &str,
    parts: &mut Vec<String>,
    json_rows: &mut Vec<Value>,
) -> Result<()> {
    match config.format {
        DataFormat::Json => {
            for line in text.lines().filter(|line| !line.trim().is_empty()) {
                json_rows.push(serde_json::from_str(line)?);
            }
        }
        DataFormat::Csv | DataFormat::Txt => parts.push(text.to_string()),
        DataFormat::Sql | DataFormat::Xml => unreachable!("handled by fallback"),
    }
    Ok(())
}

fn output_text(config: &ExportConfig, parts: Vec<String>, json_rows: Vec<Value>) -> Result<String> {
    match config.format {
        DataFormat::Json => serde_json::to_string_pretty(&json_rows).map_err(Into::into),
        DataFormat::Csv => Ok(parts.join("\n\n")),
        DataFormat::Txt => Ok(parts.join("")),
        DataFormat::Sql | DataFormat::Xml => unreachable!("handled by fallback"),
    }
}

fn exported_row_count(config: &ExportConfig, text: &str) -> u64 {
    match config.format {
        DataFormat::Json => text.lines().filter(|line| !line.trim().is_empty()).count() as u64,
        DataFormat::Csv | DataFormat::Txt => delimited_row_count(config, text),
        DataFormat::Sql | DataFormat::Xml => 0,
    }
}

fn delimited_row_count(config: &ExportConfig, text: &str) -> u64 {
    let lines = text.lines().filter(|line| !line.trim().is_empty()).count();
    let has_header = config
        .csv_config
        .as_ref()
        .map(|config| config.include_header)
        .unwrap_or(true);
    if has_header {
        lines.saturating_sub(1) as u64
    } else {
        lines as u64
    }
}

async fn fallback_export(
    plugin: &dyn DatabasePlugin,
    connection: &dyn DbConnection,
    config: &ExportConfig,
    progress_tx: Option<ExportProgressSender>,
) -> Result<ExportResult> {
    plugin::default_export_data_with_progress(plugin, connection, config, progress_tx).await
}

fn empty_to_null(value: &str) -> Value {
    if value.trim().is_empty() {
        Value::Null
    } else {
        Value::String(value.to_string())
    }
}

fn send_progress(progress_tx: &Option<ExportProgressSender>, event: ExportProgressEvent) {
    if let Some(tx) = progress_tx {
        let _ = tx.send(event);
    }
}

fn is_not_supported(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<DbError>()
        .is_some_and(|error| matches!(error, DbError::NotSupported(_)))
}
