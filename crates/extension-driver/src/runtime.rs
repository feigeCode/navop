//! [`serve`] 运行时实现:reader 任务 + per-conn worker 线程 + pump 任务。
//!
//! 详见 crate 级文档。这里是把 [`Driver`]/[`DriverConnection`] 接到 wire 上的胶水。

// ProtocolError 较大,作为 Err 类型会触发该 lint;协议层固定如此。
#![allow(clippy::result_large_err)]

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex as StdMutex};

use extension_protocol::conn::ConnId;
use extension_protocol::envelope::{Notification, Request, RequestId, Response, RpcMessage};
use extension_protocol::error::{ProtocolError, error_codes};
use extension_protocol::framing::{recv_msg_async, send_msg_async};
use extension_protocol::method;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Mutex, mpsc};
use tracing::{debug, warn};

use crate::{Driver, DriverConnection, OpenedConnection};

/// 发给 worker 线程的命令。
enum WorkerCmd {
    Call {
        id: RequestId,
        method: String,
        params: Value,
    },
    Close {
        id: RequestId,
    },
}

/// worker 线程产出的结果(回 pump → writer)。
struct Outcome {
    id: RequestId,
    method: Option<String>,
    params: Option<Value>,
    result: Result<Value, ProtocolError>,
}

/// conn/open 后台任务产物:回 reader 注册 worker 后再写回响应。
struct OpenOutcome {
    id: RequestId,
    result: Result<OpenedConnection, ProtocolError>,
}

/// reader 持有的每连接句柄。
struct Worker {
    cmd_tx: std::sync::mpsc::Sender<WorkerCmd>,
    /// 当前正在执行的请求 id(worker 设置/清除);取消时据此判断是否要硬中断。
    running: Arc<StdMutex<Option<RequestId>>>,
    /// 已被取消的请求 id 集合:worker 在出队/执行后据此把结果归一为 cancelled。
    cancelled: Arc<StdMutex<HashSet<RequestId>>>,
    interrupt: Option<crate::InterruptHook>,
}

type Inflight = Arc<StdMutex<HashMap<RequestId, ConnId>>>;
type CursorRoutes = Arc<StdMutex<HashMap<String, ConnId>>>;
type StreamRoutes = Arc<StdMutex<HashMap<String, ConnId>>>;
type ImportRoutes = Arc<StdMutex<HashMap<String, ConnId>>>;

/// reader 外后台执行的请求(`conn/open` / connless)状态。
#[derive(Clone)]
struct BackgroundRequests {
    active: Arc<StdMutex<HashSet<RequestId>>>,
    cancelled: Arc<StdMutex<HashSet<RequestId>>>,
}

impl BackgroundRequests {
    fn new() -> Self {
        Self {
            active: Arc::new(StdMutex::new(HashSet::new())),
            cancelled: Arc::new(StdMutex::new(HashSet::new())),
        }
    }

    fn start(&self, id: &RequestId) {
        self.active
            .lock()
            .expect("background active poisoned")
            .insert(id.clone());
    }

    fn cancel(&self, id: &RequestId) -> bool {
        let is_active = self
            .active
            .lock()
            .expect("background active poisoned")
            .contains(id);
        if is_active {
            self.cancelled
                .lock()
                .expect("background cancelled poisoned")
                .insert(id.clone());
        }
        is_active
    }

    fn finish_is_cancelled(&self, id: &RequestId) -> bool {
        self.active
            .lock()
            .expect("background active poisoned")
            .remove(id);
        self.cancelled
            .lock()
            .expect("background cancelled poisoned")
            .remove(id)
    }
}

/// 启动驱动运行时,接管给定的 reader/writer transport,直到 EOF 或 `shutdown`。
pub async fn serve<D, R, W>(driver: D, reader: R, writer: W) -> anyhow::Result<()>
where
    D: Driver,
    R: AsyncReadExt + Unpin + Send,
    W: AsyncWriteExt + Unpin + Send + 'static,
{
    let driver = Arc::new(driver);
    let writer = Arc::new(Mutex::new(writer));
    let inflight: Inflight = Arc::new(StdMutex::new(HashMap::new()));
    let cursor_routes: CursorRoutes = Arc::new(StdMutex::new(HashMap::new()));
    let stream_routes: StreamRoutes = Arc::new(StdMutex::new(HashMap::new()));
    let import_routes: ImportRoutes = Arc::new(StdMutex::new(HashMap::new()));
    let background = BackgroundRequests::new();
    let (out_tx, out_rx) = mpsc::unbounded_channel::<Outcome>();
    let (open_tx, open_rx) = mpsc::unbounded_channel::<OpenOutcome>();

    let pump = tokio::spawn(pump_loop(
        out_rx,
        inflight.clone(),
        cursor_routes.clone(),
        stream_routes.clone(),
        import_routes.clone(),
        writer.clone(),
    ));

    let result = reader_loop(
        driver,
        reader,
        writer,
        out_tx,
        open_tx,
        open_rx,
        inflight,
        cursor_routes,
        stream_routes,
        import_routes,
        background,
    )
    .await;

    // reader 结束(EOF / shutdown):中止 pump。worker 线程随 reader 的 workers map
    // 析构而退出(cmd_tx 被 drop);若有 worker 卡在长查询,进程退出时由 OS 回收。
    pump.abort();
    let _ = pump.await;
    result
}

async fn reader_loop<D, R, W>(
    driver: Arc<D>,
    mut reader: R,
    writer: Arc<Mutex<W>>,
    out_tx: mpsc::UnboundedSender<Outcome>,
    open_tx: mpsc::UnboundedSender<OpenOutcome>,
    mut open_rx: mpsc::UnboundedReceiver<OpenOutcome>,
    inflight: Inflight,
    cursor_routes: CursorRoutes,
    stream_routes: StreamRoutes,
    import_routes: ImportRoutes,
    background: BackgroundRequests,
) -> anyhow::Result<()>
where
    D: Driver,
    R: AsyncReadExt + Unpin + Send,
    W: AsyncWriteExt + Unpin + Send + 'static,
{
    let mut workers: HashMap<ConnId, Worker> = HashMap::new();
    let mut initialized = false;

    loop {
        tokio::select! {
            msg = recv_msg_async(&mut reader) => {
                let msg: RpcMessage = match msg {
                    Ok(m) => m,
                    Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                    Err(e) => return Err(e.into()),
                };

                match msg {
                    RpcMessage::Request(req) => {
                        let stop = handle_request(
                            &driver,
                            &writer,
                            &out_tx,
                            &open_tx,
                            &inflight,
                            &cursor_routes,
                            &stream_routes,
                            &import_routes,
                            &background,
                            &mut workers,
                            &mut initialized,
                            req,
                        )
                        .await;
                        if stop {
                            break;
                        }
                    }
                    RpcMessage::Notification(n) => {
                        handle_notification(n, &workers, &inflight, &background)
                    }
                    RpcMessage::Response(_) => {
                        warn!("driver runtime received unexpected Response; ignoring");
                    }
                }
            }
            Some(opened) = open_rx.recv() => {
                handle_open_outcome(&writer, &out_tx, &background, &mut workers, opened).await;
            }
        }
    }

    // 主动析构所有 worker:drop cmd_tx → worker 线程 recv() 返回 Err 后退出。
    workers.clear();
    Ok(())
}

/// 处理一个请求。返回 `true` 表示需要结束 serve(收到 `shutdown`)。
#[allow(clippy::too_many_arguments)]
async fn handle_request<D, W>(
    driver: &Arc<D>,
    writer: &Arc<Mutex<W>>,
    out_tx: &mpsc::UnboundedSender<Outcome>,
    open_tx: &mpsc::UnboundedSender<OpenOutcome>,
    inflight: &Inflight,
    cursor_routes: &CursorRoutes,
    stream_routes: &StreamRoutes,
    import_routes: &ImportRoutes,
    background: &BackgroundRequests,
    workers: &mut HashMap<ConnId, Worker>,
    initialized: &mut bool,
    req: Request,
) -> bool
where
    D: Driver,
    W: AsyncWriteExt + Unpin + Send + 'static,
{
    let id = req.id.clone();
    let method_name = req.method.clone();

    // init 门控:除 init/shutdown/ping 外都要求先 init。
    if !*initialized
        && method_name != method::INIT
        && method_name != method::SHUTDOWN
        && method_name != method::PING
    {
        write_result(
            writer,
            id,
            Err(ProtocolError::new(
                error_codes::NOT_INITIALIZED,
                "init must be called first",
            )),
        )
        .await;
        return false;
    }

    match method_name.as_str() {
        method::INIT => {
            let result = driver.init(&req.params);
            if result.is_ok() {
                *initialized = true;
            }
            write_result(writer, id, result).await;
        }
        method::SHUTDOWN => {
            driver.shutdown();
            write_result(writer, id, Ok(Value::Null)).await;
            return true;
        }
        method::PING => {
            write_result(writer, id, Ok(serde_json::json!({ "pong": true }))).await;
        }
        method::CONN_OPEN => {
            spawn_conn_open(
                Arc::clone(driver),
                open_tx.clone(),
                background.clone(),
                id,
                req.params,
            );
        }
        method::CONN_CLOSE => {
            match conn_id_of(&req.params).and_then(|cid| workers.remove(&cid).map(|w| (cid, w))) {
                Some((cid, worker)) => {
                    remove_routes_for_conn(cursor_routes, stream_routes, import_routes, cid);
                    // 走 worker 让 conn.close() 在拥有连接的线程上执行;响应由 pump 回写。
                    inflight
                        .lock()
                        .expect("inflight poisoned")
                        .insert(id.clone(), cid);
                    if worker
                        .cmd_tx
                        .send(WorkerCmd::Close { id: id.clone() })
                        .is_err()
                    {
                        inflight.lock().expect("inflight poisoned").remove(&id);
                        write_result(writer, id, Ok(Value::Null)).await;
                    }
                    // worker 处理完 Close 后自行退出;此处 drop 其句柄即可。
                }
                None => {
                    write_result(
                        writer,
                        id,
                        Err(ProtocolError::new(
                            error_codes::UNKNOWN_CONN_ID,
                            "unknown conn_id for conn/close",
                        )),
                    )
                    .await;
                }
            }
        }
        _ => {
            // 其余:带 conn_id → 派发给 worker;否则当作 connless 纯方法。
            if let Some(cid) = conn_id_of(&req.params)
                .or_else(|| {
                    cursor_id_of(&req.params).and_then(|cursor_id| {
                        cursor_routes
                            .lock()
                            .expect("cursor routes poisoned")
                            .get(cursor_id)
                            .copied()
                    })
                })
                .or_else(|| {
                    stream_id_of(&req.params).and_then(|stream_id| {
                        stream_routes
                            .lock()
                            .expect("stream routes poisoned")
                            .get(stream_id)
                            .copied()
                    })
                })
                .or_else(|| {
                    import_id_of(&req.params).and_then(|import_id| {
                        import_routes
                            .lock()
                            .expect("import routes poisoned")
                            .get(import_id)
                            .copied()
                    })
                })
            {
                match workers.get(&cid) {
                    Some(worker) => {
                        inflight
                            .lock()
                            .expect("inflight poisoned")
                            .insert(id.clone(), cid);
                        let cmd = WorkerCmd::Call {
                            id: id.clone(),
                            method: method_name,
                            params: req.params,
                        };
                        if worker.cmd_tx.send(cmd).is_err() {
                            inflight.lock().expect("inflight poisoned").remove(&id);
                            write_result(
                                writer,
                                id,
                                Err(ProtocolError::new(
                                    error_codes::RESOURCE_CLOSED,
                                    "connection worker is gone",
                                )),
                            )
                            .await;
                        }
                    }
                    None => {
                        write_result(
                            writer,
                            id,
                            Err(ProtocolError::new(
                                error_codes::UNKNOWN_CONN_ID,
                                format!("unknown conn_id {cid}"),
                            )),
                        )
                        .await;
                    }
                }
            } else if is_cursor_method(&method_name) {
                write_result(
                    writer,
                    id,
                    Err(ProtocolError::new(
                        error_codes::UNKNOWN_CURSOR_ID,
                        "unknown cursor_id",
                    )),
                )
                .await;
            } else if is_stream_method(&method_name) {
                write_result(
                    writer,
                    id,
                    Err(ProtocolError::new(
                        error_codes::RESOURCE_CLOSED,
                        "unknown stream_id",
                    )),
                )
                .await;
            } else if is_import_method(&method_name) {
                write_result(
                    writer,
                    id,
                    Err(ProtocolError::new(
                        error_codes::UNKNOWN_IMPORT_ID,
                        "unknown import_id",
                    )),
                )
                .await;
            } else {
                spawn_connless(
                    Arc::clone(driver),
                    out_tx.clone(),
                    background.clone(),
                    id,
                    method_name,
                    req.params,
                );
            }
        }
    }

    false
}

fn spawn_conn_open<D>(
    driver: Arc<D>,
    open_tx: mpsc::UnboundedSender<OpenOutcome>,
    background: BackgroundRequests,
    id: RequestId,
    params: Value,
) where
    D: Driver,
{
    background.start(&id);
    let request_id = id.clone();
    tokio::spawn(async move {
        let opened = tokio::task::spawn_blocking(move || driver.open_connection(&params))
            .await
            .unwrap_or_else(|join_err| {
                Err(ProtocolError::new(
                    error_codes::INTERNAL_ERROR,
                    format!("conn/open task failed: {join_err}"),
                ))
            });
        if open_tx.send(OpenOutcome { id, result: opened }).is_err() {
            let _ = background.finish_is_cancelled(&request_id);
        }
    });
}

async fn handle_open_outcome<W>(
    writer: &Arc<Mutex<W>>,
    out_tx: &mpsc::UnboundedSender<Outcome>,
    background: &BackgroundRequests,
    workers: &mut HashMap<ConnId, Worker>,
    outcome: OpenOutcome,
) where
    W: AsyncWriteExt + Unpin + Send + 'static,
{
    let was_cancelled = background.finish_is_cancelled(&outcome.id);
    if was_cancelled {
        if let Ok(opened) = outcome.result {
            close_opened_connection(opened);
        }
        write_result(writer, outcome.id, Err(cancelled_error())).await;
        return;
    }

    match outcome.result {
        Ok(opened) => {
            let open_result = spawn_worker(workers, opened, out_tx.clone());
            write_result(writer, outcome.id, Ok(open_result)).await;
        }
        Err(error) => write_result(writer, outcome.id, Err(error)).await,
    }
}

fn close_opened_connection(mut opened: OpenedConnection) {
    tokio::task::spawn_blocking(move || opened.connection.close());
}

fn spawn_connless<D>(
    driver: Arc<D>,
    out_tx: mpsc::UnboundedSender<Outcome>,
    background: BackgroundRequests,
    id: RequestId,
    method: String,
    params: Value,
) where
    D: Driver,
{
    background.start(&id);
    let request_id = id.clone();
    tokio::spawn(async move {
        let method_for_error = method.clone();
        let result = tokio::task::spawn_blocking(move || driver.call_connless(&method, &params))
            .await
            .unwrap_or_else(|join_err| {
                Err(ProtocolError::new(
                    error_codes::INTERNAL_ERROR,
                    format!("connless task `{method_for_error}` failed: {join_err}"),
                ))
            });
        let result = if background.finish_is_cancelled(&request_id) {
            Err(cancelled_error())
        } else {
            result
        };
        let _ = out_tx.send(Outcome {
            id,
            method: None,
            params: None,
            result,
        });
    });
}

/// 处理 notification:目前只关心 `$/cancelRequest`。
fn handle_notification(
    n: Notification,
    workers: &HashMap<ConnId, Worker>,
    inflight: &Inflight,
    background: &BackgroundRequests,
) {
    if n.method != method::CANCEL_REQUEST {
        debug!(method = %n.method, "driver runtime ignoring notification");
        return;
    }
    let Some(target) = parse_cancel_id(&n.params) else {
        return;
    };
    let conn = inflight
        .lock()
        .expect("inflight poisoned")
        .get(&target)
        .copied();
    let Some(cid) = conn else {
        let _ = background.cancel(&target);
        return;
    };
    let Some(worker) = workers.get(&cid) else {
        return;
    };
    // 标记取消:无论现在是否在跑,worker 出队/执行后都会据此归一为 cancelled。
    worker
        .cancelled
        .lock()
        .expect("cancelled poisoned")
        .insert(target.clone());
    // 仅当该请求确实正在执行时才硬中断(否则会误伤同连接上正在跑的别的请求)。
    let running_now = worker.running.lock().expect("running poisoned").as_ref() == Some(&target);
    if running_now {
        if let Some(hook) = &worker.interrupt {
            hook();
        }
    }
}

/// 在专属线程上起一个 worker,返回 `conn/open` 的 result。
fn spawn_worker(
    workers: &mut HashMap<ConnId, Worker>,
    opened: OpenedConnection,
    out_tx: mpsc::UnboundedSender<Outcome>,
) -> Value {
    let OpenedConnection {
        conn_id,
        open_result,
        connection,
    } = opened;
    let interrupt = connection.interrupt_hook();
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<WorkerCmd>();
    let running = Arc::new(StdMutex::new(None));
    let cancelled = Arc::new(StdMutex::new(HashSet::new()));

    let worker_running = Arc::clone(&running);
    let worker_cancelled = Arc::clone(&cancelled);
    // detached:线程在 cmd_tx 被 drop 或处理完 Close 后自行退出。
    std::thread::spawn(move || {
        worker_loop(connection, cmd_rx, out_tx, worker_running, worker_cancelled);
    });

    workers.insert(
        conn_id,
        Worker {
            cmd_tx,
            running,
            cancelled,
            interrupt,
        },
    );
    open_result
}

/// worker 线程主体:FIFO 串行执行,阻塞调用在此线程上安全进行。
fn worker_loop(
    mut connection: Box<dyn DriverConnection>,
    cmd_rx: std::sync::mpsc::Receiver<WorkerCmd>,
    out_tx: mpsc::UnboundedSender<Outcome>,
    running: Arc<StdMutex<Option<RequestId>>>,
    cancelled: Arc<StdMutex<HashSet<RequestId>>>,
) {
    while let Ok(cmd) = cmd_rx.recv() {
        match cmd {
            WorkerCmd::Call { id, method, params } => {
                // 出队即取消:还没开跑就被取消,直接回 cancelled。
                if cancelled.lock().expect("cancelled poisoned").remove(&id) {
                    let _ = out_tx.send(Outcome {
                        id,
                        method: Some(method),
                        params: Some(params),
                        result: Err(cancelled_error()),
                    });
                    continue;
                }

                *running.lock().expect("running poisoned") = Some(id.clone());
                let raw = connection.call(&method, &params);
                *running.lock().expect("running poisoned") = None;

                // 执行期间被取消:把(被中断产生的)错误归一为 cancelled。
                let was_cancelled = cancelled.lock().expect("cancelled poisoned").remove(&id);
                let result = match raw {
                    Err(_) if was_cancelled => Err(cancelled_error()),
                    other => other,
                };
                let _ = out_tx.send(Outcome {
                    id,
                    method: Some(method),
                    params: Some(params),
                    result,
                });
            }
            WorkerCmd::Close { id } => {
                connection.close();
                let _ = out_tx.send(Outcome {
                    id,
                    method: None,
                    params: None,
                    result: Ok(Value::Null),
                });
                break;
            }
        }
    }
}

async fn pump_loop<W>(
    mut out_rx: mpsc::UnboundedReceiver<Outcome>,
    inflight: Inflight,
    cursor_routes: CursorRoutes,
    stream_routes: StreamRoutes,
    import_routes: ImportRoutes,
    writer: Arc<Mutex<W>>,
) where
    W: AsyncWriteExt + Unpin + Send,
{
    while let Some(outcome) = out_rx.recv().await {
        let conn_id = inflight
            .lock()
            .expect("inflight poisoned")
            .remove(&outcome.id);
        update_cursor_routes(&cursor_routes, conn_id, &outcome);
        update_stream_routes(&stream_routes, conn_id, &outcome);
        update_import_routes(&import_routes, conn_id, &outcome);
        let resp = match outcome.result {
            Ok(value) => Response::ok(outcome.id, value),
            Err(error) => Response::err(outcome.id, error),
        };
        let mut guard = writer.lock().await;
        if let Err(e) = send_msg_async(&mut *guard, &RpcMessage::Response(resp)).await {
            warn!(error = %e, "driver runtime failed to write worker response");
        }
    }
}

async fn write_result<W>(
    writer: &Arc<Mutex<W>>,
    id: RequestId,
    result: Result<Value, ProtocolError>,
) where
    W: AsyncWriteExt + Unpin + Send,
{
    let resp = match result {
        Ok(value) => Response::ok(id, value),
        Err(error) => Response::err(id, error),
    };
    let mut guard = writer.lock().await;
    if let Err(e) = send_msg_async(&mut *guard, &RpcMessage::Response(resp)).await {
        warn!(error = %e, "driver runtime failed to write response");
    }
}

fn conn_id_of(params: &Value) -> Option<ConnId> {
    params.get("conn_id").and_then(Value::as_u64)
}

fn cursor_id_of(params: &Value) -> Option<&str> {
    params.get("cursor_id").and_then(Value::as_str)
}

fn stream_id_of(params: &Value) -> Option<&str> {
    params.get("stream_id").and_then(Value::as_str)
}

fn import_id_of(params: &Value) -> Option<&str> {
    params.get("import_id").and_then(Value::as_str)
}

fn is_cursor_method(method_name: &str) -> bool {
    matches!(
        method_name,
        method::CURSOR_FETCH | method::CURSOR_CLOSE | method::CURSOR_CANCEL
    )
}

fn is_stream_method(method_name: &str) -> bool {
    matches!(method_name, method::STREAM_READ | method::STREAM_CLOSE)
}

fn is_import_method(method_name: &str) -> bool {
    matches!(
        method_name,
        method::DATA_IMPORT_CHUNK | method::DATA_IMPORT_COMMIT | method::DATA_IMPORT_ABORT
    )
}

fn remove_routes_for_conn(
    cursor_routes: &CursorRoutes,
    stream_routes: &StreamRoutes,
    import_routes: &ImportRoutes,
    conn_id: ConnId,
) {
    cursor_routes
        .lock()
        .expect("cursor routes poisoned")
        .retain(|_, route_conn_id| *route_conn_id != conn_id);
    stream_routes
        .lock()
        .expect("stream routes poisoned")
        .retain(|_, route_conn_id| *route_conn_id != conn_id);
    import_routes
        .lock()
        .expect("import routes poisoned")
        .retain(|_, route_conn_id| *route_conn_id != conn_id);
}

fn update_cursor_routes(cursor_routes: &CursorRoutes, conn_id: Option<ConnId>, outcome: &Outcome) {
    let Some(method_name) = outcome.method.as_deref() else {
        return;
    };
    match method_name {
        method::QUERY_START => {
            if let (Some(conn_id), Ok(value)) = (conn_id, &outcome.result)
                && let Some(cursor_id) = value.get("cursor_id").and_then(Value::as_str)
            {
                cursor_routes
                    .lock()
                    .expect("cursor routes poisoned")
                    .insert(cursor_id.to_string(), conn_id);
            }
        }
        method::CURSOR_CLOSE => {
            if let Some(cursor_id) = outcome
                .params
                .as_ref()
                .and_then(|params| params.get("cursor_id"))
                .and_then(Value::as_str)
            {
                cursor_routes
                    .lock()
                    .expect("cursor routes poisoned")
                    .remove(cursor_id);
            }
        }
        _ => {}
    }
}

fn update_stream_routes(stream_routes: &StreamRoutes, conn_id: Option<ConnId>, outcome: &Outcome) {
    let Some(method_name) = outcome.method.as_deref() else {
        return;
    };
    match method_name {
        method::DATA_EXPORT => {
            if let (Some(conn_id), Ok(_)) = (conn_id, &outcome.result)
                && let Some(stream_id) = outcome
                    .params
                    .as_ref()
                    .and_then(|params| params.get("stream_id"))
                    .and_then(Value::as_str)
            {
                stream_routes
                    .lock()
                    .expect("stream routes poisoned")
                    .insert(stream_id.to_string(), conn_id);
            }
        }
        method::STREAM_CLOSE => {
            if let Some(stream_id) = outcome
                .params
                .as_ref()
                .and_then(|params| params.get("stream_id"))
                .and_then(Value::as_str)
            {
                stream_routes
                    .lock()
                    .expect("stream routes poisoned")
                    .remove(stream_id);
            }
        }
        _ => {}
    }
}

fn update_import_routes(import_routes: &ImportRoutes, conn_id: Option<ConnId>, outcome: &Outcome) {
    let Some(method_name) = outcome.method.as_deref() else {
        return;
    };
    match method_name {
        method::DATA_IMPORT_BEGIN => {
            if let (Some(conn_id), Ok(value)) = (conn_id, &outcome.result)
                && let Some(import_id) = value.get("import_id").and_then(Value::as_str)
            {
                import_routes
                    .lock()
                    .expect("import routes poisoned")
                    .insert(import_id.to_string(), conn_id);
            }
        }
        method::DATA_IMPORT_COMMIT | method::DATA_IMPORT_ABORT => {
            if let Some(import_id) = outcome
                .params
                .as_ref()
                .and_then(|params| params.get("import_id"))
                .and_then(Value::as_str)
            {
                import_routes
                    .lock()
                    .expect("import routes poisoned")
                    .remove(import_id);
            }
        }
        _ => {}
    }
}

fn parse_cancel_id(params: &Value) -> Option<RequestId> {
    let v = params.get("id")?;
    if let Some(n) = v.as_i64() {
        Some(RequestId::Number(n))
    } else {
        v.as_str().map(|s| RequestId::String(s.to_string()))
    }
}

fn cancelled_error() -> ProtocolError {
    ProtocolError::new(error_codes::REQUEST_CANCELLED, "request cancelled")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use crate::InterruptHook;
    use extension_protocol::envelope::{Notification, Request, ResponseBody};
    use tokio::io::{ReadHalf, WriteHalf, duplex};

    // ---- fake driver ----

    struct FakeDriver;

    struct FakeConn {
        cancel: Arc<AtomicBool>,
    }

    impl Driver for FakeDriver {
        fn init(&self, _params: &Value) -> Result<Value, ProtocolError> {
            Ok(serde_json::json!({ "ok": true }))
        }
        fn open_connection(&self, _params: &Value) -> Result<OpenedConnection, ProtocolError> {
            if _params
                .get("slow")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                std::thread::sleep(Duration::from_millis(500));
            }
            Ok(OpenedConnection {
                conn_id: 1,
                open_result: serde_json::json!({ "conn_id": 1 }),
                connection: Box::new(FakeConn {
                    cancel: Arc::new(AtomicBool::new(false)),
                }),
            })
        }
        fn call_connless(&self, method: &str, _params: &Value) -> Result<Value, ProtocolError> {
            match method {
                "ddl/noop" => Ok(serde_json::json!({ "connless": true })),
                "ddl/slow" => {
                    std::thread::sleep(Duration::from_millis(500));
                    Ok(serde_json::json!({ "connless": "slow" }))
                }
                _ => Err(ProtocolError::new(error_codes::METHOD_NOT_FOUND, method)),
            }
        }
    }

    impl DriverConnection for FakeConn {
        fn call(&mut self, method: &str, params: &Value) -> Result<Value, ProtocolError> {
            match method {
                "echo" => Ok(params.clone()),
                method::QUERY_START => Ok(serde_json::json!({
                    "cursor_id": "cursor-1",
                    "columns": [],
                    "row_count_known": true,
                    "row_count_estimate": 1
                })),
                method::CURSOR_FETCH => Ok(serde_json::json!({
                    "rows": [[{"type": "i64", "value": 7}]],
                    "done": true
                })),
                method::DATA_EXPORT => Ok(serde_json::json!({
                    "estimated_rows": 1,
                    "metadata": {"stream_id": params["stream_id"].clone()}
                })),
                method::STREAM_READ => Ok(serde_json::json!({
                    "data": "aGVsbG8=",
                    "done": true
                })),
                method::STREAM_CLOSE => Err(ProtocolError::new(
                    error_codes::RESOURCE_CLOSED,
                    "stream already closed",
                )),
                method::DATA_IMPORT_BEGIN => Ok(serde_json::json!({
                    "import_id": "import-1"
                })),
                method::DATA_IMPORT_CHUNK => Ok(serde_json::json!({
                    "inserted": params["rows"].as_array().map(Vec::len).unwrap_or_default(),
                    "failed": []
                })),
                method::DATA_IMPORT_COMMIT => Ok(serde_json::json!({
                    "inserted": 1,
                    "updated": 0,
                    "deleted": 0,
                    "failed": []
                })),
                method::DATA_IMPORT_ABORT => Err(ProtocolError::new(
                    error_codes::RESOURCE_CLOSED,
                    "import already closed",
                )),
                // 可中断的"慢"调用:循环检查 cancel flag(模拟 DuckDB interrupt)。
                "slow" => {
                    for _ in 0..200 {
                        if self.cancel.load(Ordering::SeqCst) {
                            return Err(ProtocolError::new(
                                error_codes::INTERNAL_ERROR,
                                "interrupted",
                            ));
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Ok(serde_json::json!({ "done": true }))
                }
                other => Err(ProtocolError::new(error_codes::METHOD_NOT_FOUND, other)),
            }
        }
        fn interrupt_hook(&self) -> Option<InterruptHook> {
            let flag = Arc::clone(&self.cancel);
            Some(Arc::new(move || flag.store(true, Ordering::SeqCst)))
        }
    }

    // ---- harness ----

    fn start() -> (
        WriteHalf<tokio::io::DuplexStream>,
        ReadHalf<tokio::io::DuplexStream>,
    ) {
        let (client, server) = duplex(64 * 1024);
        let (s_read, s_write) = tokio::io::split(server);
        tokio::spawn(async move {
            let _ = serve(FakeDriver, s_read, s_write).await;
        });
        let (c_read, c_write) = tokio::io::split(client);
        (c_write, c_read)
    }

    async fn send(w: &mut WriteHalf<tokio::io::DuplexStream>, msg: RpcMessage) {
        send_msg_async(w, &msg).await.unwrap();
    }

    async fn recv(r: &mut ReadHalf<tokio::io::DuplexStream>) -> RpcMessage {
        recv_msg_async(r).await.unwrap()
    }

    fn req(id: i64, method: &str, params: Value) -> RpcMessage {
        RpcMessage::Request(Request::new(id, method, params))
    }

    fn resp_of(msg: RpcMessage) -> Response {
        match msg {
            RpcMessage::Response(r) => r,
            other => panic!("expected response, got {other:?}"),
        }
    }

    async fn init_and_open(
        w: &mut WriteHalf<tokio::io::DuplexStream>,
        r: &mut ReadHalf<tokio::io::DuplexStream>,
    ) {
        send(w, req(1, method::INIT, serde_json::json!({}))).await;
        let _ = recv(r).await;
        send(
            w,
            req(
                2,
                method::CONN_OPEN,
                serde_json::json!({ "driver_id": "fake" }),
            ),
        )
        .await;
        let open = resp_of(recv(r).await);
        assert!(open.body.is_ok());
    }

    #[tokio::test]
    async fn echo_round_trips_through_worker() {
        let (mut w, mut r) = start();
        init_and_open(&mut w, &mut r).await;
        send(
            &mut w,
            req(3, "echo", serde_json::json!({ "conn_id": 1, "v": 42 })),
        )
        .await;
        let resp = resp_of(recv(&mut r).await);
        assert_eq!(resp.result().unwrap()["v"], 42);
    }

    #[tokio::test]
    async fn rejects_before_init() {
        let (mut w, mut r) = start();
        send(&mut w, req(1, "echo", serde_json::json!({ "conn_id": 1 }))).await;
        let resp = resp_of(recv(&mut r).await);
        assert_eq!(resp.error().unwrap().code, error_codes::NOT_INITIALIZED);
    }

    #[tokio::test]
    async fn unknown_conn_id_errs() {
        let (mut w, mut r) = start();
        init_and_open(&mut w, &mut r).await;
        send(
            &mut w,
            req(3, "echo", serde_json::json!({ "conn_id": 999 })),
        )
        .await;
        let resp = resp_of(recv(&mut r).await);
        assert_eq!(resp.error().unwrap().code, error_codes::UNKNOWN_CONN_ID);
    }

    #[tokio::test]
    async fn connless_method_routes_to_driver() {
        let (mut w, mut r) = start();
        init_and_open(&mut w, &mut r).await;
        send(&mut w, req(3, "ddl/noop", serde_json::json!({}))).await;
        let resp = resp_of(recv(&mut r).await);
        assert_eq!(resp.result().unwrap()["connless"], true);
    }

    #[tokio::test]
    async fn cursor_fetch_routes_by_cursor_id_without_conn_id() {
        let (mut w, mut r) = start();
        init_and_open(&mut w, &mut r).await;
        send(
            &mut w,
            req(
                3,
                method::QUERY_START,
                serde_json::json!({ "conn_id": 1, "sql": "select 1" }),
            ),
        )
        .await;
        let started = resp_of(recv(&mut r).await);
        assert_eq!(started.result().unwrap()["cursor_id"], "cursor-1");

        send(
            &mut w,
            req(
                4,
                method::CURSOR_FETCH,
                serde_json::json!({ "cursor_id": "cursor-1", "n": 10 }),
            ),
        )
        .await;
        let fetched = resp_of(recv(&mut r).await);

        assert_eq!(fetched.result().unwrap()["rows"][0][0]["value"], 7);
    }

    #[tokio::test]
    async fn stream_read_routes_by_stream_id_without_conn_id() {
        let (mut w, mut r) = start();
        init_and_open(&mut w, &mut r).await;
        send(
            &mut w,
            req(
                3,
                method::DATA_EXPORT,
                serde_json::json!({
                    "conn_id": 1,
                    "sql": "select 1",
                    "format": "csv",
                    "stream_id": "stream-1"
                }),
            ),
        )
        .await;
        let exported = resp_of(recv(&mut r).await);
        assert_eq!(exported.result().unwrap()["estimated_rows"], 1);

        send(
            &mut w,
            req(
                4,
                method::STREAM_READ,
                serde_json::json!({ "stream_id": "stream-1", "max_bytes": 16 }),
            ),
        )
        .await;
        let read = resp_of(recv(&mut r).await);

        assert_eq!(read.result().unwrap()["data"], "aGVsbG8=");
    }

    #[tokio::test]
    async fn failed_cursor_close_removes_cursor_route() {
        let (mut w, mut r) = start();
        init_and_open(&mut w, &mut r).await;
        send(
            &mut w,
            req(
                3,
                method::QUERY_START,
                serde_json::json!({ "conn_id": 1, "sql": "select 1" }),
            ),
        )
        .await;
        assert_eq!(
            resp_of(recv(&mut r).await).result().unwrap()["cursor_id"],
            "cursor-1"
        );

        send(
            &mut w,
            req(
                4,
                method::CURSOR_CLOSE,
                serde_json::json!({ "cursor_id": "cursor-1" }),
            ),
        )
        .await;
        assert_eq!(
            resp_of(recv(&mut r).await).error().unwrap().code,
            error_codes::METHOD_NOT_FOUND
        );

        send(
            &mut w,
            req(
                5,
                method::CURSOR_FETCH,
                serde_json::json!({ "cursor_id": "cursor-1", "n": 10 }),
            ),
        )
        .await;
        assert_eq!(
            resp_of(recv(&mut r).await).error().unwrap().code,
            error_codes::UNKNOWN_CURSOR_ID
        );
    }

    #[tokio::test]
    async fn import_chunk_routes_by_import_id_without_conn_id() {
        let (mut w, mut r) = start();
        init_and_open(&mut w, &mut r).await;
        send(
            &mut w,
            req(
                3,
                method::DATA_IMPORT_BEGIN,
                serde_json::json!({
                    "conn_id": 1,
                    "table": "users",
                    "format": "json",
                    "columns": ["id"]
                }),
            ),
        )
        .await;
        let begun = resp_of(recv(&mut r).await);
        assert_eq!(begun.result().unwrap()["import_id"], "import-1");

        send(
            &mut w,
            req(
                4,
                method::DATA_IMPORT_CHUNK,
                serde_json::json!({
                    "import_id": "import-1",
                    "rows": [[{"type": "i64", "value": 1}]]
                }),
            ),
        )
        .await;
        let chunked = resp_of(recv(&mut r).await);

        assert_eq!(chunked.result().unwrap()["inserted"], 1);
    }

    #[tokio::test]
    async fn failed_stream_close_removes_stream_route() {
        let (mut w, mut r) = start();
        init_and_open(&mut w, &mut r).await;
        send(
            &mut w,
            req(
                3,
                method::DATA_EXPORT,
                serde_json::json!({
                    "conn_id": 1,
                    "sql": "select 1",
                    "format": "csv",
                    "stream_id": "stream-stale"
                }),
            ),
        )
        .await;
        assert!(resp_of(recv(&mut r).await).body.is_ok());

        send(
            &mut w,
            req(
                4,
                method::STREAM_CLOSE,
                serde_json::json!({ "stream_id": "stream-stale" }),
            ),
        )
        .await;
        assert_eq!(
            resp_of(recv(&mut r).await).error().unwrap().code,
            error_codes::RESOURCE_CLOSED
        );

        send(
            &mut w,
            req(
                5,
                method::STREAM_READ,
                serde_json::json!({ "stream_id": "stream-stale", "max_bytes": 16 }),
            ),
        )
        .await;
        assert_eq!(
            resp_of(recv(&mut r).await).error().unwrap().code,
            error_codes::RESOURCE_CLOSED
        );
    }

    #[tokio::test]
    async fn failed_import_abort_removes_import_route() {
        let (mut w, mut r) = start();
        init_and_open(&mut w, &mut r).await;
        send(
            &mut w,
            req(
                3,
                method::DATA_IMPORT_BEGIN,
                serde_json::json!({
                    "conn_id": 1,
                    "table": "users",
                    "format": "json",
                    "columns": ["id"]
                }),
            ),
        )
        .await;
        assert_eq!(
            resp_of(recv(&mut r).await).result().unwrap()["import_id"],
            "import-1"
        );

        send(
            &mut w,
            req(
                4,
                method::DATA_IMPORT_ABORT,
                serde_json::json!({ "import_id": "import-1" }),
            ),
        )
        .await;
        assert_eq!(
            resp_of(recv(&mut r).await).error().unwrap().code,
            error_codes::RESOURCE_CLOSED
        );

        send(
            &mut w,
            req(
                5,
                method::DATA_IMPORT_CHUNK,
                serde_json::json!({
                    "import_id": "import-1",
                    "rows": [[{"type": "i64", "value": 1}]]
                }),
            ),
        )
        .await;
        assert_eq!(
            resp_of(recv(&mut r).await).error().unwrap().code,
            error_codes::UNKNOWN_IMPORT_ID
        );
    }

    #[tokio::test]
    async fn connless_answered_while_slow_query_runs() {
        let (mut w, mut r) = start();
        init_and_open(&mut w, &mut r).await;
        send(&mut w, req(10, "slow", serde_json::json!({ "conn_id": 1 }))).await;
        send(&mut w, req(11, "ddl/noop", serde_json::json!({}))).await;

        let first = resp_of(recv(&mut r).await);
        assert_eq!(first.id, RequestId::Number(11));
        assert_eq!(first.result().unwrap()["connless"], true);
    }

    #[tokio::test]
    async fn ping_answered_while_slow_connless_runs() {
        let (mut w, mut r) = start();
        init_and_open(&mut w, &mut r).await;
        send(&mut w, req(10, "ddl/slow", serde_json::json!({}))).await;
        send(&mut w, req(11, method::PING, serde_json::json!({}))).await;

        let first = tokio::time::timeout(Duration::from_millis(200), recv(&mut r))
            .await
            .expect("ping should not wait for connless work");
        let first = resp_of(first);
        assert_eq!(first.id, RequestId::Number(11));
        assert!(first.result().unwrap()["pong"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn ping_answered_while_slow_conn_open_runs() {
        let (mut w, mut r) = start();
        send(&mut w, req(1, method::INIT, serde_json::json!({}))).await;
        let init = resp_of(recv(&mut r).await);
        assert!(init.body.is_ok());

        send(
            &mut w,
            req(
                2,
                method::CONN_OPEN,
                serde_json::json!({ "driver_id": "fake", "slow": true }),
            ),
        )
        .await;
        send(&mut w, req(3, method::PING, serde_json::json!({}))).await;

        let first = tokio::time::timeout(Duration::from_millis(200), recv(&mut r))
            .await
            .expect("ping should not wait for conn/open");
        let first = resp_of(first);
        assert_eq!(first.id, RequestId::Number(3));
        assert!(first.result().unwrap()["pong"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn cancelled_conn_open_does_not_register_worker() {
        let (mut w, mut r) = start();
        send(&mut w, req(1, method::INIT, serde_json::json!({}))).await;
        let init = resp_of(recv(&mut r).await);
        assert!(init.body.is_ok());

        send(
            &mut w,
            req(
                2,
                method::CONN_OPEN,
                serde_json::json!({ "driver_id": "fake", "slow": true }),
            ),
        )
        .await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        send(
            &mut w,
            RpcMessage::Notification(Notification::new(
                method::CANCEL_REQUEST,
                serde_json::json!({ "id": 2 }),
            )),
        )
        .await;

        let open = tokio::time::timeout(Duration::from_secs(1), recv(&mut r))
            .await
            .expect("cancelled conn/open should respond after open task exits");
        let open = resp_of(open);
        assert_eq!(open.id, RequestId::Number(2));
        assert_eq!(open.error().unwrap().code, error_codes::REQUEST_CANCELLED);

        send(
            &mut w,
            req(3, "echo", serde_json::json!({ "conn_id": 1, "v": 42 })),
        )
        .await;
        let echo = resp_of(recv(&mut r).await);
        assert_eq!(echo.error().unwrap().code, error_codes::UNKNOWN_CONN_ID);
    }

    #[tokio::test]
    async fn ping_answered_while_slow_query_runs() {
        let (mut w, mut r) = start();
        init_and_open(&mut w, &mut r).await;
        // 起一个慢查询(id=10),不等它返回。
        send(&mut w, req(10, "slow", serde_json::json!({ "conn_id": 1 }))).await;
        // 立刻 ping(id=11)。
        send(&mut w, req(11, method::PING, serde_json::json!({}))).await;
        // 第一个回来的应该是 ping,而不是 slow——证明 reader 没被阻塞。
        let first = resp_of(recv(&mut r).await);
        assert_eq!(first.id, RequestId::Number(11));
        assert!(first.result().unwrap()["pong"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn cancel_aborts_running_query() {
        let (mut w, mut r) = start();
        init_and_open(&mut w, &mut r).await;
        send(&mut w, req(10, "slow", serde_json::json!({ "conn_id": 1 }))).await;
        // 给 worker 一点时间真正开跑。
        tokio::time::sleep(Duration::from_millis(50)).await;
        send(
            &mut w,
            RpcMessage::Notification(Notification::new(
                method::CANCEL_REQUEST,
                serde_json::json!({ "id": 10 }),
            )),
        )
        .await;
        // slow 自然完成要 ~2s;取消后应远早于此返回 cancelled。
        let resp = tokio::time::timeout(Duration::from_secs(1), recv(&mut r))
            .await
            .expect("cancel should return promptly");
        let resp = resp_of(resp);
        assert_eq!(resp.id, RequestId::Number(10));
        match resp.body {
            ResponseBody::Err { error } => {
                assert_eq!(error.code, error_codes::REQUEST_CANCELLED)
            }
            other => panic!("expected cancelled error, got {other:?}"),
        }
    }
}
