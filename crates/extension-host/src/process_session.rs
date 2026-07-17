//! 通用的进程 RPC session。
//!
//! 这一层只拥有 native child process 和 JSON-RPC 生命周期，不知道数据库、
//! Redis、MongoDB 或任何业务 method。具体业务只需要把 `SpawnConfig` 与
//! `NegotiationConfig` 组装好，再通过 [`ProcessRpcSession`] 发送 typed/raw
//! request。

use std::sync::Mutex as StdMutex;
use std::time::Duration;

use extension_protocol::Notification;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::warn;

use crate::client::{JsonRpcClient, JsonRpcClientHandle, RequestOptions};
use crate::error::{HostError, HostResult};
use crate::negotiation::{ExtensionSession, NegotiationConfig, negotiate, shutdown};
use crate::process::ProcessHandle;
use crate::transport::FramedTransport;

/// 通用 session 默认单次请求超时。
pub const DEFAULT_SESSION_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// 通用 session 默认优雅关闭窗口。
pub const DEFAULT_SESSION_SHUTDOWN_GRACE: u32 = 5_000;

/// Host 接收到的低频扩展 notification。
pub type NotificationReceiver = mpsc::UnboundedReceiver<Notification>;

/// 启动通用进程 RPC session 所需的配置。
#[derive(Debug, Clone)]
pub struct ProcessRpcSessionConfig {
    pub spawn: crate::process::SpawnConfig,
    pub negotiation: NegotiationConfig,
    pub request_timeout: Duration,
    pub shutdown_grace_ms: u32,
    /// 仅用于日志和错误上下文，不会发送给扩展进程。
    pub label: String,
}

impl ProcessRpcSessionConfig {
    pub fn new(spawn: crate::process::SpawnConfig, negotiation: NegotiationConfig) -> Self {
        let label = spawn.program.display().to_string();
        Self {
            spawn,
            negotiation,
            request_timeout: DEFAULT_SESSION_REQUEST_TIMEOUT,
            shutdown_grace_ms: DEFAULT_SESSION_SHUTDOWN_GRACE,
            label,
        }
    }

    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    pub fn with_shutdown_grace_ms(mut self, grace_ms: u32) -> Self {
        self.shutdown_grace_ms = grace_ms;
        self
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }
}

struct ProcessRpcSessionOwner {
    client: JsonRpcClient,
    process: Option<ProcessHandle>,
}

/// 一个与业务无关的 native child process JSON-RPC session。
pub struct ProcessRpcSession {
    handle: JsonRpcClientHandle,
    session: ExtensionSession,
    owner: StdMutex<Option<ProcessRpcSessionOwner>>,
    notifications: StdMutex<Option<NotificationReceiver>>,
    request_timeout: Duration,
    shutdown_grace_ms: u32,
    label: String,
}

impl ProcessRpcSession {
    /// 启动子进程、建立 transport、执行 init 握手并返回 session。
    pub async fn start(config: ProcessRpcSessionConfig) -> HostResult<Self> {
        let mut process = crate::process::spawn(config.spawn.clone()).await?;
        let stream = process.take_stream().ok_or_else(|| {
            HostError::Config(format!(
                "process `{}` did not return a connected transport",
                config.label
            ))
        })?;

        let (reader, writer) = tokio::io::split(stream);
        let transport = FramedTransport::new(reader, writer);
        let client = JsonRpcClient::start(transport);
        Self::start_with_client(client, Some(process), config).await
    }

    async fn start_with_client(
        mut client: JsonRpcClient,
        process: Option<ProcessHandle>,
        config: ProcessRpcSessionConfig,
    ) -> HostResult<Self> {
        let notifications = client.take_notifications();
        let handle = client.handle();
        let session = negotiate(&handle, config.negotiation).await?;

        Ok(Self {
            handle,
            session,
            owner: StdMutex::new(Some(ProcessRpcSessionOwner { client, process })),
            notifications: StdMutex::new(notifications),
            request_timeout: config.request_timeout,
            shutdown_grace_ms: config.shutdown_grace_ms,
            label: config.label,
        })
    }

    pub async fn request<T>(&self, method: &str, params: Value) -> HostResult<T>
    where
        T: DeserializeOwned,
    {
        let raw = self.request_value(method, params).await?;
        serde_json::from_value(raw).map_err(HostError::from)
    }

    pub async fn request_value(&self, method: &str, params: Value) -> HostResult<Value> {
        self.request_value_with_options(method, params, RequestOptions::default())
            .await
    }

    pub async fn request_value_with_options(
        &self,
        method: &str,
        params: Value,
        mut options: RequestOptions,
    ) -> HostResult<Value> {
        if options.timeout.is_none() {
            options.timeout = Some(self.request_timeout);
        }
        self.handle.call_raw(method, params, options).await
    }

    pub async fn notify(&self, method: &str, params: Value) -> HostResult<()> {
        self.handle.notify(method, params).await
    }

    pub fn session(&self) -> &ExtensionSession {
        &self.session
    }

    pub fn supports(&self, capability: &str) -> bool {
        self.session.has_feature(capability)
    }

    pub fn declares_method(&self, method: &str) -> bool {
        self.session.declares_method(method)
    }

    pub fn take_notifications(&self) -> Option<NotificationReceiver> {
        self.notifications
            .lock()
            .expect("notifications mutex poisoned")
            .take()
    }

    pub fn is_closed(&self) -> bool {
        self.handle.is_closed()
    }

    /// 先请求扩展优雅退出，再关闭 reader 并回收 child。
    pub async fn shutdown(&self) {
        if let Err(error) = shutdown(&self.handle, self.shutdown_grace_ms).await {
            warn!(
                label = %self.label,
                error = %error,
                "native process graceful shutdown failed; dropping process owner"
            );
        }

        let owner = self
            .owner
            .lock()
            .expect("process owner mutex poisoned")
            .take();
        if let Some(ProcessRpcSessionOwner { client, process }) = owner {
            client.shutdown().await;
            drop(process);
        }
    }
}

impl Drop for ProcessRpcSession {
    fn drop(&mut self) {
        self.handle.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::CancellationToken;
    use crate::process::SpawnConfig;
    use crate::transport::{FramedTransport, recv_async, send_async};
    use extension_protocol::envelope::{Notification, Response, RpcMessage};
    use extension_protocol::lifecycle::InitResult;
    use extension_protocol::method;
    use serde_json::json;
    use tokio::io::duplex;

    async fn fake_extension(
        mut reader: tokio::io::ReadHalf<tokio::io::DuplexStream>,
        mut writer: tokio::io::WriteHalf<tokio::io::DuplexStream>,
    ) {
        loop {
            let Ok(message) = recv_async::<_, RpcMessage>(&mut reader).await else {
                break;
            };
            match message {
                RpcMessage::Request(request) if request.method == method::INIT => {
                    let result = InitResult::new("1.0.0")
                        .with_api("test", "1.0")
                        .with_method("x/test/echo")
                        .with_feature("test-feature");
                    let response = Response::ok(
                        request.id,
                        serde_json::to_value(result).expect("serialize init result"),
                    );
                    send_async(&mut writer, &RpcMessage::Response(response))
                        .await
                        .expect("send init response");
                }
                RpcMessage::Request(request) if request.method == "x/test/echo" => {
                    let response = Response::ok(request.id, request.params);
                    send_async(&mut writer, &RpcMessage::Response(response))
                        .await
                        .expect("send echo response");
                }
                RpcMessage::Request(request) if request.method == "x/test/emit" => {
                    let notification = Notification::new("x/test/event", request.params.clone());
                    send_async(&mut writer, &RpcMessage::Notification(notification))
                        .await
                        .expect("send notification");
                    let response = Response::ok(request.id, Value::Null);
                    send_async(&mut writer, &RpcMessage::Response(response))
                        .await
                        .expect("send emit response");
                }
                RpcMessage::Request(request) if request.method == "x/test/sleep" => {
                    let millis = request
                        .params
                        .get("millis")
                        .and_then(Value::as_u64)
                        .unwrap_or(250);
                    tokio::time::sleep(Duration::from_millis(millis)).await;
                    let response = Response::ok(request.id, json!({ "slept": millis }));
                    let _ = send_async(&mut writer, &RpcMessage::Response(response)).await;
                }
                RpcMessage::Request(request) if request.method == method::SHUTDOWN => {
                    let response = Response::ok(request.id, Value::Null);
                    let _ = send_async(&mut writer, &RpcMessage::Response(response)).await;
                    break;
                }
                RpcMessage::Notification(_) | RpcMessage::Response(_) => {}
                RpcMessage::Request(request) => {
                    let response = Response::ok(request.id, Value::Null);
                    let _ = send_async(&mut writer, &RpcMessage::Response(response)).await;
                }
            }
        }
    }

    async fn test_session() -> ProcessRpcSession {
        let (client_side, extension_side) = duplex(16 * 1024);
        let (client_reader, client_writer) = tokio::io::split(client_side);
        let (extension_reader, extension_writer) = tokio::io::split(extension_side);
        tokio::spawn(fake_extension(extension_reader, extension_writer));

        let client = JsonRpcClient::start(FramedTransport::new(client_reader, client_writer));
        let config = ProcessRpcSessionConfig::new(
            SpawnConfig::new("test-driver"),
            NegotiationConfig::new("1.0.0", "instance").offer_api("test", "1.0"),
        )
        .with_request_timeout(Duration::from_millis(100));
        ProcessRpcSession::start_with_client(client, None, config)
            .await
            .expect("start test session")
    }

    #[test]
    fn config_defaults_are_bounded_and_labelled() {
        let config = ProcessRpcSessionConfig::new(
            SpawnConfig::new("/tmp/demo-driver"),
            NegotiationConfig::new("1.0.0", "instance"),
        );

        assert_eq!(DEFAULT_SESSION_REQUEST_TIMEOUT, config.request_timeout);
        assert_eq!(DEFAULT_SESSION_SHUTDOWN_GRACE, config.shutdown_grace_ms);
        assert_eq!("/tmp/demo-driver", config.label);
    }

    #[test]
    fn config_builders_replace_runtime_limits_and_label() {
        let config = ProcessRpcSessionConfig::new(
            SpawnConfig::new("driver"),
            NegotiationConfig::new("1.0.0", "instance"),
        )
        .with_request_timeout(Duration::from_millis(125))
        .with_shutdown_grace_ms(250)
        .with_label("redis-driver");

        assert_eq!(Duration::from_millis(125), config.request_timeout);
        assert_eq!(250, config.shutdown_grace_ms);
        assert_eq!("redis-driver", config.label);
    }

    #[tokio::test]
    async fn session_negotiates_requests_and_receives_notifications() {
        let session = test_session().await;

        assert!(session.supports("test-feature"));
        assert!(session.declares_method("x/test/echo"));
        let actual: Value = session
            .request("x/test/echo", json!({ "value": 42 }))
            .await
            .expect("echo request");
        assert_eq!(json!({ "value": 42 }), actual);

        let mut notifications = session.take_notifications().expect("notification receiver");
        session
            .request_value("x/test/emit", json!({ "event": "ready" }))
            .await
            .expect("emit request");
        let notification = notifications.recv().await.expect("notification");
        assert_eq!("x/test/event", notification.method);
        assert_eq!(json!({ "event": "ready" }), notification.params);

        session.shutdown().await;
        assert!(session.is_closed());
    }

    #[tokio::test]
    async fn session_applies_default_timeout() {
        let session = test_session().await;
        let error = session
            .request_value("x/test/sleep", json!({ "millis": 500 }))
            .await
            .expect_err("request should time out");

        assert!(matches!(
            error,
            HostError::Timeout {
                method,
                timeout_ms: 100
            } if method == "x/test/sleep"
        ));
    }

    #[tokio::test]
    async fn session_forwards_explicit_cancellation() {
        let session = test_session().await;
        let token = CancellationToken::new();
        let cancel = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            cancel.cancel();
        });

        let error = session
            .request_value_with_options(
                "x/test/sleep",
                json!({ "millis": 500 }),
                RequestOptions::default().with_cancel(token),
            )
            .await
            .expect_err("request should be cancelled");

        assert!(matches!(
            error,
            HostError::Cancelled { method } if method == "x/test/sleep"
        ));
    }
}
