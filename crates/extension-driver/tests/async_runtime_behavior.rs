use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use extension_driver::{
    AsyncDriverConnection, AsyncNativeDriver, AsyncOpenedConnection, serve_async,
};
use extension_protocol::envelope::{Notification, Request, RequestId, Response, RpcMessage};
use extension_protocol::error::{ProtocolError, error_codes};
use extension_protocol::framing::{recv_msg_async, send_msg_async};
use extension_protocol::method;
use serde_json::{Value, json};
use tokio::io::{DuplexStream, ReadHalf, WriteHalf};
use tokio::sync::Notify;

#[derive(Default)]
struct DriverState {
    init_count: AtomicUsize,
    shutdown_count: AtomicUsize,
    close_counts: Mutex<HashMap<u64, usize>>,
    active_calls: AtomicUsize,
    max_active_calls: AtomicUsize,
    started_calls: AtomicUsize,
    started: Notify,
    release: Notify,
    cancelled_futures: AtomicUsize,
}

#[derive(Clone)]
struct FakeDriver {
    state: Arc<DriverState>,
}

struct FakeConnection {
    conn_id: u64,
    state: Arc<DriverState>,
}

struct CancelGuard(Arc<DriverState>);

impl Drop for CancelGuard {
    fn drop(&mut self) {
        self.0.cancelled_futures.fetch_add(1, Ordering::SeqCst);
    }
}

#[async_trait]
impl AsyncDriverConnection for FakeConnection {
    async fn call(&mut self, method: &str, _params: &Value) -> Result<Value, ProtocolError> {
        match method {
            "x/block" => {
                let active = self.state.active_calls.fetch_add(1, Ordering::SeqCst) + 1;
                self.state
                    .max_active_calls
                    .fetch_max(active, Ordering::SeqCst);
                self.state.started_calls.fetch_add(1, Ordering::SeqCst);
                self.state.started.notify_waiters();
                self.state.release.notified().await;
                self.state.active_calls.fetch_sub(1, Ordering::SeqCst);
                Ok(json!({ "conn_id": self.conn_id }))
            }
            method::EVENT_OPEN => Ok(json!({ "stream_id": format!("events-{}", self.conn_id) })),
            method::EVENT_READ => Ok(json!({ "owner_conn_id": self.conn_id, "events": [] })),
            method::EVENT_CLOSE => Ok(Value::Null),
            method::BLOB_OPEN => Ok(json!({ "blob_id": format!("blob-{}", self.conn_id) })),
            method::BLOB_READ => Ok(json!({ "owner_conn_id": self.conn_id, "done": true })),
            method::BLOB_CLOSE => Ok(Value::Null),
            _ => Ok(json!({ "conn_id": self.conn_id, "method": method })),
        }
    }

    async fn close(&mut self) {
        *self
            .state
            .close_counts
            .lock()
            .expect("close counts mutex poisoned")
            .entry(self.conn_id)
            .or_default() += 1;
    }
}

#[async_trait]
impl AsyncNativeDriver for FakeDriver {
    async fn init(&self, _params: &Value) -> Result<Value, ProtocolError> {
        self.state.init_count.fetch_add(1, Ordering::SeqCst);
        Ok(json!({ "ready": true }))
    }

    async fn open_connection(
        &self,
        params: &Value,
    ) -> Result<AsyncOpenedConnection, ProtocolError> {
        let conn_id = params
            .get("conn_id")
            .and_then(Value::as_u64)
            .ok_or_else(|| ProtocolError::new(error_codes::INVALID_PARAMS, "missing conn_id"))?;
        Ok(AsyncOpenedConnection {
            conn_id,
            open_result: json!({ "conn_id": conn_id }),
            connection: Box::new(FakeConnection {
                conn_id,
                state: Arc::clone(&self.state),
            }),
        })
    }

    async fn call_connless(&self, method: &str, _params: &Value) -> Result<Value, ProtocolError> {
        if method == "x/wait_forever" {
            let _guard = CancelGuard(Arc::clone(&self.state));
            std::future::pending().await
        }
        Ok(json!({ "method": method }))
    }

    async fn shutdown(&self) {
        self.state.shutdown_count.fetch_add(1, Ordering::SeqCst);
    }
}

struct Harness {
    reader: ReadHalf<DuplexStream>,
    writer: WriteHalf<DuplexStream>,
    task: tokio::task::JoinHandle<anyhow::Result<()>>,
}

impl Harness {
    fn start(state: Arc<DriverState>) -> Self {
        let (client, server) = tokio::io::duplex(64 * 1024);
        let (reader, writer) = tokio::io::split(client);
        let (server_reader, server_writer) = tokio::io::split(server);
        let task = tokio::spawn(serve_async(
            FakeDriver { state },
            server_reader,
            server_writer,
        ));
        Self {
            reader,
            writer,
            task,
        }
    }

    async fn send_request(&mut self, id: i64, method: &str, params: Value) {
        send_msg_async(
            &mut self.writer,
            &RpcMessage::Request(Request::new(id, method, params)),
        )
        .await
        .expect("send request");
    }

    async fn send_cancel(&mut self, id: i64) {
        send_msg_async(
            &mut self.writer,
            &RpcMessage::Notification(Notification::new(
                method::CANCEL_REQUEST,
                json!({ "id": id }),
            )),
        )
        .await
        .expect("send cancellation");
    }

    async fn response(&mut self) -> Response {
        let message = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            recv_msg_async(&mut self.reader),
        )
        .await
        .expect("timed out waiting for response")
        .expect("receive response");
        match message {
            RpcMessage::Response(response) => response,
            other => panic!("expected response, got {other:?}"),
        }
    }

    async fn request(&mut self, id: i64, method: &str, params: Value) -> Response {
        self.send_request(id, method, params).await;
        self.response().await
    }

    async fn init(&mut self) {
        let response = self.request(1, method::INIT, json!({})).await;
        assert!(response.error().is_none(), "init failed: {response:?}");
    }

    async fn finish(self) {
        tokio::time::timeout(std::time::Duration::from_secs(2), self.task)
            .await
            .expect("timed out waiting for runtime shutdown")
            .expect("runtime task panicked")
            .unwrap();
    }
}

async fn wait_for_count(counter: &AtomicUsize, notify: &Notify, expected: usize) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while counter.load(Ordering::SeqCst) < expected {
            notify.notified().await;
        }
    })
    .await
    .expect("timed out waiting for calls to start");
}

fn assert_error_code(response: &Response, expected: i32) {
    assert_eq!(
        Some(expected),
        response.error().map(|error| error.code),
        "unexpected response: {response:?}"
    );
}

#[tokio::test]
async fn lifecycle_is_gated_and_shutdown_runs_once() {
    let state = Arc::new(DriverState::default());
    let mut harness = Harness::start(Arc::clone(&state));

    let before_init = harness.request(1, "x/echo", json!({})).await;
    assert_error_code(&before_init, error_codes::NOT_INITIALIZED);

    harness.init().await;
    let second_init = harness.request(2, method::INIT, json!({})).await;
    assert_error_code(&second_init, error_codes::ALREADY_INITIALIZED);

    let shutdown = harness.request(3, method::SHUTDOWN, Value::Null).await;
    assert!(shutdown.error().is_none());
    harness.finish().await;

    assert_eq!(1, state.init_count.load(Ordering::SeqCst));
    assert_eq!(1, state.shutdown_count.load(Ordering::SeqCst));
}

#[tokio::test]
async fn routes_calls_and_closes_connections() {
    let state = Arc::new(DriverState::default());
    let mut harness = Harness::start(Arc::clone(&state));
    harness.init().await;

    let opened = harness
        .request(2, method::CONN_OPEN, json!({ "conn_id": 41 }))
        .await;
    assert_eq!(Some(&json!({ "conn_id": 41 })), opened.result());

    let called = harness.request(3, "x/echo", json!({ "conn_id": 41 })).await;
    assert_eq!(
        Some(&json!({ "conn_id": 41, "method": "x/echo" })),
        called.result()
    );

    let closed = harness
        .request(4, method::CONN_CLOSE, json!({ "conn_id": 41 }))
        .await;
    assert!(closed.error().is_none());
    assert_eq!(
        Some(&1),
        state
            .close_counts
            .lock()
            .expect("close counts mutex poisoned")
            .get(&41)
    );

    let after_close = harness.request(5, "x/echo", json!({ "conn_id": 41 })).await;
    assert_error_code(&after_close, error_codes::UNKNOWN_CONN_ID);

    harness.request(6, method::SHUTDOWN, Value::Null).await;
    harness.finish().await;
}

#[tokio::test]
async fn routes_blob_and_event_resources_and_cleans_them_on_close() {
    let state = Arc::new(DriverState::default());
    let mut harness = Harness::start(Arc::clone(&state));
    harness.init().await;
    harness
        .request(2, method::CONN_OPEN, json!({ "conn_id": 41 }))
        .await;

    let event = harness
        .request(
            3,
            method::EVENT_OPEN,
            json!({ "conn_id": 41, "kind": "messages" }),
        )
        .await;
    assert_eq!(Some(&json!({ "stream_id": "events-41" })), event.result());
    let event_read = harness
        .request(4, method::EVENT_READ, json!({ "stream_id": "events-41" }))
        .await;
    assert_eq!(
        Some(&json!({ "owner_conn_id": 41, "events": [] })),
        event_read.result()
    );

    let blob = harness
        .request(5, method::BLOB_OPEN, json!({ "conn_id": 41 }))
        .await;
    assert_eq!(Some(&json!({ "blob_id": "blob-41" })), blob.result());
    let blob_read = harness
        .request(6, method::BLOB_READ, json!({ "blob_id": "blob-41" }))
        .await;
    assert_eq!(
        Some(&json!({ "owner_conn_id": 41, "done": true })),
        blob_read.result()
    );

    harness
        .request(7, method::EVENT_CLOSE, json!({ "stream_id": "events-41" }))
        .await;
    let event_after_close = harness
        .request(8, method::EVENT_READ, json!({ "stream_id": "events-41" }))
        .await;
    assert_error_code(&event_after_close, error_codes::RESOURCE_CLOSED);

    harness
        .request(9, method::CONN_CLOSE, json!({ "conn_id": 41 }))
        .await;
    let blob_after_conn_close = harness
        .request(10, method::BLOB_READ, json!({ "blob_id": "blob-41" }))
        .await;
    assert_error_code(&blob_after_conn_close, error_codes::RESOURCE_CLOSED);

    harness.request(11, method::SHUTDOWN, Value::Null).await;
    harness.finish().await;
}

#[tokio::test]
async fn serializes_one_connection_but_runs_distinct_connections_concurrently() {
    let state = Arc::new(DriverState::default());
    let mut harness = Harness::start(Arc::clone(&state));
    harness.init().await;
    harness
        .request(2, method::CONN_OPEN, json!({ "conn_id": 1 }))
        .await;
    harness
        .request(3, method::CONN_OPEN, json!({ "conn_id": 2 }))
        .await;

    harness
        .send_request(4, "x/block", json!({ "conn_id": 1 }))
        .await;
    harness
        .send_request(5, "x/block", json!({ "conn_id": 1 }))
        .await;
    wait_for_count(&state.started_calls, &state.started, 1).await;
    tokio::task::yield_now().await;
    assert_eq!(1, state.started_calls.load(Ordering::SeqCst));

    harness
        .send_request(6, "x/block", json!({ "conn_id": 2 }))
        .await;
    wait_for_count(&state.started_calls, &state.started, 2).await;
    assert_eq!(2, state.max_active_calls.load(Ordering::SeqCst));

    state.release.notify_waiters();
    let first = harness.response().await;
    let second = harness.response().await;
    assert!(matches!(first.id, RequestId::Number(4 | 6)));
    assert!(matches!(second.id, RequestId::Number(4 | 6)));

    wait_for_count(&state.started_calls, &state.started, 3).await;
    state.release.notify_waiters();
    let third = harness.response().await;
    assert_eq!(RequestId::Number(5), third.id);

    harness.request(7, method::SHUTDOWN, Value::Null).await;
    harness.finish().await;
}

#[tokio::test]
async fn cancellation_aborts_work_and_emits_only_cancelled_response() {
    let state = Arc::new(DriverState::default());
    let mut harness = Harness::start(Arc::clone(&state));
    harness.init().await;

    harness.send_request(2, "x/wait_forever", json!({})).await;
    tokio::task::yield_now().await;
    harness.send_cancel(2).await;

    let cancelled = harness.response().await;
    assert_eq!(RequestId::Number(2), cancelled.id);
    assert_error_code(&cancelled, error_codes::REQUEST_CANCELLED);
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while state.cancelled_futures.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cancelled future was not dropped");

    let ping = harness.request(3, method::PING, Value::Null).await;
    assert_eq!(Some(&json!({ "pong": true })), ping.result());

    harness.request(4, method::SHUTDOWN, Value::Null).await;
    harness.finish().await;
}

#[tokio::test]
async fn eof_aborts_pending_work_and_closes_every_connection() {
    let state = Arc::new(DriverState::default());
    let mut harness = Harness::start(Arc::clone(&state));
    harness.init().await;
    harness
        .request(2, method::CONN_OPEN, json!({ "conn_id": 7 }))
        .await;
    harness.send_request(3, "x/wait_forever", json!({})).await;
    tokio::task::yield_now().await;

    drop(harness.reader);
    drop(harness.writer);
    tokio::time::timeout(std::time::Duration::from_secs(2), harness.task)
        .await
        .expect("timed out waiting for EOF cleanup")
        .expect("runtime task panicked")
        .unwrap();

    assert_eq!(1, state.cancelled_futures.load(Ordering::SeqCst));
    assert_eq!(
        Some(&1),
        state
            .close_counts
            .lock()
            .expect("close counts mutex poisoned")
            .get(&7)
    );
    assert_eq!(1, state.shutdown_count.load(Ordering::SeqCst));
}
