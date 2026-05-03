use crate::connection::DbError;
use crate::ipc::protocol::{JsonRpcRequest, JsonRpcResponse};
use crate::ipc::registry::IpcDriverManifest;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tracing::warn;

pub struct JsonRpcStdioClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl JsonRpcStdioClient {
    pub async fn start(driver: &IpcDriverManifest) -> Result<Self, DbError> {
        let mut command = Command::new(&driver.entry.command);
        command
            .args(&driver.entry.args)
            .current_dir(driver.command_working_dir())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = command.spawn().map_err(|error| {
            DbError::connection_with_source(
                format!("failed to start external driver '{}'", driver.id),
                error,
            )
        })?;

        let stdin = child.stdin.take().ok_or_else(|| {
            DbError::connection(format!("external driver '{}' stdin unavailable", driver.id))
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            DbError::connection(format!(
                "external driver '{}' stdout unavailable",
                driver.id
            ))
        })?;

        if let Some(stderr) = child.stderr.take() {
            spawn_stderr_logger(driver.id.clone(), stderr);
        }

        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        })
    }

    pub async fn request<T>(&mut self, method: &str, params: Value) -> Result<T, DbError>
    where
        T: DeserializeOwned,
    {
        let value = self.request_value(method, params).await?;
        serde_json::from_value(value)
            .map_err(|error| DbError::query_with_source("invalid external driver response", error))
    }

    pub async fn request_value(&mut self, method: &str, params: Value) -> Result<Value, DbError> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let request = JsonRpcRequest::new(id, method, params);
        let mut line = serde_json::to_string(&request).map_err(|error| {
            DbError::query_with_source("failed to encode JSON-RPC request", error)
        })?;
        line.push('\n');

        self.stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|error| {
                DbError::query_with_source("failed to write JSON-RPC request", error)
            })?;
        self.stdin.flush().await.map_err(|error| {
            DbError::query_with_source("failed to flush JSON-RPC request", error)
        })?;

        self.read_response(id).await
    }

    pub async fn shutdown(&mut self) {
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }

    async fn read_response(&mut self, expected_id: u64) -> Result<Value, DbError> {
        let mut line = String::new();
        let bytes = self.stdout.read_line(&mut line).await.map_err(|error| {
            DbError::query_with_source("failed to read JSON-RPC response", error)
        })?;
        if bytes == 0 {
            return Err(DbError::connection("external driver closed stdout"));
        }

        let response: JsonRpcResponse = serde_json::from_str(line.trim_end())
            .map_err(|error| DbError::query_with_source("invalid JSON-RPC response", error))?;
        validate_response_header(&response, expected_id)?;
        if let Some(error) = response.error {
            return Err(DbError::query(format!(
                "external driver error {}: {}",
                error.code, error.message
            )));
        }
        response
            .result
            .ok_or_else(|| DbError::query("JSON-RPC response missing result"))
    }
}

fn validate_response_header(response: &JsonRpcResponse, expected_id: u64) -> Result<(), DbError> {
    if response.jsonrpc != "2.0" {
        return Err(DbError::query("invalid JSON-RPC version"));
    }
    if response.id != expected_id {
        return Err(DbError::query(format!(
            "JSON-RPC response id mismatch: expected {}, got {}",
            expected_id, response.id
        )));
    }
    Ok(())
}

fn spawn_stderr_logger(driver_id: String, stderr: tokio::process::ChildStderr) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            warn!(driver = %driver_id, "external driver stderr: {}", line);
        }
    });
}
