//! JSON-RPC 2.0 客户端。
//!
//! 单 transport 多 caller 并发:
//!
//! - `writer` 用 [`tokio::sync::Mutex`] 串行化「写一帧」(JSON 序列化 + send_msg)。
//! - `pending` 用 [`std::sync::Mutex`] 保护 `HashMap<i64, oneshot::Sender>`,
//!   持锁时间极短(insert/remove),允许在 `Drop` 中同步 lock(cancel-safe)。
//! - `next_id` 用 [`AtomicI64`] 无锁分配。
//! - reader task 把响应按 `id` 路由到对应 caller 的 oneshot;遇到 notification
//!   走 `notification_sink`(可选);EOF / IO 错误置 `closed` 并唤醒所有 caller。
//!
//! caller drop / timeout / 写失败都不会泄漏 pending 条目——`PendingGuard` 的
//! RAII Drop 保证。

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::Duration;

use extension_protocol::envelope::{Notification, Request, RequestId, ResponseBody, RpcMessage};
use extension_protocol::error::{ProtocolError, error_codes};
use extension_protocol::method;
use futures::future::BoxFuture;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::{Mutex, Notify, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tracing::{debug, trace, warn};

use crate::error::{HostError, HostResult};
use crate::host_api::HostApiHandler;
use crate::transport::{FramedTransport, ReadFramed, WriteFramed, recv_async, send_async};

/// 单次请求的可选项。
#[derive(Debug, Clone, Default)]
pub struct RequestOptions {
    /// 超时;`None` 走客户端默认值。
    pub timeout: Option<Duration>,
    /// 提供该 token 时,触发 cancel → 发 `$/cancelRequest` 并立刻返回
    /// [`HostError::Cancelled`]。
    pub cancel: Option<CancellationToken>,
}

impl RequestOptions {
    pub fn with_timeout(mut self, t: Duration) -> Self {
        self.timeout = Some(t);
        self
    }

    pub fn with_cancel(mut self, token: CancellationToken) -> Self {
        self.cancel = Some(token);
        self
    }
}

/// 可等待的取消令牌——多份 clone 共享状态并立即唤醒等待者。
#[derive(Debug, Clone)]
pub struct CancellationToken {
    flag: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
            notify: Arc::new(Notify::new()),
        }
    }
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    /// 触发取消;所有 clone 立即可见。
    pub fn cancel(&self) {
        if !self.flag.swap(true, Ordering::SeqCst) {
            self.notify.notify_waiters();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    pub async fn cancelled(&self) {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

/// 客户端句柄(轻量 clone,所有 clone 共享同一个底层 transport)。
#[derive(Clone)]
pub struct JsonRpcClientHandle {
    inner: Arc<ClientShared>,
}

impl JsonRpcClientHandle {
    /// 是否已关闭(reader task 退出 / 用户调 close)。
    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::SeqCst)
    }

    /// 发送一个 `Request` 并等待匹配的 `Response`。
    ///
    /// `params` 由 caller 序列化好(`serde_json::Value`),`R` 是期望的结果类型,
    /// client 帮做最终反序列化(失败折回 [`HostError::Serde`])。
    pub async fn call<R>(
        &self,
        method: &str,
        params: Value,
        options: RequestOptions,
    ) -> HostResult<R>
    where
        R: DeserializeOwned,
    {
        let raw = self.call_raw(method, params, options).await?;
        let parsed = serde_json::from_value::<R>(raw)?;
        Ok(parsed)
    }

    /// 同 [`call`],但不解码 result,返回原始 [`serde_json::Value`]。
    pub async fn call_raw(
        &self,
        method: &str,
        params: Value,
        options: RequestOptions,
    ) -> HostResult<Value> {
        if self.is_closed() {
            return Err(HostError::Closed);
        }

        if let Some(t) = &options.cancel {
            if t.is_cancelled() {
                return Err(HostError::Cancelled {
                    method: method.to_string(),
                });
            }
        }

        let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        let guard = PendingGuard::insert(Arc::clone(&self.inner), id, tx);

        let req = Request::new(id, method, params);
        self.send_message(&RpcMessage::Request(req)).await?;

        let to = options
            .timeout
            .unwrap_or_else(|| Duration::from_millis(crate::DEFAULT_REQUEST_TIMEOUT_MS));

        // 等响应:同时监听 cancel + timeout
        let recv_fut = rx;
        let resp = if let Some(token) = options.cancel.clone() {
            tokio::select! {
                biased;
                r = timeout(to, recv_fut) => r,
                _ = token.cancelled() => {
                    drop(guard);
                    let _ = self.send_cancel(id).await;
                    return Err(HostError::Cancelled { method: method.to_string() });
                }
            }
        } else {
            timeout(to, recv_fut).await
        };

        match resp {
            Ok(Ok(body)) => match body {
                ResponseBody::Ok { result } => Ok(result),
                ResponseBody::Err { error } => Err(HostError::Protocol(error)),
            },
            Ok(Err(_)) => {
                // oneshot 被丢:reader task 退出了
                Err(HostError::Closed)
            }
            Err(_) => {
                // timeout;guard drop 自动从 pending 摘出
                let _ = self.send_cancel(id).await;
                Err(HostError::Timeout {
                    method: method.to_string(),
                    timeout_ms: to.as_millis() as u64,
                })
            }
        }
    }

    /// 发送一个 Notification(无 id,无应答)。
    pub async fn notify(&self, method: &str, params: Value) -> HostResult<()> {
        if self.is_closed() {
            return Err(HostError::Closed);
        }
        let n = Notification::new(method, params);
        self.send_message(&RpcMessage::Notification(n)).await
    }

    /// 主动关闭:停 reader、唤醒所有 pending、置 closed。
    pub fn close(&self) {
        if !self.inner.closed.swap(true, Ordering::SeqCst) {
            wake_all_pending(&self.inner);
        }
    }

    async fn send_cancel(&self, id: i64) -> HostResult<()> {
        let n = Notification::new(method::CANCEL_REQUEST, serde_json::json!({ "id": id }));
        self.send_message(&RpcMessage::Notification(n)).await
    }

    async fn send_message<M: Serialize>(&self, msg: &M) -> HostResult<()> {
        let mut w = self.inner.writer.lock().await;
        send_async(&mut *w, msg).await.map_err(HostError::Io)
    }
}

/// JSON-RPC 客户端 owner——持有 reader task,Drop 时会 abort task。
///
/// 一般用法:`let (client, handle) = JsonRpcClient::start(transport);`
/// 然后把 `handle` 克隆传给业务模块,owner 保留在长生命周期 holder 处。
pub struct JsonRpcClient {
    handle: JsonRpcClientHandle,
    reader_task: JoinHandle<()>,
    notif_rx: Option<mpsc::UnboundedReceiver<Notification>>,
}

impl JsonRpcClient {
    /// 启动客户端:spawn reader task,返回 owner + handle。
    pub fn start<R, W>(transport: FramedTransport<R, W>) -> Self
    where
        R: ReadFramed + 'static,
        W: WriteFramed + 'static,
    {
        Self::start_with_notif_channel(transport, true)
    }

    /// Starts a client that can also answer reverse `host/*` requests.
    ///
    /// Inbound requests are handled in separate tasks. This is important for
    /// composability: a host implementation may eventually issue another RPC to
    /// the same extension, which requires the reader loop to keep routing.
    pub fn start_with_host_api<R, W>(
        transport: FramedTransport<R, W>,
        host_api: Arc<HostApiHandler>,
    ) -> Self
    where
        R: ReadFramed + 'static,
        W: WriteFramed + 'static,
    {
        Self::start_with_options(transport, true, Some(host_api))
    }

    /// 进阶:不创建 notification channel(notification 直接丢弃)。
    pub fn start_without_notifications<R, W>(transport: FramedTransport<R, W>) -> Self
    where
        R: ReadFramed + 'static,
        W: WriteFramed + 'static,
    {
        Self::start_with_notif_channel(transport, false)
    }

    fn start_with_notif_channel<R, W>(transport: FramedTransport<R, W>, with_sink: bool) -> Self
    where
        R: ReadFramed + 'static,
        W: WriteFramed + 'static,
    {
        Self::start_with_options(transport, with_sink, None)
    }

    fn start_with_options<R, W>(
        transport: FramedTransport<R, W>,
        with_sink: bool,
        host_api: Option<Arc<HostApiHandler>>,
    ) -> Self
    where
        R: ReadFramed + 'static,
        W: WriteFramed + 'static,
    {
        let (reader, writer) = transport.split();
        let writer: Box<dyn WriteFramed> = Box::new(writer);

        let (notif_tx, notif_rx) = if with_sink {
            let (tx, rx) = mpsc::unbounded_channel();
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };

        let shared = Arc::new(ClientShared {
            writer: Mutex::new(writer),
            pending: StdMutex::new(HashMap::new()),
            next_id: AtomicI64::new(1),
            closed: AtomicBool::new(false),
            inbound_handler: host_api.map(|handler| {
                Arc::new(move |request: Request| {
                    let handler = Arc::clone(&handler);
                    Box::pin(async move {
                        handler
                            .handle(&request.method, request.params)
                            .await
                            .map_err(host_error_to_protocol)
                    }) as BoxFuture<'static, Result<Value, ProtocolError>>
                }) as InboundHandler
            }),
            notif_tx,
        });

        let reader_shared = Arc::clone(&shared);
        let reader_task = tokio::spawn(async move {
            reader_loop(reader, reader_shared).await;
        });

        Self {
            handle: JsonRpcClientHandle { inner: shared },
            reader_task,
            notif_rx,
        }
    }

    /// 取出 notification 接收端;只允许 take 一次。
    pub fn take_notifications(&mut self) -> Option<mpsc::UnboundedReceiver<Notification>> {
        self.notif_rx.take()
    }

    /// 克隆轻量 handle,可分发给多个调用方。
    pub fn handle(&self) -> JsonRpcClientHandle {
        self.handle.clone()
    }

    /// 关闭并等待 reader task 结束。
    pub async fn shutdown(mut self) {
        self.handle.close();
        self.reader_task.abort();
        let _ = (&mut self.reader_task).await;
    }
}

impl Drop for JsonRpcClient {
    fn drop(&mut self) {
        self.handle.close();
        self.reader_task.abort();
    }
}

// ---------------- internal ----------------

struct ClientShared {
    writer: Mutex<Box<dyn WriteFramed>>,
    pending: StdMutex<HashMap<i64, oneshot::Sender<ResponseBody>>>,
    next_id: AtomicI64,
    closed: AtomicBool,
    inbound_handler: Option<InboundHandler>,
    notif_tx: Option<mpsc::UnboundedSender<Notification>>,
}

type InboundHandler =
    Arc<dyn Fn(Request) -> BoxFuture<'static, Result<Value, ProtocolError>> + Send + Sync>;

/// pending 表的 RAII Guard:Drop 时把自己从 HashMap 摘出,避免泄漏。
struct PendingGuard {
    shared: Arc<ClientShared>,
    id: i64,
    armed: bool,
}

impl PendingGuard {
    fn insert(shared: Arc<ClientShared>, id: i64, tx: oneshot::Sender<ResponseBody>) -> Self {
        shared.pending.lock().unwrap().insert(id, tx);
        Self {
            shared,
            id,
            armed: true,
        }
    }
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        if self.armed {
            self.shared.pending.lock().unwrap().remove(&self.id);
        }
    }
}

async fn reader_loop<R>(mut reader: R, shared: Arc<ClientShared>)
where
    R: ReadFramed,
{
    loop {
        let msg: Result<RpcMessage, _> = recv_async(&mut reader).await;
        match msg {
            Ok(RpcMessage::Response(resp)) => {
                let id = match resp.id {
                    RequestId::Number(n) => n,
                    RequestId::String(ref s) => {
                        if let Ok(n) = s.parse::<i64>() {
                            n
                        } else {
                            warn!(id = %s, "skipping response with non-numeric id");
                            continue;
                        }
                    }
                    RequestId::Null => {
                        warn!("skipping response with null id");
                        continue;
                    }
                };
                trace!(?id, "received response");
                let tx = shared.pending.lock().unwrap().remove(&id);
                if let Some(tx) = tx {
                    let _ = tx.send(resp.body);
                } else {
                    warn!(id, "received response for unknown request");
                }
            }
            Ok(RpcMessage::Notification(n)) => {
                trace!(method = %n.method, "received notification");
                if let Some(tx) = &shared.notif_tx {
                    let _ = tx.send(n);
                }
            }
            Ok(RpcMessage::Request(req)) => {
                if let Some(handler) = shared.inbound_handler.clone() {
                    trace!(method = %req.method, "received inbound request");
                    let request_id = req.id.clone();
                    let response_shared = Arc::clone(&shared);
                    tokio::spawn(async move {
                        let response = match handler(req).await {
                            Ok(result) => {
                                extension_protocol::envelope::Response::ok(request_id, result)
                            }
                            Err(error) => {
                                extension_protocol::envelope::Response::err(request_id, error)
                            }
                        };
                        send_response(response_shared, response).await;
                    });
                } else {
                    warn!(method = %req.method, "received inbound request; replying MethodNotFound");
                    let response = extension_protocol::envelope::Response::err(
                        req.id,
                        ProtocolError::new(
                            error_codes::METHOD_NOT_FOUND,
                            "host: no inbound handler",
                        ),
                    );
                    send_response(Arc::clone(&shared), response).await;
                }
            }
            Err(e) => {
                if shared.closed.load(Ordering::SeqCst) {
                    debug!(error = %e, "reader exiting (client closed)");
                } else {
                    debug!(error = %e, "reader exiting (transport error)");
                }
                break;
            }
        }
    }
    shared.closed.store(true, Ordering::SeqCst);
    wake_all_pending(&shared);
}

async fn send_response(
    shared: Arc<ClientShared>,
    response: extension_protocol::envelope::Response,
) {
    let mut writer = shared.writer.lock().await;
    if let Err(error) = send_async(&mut *writer, &RpcMessage::Response(response)).await {
        warn!(error = %error, "failed to send inbound host response");
    }
}

fn host_error_to_protocol(error: HostError) -> ProtocolError {
    match error {
        HostError::Protocol(error) => *error,
        HostError::InvalidParams { message, .. } => {
            ProtocolError::new(error_codes::INVALID_PARAMS, message)
        }
        other => ProtocolError::new(
            error_codes::INTERNAL_ERROR,
            format!("host API failed: {other}"),
        ),
    }
}

fn wake_all_pending(shared: &Arc<ClientShared>) {
    let mut pending = shared.pending.lock().unwrap();
    for (_id, tx) in pending.drain() {
        // 通过丢 sender 让 caller 的 oneshot 收到 Err,call_raw 转成 HostError::Closed。
        drop(tx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use extension_protocol::envelope::{Notification, Response, RpcMessage};
    use extension_protocol::host::{ResolveSecretParams, ResolveSecretResult};
    use extension_protocol::{conn::SecretRef, host};
    use tokio::io::duplex;
    use tokio::sync::mpsc;

    async fn fake_extension_with_reverse_request(
        mut reader: tokio::io::ReadHalf<tokio::io::DuplexStream>,
        mut writer: tokio::io::WriteHalf<tokio::io::DuplexStream>,
        ready: mpsc::Sender<()>,
    ) {
        let request = Request::new(
            99,
            method::HOST_RESOLVE_SECRET,
            serde_json::to_value(ResolveSecretParams {
                secret_ref: SecretRef::new("secret://elasticsearch/api_key"),
            })
            .unwrap(),
        );
        send_async(&mut writer, &RpcMessage::Request(request))
            .await
            .unwrap();

        ready.send(()).await.unwrap();

        let inbound: RpcMessage = recv_async(&mut reader).await.unwrap();
        let RpcMessage::Response(response) = inbound else {
            panic!("expected reverse response, received {inbound:?}");
        };
        assert_eq!(RequestId::Number(99), response.id);
        assert_eq!(
            Some(&serde_json::json!({ "value": "dG9rZW4tdmFsdWU=" })),
            response.result()
        );

        let outbound = recv_async(&mut reader).await.unwrap();
        let RpcMessage::Request(request) = outbound else {
            panic!("expected outbound request");
        };
        let response = Response::ok(request.id, request.params);
        ready.send(()).await.unwrap();
        send_async(&mut writer, &RpcMessage::Response(response))
            .await
            .unwrap();
    }

    /// 启动一个「假扩展」:把读到的 Request 按规则回 Response,
    /// 收到的 Notification 落到 `notifications` 通道里方便断言。
    async fn fake_extension(
        mut reader: tokio::io::ReadHalf<tokio::io::DuplexStream>,
        mut writer: tokio::io::WriteHalf<tokio::io::DuplexStream>,
        notifications: tokio::sync::mpsc::UnboundedSender<Notification>,
    ) {
        loop {
            let msg: Result<RpcMessage, _> = recv_async(&mut reader).await;
            match msg {
                Ok(RpcMessage::Request(req)) => match req.method.as_str() {
                    "echo" => {
                        let resp = Response::ok(req.id, req.params);
                        send_async(&mut writer, &RpcMessage::Response(resp))
                            .await
                            .unwrap();
                    }
                    "fail" => {
                        let pe = ProtocolError::new(error_codes::SQL_SYNTAX_ERROR, "boom");
                        let resp = Response::err(req.id, pe);
                        send_async(&mut writer, &RpcMessage::Response(resp))
                            .await
                            .unwrap();
                    }
                    "sleep" => {
                        let ms = req
                            .params
                            .get("ms")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(1000);
                        tokio::time::sleep(Duration::from_millis(ms)).await;
                        let resp = Response::ok(req.id, serde_json::json!({"slept": ms}));
                        send_async(&mut writer, &RpcMessage::Response(resp))
                            .await
                            .unwrap();
                    }
                    _ => {
                        let pe = ProtocolError::new(error_codes::METHOD_NOT_FOUND, "unknown");
                        let resp = Response::err(req.id, pe);
                        send_async(&mut writer, &RpcMessage::Response(resp))
                            .await
                            .unwrap();
                    }
                },
                Ok(RpcMessage::Notification(n)) => {
                    let _ = notifications.send(n);
                }
                Ok(RpcMessage::Response(_)) => { /* 测试中不会有 */ }
                Err(_) => break,
            }
        }
    }

    fn build_client_and_server() -> (
        JsonRpcClient,
        tokio::task::JoinHandle<()>,
        tokio::sync::mpsc::UnboundedReceiver<Notification>,
    ) {
        let (client_side, server_side) = duplex(8192);
        let (cr, cw) = tokio::io::split(client_side);
        let (sr, sw) = tokio::io::split(server_side);
        let (n_tx, n_rx) = tokio::sync::mpsc::unbounded_channel();
        let server = tokio::spawn(async move { fake_extension(sr, sw, n_tx).await });
        let client = JsonRpcClient::start(FramedTransport::new(cr, cw));
        (client, server, n_rx)
    }

    #[tokio::test]
    async fn inbound_host_request_is_dispatched_without_stalling_outbound_calls() {
        struct SecretHost;

        #[async_trait::async_trait]
        impl crate::HostApiProvider for SecretHost {
            async fn request_credential(
                &self,
                _params: host::RequestCredentialParams,
            ) -> HostResult<host::RequestCredentialResult> {
                unimplemented!()
            }

            async fn resolve_secret(
                &self,
                params: host::ResolveSecretParams,
            ) -> HostResult<host::ResolveSecretResult> {
                assert_eq!(
                    "secret://elasticsearch/api_key",
                    params.secret_ref.secret_ref
                );
                Ok(ResolveSecretResult {
                    value: b"token-value".to_vec(),
                })
            }

            async fn notify(&self, _params: host::NotifyParams) -> HostResult<host::NotifyResult> {
                unimplemented!()
            }

            async fn storage_get(
                &self,
                _params: host::StorageGetParams,
            ) -> HostResult<host::StorageGetResult> {
                unimplemented!()
            }

            async fn storage_set(&self, _params: host::StorageSetParams) -> HostResult<()> {
                unimplemented!()
            }

            async fn log(&self, _params: host::LogParams) -> HostResult<()> {
                unimplemented!()
            }
        }

        let (client_side, server_side) = duplex(8192);
        let (client_reader, client_writer) = tokio::io::split(client_side);
        let client = JsonRpcClient::start_with_host_api(
            FramedTransport::new(client_reader, client_writer),
            Arc::new(crate::HostApiHandler::new(Arc::new(SecretHost))),
        );
        let handle = client.handle();

        let (ready_tx, mut ready_rx) = mpsc::channel(2);
        let (server_reader, server_writer) = tokio::io::split(server_side);
        let server = tokio::spawn(async move {
            fake_extension_with_reverse_request(server_reader, server_writer, ready_tx).await
        });

        let (call_started_tx, call_started_rx) = tokio::sync::oneshot::channel();
        let call = tokio::spawn({
            let handle = handle.clone();
            async move {
                call_started_tx.send(()).unwrap();
                tokio::task::yield_now().await;
                handle
                    .call_raw("echo", serde_json::json!({}), RequestOptions::default())
                    .await
            }
        });
        call_started_rx.await.unwrap();
        call.await.unwrap().unwrap();
        ready_rx.recv().await.unwrap();
        server.await.unwrap();
        client.shutdown().await;
    }

    #[tokio::test]
    async fn inbound_request_without_host_api_returns_method_not_found() {
        let (client_side, mut server_side) = duplex(8192);
        let (client_reader, client_writer) = tokio::io::split(client_side);
        let client = JsonRpcClient::start(FramedTransport::new(client_reader, client_writer));
        let request = Request::new(7, "host/unknown", Value::Null);
        send_async(&mut server_side, &RpcMessage::Request(request))
            .await
            .unwrap();
        let response = timeout(Duration::from_secs(2), recv_async(&mut server_side))
            .await
            .unwrap()
            .unwrap();
        let RpcMessage::Response(response) = response else {
            panic!("expected response");
        };
        match response.body {
            ResponseBody::Err { error } => {
                assert_eq!(error_codes::METHOD_NOT_FOUND, error.code)
            }
            ResponseBody::Ok { result } => panic!("unexpected success: {result}"),
        }
        client.shutdown().await;
    }

    #[tokio::test]
    async fn call_echo_returns_params() {
        let (client, _server, _n_rx) = build_client_and_server();
        let h = client.handle();
        let v: Value = h
            .call_raw(
                "echo",
                serde_json::json!({"x": 1}),
                RequestOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(v, serde_json::json!({"x": 1}));
    }

    #[tokio::test]
    async fn call_returns_protocol_error_on_failure() {
        let (client, _server, _n_rx) = build_client_and_server();
        let h = client.handle();
        let err = h
            .call_raw("fail", serde_json::json!({}), RequestOptions::default())
            .await
            .unwrap_err();
        match err {
            HostError::Protocol(pe) => {
                assert_eq!(pe.code, error_codes::SQL_SYNTAX_ERROR);
                assert!(pe.message.contains("boom"));
            }
            other => panic!("expected Protocol, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn call_method_not_found_maps_to_protocol_error() {
        let (client, _server, _n_rx) = build_client_and_server();
        let h = client.handle();
        let err = h
            .call_raw("nope", serde_json::json!({}), RequestOptions::default())
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            HostError::Protocol(ref pe) if pe.code == error_codes::METHOD_NOT_FOUND
        ));
    }

    #[tokio::test]
    async fn call_with_timeout_returns_timeout_error() {
        let (client, _server, _n_rx) = build_client_and_server();
        let h = client.handle();
        let err = h
            .call_raw(
                "sleep",
                serde_json::json!({"ms": 5000}),
                RequestOptions::default().with_timeout(Duration::from_millis(50)),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, HostError::Timeout { ref method, .. } if method == "sleep"));
    }

    #[tokio::test]
    async fn call_with_cancel_token_returns_cancelled() {
        let (client, _server, _n_rx) = build_client_and_server();
        let h = client.handle();
        let token = CancellationToken::new();

        let h2 = h.clone();
        let token2 = token.clone();
        let task = tokio::spawn(async move {
            h2.call_raw(
                "sleep",
                serde_json::json!({"ms": 5000}),
                RequestOptions::default().with_cancel(token2),
            )
            .await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        token.cancel();
        let err = task.await.unwrap().unwrap_err();
        assert!(matches!(err, HostError::Cancelled { .. }));
    }

    #[tokio::test]
    async fn notify_reaches_fake_extension() {
        let (client, _server, mut n_rx) = build_client_and_server();
        let h = client.handle();
        h.notify("conn/lost", serde_json::json!({"reason":"x"}))
            .await
            .unwrap();
        let n = tokio::time::timeout(Duration::from_secs(1), n_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(n.method, "conn/lost");
        assert_eq!(n.params, serde_json::json!({"reason":"x"}));
    }

    #[tokio::test]
    async fn call_after_close_returns_closed() {
        let (mut client, _server, _n_rx) = build_client_and_server();
        let h = client.handle();
        client.handle.close();
        let err = h
            .call_raw("echo", serde_json::json!({}), RequestOptions::default())
            .await
            .unwrap_err();
        assert!(matches!(err, HostError::Closed));
        // 也清理 reader task
        let _ = client.take_notifications();
    }

    #[tokio::test]
    async fn shutdown_closes_client_and_aborts_reader() {
        let (client, _server, _n_rx) = build_client_and_server();
        let h = client.handle();
        client.shutdown().await;
        assert!(h.is_closed());
    }

    #[tokio::test]
    async fn typed_call_decodes_result() {
        let (client, _server, _n_rx) = build_client_and_server();
        let h = client.handle();

        #[derive(serde::Deserialize, PartialEq, Debug)]
        struct Slept {
            slept: u64,
        }

        let r: Slept = h
            .call(
                "sleep",
                serde_json::json!({"ms": 10}),
                RequestOptions::default().with_timeout(Duration::from_millis(2000)),
            )
            .await
            .unwrap();
        assert_eq!(r, Slept { slept: 10 });
    }

    #[tokio::test]
    async fn concurrent_calls_route_independently() {
        let (client, _server, _n_rx) = build_client_and_server();
        let h = client.handle();
        let h1 = h.clone();
        let h2 = h.clone();
        let h3 = h.clone();

        let f1 = tokio::spawn(async move {
            h1.call_raw(
                "echo",
                serde_json::json!({"v": 1}),
                RequestOptions::default(),
            )
            .await
        });
        let f2 = tokio::spawn(async move {
            h2.call_raw(
                "echo",
                serde_json::json!({"v": 2}),
                RequestOptions::default(),
            )
            .await
        });
        let f3 = tokio::spawn(async move {
            h3.call_raw(
                "echo",
                serde_json::json!({"v": 3}),
                RequestOptions::default(),
            )
            .await
        });

        let r1 = f1.await.unwrap().unwrap();
        let r2 = f2.await.unwrap().unwrap();
        let r3 = f3.await.unwrap().unwrap();
        assert_eq!(r1, serde_json::json!({"v": 1}));
        assert_eq!(r2, serde_json::json!({"v": 2}));
        assert_eq!(r3, serde_json::json!({"v": 3}));
    }

    #[tokio::test]
    async fn handle_is_closed_after_reader_eof() {
        let (mut client, server, _n_rx) = build_client_and_server();
        let h = client.handle();
        // abort fake extension
        server.abort();
        let _ = server.await;
        // 给 reader task 一点时间感知 EOF
        for _ in 0..20 {
            if h.is_closed() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(h.is_closed());
        let _ = client.take_notifications();
    }

    #[tokio::test]
    async fn cancellation_token_helpers() {
        let t = CancellationToken::new();
        assert!(!t.is_cancelled());
        let t2 = t.clone();
        t.cancel();
        assert!(t.is_cancelled());
        assert!(t2.is_cancelled());
    }

    #[tokio::test]
    async fn request_options_builder() {
        let token = CancellationToken::new();
        let o = RequestOptions::default()
            .with_timeout(Duration::from_millis(123))
            .with_cancel(token.clone());
        assert_eq!(o.timeout, Some(Duration::from_millis(123)));
        assert!(o.cancel.is_some());
    }
}
