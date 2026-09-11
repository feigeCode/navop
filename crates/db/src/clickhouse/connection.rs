use crate::connection::{DbConnection, DbError, StreamingProgress};
use crate::executor::{
    ExecOptions, ExecResult, QueryColumnMeta, QueryResult, SqlErrorInfo, SqlResult, SqlSource,
};
use crate::ssh_tunnel::resolve_connection_target;
use crate::types::FieldType;
use crate::{DatabasePlugin, format_message, truncate_str};

use super::codec;
use async_trait::async_trait;
use clickhouse::Client;
use connection_tunnel::TunnelGuard;
use db_value::{CellState, ColumnDescriptor, DbValue, Nullability, ResultBatch, ResultRow};
use one_core::storage::DbConnectionConfig;
use serde::Deserialize;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio::time::timeout;
use tracing::{debug, error, info};

pub struct ClickHouseDbConnection {
    config: DbConnectionConfig,
    client: Option<Client>,
    tunnel: Option<TunnelGuard>,
}

impl ClickHouseDbConnection {
    pub fn new(config: DbConnectionConfig) -> Self {
        Self {
            config,
            client: None,
            tunnel: None,
        }
    }

    fn ensure_connected(&self) -> Result<&Client, DbError> {
        self.client.as_ref().ok_or(DbError::NotConnected)
    }

    fn build_exec_result(sql: String, rows_affected: u64, elapsed_ms: u128) -> SqlResult {
        let message = format_message(&sql, rows_affected);
        SqlResult::Exec(ExecResult {
            sql,
            rows_affected,
            elapsed_ms,
            message: Some(message),
        })
    }

    /// 把 JSONCompact 结果组装为带类型的 [`ResultBatch`],再经
    /// [`QueryResult::from_typed_batch`] 携带 `typed_batch`。
    ///
    /// `typed_batch` 是类型权威;`rows`/`column_meta` 仍是现有 legacy 兼容投影,
    /// 由本函数显式覆盖为原有行为,避免与既有消费者漂移。JSONCompact 无法无损承载
    /// 任意二进制 `String` 字节,`codec` 对此保守处理(见 `clickhouse::codec` 模块说明)。
    fn build_typed_query_result(
        meta: &[ClickHouseJsonMeta],
        data: &[Vec<serde_json::Value>],
        sql: String,
        elapsed_ms: u128,
    ) -> Result<SqlResult, DbError> {
        let columns = Self::column_descriptors(meta);
        let rows = Self::typed_rows(&columns, data);
        let batch = ResultBatch::try_new(0, columns, rows, true).map_err(|error| {
            DbError::query(format!("failed to build typed result batch: {error}"))
        })?;
        let mut query_result =
            QueryResult::from_typed_batch(sql, batch, elapsed_ms).map_err(|error| {
                DbError::query(format!("failed to build typed query result: {error}"))
            })?;

        let column_meta: Vec<QueryColumnMeta> = meta
            .iter()
            .map(|meta| QueryColumnMeta::new(meta.name.clone(), meta.data_type.clone()))
            .collect();
        query_result.rows = Self::map_json_rows(&column_meta, data);
        query_result.column_meta = column_meta;
        query_result.binary_cells = Vec::new();
        Ok(SqlResult::Query(query_result))
    }

    fn column_descriptors(meta: &[ClickHouseJsonMeta]) -> Vec<ColumnDescriptor> {
        meta.iter()
            .enumerate()
            .map(|(index, column)| ColumnDescriptor {
                id: format!("column:{index}"),
                label: column.name.clone(),
                native_type: column.data_type.clone(),
                logical_type: format!("{:?}", FieldType::from_db_type(&column.data_type)),
                nullable: if codec::is_nullable(&column.data_type) {
                    Nullability::Yes
                } else {
                    Nullability::Unknown
                },
                charset: None,
                collation: None,
                precision: None,
                scale: None,
            })
            .collect()
    }

    fn typed_rows(columns: &[ColumnDescriptor], data: &[Vec<serde_json::Value>]) -> Vec<ResultRow> {
        data.iter()
            .enumerate()
            .map(|(row_index, row)| ResultRow {
                id: row_index as u64,
                cells: columns
                    .iter()
                    .enumerate()
                    .map(|(column_index, column)| match row.get(column_index) {
                        Some(value) => codec::decode_cell(&column.native_type, value),
                        None => CellState::Decoded(DbValue::Null),
                    })
                    .collect(),
            })
            .collect()
    }

    async fn execute_single(client: &Client, sql: &str) -> Result<SqlResult, DbError> {
        let start = Instant::now();
        let sql_string = sql.to_string();
        let sql_preview = if sql.len() > 200 {
            format!("{}...", truncate_str(&sql, 200))
        } else {
            sql.to_string()
        };
        debug!("[ClickHouse] Executing SQL: {}", sql_preview);

        match Self::fetch_json_compact(client, sql).await {
            Ok(result) => {
                let elapsed_ms = start.elapsed().as_millis();
                debug!(
                    "[ClickHouse] Query completed: {} rows, {} columns, {}ms",
                    result.data.len(),
                    result.meta.len(),
                    elapsed_ms
                );
                Self::build_typed_query_result(&result.meta, &result.data, sql_string, elapsed_ms)
            }
            Err(query_err) => {
                debug!(
                    "[ClickHouse] Query fetch failed, trying execute: {}",
                    query_err
                );
                match client.query(sql).execute().await {
                    Ok(_) => {
                        let elapsed_ms = start.elapsed().as_millis();
                        debug!("[ClickHouse] Execute completed: {}ms", elapsed_ms);
                        Ok(Self::build_exec_result(sql_string, 0, elapsed_ms))
                    }
                    Err(e) => {
                        error!("[ClickHouse] Execute failed: {}, SQL: {}", e, sql_preview);
                        Ok(SqlResult::Error(SqlErrorInfo {
                            sql: sql_string,
                            message: e.to_string(),
                        }))
                    }
                }
            }
        }
    }

    async fn fetch_json_compact(
        client: &Client,
        sql: &str,
    ) -> Result<ClickHouseJsonCompactResult, DbError> {
        let mut cursor = client
            .query(sql)
            .fetch_bytes("JSONCompact")
            .map_err(|e| DbError::query_with_source("failed to fetch query bytes", e))?;
        let bytes = cursor
            .collect()
            .await
            .map_err(|e| DbError::query_with_source("failed to read query bytes", e))?;
        serde_json::from_slice(&bytes)
            .map_err(|e| DbError::query_with_source("failed to parse JSON response", e))
    }

    fn map_json_rows(
        columns: &[QueryColumnMeta],
        data: &[Vec<serde_json::Value>],
    ) -> Vec<Vec<Option<String>>> {
        data.iter()
            .map(|row| {
                let mut values = Vec::with_capacity(columns.len());
                for index in 0..columns.len() {
                    let value = row
                        .get(index)
                        .and_then(|value| Self::json_value_to_string(value))
                        .map(|value| {
                            if Self::is_fixed_string_type(&columns[index].db_type) {
                                value.trim_end_matches('\0').to_string()
                            } else {
                                value
                            }
                        });
                    values.push(value);
                }
                values
            })
            .collect()
    }

    fn json_value_to_string(value: &serde_json::Value) -> Option<String> {
        match value {
            serde_json::Value::Null => None,
            serde_json::Value::Bool(_) => Some(value.to_string()),
            serde_json::Value::Number(_) => Some(value.to_string()),
            serde_json::Value::String(text) => Some(text.clone()),
            serde_json::Value::Array(_) => Some(value.to_string()),
            serde_json::Value::Object(_) => Some(value.to_string()),
        }
    }

    fn is_fixed_string_type(data_type: &str) -> bool {
        let mut data_type = data_type.trim();
        loop {
            let Some(open) = data_type.find('(') else {
                return false;
            };
            let wrapper = data_type[..open].trim();
            if wrapper.eq_ignore_ascii_case("FixedString") {
                return true;
            }
            if !matches!(
                wrapper.to_ascii_uppercase().as_str(),
                "NULLABLE" | "LOWCARDINALITY"
            ) || !data_type.ends_with(')')
            {
                return false;
            }
            data_type = data_type[open + 1..data_type.len() - 1].trim();
        }
    }

    fn configured_database(config: &DbConnectionConfig) -> Option<String> {
        config
            .database
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }
}

#[derive(Debug, Deserialize)]
struct ClickHouseJsonMeta {
    name: String,
    #[serde(rename = "type")]
    data_type: String,
}

#[derive(Debug, Deserialize)]
struct ClickHouseJsonCompactResult {
    #[serde(default)]
    meta: Vec<ClickHouseJsonMeta>,
    #[serde(default)]
    data: Vec<Vec<serde_json::Value>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_string_padding_is_removed_from_display_and_edit_value() {
        let columns = vec![QueryColumnMeta::new("code", "FixedString(8)")];
        let rows = ClickHouseDbConnection::map_json_rows(
            &columns,
            &[vec![serde_json::Value::String("abc\0\0\0\0\0".to_string())]],
        );

        assert_eq!(rows, vec![vec![Some("abc".to_string())]]);
    }

    #[test]
    fn wrapped_fixed_string_padding_is_removed_but_regular_string_nul_is_preserved() {
        assert!(ClickHouseDbConnection::is_fixed_string_type(
            "Nullable(LowCardinality(FixedString(8)))"
        ));

        let columns = vec![
            QueryColumnMeta::new("fixed", "LowCardinality(FixedString(4))"),
            QueryColumnMeta::new("regular", "String"),
        ];
        let rows = ClickHouseDbConnection::map_json_rows(
            &columns,
            &[vec![
                serde_json::Value::String("a\0\0\0".to_string()),
                serde_json::Value::String("a\0".to_string()),
            ]],
        );

        assert_eq!(rows[0][0].as_deref(), Some("a"));
        assert_eq!(rows[0][1].as_deref(), Some("a\0"));
    }

    fn meta(name: &str, data_type: &str) -> ClickHouseJsonMeta {
        ClickHouseJsonMeta {
            name: name.to_string(),
            data_type: data_type.to_string(),
        }
    }

    #[test]
    fn typed_result_carries_batch_and_preserves_legacy_projection() {
        let meta = vec![
            meta("id", "UInt64"),
            meta("amount", "Decimal(18, 4)"),
            meta("name", "String"),
            meta("fixed", "FixedString(4)"),
            meta("seen_at", "Nullable(DateTime64(3))"),
        ];
        let data = vec![vec![
            serde_json::Value::String("18446744073709551615".to_string()),
            serde_json::Value::String("12.3400".to_string()),
            serde_json::Value::String("héllo".to_string()),
            serde_json::Value::String("a\0\0\0".to_string()),
            serde_json::Value::Null,
        ]];

        let result =
            ClickHouseDbConnection::build_typed_query_result(&meta, &data, "select".into(), 3)
                .unwrap();
        let SqlResult::Query(query) = result else {
            panic!("expected query result");
        };

        let batch = query.typed_batch().expect("typed batch is carried");
        assert_eq!(batch.columns.len(), 5);
        assert_eq!(
            batch.rows[0].cells,
            vec![
                CellState::Decoded(DbValue::Unsigned("18446744073709551615".into())),
                CellState::Decoded(DbValue::Decimal("12.3400".into())),
                CellState::Decoded(DbValue::LegacyText("héllo".into())),
                CellState::Decoded(DbValue::LegacyText("a".into())),
                CellState::Decoded(DbValue::Null),
            ]
        );

        // legacy rows 保持现有投影:FixString 去尾 NUL、String 原样、NULL 为 None。
        assert_eq!(
            query.rows,
            vec![vec![
                Some("18446744073709551615".to_string()),
                Some("12.3400".to_string()),
                Some("héllo".to_string()),
                Some("a".to_string()),
                None,
            ]]
        );
        assert!(query.binary_cells.is_empty());
        assert_eq!(query.column_meta[1].db_type, "Decimal(18, 4)");
    }
}

#[async_trait]
impl DbConnection for ClickHouseDbConnection {
    fn config(&self) -> &DbConnectionConfig {
        &self.config
    }

    fn set_config_database(&mut self, database: Option<String>) {
        self.config.database = database;
    }

    async fn connect(&mut self) -> Result<(), DbError> {
        let config = &self.config;
        info!("[ClickHouse] Connecting to {}:{}", config.host, config.port);
        let target = resolve_connection_target(config).await?;
        self.tunnel = target.tunnel;

        let protocol = config
            .get_param("schema")
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| matches!(value.as_str(), "http" | "https"))
            .unwrap_or_else(|| "http".to_string());

        let url = format!("{}://{}:{}", protocol, target.host, target.port);

        let mut client = Client::default()
            .with_url(&url)
            .with_user(&config.username)
            .with_password(&config.password);

        if let Some(db) = Self::configured_database(config) {
            client = client.with_database(&db);
            debug!("[ClickHouse] Using database: {}", db);
        }

        if let Some(compression) = config.get_param("compression") {
            if compression == "lz4" {
                client = client.with_compression(clickhouse::Compression::Lz4);
                debug!("[ClickHouse] Using LZ4 compression");
            }
        }

        // 获取连接超时，默认 30 秒
        let connect_timeout_secs = config.get_param_as::<u64>("connect_timeout").unwrap_or(30);
        debug!(
            "[ClickHouse] Testing connection with timeout {}s...",
            connect_timeout_secs
        );

        // 使用 tokio::timeout 包装连接测试
        let test_result = timeout(
            Duration::from_secs(connect_timeout_secs),
            client.query("SELECT 1").fetch_all::<u8>(),
        )
        .await;

        match test_result {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                error!("[ClickHouse] Connection failed: {}", e);
                return Err(DbError::connection_with_source("failed to connect", e));
            }
            Err(_) => {
                error!(
                    "[ClickHouse] Connection timed out after {}s",
                    connect_timeout_secs
                );
                return Err(DbError::connection(format!(
                    "connection timed out after {}s",
                    connect_timeout_secs
                )));
            }
        }

        self.client = Some(client);
        info!("[ClickHouse] Connected successfully");

        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), DbError> {
        debug!("[ClickHouse] Disconnecting...");
        self.client = None;
        self.tunnel = None;
        info!("[ClickHouse] Disconnected");
        Ok(())
    }

    async fn execute(
        &self,
        plugin: &dyn DatabasePlugin,
        script: &str,
        options: ExecOptions,
    ) -> Result<Vec<SqlResult>, DbError> {
        debug!(
            "[ClickHouse] execute() called, stop_on_error={}",
            options.stop_on_error
        );
        let client = self.ensure_connected()?;

        let parser = plugin
            .create_parser(SqlSource::Script(script.to_string()))
            .map_err(|e| DbError::query(format!("Failed to create parser: {}", e)))?;
        let statements: Vec<String> = parser
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(|e| DbError::query_with_source("failed to parse SQL script", e))?
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        debug!("[ClickHouse] Split into {} statement(s)", statements.len());
        let mut results = Vec::new();

        for (idx, sql) in statements.iter().enumerate() {
            let sql = sql.trim();
            if sql.is_empty() {
                continue;
            }

            debug!(
                "[ClickHouse] Executing statement {}/{}",
                idx + 1,
                statements.len()
            );
            let sql_to_execute = plugin.apply_query_max_rows(sql, options.max_rows);
            let result = Self::execute_single(client, sql_to_execute.as_ref()).await?;

            let is_error = result.is_error();
            if is_error {
                debug!(
                    "[ClickHouse] Statement {}/{} returned error",
                    idx + 1,
                    statements.len()
                );
            }
            results.push(result.with_original_sql(sql));

            if is_error && options.stop_on_error {
                debug!("[ClickHouse] Stopping execution due to error (stop_on_error=true)");
                break;
            }
        }

        debug!(
            "[ClickHouse] execute() completed with {} result(s)",
            results.len()
        );
        Ok(results)
    }

    async fn query(&self, query: &str) -> Result<SqlResult, DbError> {
        debug!("[ClickHouse] query() called");
        let client = self.ensure_connected()?;

        Self::execute_single(client, query).await
    }

    async fn current_database(&self) -> Result<Option<String>, DbError> {
        debug!("[ClickHouse] Querying current database");
        let client = self.ensure_connected()?;
        let result = Self::execute_single(client, "SELECT currentDatabase() as name").await;
        match result {
            Ok(SqlResult::Query(query_result)) => {
                if let Some(row) = query_result.rows.first() {
                    if let Some(Some(name)) = row.first() {
                        debug!("[ClickHouse] Current database: {}", name);
                        return Ok(Some(name.clone()));
                    }
                }
                Err(DbError::query(
                    "current database query returned no database name",
                ))
            }
            Ok(SqlResult::Error(error_info)) => {
                error!(
                    "[ClickHouse] Failed to query current database: {}",
                    error_info.message
                );
                Err(DbError::query(format!(
                    "failed to query current database: {}",
                    error_info.message
                )))
            }
            Ok(other) => {
                error!(
                    "[ClickHouse] Unexpected result when querying current database: {:?}",
                    other
                );
                Err(DbError::query(format!(
                    "unexpected result when querying current database: {other:?}"
                )))
            }
            Err(e) => {
                error!("[ClickHouse] Failed to query current database: {}", e);
                Err(e)
            }
        }
    }

    async fn switch_database(&self, database: &str) -> Result<(), DbError> {
        debug!("[ClickHouse] Switching to database: {}", database);
        let client = self.ensure_connected()?;

        let sql = format!("USE `{}`", database.replace("`", "``"));
        debug!("[ClickHouse] Executing: {}", sql);
        client.query(&sql).execute().await.map_err(|e| {
            error!(
                "[ClickHouse] Failed to switch database: {}, SQL: {}",
                e, sql
            );
            DbError::query_with_source("failed to switch database", e)
        })?;

        info!("[ClickHouse] Switched to database: {}", database);
        Ok(())
    }

    async fn execute_streaming(
        &self,
        plugin: &dyn DatabasePlugin,
        source: SqlSource,
        options: ExecOptions,
        sender: mpsc::Sender<StreamingProgress>,
    ) -> Result<(), DbError> {
        debug!(
            "[ClickHouse] execute_streaming() called, streaming={}",
            options.streaming
        );
        let client = self.ensure_connected()?;

        let total_size = source.file_size().unwrap_or(0);
        let is_file_source = source.is_file();

        let mut parser = plugin
            .create_parser(source)
            .map_err(|e| DbError::query(format!("Failed to create parser: {}", e)))?;

        if options.streaming || is_file_source {
            let mut current = 0usize;

            while let Some(stmt_result) = parser.next() {
                let bytes_read = parser.bytes_read();
                let sql = match stmt_result {
                    Ok(s) if !s.trim().is_empty() => s,
                    Ok(_) => continue,
                    Err(e) => {
                        let progress = StreamingProgress::with_file_progress(
                            current,
                            SqlResult::Error(SqlErrorInfo {
                                sql: String::new(),
                                message: format!("Parse error: {}", e),
                            }),
                            bytes_read,
                            total_size,
                        );
                        let _ = sender.send(progress).await;
                        if options.stop_on_error {
                            break;
                        }
                        continue;
                    }
                };

                current += 1;
                debug!("[ClickHouse] Streaming statement {}", current);

                let sql_to_execute = plugin.apply_query_max_rows(&sql, options.max_rows);
                let result = match Self::execute_single(client, sql_to_execute.as_ref()).await {
                    Ok(r) => r,
                    Err(e) => {
                        let sql_preview = if sql.len() > 200 {
                            format!("{}...", truncate_str(&sql, 200))
                        } else {
                            sql.clone()
                        };
                        error!(
                            "[ClickHouse] Streaming statement {} failed: {}, SQL: {}",
                            current, e, sql_preview
                        );
                        SqlResult::Error(SqlErrorInfo {
                            sql: sql.clone(),
                            message: e.to_string(),
                        })
                    }
                };

                let result = result.with_original_sql(sql.as_str());
                let is_error = result.is_error();
                let progress =
                    StreamingProgress::with_file_progress(current, result, bytes_read, total_size);
                if sender.send(progress).await.is_err() {
                    break;
                }

                if is_error && options.stop_on_error {
                    break;
                }
            }
        } else {
            let statements: Vec<String> = parser
                .collect::<std::io::Result<Vec<_>>>()
                .map_err(|e| DbError::query_with_source("failed to parse SQL script", e))?
                .into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();

            let total = statements.len();
            debug!("[ClickHouse] Streaming {} statement(s)", total);

            for (index, sql) in statements.into_iter().enumerate() {
                let current = index + 1;
                debug!("[ClickHouse] Streaming statement {}/{}", current, total);

                let sql_to_execute = plugin.apply_query_max_rows(&sql, options.max_rows);
                let result = match Self::execute_single(client, sql_to_execute.as_ref()).await {
                    Ok(r) => r,
                    Err(e) => {
                        let sql_preview = if sql.len() > 200 {
                            format!("{}...", truncate_str(&sql, 200))
                        } else {
                            sql.clone()
                        };
                        error!(
                            "[ClickHouse] Streaming statement {}/{} failed: {}, SQL: {}",
                            current, total, e, sql_preview
                        );
                        SqlResult::Error(SqlErrorInfo {
                            sql: sql.clone(),
                            message: e.to_string(),
                        })
                    }
                };

                let result = result.with_original_sql(sql.as_str());
                let is_error = result.is_error();
                let progress = StreamingProgress::new(current, total, result);
                if sender.send(progress).await.is_err() {
                    break;
                }

                if is_error && options.stop_on_error {
                    break;
                }
            }
        }

        debug!("[ClickHouse] execute_streaming() completed");
        Ok(())
    }
}
