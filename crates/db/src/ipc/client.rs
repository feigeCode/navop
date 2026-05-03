use crate::connection::DbError;
use crate::ipc::registry::IpcDriverManifest;
use interprocess::local_socket::{
    tokio::{prelude::*, Stream as LocalSocketStream},
    GenericNamespaced,
};
use ipc::{
    IpcRequest, IpcResponse,
    framing::{recv_msg_async, send_msg_async},
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::{Instant, error::Elapsed, sleep, timeout};
use tracing::warn;

const REQUEST_TIMEOUT_MS: u64 = 30_000;

pub struct JsonRpcClient {
    child: Option<Child>,
    stream: LocalSocketStream,
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
            stream,
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
        let request = IpcRequest::new(id, method, params);

        send_msg_async(&mut self.stream, &request).await.map_err(|error| {
            DbError::query_with_source("failed to write IPC request", error)
        })?;

        timeout(
            Duration::from_millis(REQUEST_TIMEOUT_MS),
            recv_msg_async::<_, IpcResponse>(&mut self.stream),
        )
        .await
        .map_err(request_timeout_error)?
        .map_err(|error| DbError::query_with_source("failed to read IPC response", error))
        .and_then(|response| validate_response(response, id))
    }

    pub async fn shutdown(&mut self) {
        shutdown_child(&mut self.child).await;
    }
}

fn validate_response(response: IpcResponse, expected_id: u64) -> Result<Value, DbError> {
    let version = response.protocol_version;
    if !ipc::IPC_VERSION.is_compatible_with(version) {
        return Err(DbError::connection(format!(
            "IPC protocol version mismatch: local {:?}, remote {:?}",
            ipc::IPC_VERSION,
            version
        )));
    }
    if response.request_id != expected_id {
        return Err(DbError::query(format!(
            "IPC response id mismatch: expected {}, got {}",
            expected_id,
            response.request_id
        )));
    }
    if let Some(error) = response.error {
        return Err(DbError::query(format!(
            "external driver error {:?}: {}",
            error.code, error.message
        )));
    }
    response
        .result
        .ok_or_else(|| DbError::query("IPC response missing result"))
}

async fn shutdown_child(child: &mut Option<Child>) {
    if let Some(child) = child.as_mut() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
}

fn request_timeout_error(error: Elapsed) -> DbError {
    DbError::query_with_source("timed out waiting for IPC response", error)
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
    use ipc::{IpcErrorCode, ProtocolVersion};

    #[test]
    fn accepts_matching_response() {
        let response = IpcResponse::result(7, Value::String("ok".into()));
        assert!(validate_response(response, 7).is_ok());
    }

    #[test]
    fn rejects_mismatched_response_id() {
        let response = IpcResponse::result(8, Value::String("ok".into()));
        assert!(validate_response(response, 7).is_err());
    }

    #[test]
    fn rejects_incompatible_protocol_version() {
        let response = IpcResponse {
            protocol_version: ProtocolVersion::new(99, 0),
            request_id: 7,
            result: Some(Value::String("ok".into())),
            error: None,
        };
        assert!(validate_response(response, 7).is_err());
    }

    #[test]
    fn propagates_ipc_error() {
        let response = IpcResponse::error(7, IpcErrorCode::Internal, "boom");
        let result = validate_response(response, 7);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("boom"));
    }
}
