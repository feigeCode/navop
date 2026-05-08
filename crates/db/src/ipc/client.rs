use crate::connection::DbError;
use crate::ipc::registry::IpcDriverManifest;
use interprocess::local_socket::{
    GenericNamespaced,
    tokio::{Stream as LocalSocketStream, prelude::*},
};
use ipc::{
    IpcErrorCode, IpcRequest, IpcResponse,
    framing::{recv_msg_async, send_msg_async},
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader, ReadHalf, WriteHalf, split};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep, timeout};
use tracing::warn;

const REQUEST_TIMEOUT_MS: u64 = 30_000;

/// 通过该环境变量把 client 生成的动态 socket 名透传给 driver 子进程。
///
/// driver 启动时优先读这个变量来决定 listen 名,从而支持「同 driver 多实例」
/// 场景:每个 ExternalDbConnection 都拿到独立的 socket,互不冲突。
pub const SOCKET_ENV_VAR: &str = "ONETCLI_IPC_SOCKET";

/// 客户端「写半 / 路由表 / 关闭标记」共享状态。
///
/// - `writer`: tokio::sync::Mutex 串行化「写一帧」操作。
/// - `pending`: std::sync::Mutex 持锁时间极短(insert/remove HashMap),且允许在
///   Drop 中同步 lock,这是 cancel-safety 的关键。
/// - `next_id`: AtomicU64,无锁分配 request id。
/// - `closed`: AtomicBool,reader task 退出后置位,后续 caller 立即失败。
struct ClientShared {
    writer: Mutex<WriteHalf<LocalSocketStream>>,
    pending: StdMutex<HashMap<u64, oneshot::Sender<IpcResponse>>>,
    next_id: AtomicU64,
    closed: AtomicBool,
}

/// JSON-RPC over IPC client。
///
/// 单 stream 多 caller 并发:writer mutex 串行化写,reader task 把响应按
/// `request_id` 路由到对应 caller 的 oneshot。caller drop / timeout / 写失败
/// 均不会泄漏 pending 表条目(由 PendingGuard 的 RAII Drop 保证)。
pub struct JsonRpcClient {
    shared: Arc<ClientShared>,
    reader_task: JoinHandle<()>,
    /// 子进程 owner;包在 std Mutex 里以让 `JsonRpcClient: Sync`。
    /// `kill_on_drop=true` 保证 child 被 drop 时进程被 OS 回收。
    child: StdMutex<Option<Child>>,
}

impl JsonRpcClient {
    pub async fn start(driver: &IpcDriverManifest) -> Result<Self, DbError> {
        // command 为空 → 测试 / 预 listen 模式:server 已绑定 transport.name,直接连。
        // 否则 → 生产模式:每实例生成独立 socket 名,通过 env var 透传给 driver。
        let socket_name = if driver.entry.command.trim().is_empty() {
            driver.transport.name.clone()
        } else {
            make_socket_name(driver)
        };

        let mut child = if driver.entry.command.trim().is_empty() {
            None
        } else {
            Some(spawn_driver_process(driver, &socket_name).await?)
        };

        let stream =
            match connect_local_socket(&socket_name, driver.transport.connect_timeout_ms()).await {
                Ok(stream) => stream,
                Err(error) => {
                    shutdown_child(&mut child).await;
                    return Err(error);
                }
            };

        let (read_half, write_half) = split(stream);

        let shared = Arc::new(ClientShared {
            writer: Mutex::new(write_half),
            pending: StdMutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            closed: AtomicBool::new(false),
        });

        let reader_shared = Arc::clone(&shared);
        let reader_task = tokio::spawn(async move {
            reader_loop(read_half, reader_shared).await;
        });

        Ok(Self {
            shared,
            reader_task,
            child: StdMutex::new(child),
        })
    }

    pub async fn request<T>(&self, method: &str, params: Value) -> Result<T, DbError>
    where
        T: DeserializeOwned,
    {
        let value = self.request_value(method, params).await?;
        serde_json::from_value(value)
            .map_err(|error| DbError::query_with_source("invalid external driver response", error))
    }

    pub async fn request_value(&self, method: &str, params: Value) -> Result<Value, DbError> {
        if self.shared.closed.load(Ordering::Acquire) {
            return Err(DbError::connection("driver disconnected"));
        }

        let id = self.shared.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();

        // 注册 pending,double-check closed 防止 reader 已 drain。
        {
            let mut pending = self.shared.pending.lock().expect("pending mutex poisoned");
            if self.shared.closed.load(Ordering::Acquire) {
                return Err(DbError::connection("driver disconnected"));
            }
            pending.insert(id, tx);
        }

        // RAII guard:future cancel / timeout / 写失败时从 pending 拿掉 sender,避免泄漏。
        let mut guard = PendingGuard {
            shared: Arc::clone(&self.shared),
            id,
            armed: true,
        };

        // 写一帧;writer mutex 仅在写期间持锁,写完立刻释放允许下个 caller 写。
        let request = IpcRequest::new(id, method, params);
        let send_result = {
            let mut writer = self.shared.writer.lock().await;
            send_msg_async(&mut *writer, &request).await
        };
        if let Err(error) = send_result {
            return Err(DbError::query_with_source(
                "failed to write IPC request",
                error,
            ));
            // guard.drop → remove pending entry
        }

        // 等回复。
        match timeout(Duration::from_millis(REQUEST_TIMEOUT_MS), rx).await {
            Ok(Ok(response)) => {
                guard.armed = false; // reader 已 take 走 sender,不需再清理
                validate_response(response, id)
            }
            Ok(Err(_)) => {
                guard.armed = false; // reader 关闭已 drain pending
                Err(DbError::connection("driver disconnected"))
            }
            Err(_) => {
                // timeout:guard.drop 清理 sender,reader 后到的 response 静默丢弃
                Err(DbError::query("timed out waiting for IPC response"))
            }
        }
    }

    /// 显式关闭:abort reader,kill + wait child。
    /// 通常在 ExternalDbConnection::disconnect 末尾调用,确保子进程退出后才返回。
    pub async fn shutdown(&self) {
        close_and_drain(&self.shared);
        self.reader_task.abort();
        let mut taken = {
            let mut guard = self.child.lock().expect("child mutex poisoned");
            guard.take()
        };
        shutdown_child(&mut taken).await;
    }

    /// reader task 是否已经退出(stream EOF / error / abort)。
    ///
    /// 一旦置位,所有后续 `request` 调用都会立即得到 disconnected 错误。
    /// ExternalDbConnection 用这个信号触发 client eviction(P0-4)。
    pub fn is_closed(&self) -> bool {
        self.shared.closed.load(Ordering::Acquire)
    }
}

impl Drop for JsonRpcClient {
    fn drop(&mut self) {
        // 兜底:abort reader task,child 由 kill_on_drop=true 自动回收。
        // 不在 Drop 里 await,避免阻塞 runtime。
        self.reader_task.abort();
    }
}

/// RAII 保护 pending 表条目的 cancel-safety。
///
/// 当 caller 的 future 被 cancel / timeout / 写失败时,Drop 自动移除 pending sender,
/// 避免内存泄漏与 reader 找不到对应 caller 时的隐性丢弃。
struct PendingGuard {
    shared: Arc<ClientShared>,
    id: u64,
    armed: bool,
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Ok(mut pending) = self.shared.pending.lock() {
            pending.remove(&self.id);
        }
    }
}

fn close_and_drain(shared: &ClientShared) {
    shared.closed.store(true, Ordering::Release);
    if let Ok(mut pending) = shared.pending.lock() {
        pending.clear();
        // oneshot::Sender 被 drop → caller 的 rx 收 RecvError → 报 disconnected
    }
}

async fn reader_loop(mut reader: ReadHalf<LocalSocketStream>, shared: Arc<ClientShared>) {
    /// 无论 reader_loop 怎么退出(EOF / Err / task abort),都标记 closed + drain
    /// pending,把所有 caller 唤醒为 disconnected。
    struct CloseGuard {
        shared: Arc<ClientShared>,
    }
    impl Drop for CloseGuard {
        fn drop(&mut self) {
            close_and_drain(&self.shared);
        }
    }
    let _guard = CloseGuard {
        shared: Arc::clone(&shared),
    };

    while let Ok(response) = recv_msg_async::<_, IpcResponse>(&mut reader).await {
        let sender = match shared.pending.lock() {
            Ok(mut pending) => pending.remove(&response.request_id),
            Err(_) => break, // pending mutex poisoned — 走 CloseGuard 兜底
        };
        if let Some(sender) = sender {
            // caller 已超时 / cancel drop 了 rx 时 send 失败 — 静默忽略
            let _ = sender.send(response);
        }
        // 找不到 sender:caller 已 timeout / cancel,response 静默丢弃
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
            expected_id, response.request_id
        )));
    }
    if let Some(error) = response.error {
        if error.code == IpcErrorCode::UnsupportedMethod {
            return Err(DbError::NotSupported(error.message));
        }
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

/// 为 driver 生成本次启动的最终 socket 名。
///
/// 使用短前缀避免 macOS `sockaddr_un.sun_path` 容量限制。
fn make_socket_name(driver: &IpcDriverManifest) -> String {
    format!(
        "onetcli-{}-{}.sock",
        driver.id,
        uuid::Uuid::new_v4().simple()
    )
}

/// 构造 driver 启动 Command,设置 `ONETCLI_IPC_SOCKET` env var 把动态 socket
/// 名透传给子进程。抽出独立函数便于在 Drop / multi-instance 测试中验证 env。
fn build_driver_command(driver: &IpcDriverManifest, socket_name: &str) -> Command {
    let mut command = Command::new(&driver.entry.command);
    command
        .args(&driver.entry.args)
        .env(SOCKET_ENV_VAR, socket_name)
        .current_dir(driver.command_working_dir())
        // 关键:确保 client 异常 drop 时子进程被回收,不变孤儿。
        // 详见 P0-3 改造 — 仅 `Child::kill().await` 不足以应对 panic / runtime abort 场景。
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    command
}

async fn spawn_driver_process(
    driver: &IpcDriverManifest,
    socket_name: &str,
) -> Result<Child, DbError> {
    let mut command = build_driver_command(driver, socket_name);
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

    #[test]
    fn maps_unsupported_method_to_not_supported() {
        let response = IpcResponse::error(7, IpcErrorCode::UnsupportedMethod, "missing");
        let result = validate_response(response, 7);
        assert!(matches!(result, Err(DbError::NotSupported(message)) if message == "missing"));
    }

    #[test]
    fn make_socket_name_generates_distinct_names_with_manifest_prefix() {
        let driver = make_test_manifest("driver.sock");
        let first = make_socket_name(&driver);
        let second = make_socket_name(&driver);

        assert_ne!(first, second);
        assert!(first.starts_with("onetcli-socket-test-"));
        assert!(second.starts_with("onetcli-socket-test-"));
        assert!(first.ends_with(".sock"));
        assert!(second.ends_with(".sock"));
    }

    fn make_test_manifest(socket_name: &str) -> IpcDriverManifest {
        IpcDriverManifest {
            id: "socket-test".into(),
            name: "Socket Test".into(),
            description: String::new(),
            version: String::new(),
            entry: crate::ipc::registry::IpcDriverEntry {
                command: "sleep".into(),
                args: vec!["30".into()],
                working_dir: None,
            },
            transport: crate::ipc::registry::IpcDriverTransport::local_socket(socket_name),
            dialect: Default::default(),
            capabilities: None,
            ui: Default::default(),
            manifest_dir: std::path::PathBuf::from("/tmp"),
        }
    }
}

#[cfg(all(test, unix))]
mod lifecycle_tests {
    use super::*;
    use crate::ipc::registry::{IpcDriverEntry, IpcDriverManifest, IpcDriverTransport};
    use std::path::PathBuf;
    use std::time::Duration;

    /// 构造一个跑 `sleep 30` 的 manifest,作为「永远不会主动退出」的 driver 占位。
    fn make_sleep_manifest() -> IpcDriverManifest {
        IpcDriverManifest {
            id: "lifecycle-test".into(),
            name: "Lifecycle Test".into(),
            description: String::new(),
            version: String::new(),
            entry: IpcDriverEntry {
                command: "sleep".into(),
                args: vec!["30".into()],
                working_dir: None,
            },
            transport: IpcDriverTransport::local_socket("onetcli-lifecycle-test.sock"),
            dialect: Default::default(),
            capabilities: None,
            ui: Default::default(),
            manifest_dir: PathBuf::from("/tmp"),
        }
    }

    /// 通过 `kill -0 <pid>` 检测 unix 进程是否仍存活。
    fn process_alive(pid: u32) -> bool {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// 兜底回收:测试失败时也不要把 sleep 进程留给 CI。
    fn force_kill(pid: u32) {
        let _ = std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .status();
    }

    #[tokio::test]
    async fn spawn_driver_process_kills_child_when_handle_drops() {
        let manifest = make_sleep_manifest();
        let socket_name = manifest.transport.name.clone();
        let child = spawn_driver_process(&manifest, &socket_name)
            .await
            .expect("spawn driver child process");
        let pid = child.id().expect("child pid should be available");

        assert!(
            process_alive(pid),
            "child should be alive immediately after spawn"
        );

        drop(child);

        // 给 OS 至多 2 秒时间发送信号并清理 zombie。
        let mut reaped = false;
        for _ in 0..20 {
            if !process_alive(pid) {
                reaped = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        if !reaped {
            force_kill(pid);
        }
        assert!(
            reaped,
            "child pid={pid} should be killed within 2s after Child handle drops"
        );
    }
}
