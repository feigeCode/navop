use crate::connection::{DbConnection, DbError, StreamingProgress};
use crate::executor::{ExecOptions, QueryColumnMeta, QueryResult, SqlResult, SqlSource};
use crate::ipc::client::JsonRpcClient;
use crate::ipc::protocol::{
    connection_config_params, database_params, empty_params, schema_params, sql_params,
};
use crate::ipc::registry::IpcDriverManifest;
use crate::{DatabasePlugin, SqlErrorInfo, truncate_str};
use async_trait::async_trait;
use one_core::storage::DbConnectionConfig;
use tokio::sync::{Mutex, mpsc};
use tracing::{debug, error};

pub struct ExternalDbConnection {
    config: DbConnectionConfig,
    driver: IpcDriverManifest,
    client: Mutex<Option<JsonRpcClient>>,
}

impl ExternalDbConnection {
    pub fn new(config: DbConnectionConfig, driver: IpcDriverManifest) -> Self {
        Self {
            config,
            driver,
            client: Mutex::new(None),
        }
    }

    async fn request<T>(&self, method: &str, params: serde_json::Value) -> Result<T, DbError>
    where
        T: serde::de::DeserializeOwned,
    {
        let mut guard = self.client.lock().await;
        let client = guard.as_mut().ok_or(DbError::NotConnected)?;
        client.request(method, params).await
    }
}

fn metadata_result(sql: &str, value: serde_json::Value) -> SqlResult {
    SqlResult::Query(QueryResult {
        sql: sql.to_string(),
        columns: vec!["json".to_string()],
        column_meta: vec![QueryColumnMeta::new("json", "JSON")],
        rows: vec![vec![Some(value.to_string())]],
        elapsed_ms: 0,
    })
}

#[async_trait]
impl DbConnection for ExternalDbConnection {
    fn config(&self) -> &DbConnectionConfig {
        &self.config
    }

    fn set_config_database(&mut self, database: Option<String>) {
        self.config.database = database;
    }

    async fn connect(&mut self) -> Result<(), DbError> {
        let mut client = JsonRpcClient::start(&self.driver).await?;
        let _: serde_json::Value = client.request("initialize", empty_params()).await?;
        let _: serde_json::Value = client
            .request("connect", connection_config_params(&self.config))
            .await?;
        *self.client.lock().await = Some(client);
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), DbError> {
        let mut client = self.client.lock().await.take();
        if let Some(client) = client.as_mut() {
            let _: Result<serde_json::Value, DbError> =
                client.request("disconnect", empty_params()).await;
            client.shutdown().await;
        }
        Ok(())
    }

    async fn execute(
        &self,
        plugin: &dyn DatabasePlugin,
        script: &str,
        options: ExecOptions,
    ) -> Result<Vec<SqlResult>, DbError> {
        let statements = plugin.split_sql_statements(script);
        let mut results = Vec::with_capacity(statements.len());
        for statement in statements {
            let result = self.query(&statement).await?;
            let should_stop = options.stop_on_error && result.is_error();
            results.push(result);
            if should_stop {
                break;
            }
        }
        Ok(results)
    }

    async fn query(&self, query: &str) -> Result<SqlResult, DbError> {
        if let Some(request) = query.strip_prefix("/*onetcli-ipc-metadata*/ ") {
            let value: serde_json::Value = serde_json::from_str(request)
                .map_err(|error| DbError::query_with_source("invalid metadata request", error))?;
            let method = value
                .get("method")
                .and_then(|value| value.as_str())
                .ok_or_else(|| DbError::query("metadata request method is required"))?;
            let params = value
                .get("params")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            let result: serde_json::Value = self.request(method, params).await?;
            return Ok(metadata_result(query, result));
        }

        self.request("query", sql_params(query)).await
    }

    async fn ping(&self) -> Result<(), DbError> {
        let _: serde_json::Value = self.request("ping", empty_params()).await?;
        Ok(())
    }

    async fn current_database(&self) -> Result<Option<String>, DbError> {
        self.request("current_database", empty_params()).await
    }

    async fn switch_database(&self, database: &str) -> Result<(), DbError> {
        let _: serde_json::Value = self
            .request("switch_database", database_params(database))
            .await?;
        Ok(())
    }

    async fn switch_schema(&self, schema: &str) -> Result<(), DbError> {
        let _: serde_json::Value = self.request("switch_schema", schema_params(schema)).await?;
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
            "[MySQL] execute_streaming() called, transactional={}, streaming={}",
            options.transactional, options.streaming
        );

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
                debug!("[MySQL] Streaming statement {}", current);

                let result = match self.query(&sql).await {
                    Ok(r) => r,
                    Err(e) => {
                        let sql_preview = if sql.len() > 200 {
                            format!("{}...", truncate_str(&sql, 200))
                        } else {
                            sql.clone()
                        };
                        error!(
                            "[MySQL] Streaming statement {} failed: {}, SQL: {}",
                            current, e, sql_preview
                        );
                        SqlResult::Error(SqlErrorInfo {
                            sql: sql.clone(),
                            message: e.to_string(),
                        })
                    }
                };

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
                .filter_map(|r| r.ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            let total = statements.len();
            debug!("[MySQL] Streaming {} statement(s)", total);

            if total == 0 {
                debug!("[MySQL] No statements to execute");
                return Ok(());
            }

            for (index, sql) in statements.into_iter().enumerate() {
                let current = index + 1;
                debug!("[MySQL] Streaming statement {}/{}", current, total);

                let result = match self.query(&sql).await {
                    Ok(r) => r,
                    Err(e) => {
                        let sql_preview = if sql.len() > 200 {
                            format!("{}...", truncate_str(&sql, 200))
                        } else {
                            sql.clone()
                        };
                        error!(
                            "[MySQL] Streaming statement {}/{} failed: {}, SQL: {}",
                            current, total, e, sql_preview
                        );
                        SqlResult::Error(SqlErrorInfo {
                            sql: sql.clone(),
                            message: e.to_string(),
                        })
                    }
                };

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

        debug!("[MySQL] execute_streaming() completed");
        Ok(())
    }
}
