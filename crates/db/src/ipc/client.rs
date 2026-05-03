use crate::connection::DbError;
use crate::ipc::protocol::{JsonRpcRequest, JsonRpcResponse};
use crate::ipc::registry::IpcDriverManifest;
use interprocess::local_socket::{
    tokio::{prelude::*, Stream as LocalSocketStream},
    GenericNamespaced,
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::{error::Elapsed, sleep, timeout, Instant};
use tracing::warn;

const REQUEST_TIMEOUT_MS: u64 = 30_000;

pub struct JsonRpcClient {
    child: Option<Child>,
    stream: BufReader<LocalSocketStream>,
    next_id: u64,
}

impl JsonRpcClient {
    pub async fn start(driver: &IpcDriverManifest) -> Result<Self, DbError> {
        let mut child = if driver.entry.command.trim().is_empty() {
            None
        } else {
            Some(spawn_driver_process(driver).await?)
        };
        let stream = match connect_local_socket(
            &driver.transport.name,
            driver.transport.connect_timeout_ms(),
        )
        .await
        {
            Ok(stream) => stream,
            Err(error) => {
                shutdown_child(&mut child).await;
                return Err(error);
            }
        };

        Ok(Self {
            child,
            stream: BufReader::new(stream),
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

        self.write_line(&line).await?;
        timeout(
            Duration::from_millis(REQUEST_TIMEOUT_MS),
            self.read_response(id),
        )
        .await
        .map_err(request_timeout_error)?
    }

    pub async fn shutdown(&mut self) {
        shutdown_child(&mut self.child).await;
    }

    async fn write_line(&mut self, line: &str) -> Result<(), DbError> {
        self.stream
            .get_mut()
            .write_all(line.as_bytes())
            .await
            .map_err(|error| {
                DbError::query_with_source("failed to write JSON-RPC request", error)
            })?;
        self.stream
            .get_mut()
            .flush()
            .await
            .map_err(|error| DbError::query_with_source("failed to flush JSON-RPC request", error))
    }

    async fn read_response(&mut self, expected_id: u64) -> Result<Value, DbError> {
        let mut line = String::new();
        let bytes = self.stream.read_line(&mut line).await.map_err(|error| {
            DbError::query_with_source("failed to read JSON-RPC response", error)
        })?;
        if bytes == 0 {
            return Err(DbError::connection("external driver closed IPC stream"));
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

async fn shutdown_child(child: &mut Option<Child>) {
    if let Some(child) = child.as_mut() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
}

fn request_timeout_error(error: Elapsed) -> DbError {
    DbError::query_with_source("timed out waiting for JSON-RPC response", error)
}

async fn spawn_driver_process(driver: &IpcDriverManifest) -> Result<Child, DbError> {
    let mut command = Command::new(&driver.entry.command);
    command
        .args(&driver.entry.args)
        .current_dir(driver.command_working_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    let mut child = command.spawn().map_err(|error| {
        DbError::connection_with_source(
            format!("failed to start external driver '{}'", driver.id),
            error,
        )
    })?;

    if let Some(stderr) = child.stderr.take() {
        spawn_stderr_logger(driver.id.clone(), stderr);
    }

    Ok(child)
}

async fn connect_local_socket(name: &str, timeout_ms: u64) -> Result<LocalSocketStream, DbError> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let name = name
        .to_ns_name::<GenericNamespaced>()
        .map_err(|error| DbError::connection_with_source("invalid local socket name", error))?;

    loop {
        match timeout(
            Duration::from_millis(200),
            LocalSocketStream::connect(name.clone()),
        )
        .await
        {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(error)) if Instant::now() < deadline => {
                sleep(Duration::from_millis(50)).await;
                let _ = error;
            }
            Ok(Err(error)) => {
                return Err(DbError::connection_with_source(
                    "failed to connect local socket",
                    error,
                ));
            }
            Err(error) if Instant::now() < deadline => {
                sleep(Duration::from_millis(50)).await;
                let _ = error;
            }
            Err(error) => {
                return Err(DbError::connection_with_source(
                    "timed out connecting local socket",
                    error,
                ));
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn response(json: &str) -> JsonRpcResponse {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn accepts_matching_response_header() {
        let parsed = response(r#"{"jsonrpc":"2.0","id":7,"result":{}}"#);

        assert!(validate_response_header(&parsed, 7).is_ok());
    }

    #[test]
    fn rejects_mismatched_response_id() {
        let parsed = response(r#"{"jsonrpc":"2.0","id":8,"result":{}}"#);

        assert!(validate_response_header(&parsed, 7).is_err());
    }

    #[test]
    fn rejects_invalid_json_rpc_version() {
        let parsed = response(r#"{"jsonrpc":"1.0","id":7,"result":{}}"#);

        assert!(validate_response_header(&parsed, 7).is_err());
    }
}
