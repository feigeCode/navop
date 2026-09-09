//! TDengine 连接实现(基于官方 taos 连接器的 WebSocket 通道,经 taosAdapter 6041 端口)。

use crate::connection::{DbConnection, DbError, StreamingProgress};
use crate::executor::{
    ExecOptions, ExecResult, QueryColumnMeta, QueryResult, SqlErrorInfo, SqlResult, SqlSource,
};
use crate::ssh_tunnel::resolve_connection_target;
use crate::{DatabasePlugin, format_message, truncate_str};

use async_trait::async_trait;
use connection_tunnel::TunnelGuard;
use one_core::storage::DbConnectionConfig;
use std::time::{Duration, Instant};
use taos::{AsyncFetchable, AsyncQueryable, AsyncTBuilder, Field, Value};
use taos::{Taos, TaosBuilder};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tracing::{debug, error, info};

/// TDengine 默认连接超时时间(秒)。
const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 30;

/// TDengine 数据库连接(无状态外壳 + 惰性建立的 WebSocket 客户端)。
pub struct TdengineDbConnection {
    config: DbConnectionConfig,
    taos: Option<Taos>,
    tunnel: Option<TunnelGuard>,
}

impl TdengineDbConnection {
    pub fn new(config: DbConnectionConfig) -> Self {
        Self {
            config,
            taos: None,
            tunnel: None,
        }
    }

    /// 取出已建立的客户端;未连接时返回 NotConnected 错误。
    fn ensure_connected(&self) -> Result<&Taos, DbError> {
        self.taos.as_ref().ok_or(DbError::NotConnected)
    }

    /// 执行单条语句:有结果集时转换为 Query,无结果集时转换为 Exec,失败转为 Error 结果。
    async fn execute_single(taos: &Taos, sql: &str) -> Result<SqlResult, DbError> {
        let start = Instant::now();
        let sql_preview = if sql.len() > 200 {
            format!("{}...", truncate_str(sql, 200))
        } else {
            sql.to_string()
        };
        debug!("[TDengine] Executing SQL: {}", sql_preview);

        let mut result_set = match taos.query(sql).await {
            Ok(result_set) => result_set,
            Err(e) => {
                error!("[TDengine] Execute failed: {}, SQL: {}", e, sql_preview);
                return Ok(SqlResult::Error(SqlErrorInfo {
                    sql: sql.to_string(),
                    message: e.to_string(),
                }));
            }
        };

        let elapsed_ms = start.elapsed().as_millis();

        // 无字段说明是非查询语句(DDL/DML),使用受影响行数构造执行结果。
        if result_set.fields().is_empty() {
            let rows_affected = result_set.affected_rows().max(0) as u64;
            debug!(
                "[TDengine] Execute completed: {} row(s) affected, {}ms",
                rows_affected, elapsed_ms
            );
            return Ok(SqlResult::Exec(ExecResult {
                sql: sql.to_string(),
                rows_affected,
                elapsed_ms,
                message: Some(format_message(sql, rows_affected)),
            }));
        }

        let columns: Vec<String> = result_set
            .fields()
            .iter()
            .map(|field| field.name().to_string())
            .collect();
        let column_meta: Vec<QueryColumnMeta> = result_set
            .fields()
            .iter()
            .map(|field| QueryColumnMeta::new(field.name(), field_type_name(field)))
            .collect();

        let records = match result_set.to_records().await {
            Ok(records) => records,
            Err(e) => {
                error!("[TDengine] Fetch rows failed: {}, SQL: {}", e, sql_preview);
                return Ok(SqlResult::Error(SqlErrorInfo {
                    sql: sql.to_string(),
                    message: e.to_string(),
                }));
            }
        };
        let rows: Vec<Vec<Option<String>>> = records
            .into_iter()
            .map(|record| record.into_iter().map(value_to_cell).collect())
            .collect();

        debug!(
            "[TDengine] Query completed: {} rows, {} columns, {}ms",
            rows.len(),
            columns.len(),
            elapsed_ms
        );

        Ok(SqlResult::Query(QueryResult {
            sql: sql.to_string(),
            columns,
            column_meta,
            rows,
            binary_cells: vec![],
            elapsed_ms,
        }))
    }
}

/// 生成字段类型文本;变长类型附带字节宽度,例如 `BINARY(16)`、`NCHAR(8)`。
fn field_type_name(field: &Field) -> String {
    let ty = field.ty();
    if ty.is_var_type() && field.bytes() > 0 {
        format!("{}({})", ty.name(), field.bytes())
    } else {
        ty.name().to_string()
    }
}

/// 将 taos 值转换为表格单元:NULL 转 None,其余转显示字符串。
fn value_to_cell(value: Value) -> Option<String> {
    match value {
        Value::Null(_) => None,
        // taos 的 to_string 对二进制类值可能失败,回退到 Display 形式。
        other => Some(other.to_string().unwrap_or_else(|_| format!("{other}"))),
    }
}

/// 配置中的默认数据库(去空白,空串视为未指定)。
fn configured_database(config: &DbConnectionConfig) -> Option<String> {
    config
        .database
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// 组装 taos DSN:`ws://user:pass@host:port[/database]`。
///
/// - scheme 由 `schema` 参数决定,默认 `ws`(经 taosAdapter 的 WebSocket 通道),可选 `wss`;
/// - 用户名/密码/库名按百分号编码,避免特殊字符破坏 DSN 解析;
/// - 用户名或密码为空时省略,交由驱动侧默认值(root / taosdata)。
pub(crate) fn build_dsn(config: &DbConnectionConfig, host: &str, port: u16) -> String {
    // scheme 参数支持 ws/wss(以及 http/https 别名),默认 ws。
    let scheme = match config
        .get_param("schema")
        .map(|value| value.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("http") | Some("ws") => "ws",
        Some("https") | Some("wss") => "wss",
        _ => "ws",
    };

    let mut dsn = format!("{}://", scheme);

    let username = config.username.trim();
    if !username.is_empty() {
        dsn.push_str(&percent_encode_dsn_component(username));
        if !config.password.is_empty() {
            dsn.push(':');
            dsn.push_str(&percent_encode_dsn_component(&config.password));
        }
        dsn.push('@');
    }

    dsn.push_str(host);
    dsn.push(':');
    dsn.push_str(&port.to_string());

    if let Some(database) = configured_database(config) {
        dsn.push('/');
        dsn.push_str(&percent_encode_dsn_component(&database));
    }

    dsn
}

/// 对 DSN 组件做百分号编码(仅保留 URI 非保留字符)。
fn percent_encode_dsn_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(*byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

#[async_trait]
impl DbConnection for TdengineDbConnection {
    fn config(&self) -> &DbConnectionConfig {
        &self.config
    }

    fn set_config_database(&mut self, database: Option<String>) {
        self.config.database = database;
    }

    async fn connect(&mut self) -> Result<(), DbError> {
        let config = &self.config;
        info!("[TDengine] Connecting to {}:{}", config.host, config.port);

        // 先解析 SSH 隧道/代理,得到实际可达的地址。
        let target = resolve_connection_target(config).await?;
        self.tunnel = target.tunnel;

        let dsn = build_dsn(config, &target.host, target.port);
        debug!("[TDengine] DSN: {}", dsn);

        let builder = TaosBuilder::from_dsn(&dsn)
            .map_err(|e| DbError::connection_with_source("invalid TDengine DSN", e))?;
        let taos = AsyncTBuilder::build(&builder)
            .await
            .map_err(|e| DbError::connection_with_source("failed to create TDengine builder", e))?;

        // WebSocket 客户端是惰性建立的,这里用测试查询在超时内完成实际握手。
        let connect_timeout_secs = config
            .get_param_as::<u64>("connect_timeout")
            .unwrap_or(DEFAULT_CONNECT_TIMEOUT_SECS);
        debug!(
            "[TDengine] Testing connection with timeout {}s...",
            connect_timeout_secs
        );

        let test_result = timeout(
            Duration::from_secs(connect_timeout_secs),
            taos.query("SELECT SERVER_STATUS()"),
        )
        .await;

        match test_result {
            Ok(Ok(mut result_set)) => {
                // 拉取并丢弃测试结果,确保结果集被完整消费。
                let _ = result_set.to_records().await;
            }
            Ok(Err(e)) => {
                error!("[TDengine] Connection failed: {}", e);
                return Err(DbError::connection_with_source("failed to connect", e));
            }
            Err(_) => {
                error!(
                    "[TDengine] Connection timed out after {}s",
                    connect_timeout_secs
                );
                return Err(DbError::connection(format!(
                    "connection timed out after {}s",
                    connect_timeout_secs
                )));
            }
        }

        self.taos = Some(taos);
        info!("[TDengine] Connected successfully");
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), DbError> {
        debug!("[TDengine] Disconnecting...");
        self.taos = None;
        self.tunnel = None;
        info!("[TDengine] Disconnected");
        Ok(())
    }

    async fn execute(
        &self,
        plugin: &dyn DatabasePlugin,
        script: &str,
        options: ExecOptions,
    ) -> Result<Vec<SqlResult>, DbError> {
        debug!(
            "[TDengine] execute() called, stop_on_error={}",
            options.stop_on_error
        );
        let taos = self.ensure_connected()?;

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
        debug!("[TDengine] Split into {} statement(s)", statements.len());

        let mut results = Vec::new();
        for (idx, sql) in statements.iter().enumerate() {
            debug!(
                "[TDengine] Executing statement {}/{}",
                idx + 1,
                statements.len()
            );
            let sql_to_execute = plugin.apply_query_max_rows(sql, options.max_rows);
            let result = Self::execute_single(taos, sql_to_execute.as_ref()).await?;
            let is_error = result.is_error();
            results.push(result.with_original_sql(sql));

            if is_error && options.stop_on_error {
                debug!("[TDengine] Stopping execution due to error (stop_on_error=true)");
                break;
            }
        }

        debug!(
            "[TDengine] execute() completed with {} result(s)",
            results.len()
        );
        Ok(results)
    }

    async fn query(&self, query: &str) -> Result<SqlResult, DbError> {
        debug!("[TDengine] query() called");
        let taos = self.ensure_connected()?;
        Self::execute_single(taos, query).await
    }

    fn ping_query(&self) -> &'static str {
        "SELECT SERVER_STATUS()"
    }

    async fn current_database(&self) -> Result<Option<String>, DbError> {
        debug!("[TDengine] Querying current database");
        let taos = self.ensure_connected()?;
        let result = Self::execute_single(taos, "SELECT DATABASE()").await;
        match result {
            Ok(SqlResult::Query(query_result)) => {
                let name = query_result
                    .rows
                    .first()
                    .and_then(|row| row.first())
                    .and_then(|value| value.clone());
                debug!("[TDengine] Current database: {:?}", name);
                Ok(name)
            }
            Ok(SqlResult::Error(error_info)) => {
                error!(
                    "[TDengine] Failed to query current database: {}",
                    error_info.message
                );
                Err(DbError::query(format!(
                    "failed to query current database: {}",
                    error_info.message
                )))
            }
            Ok(other) => Err(DbError::query(format!(
                "unexpected result when querying current database: {other:?}"
            ))),
            Err(e) => {
                error!("[TDengine] Failed to query current database: {}", e);
                Err(e)
            }
        }
    }

    async fn switch_database(&self, database: &str) -> Result<(), DbError> {
        debug!("[TDengine] Switching to database: {}", database);
        let taos = self.ensure_connected()?;

        let sql = format!("USE `{}`", database.replace('`', "``"));
        taos.query(&sql).await.map_err(|e| {
            error!("[TDengine] Failed to switch database: {}, SQL: {}", e, sql);
            DbError::query_with_source("failed to switch database", e)
        })?;

        info!("[TDengine] Switched to database: {}", database);
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
            "[TDengine] execute_streaming() called, streaming={}",
            options.streaming
        );
        let taos = self.ensure_connected()?;

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
                debug!("[TDengine] Streaming statement {}", current);

                let sql_to_execute = plugin.apply_query_max_rows(&sql, options.max_rows);
                let result = match Self::execute_single(taos, sql_to_execute.as_ref()).await {
                    Ok(result) => result,
                    Err(e) => {
                        error!(
                            "[TDengine] Streaming statement {} failed: {}, SQL: {}",
                            current, e, sql
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
            debug!("[TDengine] Streaming {} statement(s)", total);

            for (index, sql) in statements.into_iter().enumerate() {
                let current = index + 1;
                debug!("[TDengine] Streaming statement {}/{}", current, total);

                let sql_to_execute = plugin.apply_query_max_rows(&sql, options.max_rows);
                let result = match Self::execute_single(taos, sql_to_execute.as_ref()).await {
                    Ok(result) => result,
                    Err(e) => {
                        error!(
                            "[TDengine] Streaming statement {}/{} failed: {}, SQL: {}",
                            current, total, e, sql
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

        debug!("[TDengine] execute_streaming() completed");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn build_config(extra_params: HashMap<String, String>) -> DbConnectionConfig {
        DbConnectionConfig {
            id: "test".to_string(),
            database_type: one_core::storage::DatabaseType::TDengine,
            name: "test".to_string(),
            host: "127.0.0.1".to_string(),
            port: 6041,
            username: "root".to_string(),
            password: "taosdata".to_string(),
            database: None,
            service_name: None,
            sid: None,
            workspace_id: None,
            proxy: None,
            credential_reference: None,
            extra_params,
        }
    }

    #[test]
    fn dsn_uses_ws_scheme_and_credentials_by_default() {
        let config = build_config(HashMap::new());
        assert_eq!(
            build_dsn(&config, "localhost", 6041),
            "ws://root:taosdata@localhost:6041"
        );
    }

    #[test]
    fn dsn_appends_database_when_present() {
        let mut config = build_config(HashMap::new());
        config.database = Some(" power_db ".to_string());
        assert_eq!(
            build_dsn(&config, "db.internal", 6041),
            "ws://root:taosdata@db.internal:6041/power_db"
        );
    }

    #[test]
    fn dsn_supports_wss_scheme_param() {
        let mut extra = HashMap::new();
        extra.insert("schema".to_string(), "wss".to_string());
        let config = build_config(extra);
        assert_eq!(
            build_dsn(&config, "cloud.tdengine.com", 443),
            "wss://root:taosdata@cloud.tdengine.com:443"
        );
    }

    #[test]
    fn dsn_omits_userinfo_when_username_empty() {
        let mut config = build_config(HashMap::new());
        config.username = String::new();
        // 用户名省略后由驱动侧使用默认 root/taosdata。
        assert_eq!(build_dsn(&config, "localhost", 6041), "ws://localhost:6041");
    }

    #[test]
    fn dsn_percent_encodes_special_characters() {
        let mut config = build_config(HashMap::new());
        config.password = "p@ss/w:rd".to_string();
        assert_eq!(
            build_dsn(&config, "localhost", 6041),
            "ws://root:p%40ss%2Fw%3Ard@localhost:6041"
        );
    }

    #[test]
    fn null_value_maps_to_none_and_scalars_map_to_text() {
        use taos::Ty;
        assert_eq!(value_to_cell(Value::Null(Ty::Int)), None);
        assert_eq!(value_to_cell(Value::Int(42)), Some("42".to_string()));
        assert_eq!(value_to_cell(Value::Bool(true)), Some("true".to_string()));
        assert_eq!(
            value_to_cell(Value::NChar("涛思".to_string())),
            Some("涛思".to_string())
        );
    }
}
