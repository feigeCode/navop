use crate::connection::{DbConnection, DbError, StreamingProgress};
use crate::executor::{ExecOptions, QueryColumnMeta, QueryResult, SqlResult, SqlSource};
use crate::ipc::client::JsonRpcStdioClient;
use crate::ipc::protocol::{
    connection_config_params, database_params, empty_params, schema_params, sql_params,
};
use crate::ipc::registry::IpcDriverManifest;
use crate::DatabasePlugin;
use async_trait::async_trait;
use one_core::storage::DbConnectionConfig;
use tokio::sync::{mpsc, Mutex};

pub struct ExternalDbConnection {
    config: DbConnectionConfig,
    driver: IpcDriverManifest,
    client: Mutex<Option<JsonRpcStdioClient>>,
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
        let mut client = JsonRpcStdioClient::start(&self.driver).await?;
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
        _plugin: &dyn DatabasePlugin,
        _source: SqlSource,
        _options: ExecOptions,
        _sender: mpsc::Sender<StreamingProgress>,
    ) -> Result<(), DbError> {
        Err(DbError::NotSupported(
            "external database streaming execution".to_string(),
        ))
    }
}
