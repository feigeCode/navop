//! 按配置目录隔离的 Windows 单实例门禁。
//!
//! 设计要点：
//!
//! - **互斥语义**由命名管道的 `FILE_FLAG_FIRST_PIPE_INSTANCE` 提供：同名管道
//!   只允许创建首个实例。抢到的进程才是主实例，其余进程创建必然失败，因此
//!   "谁拥有这个配置目录"不依赖任何猜测。
//! - 建不出首个实例只说明"名字被占用"，此时尝试连接并把启动请求转发给主实例。
//! - 主实例在每次接受连接后**先补建下一个实例、再移交当前连接**，保证任意时刻
//!   都至少有一个实例持有该名称，别的进程不会在这条缝隙里抢到主实例资格。
//! - 所有 I/O 都有明确期限，半个请求不会拖住后续启动请求。
//!
//! 此前用的是 `interprocess` 2.4.4，它在 Windows 上有两处不可用：`set_recv_timeout`
//! / `set_send_timeout` 直接返回 `ErrorKind::Unsupported`（"named pipes do not
//! support I/O timeouts"），且名字被占用时报的是 `ErrorKind::PermissionDenied`
//! 而不是 `AddrInUse`。这里改用 tokio 命名管道 + [`tokio::time::timeout`]，
//! 并按原始 Win32 错误码判断占用。

use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

const MAX_PATH_COUNT: usize = 256;
const MAX_PATH_BYTES: usize = 32 * 1024;
const MAX_PAYLOAD_BYTES: usize = 4 * 1024 * 1024;

/// 主实例回执：请求已经进入启动队列。
#[cfg(target_os = "windows")]
const ACKNOWLEDGEMENT_ACCEPTED: u8 = 1;
/// 主实例回执：启动队列不可用，请求没有被接管。
#[cfg(target_os = "windows")]
const ACKNOWLEDGEMENT_REJECTED: u8 = 0;

/// 单条启动请求的读写期限，防止半开连接长期占用处理任务。
#[cfg(all(target_os = "windows", not(test)))]
const REQUEST_IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
/// 测试里缩短，让"半开连接被超时断开"可以在毫秒级被断言。
#[cfg(all(target_os = "windows", test))]
const REQUEST_IO_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(200);

/// 取得主实例资格（或把请求交给主实例）的总期限。
///
/// 超过它只能明确失败：把"无法建立通信"降级成"可以另开一个实例"会直接产出
/// 第二个 GUI，而第二个窗口既没有单实例语义、也分不清谁该处理启动文件。
#[cfg(all(target_os = "windows", not(test)))]
const CLAIM_DEADLINE: std::time::Duration = std::time::Duration::from_secs(2);
#[cfg(all(target_os = "windows", test))]
const CLAIM_DEADLINE: std::time::Duration = std::time::Duration::from_millis(400);

/// 等待工作线程上报判定的上限，仅用于兜底线程失联。
#[cfg(target_os = "windows")]
const DECISION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// 连接重试间隔：主实例切换管道实例的瞬间会短暂无法连接。
#[cfg(target_os = "windows")]
const CONNECT_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(20);

#[cfg(target_os = "windows")]
const ERROR_FILE_NOT_FOUND: i32 = 2;
#[cfg(target_os = "windows")]
const ERROR_ACCESS_DENIED: i32 = 5;
#[cfg(target_os = "windows")]
const ERROR_PIPE_BUSY: i32 = 231;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StartupRequest {
    paths: Vec<PathBuf>,
}

impl StartupRequest {
    pub(crate) fn new(paths: Vec<PathBuf>) -> Self {
        Self { paths }
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn into_paths(self) -> Vec<PathBuf> {
        self.paths
    }
}

#[cfg(target_os = "windows")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SingleInstanceOutcome {
    /// 本进程抢到了主实例资格，应当继续启动 GUI 并服务后续启动请求。
    Primary,
    /// 已有实例接管了启动请求，本进程应当退出。
    Forwarded,
}

#[cfg(target_os = "windows")]
#[derive(Debug)]
pub(crate) enum SingleInstanceError {
    /// 已经有实例占着这个配置目录，但本次启动请求没能交给它。
    Forward(io::Error),
    /// 既拿不到主实例资格，也没能和已有实例通信。
    Listen(io::Error),
}

#[cfg(target_os = "windows")]
impl SingleInstanceError {
    /// 面向用户的中文提示。release 构建没有控制台，调用方需要弹窗展示。
    pub(crate) fn user_message(&self) -> &'static str {
        match self {
            Self::Forward(_) => {
                "Navop 已经在运行，但本次启动请求没能交给它。\n\n请先查看桌面上已有的 Navop 窗口；如果它没有响应，结束该进程后重新启动。"
            }
            Self::Listen(_) => {
                "无法建立 Navop 的单实例监听，也没有找到可以通信的已有实例。\n\n为避免同时打开多个 Navop 窗口，本次启动已取消。\n\n请稍后重试；如果问题持续，请结束所有 Navop 进程后重新启动。"
            }
        }
    }
}

#[cfg(target_os = "windows")]
impl std::fmt::Display for SingleInstanceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Forward(error) => write!(
                f,
                "failed to hand the startup request over to the running instance: {error}"
            ),
            Self::Listen(error) => write!(f, "failed to claim the primary instance: {error}"),
        }
    }
}

#[cfg(target_os = "windows")]
impl std::error::Error for SingleInstanceError {}

fn instance_name_for_config_dir(config_dir: &Path) -> String {
    let path = config_dir.to_string_lossy();
    #[cfg(target_os = "windows")]
    let path = path.to_lowercase();

    let digest = Sha256::digest(path.as_bytes());
    let suffix = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("navop-single-instance-{suffix}")
}

#[cfg(target_os = "windows")]
fn pipe_name_for_config_dir(config_dir: &Path) -> String {
    format!(r"\\.\pipe\{}", instance_name_for_config_dir(config_dir))
}

fn encode_request(request: &StartupRequest) -> io::Result<Vec<u8>> {
    if request.paths.len() > MAX_PATH_COUNT {
        return Err(invalid_data("too many startup paths"));
    }

    let mut payload = Vec::new();
    append_u32(&mut payload, request.paths.len())?;
    for path in &request.paths {
        let bytes = encode_path(path);
        if bytes.len() > MAX_PATH_BYTES {
            return Err(invalid_data("startup path is too long"));
        }
        append_u32(&mut payload, bytes.len())?;
        payload.extend_from_slice(&bytes);
        if payload.len() > MAX_PAYLOAD_BYTES {
            return Err(invalid_data("startup request is too large"));
        }
    }
    Ok(payload)
}

fn decode_request(payload: &[u8]) -> io::Result<StartupRequest> {
    if payload.len() > MAX_PAYLOAD_BYTES {
        return Err(invalid_data("startup request is too large"));
    }

    let mut cursor = 0;
    let path_count = read_u32(payload, &mut cursor)?;
    if path_count > MAX_PATH_COUNT {
        return Err(invalid_data("too many startup paths"));
    }

    let mut paths = Vec::with_capacity(path_count);
    for _ in 0..path_count {
        let path_length = read_u32(payload, &mut cursor)?;
        if path_length > MAX_PATH_BYTES {
            return Err(invalid_data("startup path is too long"));
        }
        let path_end = cursor
            .checked_add(path_length)
            .ok_or_else(|| invalid_data("startup path length overflow"))?;
        let path_bytes = payload
            .get(cursor..path_end)
            .ok_or_else(|| invalid_data("truncated startup path"))?;
        paths.push(decode_path(path_bytes)?);
        cursor = path_end;
    }
    if cursor != payload.len() {
        return Err(invalid_data("startup request contains trailing data"));
    }

    Ok(StartupRequest::new(paths))
}

#[cfg(target_os = "windows")]
fn encode_path(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt as _;

    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}

#[cfg(not(target_os = "windows"))]
fn encode_path(path: &Path) -> Vec<u8> {
    path.to_string_lossy().into_owned().into_bytes()
}

#[cfg(target_os = "windows")]
fn decode_path(bytes: &[u8]) -> io::Result<PathBuf> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt as _;

    if !bytes.len().is_multiple_of(size_of::<u16>()) {
        return Err(invalid_data("startup path has truncated UTF-16 data"));
    }
    let wide = bytes
        .chunks_exact(size_of::<u16>())
        .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
        .collect::<Vec<_>>();
    Ok(PathBuf::from(OsString::from_wide(&wide)))
}

#[cfg(not(target_os = "windows"))]
fn decode_path(bytes: &[u8]) -> io::Result<PathBuf> {
    let path =
        std::str::from_utf8(bytes).map_err(|_| invalid_data("startup path is not valid UTF-8"))?;
    Ok(PathBuf::from(path))
}

fn append_u32(buffer: &mut Vec<u8>, value: usize) -> io::Result<()> {
    let value = u32::try_from(value).map_err(|_| invalid_data("length exceeds wire format"))?;
    buffer.extend_from_slice(&value.to_le_bytes());
    Ok(())
}

fn read_u32(payload: &[u8], cursor: &mut usize) -> io::Result<usize> {
    let end = cursor
        .checked_add(size_of::<u32>())
        .ok_or_else(|| invalid_data("startup request length overflow"))?;
    let bytes: [u8; 4] = payload
        .get(*cursor..end)
        .ok_or_else(|| invalid_data("truncated startup request"))?
        .try_into()
        .expect("u32 payload slice must have four bytes");
    *cursor = end;
    Ok(u32::from_le_bytes(bytes) as usize)
}

fn invalid_data(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(target_os = "windows")]
enum Claim {
    Primary(tokio::net::windows::named_pipe::NamedPipeServer),
    Forwarded,
}

#[cfg(target_os = "windows")]
enum ForwardAttempt {
    /// 主实例确认请求已经进入启动队列。
    Delivered,
    /// 连不上主实例：它可能正在切换管道实例，或刚刚退出。
    Unreachable(io::Error),
    /// 连上了，但请求被拒或链路中断，属于确定失败。
    Rejected(io::Error),
}

/// 同一个配置目录只允许一个 GUI 实例。
///
/// 返回 [`SingleInstanceOutcome::Primary`] 表示本进程已经取得主实例资格（管道
/// 实例已建立并处于监听状态），可以继续启动 GUI；返回
/// [`SingleInstanceOutcome::Forwarded`] 表示启动请求已被已有实例接管，调用方需要
/// 直接退出。任何 `Err` 都必须终止当前进程：它意味着"既当不了主实例、也联系不上
/// 已有实例"，此时继续启动就会产出第二个窗口。
///
/// `on_request` 的返回值表示启动请求是否成功进入主实例的处理队列；只有成功入队
/// 才会回成功 ACK，不需要等文件真正打开。
#[cfg(target_os = "windows")]
pub(crate) fn claim_or_forward(
    config_dir: &Path,
    request: StartupRequest,
    on_request: impl Fn(StartupRequest) -> bool + Send + Sync + 'static,
) -> Result<SingleInstanceOutcome, SingleInstanceError> {
    let pipe_name = pipe_name_for_config_dir(config_dir);
    let on_request: std::sync::Arc<dyn Fn(StartupRequest) -> bool + Send + Sync> =
        std::sync::Arc::new(on_request);
    let (outcome_tx, outcome_rx) = std::sync::mpsc::sync_channel(1);

    std::thread::Builder::new()
        .name("navop-single-instance".to_string())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = outcome_tx.send(Err(SingleInstanceError::Listen(error)));
                    return;
                }
            };

            runtime.block_on(async move {
                match claim_or_forward_async(&pipe_name, &request).await {
                    Ok(Claim::Forwarded) => {
                        let _ = outcome_tx.send(Ok(SingleInstanceOutcome::Forwarded));
                    }
                    Ok(Claim::Primary(server)) => {
                        // 首个管道实例已经建好，此后"本进程拥有这个配置目录"
                        // 才成立；先上报判定，再进入服务循环。
                        let _ = outcome_tx.send(Ok(SingleInstanceOutcome::Primary));
                        serve_forever(pipe_name, server, on_request).await;
                    }
                    Err(error) => {
                        let _ = outcome_tx.send(Err(error));
                    }
                }
            });
        })
        .map_err(SingleInstanceError::Listen)?;

    match outcome_rx.recv_timeout(DECISION_TIMEOUT) {
        Ok(outcome) => outcome,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            Err(SingleInstanceError::Listen(io::Error::new(
                io::ErrorKind::TimedOut,
                "the single-instance worker did not report a decision in time",
            )))
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(SingleInstanceError::Listen(
            io::Error::other("the single-instance worker stopped before reporting a decision"),
        )),
    }
}

#[cfg(target_os = "windows")]
async fn claim_or_forward_async(
    pipe_name: &str,
    request: &StartupRequest,
) -> Result<Claim, SingleInstanceError> {
    use tokio::time::Instant;

    let deadline = Instant::now() + CLAIM_DEADLINE;

    loop {
        match create_pipe_instance(pipe_name, true) {
            Ok(server) => return Ok(Claim::Primary(server)),
            Err(error) if is_name_in_use(&error) => {
                match forward_attempt(pipe_name, request).await {
                    // 已有实例真的接下了请求：本进程必须退出。
                    ForwardAttempt::Delivered => return Ok(Claim::Forwarded),
                    ForwardAttempt::Rejected(error) => {
                        return Err(SingleInstanceError::Forward(error));
                    }
                    ForwardAttempt::Unreachable(error) => {
                        // 名字被占着却连不上，可能是对方刚好退出。在总期限内
                        // 继续走"取得资格 / 转发"这条循环，但绝不绕过它。
                        if Instant::now() >= deadline {
                            return Err(SingleInstanceError::Forward(error));
                        }
                        tokio::time::sleep(CONNECT_RETRY_DELAY).await;
                    }
                }
            }
            Err(error) => return Err(SingleInstanceError::Listen(error)),
        }
    }
}

#[cfg(target_os = "windows")]
fn create_pipe_instance(
    pipe_name: &str,
    first: bool,
) -> io::Result<tokio::net::windows::named_pipe::NamedPipeServer> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let mut options = ServerOptions::new();
    // `first_pipe_instance` 只允许出现在首个实例上：它就是"同一配置目录只有一个
    // 主实例"这条互斥语义的来源。服务循环补建的实例必须不带这个标志，否则会把
    // 自己也挡在门外。
    options.first_pipe_instance(first);
    options.create(pipe_name)
}

#[cfg(target_os = "windows")]
fn is_name_in_use(error: &io::Error) -> bool {
    // 首个实例建不出来时，OS 用 ERROR_ACCESS_DENIED 表示"这个名字已经有实例了"
    // （`FILE_FLAG_FIRST_PIPE_INSTANCE` 的语义）。Rust std 把它映射成
    // `ErrorKind::PermissionDenied` 而不是 `AddrInUse`，所以只能按原始错误码判断。
    error.raw_os_error() == Some(ERROR_ACCESS_DENIED)
}

#[cfg(target_os = "windows")]
fn is_connect_retryable(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(ERROR_FILE_NOT_FOUND | ERROR_PIPE_BUSY | ERROR_ACCESS_DENIED)
    )
}

#[cfg(target_os = "windows")]
async fn forward_attempt(pipe_name: &str, request: &StartupRequest) -> ForwardAttempt {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::windows::named_pipe::ClientOptions;
    use tokio::time::timeout;

    let payload = match encode_request(request) {
        Ok(payload) => payload,
        Err(error) => return ForwardAttempt::Rejected(error),
    };
    let payload_length = match u32::try_from(payload.len()) {
        Ok(length) => length,
        Err(_) => return ForwardAttempt::Rejected(invalid_data("startup request is too large")),
    };

    let mut connection = match ClientOptions::new().open(pipe_name) {
        Ok(connection) => connection,
        Err(error) if is_connect_retryable(&error) => return ForwardAttempt::Unreachable(error),
        Err(error) => return ForwardAttempt::Rejected(error),
    };

    let send = async {
        connection.write_all(&payload_length.to_le_bytes()).await?;
        connection.write_all(&payload).await?;
        connection.flush().await
    };
    if let Err(elapsed) = timeout(REQUEST_IO_TIMEOUT, send).await {
        return ForwardAttempt::Rejected(timeout_error(
            "timed out sending the startup request",
            elapsed,
        ));
    }

    let mut acknowledgement = [0u8; 1];
    // tokio 的 `read_exact` 返回读到的字节数（不是 `()`），这里只关心是否成功。
    match timeout(
        REQUEST_IO_TIMEOUT,
        connection.read_exact(&mut acknowledgement),
    )
    .await
    {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => return ForwardAttempt::Rejected(error),
        Err(elapsed) => {
            return ForwardAttempt::Rejected(timeout_error(
                "timed out waiting for the primary instance to acknowledge the startup request",
                elapsed,
            ));
        }
    }

    match acknowledgement {
        [ACKNOWLEDGEMENT_ACCEPTED] => ForwardAttempt::Delivered,
        [ACKNOWLEDGEMENT_REJECTED] => ForwardAttempt::Rejected(invalid_data(
            "the primary instance could not accept the startup request",
        )),
        _ => ForwardAttempt::Rejected(invalid_data(
            "the primary instance sent an invalid acknowledgement",
        )),
    }
}

#[cfg(target_os = "windows")]
async fn serve_forever(
    pipe_name: String,
    first_instance: tokio::net::windows::named_pipe::NamedPipeServer,
    on_request: std::sync::Arc<dyn Fn(StartupRequest) -> bool + Send + Sync>,
) {
    let mut server = Some(first_instance);

    while let Some(instance) = server.take() {
        if let Err(error) = instance.connect().await {
            tracing::warn!(%error, "failed to accept a Windows single-instance connection");
            // 把实例放回去：宁可不再服务新连接，也不能让名称变成无主。
            server = Some(instance);
            break;
        }

        // 先补建下一个实例、再移交当前连接：任何时刻都至少有一个实例持有这个
        // 名称，别的进程不会在这条缝隙里抢到主实例资格。
        let next_instance = match create_pipe_instance(&pipe_name, false) {
            Ok(instance) => instance,
            Err(error) => {
                tracing::warn!(
                    %error,
                    "failed to create the next Windows single-instance pipe instance"
                );
                server = Some(instance);
                break;
            }
        };

        // 每条连接独立处理：一个半开连接不能拖住后续启动请求。
        tokio::spawn(handle_connection(
            instance,
            std::sync::Arc::clone(&on_request),
        ));
        server = Some(next_instance);
    }

    // 服务循环停在这里，说明管道已经无法继续接受连接。此时必须**保持名称被
    // 占用**（fail-closed）：一旦放手，另一个进程就会把自己当主实例并开出第二个
    // GUI。宁可让后续启动请求转发失败（会明确报错退出），也不要破坏"同一配置
    // 目录只有一个主实例"这条语义。
    if server.is_some() {
        tracing::error!("Windows single-instance listener stopped; keeping the pipe name claimed");
        std::future::pending::<()>().await;
    }
}

#[cfg(target_os = "windows")]
async fn handle_connection(
    mut connection: tokio::net::windows::named_pipe::NamedPipeServer,
    on_request: std::sync::Arc<dyn Fn(StartupRequest) -> bool + Send + Sync>,
) {
    use tokio::io::AsyncWriteExt as _;
    use tokio::time::timeout;

    let request = match timeout(REQUEST_IO_TIMEOUT, read_request(&mut connection)).await {
        Ok(Ok(request)) => request,
        Ok(Err(error)) => {
            tracing::warn!(%error, "failed to read a Windows single-instance request");
            return;
        }
        Err(_) => {
            tracing::warn!(
                timeout_ms = REQUEST_IO_TIMEOUT.as_millis(),
                "timed out reading a Windows single-instance request"
            );
            return;
        }
    };

    // ACK 只表示"请求已经进入主实例的启动队列"，不代表文件已经打开；入队失败
    // 必须回拒绝，否则第二个进程会误判请求已被接管而静默退出。
    let acknowledgement = [if on_request(request) {
        ACKNOWLEDGEMENT_ACCEPTED
    } else {
        ACKNOWLEDGEMENT_REJECTED
    }];
    match timeout(REQUEST_IO_TIMEOUT, connection.write_all(&acknowledgement)).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            tracing::warn!(%error, "failed to acknowledge a Windows single-instance request");
        }
        Err(_) => {
            tracing::warn!("timed out acknowledging a Windows single-instance request");
        }
    }
    if let Err(error) = connection.flush().await {
        tracing::warn!(%error, "failed to flush a Windows single-instance acknowledgement");
    }
}

#[cfg(target_os = "windows")]
async fn read_request(
    connection: &mut tokio::net::windows::named_pipe::NamedPipeServer,
) -> io::Result<StartupRequest> {
    use tokio::io::AsyncReadExt as _;

    let mut payload_length = [0; size_of::<u32>()];
    connection.read_exact(&mut payload_length).await?;
    let payload_length = u32::from_le_bytes(payload_length) as usize;
    if payload_length > MAX_PAYLOAD_BYTES {
        return Err(invalid_data("startup request is too large"));
    }

    let mut payload = vec![0; payload_length];
    connection.read_exact(&mut payload).await?;
    decode_request(&payload)
}

#[cfg(target_os = "windows")]
fn timeout_error(message: &'static str, elapsed: tokio::time::error::Elapsed) -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, format!("{message}: {elapsed}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_name_is_stable_and_scoped_to_config_directory() {
        let first = instance_name_for_config_dir(Path::new("C:/Users/alice/AppData/Navop"));
        let repeated = instance_name_for_config_dir(Path::new("C:/Users/alice/AppData/Navop"));
        let portable = instance_name_for_config_dir(Path::new("D:/Portable/Navop/config"));

        assert_eq!(first, repeated);
        assert_ne!(first, portable);
        assert!(first.starts_with("navop-single-instance-"));
        assert!(!first.contains("alice"));
    }

    #[test]
    fn startup_request_round_trips_empty_and_unicode_paths() {
        for paths in [
            Vec::new(),
            vec![
                PathBuf::from(r"C:\工作区\连接.navop"),
                PathBuf::from(r"D:\projects\demo.onetcli"),
            ],
        ] {
            let request = StartupRequest::new(paths);
            let encoded = encode_request(&request).expect("request should encode");
            let decoded = decode_request(&encoded).expect("request should decode");

            assert_eq!(request, decoded);
        }
    }

    #[test]
    fn decoder_rejects_truncated_and_oversized_payloads() {
        assert!(decode_request(&[]).is_err());
        assert!(decode_request(&[1, 0, 0]).is_err());

        let mut oversized_count = Vec::new();
        oversized_count.extend_from_slice(&((MAX_PATH_COUNT as u32) + 1).to_le_bytes());
        assert!(decode_request(&oversized_count).is_err());

        let oversized_payload = vec![0; MAX_PAYLOAD_BYTES + 1];
        assert!(decode_request(&oversized_payload).is_err());
    }

    #[cfg(target_os = "windows")]
    mod windows_transport {
        use super::*;
        use std::sync::{Arc, Mutex};
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        use tokio::net::windows::named_pipe::ClientOptions;

        /// 记录主实例收到的启动请求。
        type RequestLog = Arc<Mutex<Vec<StartupRequest>>>;

        fn temporary_config_dir() -> tempfile::TempDir {
            tempfile::tempdir().expect("temporary config directory")
        }

        fn recording_primary(
            config_dir: &Path,
            accepted: bool,
        ) -> (RequestLog, SingleInstanceOutcome) {
            let received: RequestLog = Arc::new(Mutex::new(Vec::new()));
            let sink = Arc::clone(&received);
            let outcome = claim_or_forward(
                config_dir,
                StartupRequest::new(Vec::new()),
                move |request| {
                    sink.lock().expect("request log lock").push(request);
                    accepted
                },
            )
            .expect("primary claim should succeed");
            (received, outcome)
        }

        fn claim(
            config_dir: &Path,
            paths: Vec<PathBuf>,
        ) -> Result<SingleInstanceOutcome, SingleInstanceError> {
            claim_or_forward(config_dir, StartupRequest::new(paths), |_| true)
        }

        #[test]
        fn first_claim_becomes_primary_and_second_one_forwards() {
            let config_dir = temporary_config_dir();
            let (received, outcome) = recording_primary(config_dir.path(), true);
            assert_eq!(outcome, SingleInstanceOutcome::Primary);

            let forwarded = claim(
                config_dir.path(),
                vec![PathBuf::from(r"C:\工作区\第二个实例.navop")],
            )
            .expect("second claim must forward instead of failing");

            assert_eq!(forwarded, SingleInstanceOutcome::Forwarded);
            assert_eq!(
                received.lock().expect("request log lock").as_slice(),
                [StartupRequest::new(vec![PathBuf::from(
                    r"C:\工作区\第二个实例.navop"
                )])]
            );
        }

        #[test]
        fn a_rejected_enqueue_must_not_be_acknowledged_as_success() {
            let config_dir = temporary_config_dir();
            // 主实例的启动队列不可用：入队回调一律返回 false。
            let (_, outcome) = recording_primary(config_dir.path(), false);
            assert_eq!(outcome, SingleInstanceOutcome::Primary);

            let forwarded = claim(config_dir.path(), Vec::new());

            assert!(
                matches!(forwarded, Err(SingleInstanceError::Forward(_))),
                "a request that never entered the startup queue must fail instead of \
                 reporting success: {forwarded:?}"
            );
        }

        #[test]
        fn half_written_request_is_dropped_without_blocking_later_requests() {
            let config_dir = temporary_config_dir();
            let (received, outcome) = recording_primary(config_dir.path(), true);
            assert_eq!(outcome, SingleInstanceOutcome::Primary);

            let pipe_name = pipe_name_for_config_dir(config_dir.path());
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("test runtime");

            // 只发出长度头的一半就停住，主实例的 read_exact 会一直等下去。
            let mut stalled = runtime.block_on(async {
                let mut connection = ClientOptions::new()
                    .open(&pipe_name)
                    .expect("stalled client should connect");
                connection
                    .write_all(&[0u8, 0])
                    .await
                    .expect("partial header should be writable");
                connection
                    .flush()
                    .await
                    .expect("partial header should flush");
                connection
            });

            // 后续启动请求必须立刻被服务，而不是排在半开连接后面。
            let started = std::time::Instant::now();
            let forwarded = claim(
                config_dir.path(),
                vec![PathBuf::from(r"C:\工作区\later.navop")],
            )
            .expect("later request should still be served");
            let waited = started.elapsed();

            assert_eq!(forwarded, SingleInstanceOutcome::Forwarded);
            assert!(
                waited < REQUEST_IO_TIMEOUT,
                "later request waited for the stalled connection: {waited:?}"
            );
            assert_eq!(received.lock().expect("request log lock").len(), 1);

            // 半开连接最终必须被超时断开。管道被对端关闭时，客户端可能读到 EOF
            // （0 字节），也可能拿到 ERROR_BROKEN_PIPE / ERROR_PIPE_NOT_CONNECTED
            // 一类的错误码，两者都说明连接已经结束。
            let mut buffer = [0u8; 1];
            let dropped = runtime.block_on(async {
                tokio::time::timeout(REQUEST_IO_TIMEOUT * 4, stalled.read(&mut buffer)).await
            });
            match dropped {
                Ok(Ok(0)) => {}
                Ok(Ok(_)) => {
                    panic!("primary instance sent unexpected data on a stalled connection")
                }
                Ok(Err(error)) => assert!(
                    matches!(
                        error.kind(),
                        io::ErrorKind::BrokenPipe
                            | io::ErrorKind::NotConnected
                            | io::ErrorKind::ConnectionAborted
                            | io::ErrorKind::ConnectionReset
                            | io::ErrorKind::UnexpectedEof
                    ),
                    "a dropped connection must not surface as {error:?}"
                ),
                Err(_) => panic!("primary instance never timed out the stalled connection"),
            }
        }

        #[test]
        fn concurrent_claims_produce_exactly_one_primary_instance() {
            let config_dir = temporary_config_dir();
            let config_dir = config_dir.path().to_path_buf();

            let outcomes = std::thread::scope(|scope| {
                let handles = (0..4)
                    .map(|_| {
                        let config_dir = config_dir.clone();
                        scope.spawn(move || claim(&config_dir, Vec::new()))
                    })
                    .collect::<Vec<_>>();

                handles
                    .into_iter()
                    .map(|handle| handle.join().expect("claim thread"))
                    .collect::<Vec<_>>()
            });

            let primaries = outcomes
                .iter()
                .filter(|outcome| matches!(outcome, Ok(SingleInstanceOutcome::Primary)))
                .count();
            let forwarded = outcomes
                .iter()
                .filter(|outcome| matches!(outcome, Ok(SingleInstanceOutcome::Forwarded)))
                .count();

            assert_eq!(
                primaries, 1,
                "exactly one claim may win the primary instance: {outcomes:?}"
            );
            assert_eq!(forwarded, 3, "every other claim must forward: {outcomes:?}");
        }

        #[test]
        fn claims_are_scoped_to_the_configuration_directory() {
            let first_dir = temporary_config_dir();
            let second_dir = temporary_config_dir();

            assert_eq!(
                claim(first_dir.path(), Vec::new()).expect("first config directory"),
                SingleInstanceOutcome::Primary
            );
            assert_eq!(
                claim(second_dir.path(), Vec::new()).expect("second config directory"),
                SingleInstanceOutcome::Primary
            );
        }
    }
}
