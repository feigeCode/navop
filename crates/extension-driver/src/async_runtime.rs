//! Tokio-first native sidecar runtime。
//!
//! 与同步 [`crate::serve`] 并列：reader 只负责收帧和路由，每个请求作为 Tokio
//! task 执行；同一连接通过 async Mutex 串行，多个连接可以并发。取消通过
//! request id 对应的 AbortHandle 完成，cancelled request 的迟到 outcome 会被丢弃。

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use extension_protocol::conn::ConnId;
use extension_protocol::envelope::{Notification, Request, RequestId, Response, RpcMessage};
use extension_protocol::error::{ProtocolError, error_codes};
use extension_protocol::framing::{recv_msg_async, send_msg_async};
use extension_protocol::method;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio::task::AbortHandle;

#[async_trait]
pub trait AsyncDriverConnection: Send + Sync {
    async fn call(&mut self, method: &str, params: &Value) -> Result<Value, ProtocolError>;

    async fn close(&mut self) {}
}

pub struct AsyncOpenedConnection {
    pub conn_id: ConnId,
    pub open_result: Value,
    pub connection: Box<dyn AsyncDriverConnection>,
}

#[async_trait]
pub trait AsyncNativeDriver: Send + Sync + 'static {
    async fn init(&self, params: &Value) -> Result<Value, ProtocolError>;

    async fn open_connection(&self, params: &Value)
    -> Result<AsyncOpenedConnection, ProtocolError>;

    async fn call_connless(&self, method: &str, params: &Value) -> Result<Value, ProtocolError>;

    async fn shutdown(&self) {}
}

type ConnectionHandle = Arc<Mutex<Box<dyn AsyncDriverConnection>>>;
type Connections = Arc<Mutex<HashMap<ConnId, ConnectionHandle>>>;
type Pending = Arc<StdMutex<HashMap<RequestId, AbortHandle>>>;
type RouteMap = Arc<StdMutex<HashMap<String, ConnId>>>;

#[derive(Clone)]
struct ResourceRoutes {
    blobs: RouteMap,
    events: RouteMap,
}

impl ResourceRoutes {
    fn new() -> Self {
        Self {
            blobs: Arc::new(StdMutex::new(HashMap::new())),
            events: Arc::new(StdMutex::new(HashMap::new())),
        }
    }

    fn remove_connection(&self, conn_id: ConnId) {
        self.blobs
            .lock()
            .expect("blob routes mutex poisoned")
            .retain(|_, owner| *owner != conn_id);
        self.events
            .lock()
            .expect("event routes mutex poisoned")
            .retain(|_, owner| *owner != conn_id);
    }
}

struct Outcome {
    id: RequestId,
    conn_id: Option<ConnId>,
    method: String,
    params: Value,
    result: Result<Value, ProtocolError>,
}

pub async fn serve_async<D, R, W>(driver: D, mut reader: R, writer: W) -> anyhow::Result<()>
where
    D: AsyncNativeDriver,
    R: AsyncReadExt + Unpin + Send,
    W: AsyncWriteExt + Unpin + Send + 'static,
{
    let driver = Arc::new(driver);
    let writer = Arc::new(Mutex::new(writer));
    let connections: Connections = Arc::new(Mutex::new(HashMap::new()));
    let pending: Pending = Arc::new(StdMutex::new(HashMap::new()));
    let resource_routes = ResourceRoutes::new();
    let (outcome_tx, outcome_rx) = mpsc::unbounded_channel();
    let pump = tokio::spawn(pump_outcomes(
        outcome_rx,
        Arc::clone(&pending),
        resource_routes.clone(),
        Arc::clone(&writer),
    ));
    let mut initialized = false;

    let result = loop {
        let message: RpcMessage = match recv_msg_async(&mut reader).await {
            Ok(message) => message,
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => break Ok(()),
            Err(error) => break Err(error.into()),
        };

        match message {
            RpcMessage::Request(request) => {
                if handle_request(
                    Arc::clone(&driver),
                    Arc::clone(&writer),
                    Arc::clone(&connections),
                    Arc::clone(&pending),
                    resource_routes.clone(),
                    outcome_tx.clone(),
                    &mut initialized,
                    request,
                )
                .await
                {
                    break Ok(());
                }
            }
            RpcMessage::Notification(notification) => {
                handle_notification(notification, &pending, &writer).await;
            }
            RpcMessage::Response(_) => {}
        }
    };

    abort_pending(&pending);
    close_all_connections(&connections).await;
    driver.shutdown().await;
    pump.abort();
    let _ = pump.await;
    result
}

#[allow(clippy::too_many_arguments)]
async fn handle_request<D, W>(
    driver: Arc<D>,
    writer: Arc<Mutex<W>>,
    connections: Connections,
    pending: Pending,
    resource_routes: ResourceRoutes,
    outcome_tx: mpsc::UnboundedSender<Outcome>,
    initialized: &mut bool,
    request: Request,
) -> bool
where
    D: AsyncNativeDriver,
    W: AsyncWriteExt + Unpin + Send + 'static,
{
    if !*initialized
        && request.method != method::INIT
        && request.method != method::SHUTDOWN
        && request.method != method::PING
    {
        write_result(
            &writer,
            request.id,
            Err(ProtocolError::new(
                error_codes::NOT_INITIALIZED,
                "init must be called first",
            )),
        )
        .await;
        return false;
    }

    match request.method.as_str() {
        method::INIT => {
            let result = if *initialized {
                Err(ProtocolError::new(
                    error_codes::ALREADY_INITIALIZED,
                    "init has already completed",
                ))
            } else {
                driver.init(&request.params).await
            };
            if result.is_ok() {
                *initialized = true;
            }
            write_result(&writer, request.id, result).await;
        }
        method::PING => {
            write_result(&writer, request.id, Ok(serde_json::json!({ "pong": true }))).await;
        }
        method::SHUTDOWN => {
            write_result(&writer, request.id, Ok(Value::Null)).await;
            return true;
        }
        method::CONN_OPEN => {
            let params = request.params;
            let call_params = params.clone();
            let future = async move {
                let opened = driver.open_connection(&call_params).await?;
                let mut guard = connections.lock().await;
                if guard.contains_key(&opened.conn_id) {
                    return Err(ProtocolError::new(
                        error_codes::INTERNAL_ERROR,
                        format!("duplicate conn_id {}", opened.conn_id),
                    ));
                }
                guard.insert(opened.conn_id, Arc::new(Mutex::new(opened.connection)));
                Ok(opened.open_result)
            };
            spawn_request(
                request.id,
                None,
                request.method,
                params,
                future,
                &pending,
                outcome_tx,
            );
        }
        method::CONN_CLOSE => {
            let Some(conn_id) = conn_id_of(&request.params) else {
                write_result(
                    &writer,
                    request.id,
                    Err(ProtocolError::new(
                        error_codes::INVALID_PARAMS,
                        "conn_id is required",
                    )),
                )
                .await;
                return false;
            };
            let connection = connections.lock().await.remove(&conn_id);
            resource_routes.remove_connection(conn_id);
            let future = async move {
                let Some(connection) = connection else {
                    return Err(ProtocolError::new(
                        error_codes::UNKNOWN_CONN_ID,
                        format!("unknown conn_id {conn_id}"),
                    ));
                };
                connection.lock().await.close().await;
                Ok(Value::Null)
            };
            spawn_request(
                request.id,
                Some(conn_id),
                request.method,
                request.params,
                future,
                &pending,
                outcome_tx,
            );
        }
        _ => {
            let routed_conn_id =
                match routed_conn_id(&request.method, &request.params, &resource_routes) {
                    Ok(conn_id) => conn_id,
                    Err(error) => {
                        write_result(&writer, request.id, Err(error)).await;
                        return false;
                    }
                };
            let method_name = request.method.clone();
            let params = request.params.clone();
            let future = if let Some(conn_id) = routed_conn_id {
                let connection = connections.lock().await.get(&conn_id).cloned();
                let method = method_name.clone();
                let call_params = params.clone();
                Box::pin(async move {
                    let Some(connection) = connection else {
                        return Err(ProtocolError::new(
                            error_codes::UNKNOWN_CONN_ID,
                            format!("unknown conn_id {conn_id}"),
                        ));
                    };
                    connection.lock().await.call(&method, &call_params).await
                }) as RequestFuture
            } else {
                let method = method_name.clone();
                let call_params = params.clone();
                Box::pin(async move { driver.call_connless(&method, &call_params).await })
                    as RequestFuture
            };
            spawn_request(
                request.id,
                routed_conn_id,
                method_name,
                params,
                future,
                &pending,
                outcome_tx,
            );
        }
    }
    false
}

type RequestFuture =
    std::pin::Pin<Box<dyn Future<Output = Result<Value, ProtocolError>> + Send + 'static>>;

fn spawn_request<F>(
    id: RequestId,
    conn_id: Option<ConnId>,
    method: String,
    params: Value,
    future: F,
    pending: &Pending,
    outcome_tx: mpsc::UnboundedSender<Outcome>,
) where
    F: Future<Output = Result<Value, ProtocolError>> + Send + 'static,
{
    let (start_tx, start_rx) = oneshot::channel();
    let outcome_id = id.clone();
    let task = tokio::spawn(async move {
        let _ = start_rx.await;
        let result = future.await;
        let _ = outcome_tx.send(Outcome {
            id: outcome_id,
            conn_id,
            method,
            params,
            result,
        });
    });
    pending
        .lock()
        .expect("pending mutex poisoned")
        .insert(id, task.abort_handle());
    let _ = start_tx.send(());
}

async fn handle_notification<W>(
    notification: Notification,
    pending: &Pending,
    writer: &Arc<Mutex<W>>,
) where
    W: AsyncWriteExt + Unpin + Send + 'static,
{
    if notification.method != method::CANCEL_REQUEST {
        return;
    }
    let Some(id) = notification
        .params
        .get("id")
        .cloned()
        .and_then(|value| serde_json::from_value::<RequestId>(value).ok())
    else {
        return;
    };
    let abort = pending.lock().expect("pending mutex poisoned").remove(&id);
    if let Some(abort) = abort {
        abort.abort();
        write_result(
            writer,
            id,
            Err(ProtocolError::new(
                error_codes::REQUEST_CANCELLED,
                "request was cancelled",
            )),
        )
        .await;
    }
}

async fn pump_outcomes<W>(
    mut outcomes: mpsc::UnboundedReceiver<Outcome>,
    pending: Pending,
    resource_routes: ResourceRoutes,
    writer: Arc<Mutex<W>>,
) where
    W: AsyncWriteExt + Unpin + Send + 'static,
{
    while let Some(outcome) = outcomes.recv().await {
        let was_pending = pending
            .lock()
            .expect("pending mutex poisoned")
            .remove(&outcome.id)
            .is_some();
        if was_pending {
            if let Ok(result) = &outcome.result {
                update_resource_routes(
                    &resource_routes,
                    outcome.conn_id,
                    &outcome.method,
                    &outcome.params,
                    result,
                );
            }
            write_result(&writer, outcome.id, outcome.result).await;
        }
    }
}

async fn write_result<W>(
    writer: &Arc<Mutex<W>>,
    id: RequestId,
    result: Result<Value, ProtocolError>,
) where
    W: AsyncWriteExt + Unpin + Send + 'static,
{
    let response = match result {
        Ok(value) => Response::ok(id, value),
        Err(error) => Response::err(id, error),
    };
    let mut writer = writer.lock().await;
    let _ = send_msg_async(&mut *writer, &RpcMessage::Response(response)).await;
}

fn conn_id_of(params: &Value) -> Option<ConnId> {
    params.get("conn_id").and_then(Value::as_u64)
}

fn routed_conn_id(
    method_name: &str,
    params: &Value,
    routes: &ResourceRoutes,
) -> Result<Option<ConnId>, ProtocolError> {
    if let Some(conn_id) = conn_id_of(params) {
        return Ok(Some(conn_id));
    }
    let resource = match method_name {
        method::BLOB_READ | method::BLOB_CLOSE => params
            .get("blob_id")
            .and_then(Value::as_str)
            .map(|id| (&routes.blobs, id)),
        method::EVENT_READ | method::EVENT_CLOSE => params
            .get("stream_id")
            .and_then(Value::as_str)
            .map(|id| (&routes.events, id)),
        _ => return Ok(None),
    };
    let Some((route_map, resource_id)) = resource else {
        return Err(ProtocolError::new(
            error_codes::INVALID_PARAMS,
            format!("resource id is required for {method_name}"),
        ));
    };
    route_map
        .lock()
        .expect("resource routes mutex poisoned")
        .get(resource_id)
        .copied()
        .map(Some)
        .ok_or_else(|| {
            ProtocolError::new(
                error_codes::RESOURCE_CLOSED,
                format!("resource `{resource_id}` is closed or unknown"),
            )
        })
}

fn update_resource_routes(
    routes: &ResourceRoutes,
    conn_id: Option<ConnId>,
    method_name: &str,
    params: &Value,
    result: &Value,
) {
    if let (Some(conn_id), Some(blob_id)) = (conn_id, result.get("blob_id").and_then(Value::as_str))
    {
        routes
            .blobs
            .lock()
            .expect("blob routes mutex poisoned")
            .insert(blob_id.to_string(), conn_id);
    }
    if let (Some(conn_id), Some(blob_id)) = (
        conn_id,
        result.get("documents_blob_id").and_then(Value::as_str),
    ) {
        routes
            .blobs
            .lock()
            .expect("blob routes mutex poisoned")
            .insert(blob_id.to_string(), conn_id);
    }
    match method_name {
        method::BLOB_OPEN => {
            if let (Some(conn_id), Some(blob_id)) =
                (conn_id, result.get("blob_id").and_then(Value::as_str))
            {
                routes
                    .blobs
                    .lock()
                    .expect("blob routes mutex poisoned")
                    .insert(blob_id.to_string(), conn_id);
            }
        }
        method::EVENT_OPEN => {
            if let (Some(conn_id), Some(stream_id)) =
                (conn_id, result.get("stream_id").and_then(Value::as_str))
            {
                routes
                    .events
                    .lock()
                    .expect("event routes mutex poisoned")
                    .insert(stream_id.to_string(), conn_id);
            }
        }
        method::BLOB_CLOSE => {
            if let Some(blob_id) = params.get("blob_id").and_then(Value::as_str) {
                routes
                    .blobs
                    .lock()
                    .expect("blob routes mutex poisoned")
                    .remove(blob_id);
            }
        }
        method::EVENT_CLOSE => {
            if let Some(stream_id) = params.get("stream_id").and_then(Value::as_str) {
                routes
                    .events
                    .lock()
                    .expect("event routes mutex poisoned")
                    .remove(stream_id);
            }
        }
        _ => {}
    }
}

fn abort_pending(pending: &Pending) {
    let handles = pending
        .lock()
        .expect("pending mutex poisoned")
        .drain()
        .map(|(_, handle)| handle)
        .collect::<Vec<_>>();
    for handle in handles {
        handle.abort();
    }
}

async fn close_all_connections(connections: &Connections) {
    let handles = connections
        .lock()
        .await
        .drain()
        .map(|(_, connection)| connection)
        .collect::<Vec<_>>();
    for connection in handles {
        connection.lock().await.close().await;
    }
}
