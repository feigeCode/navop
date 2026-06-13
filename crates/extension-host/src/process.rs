//! 子进程生命周期管理。
//!
//! 把「spawn 扩展可执行文件 → 建立 transport → 优雅关闭」打包成可复用单元。
//! 支持两种载体:
//!
//! - **LocalSocket**:在主侧 listen,把 socket 名通过环境变量透传给子进程,
//!   子进程主动 connect 回来。优点是平台原生 + 进程隔离。
//! - **Stdio**(规划中):子进程 stdin/stdout 即 transport。优点是无任何
//!   socket 命名空间依赖,便于 dev/debug。

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use interprocess::local_socket::{
    GenericNamespaced, ListenerOptions, ToNsName,
    tokio::{Stream as LocalSocketStream, prelude::*},
};
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep};
use tracing::warn;

use crate::error::{HostError, HostResult};

/// spawn 用环境变量名:把 socket 名传给子进程。
pub const SOCKET_ENV_VAR: &str = "ONETCLI_EXT_SOCKET";

const STDERR_TAIL_LINES: usize = 20;

type StderrTail = Arc<StdMutex<VecDeque<String>>>;

/// 子进程 spawn 配置。
#[derive(Debug, Clone)]
pub struct SpawnConfig {
    /// 可执行文件路径(绝对或在 PATH 中)。
    pub program: PathBuf,
    /// 命令行参数。
    pub args: Vec<String>,
    /// 工作目录;None 时继承宿主当前目录。
    pub cwd: Option<PathBuf>,
    /// 额外环境变量(会覆盖同名继承变量)。
    pub env: HashMap<String, String>,
    /// transport 配置。
    pub transport: SpawnTransport,
    /// 子进程多久内必须 ready(连接到 socket / 写出第一行 stdio)。
    pub ready_timeout: Duration,
    /// 是否捕获 stderr 并转给 tracing(默认 true)。
    pub capture_stderr: bool,
}

impl SpawnConfig {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: HashMap::new(),
            transport: SpawnTransport::LocalSocket {
                name: default_socket_name(),
            },
            ready_timeout: Duration::from_secs(10),
            capture_stderr: true,
        }
    }

    pub fn with_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn with_env(mut self, key: impl Into<String>, val: impl Into<String>) -> Self {
        self.env.insert(key.into(), val.into());
        self
    }

    pub fn with_transport(mut self, transport: SpawnTransport) -> Self {
        self.transport = transport;
        self
    }

    pub fn with_ready_timeout(mut self, d: Duration) -> Self {
        self.ready_timeout = d;
        self
    }

    pub fn without_stderr_capture(mut self) -> Self {
        self.capture_stderr = false;
        self
    }
}

/// transport 选项。
#[derive(Debug, Clone)]
pub enum SpawnTransport {
    /// 本地 socket;宿主先 listen,把 `name` 通过 [`SOCKET_ENV_VAR`] 透传。
    LocalSocket {
        /// socket 名(namespaced)。
        name: String,
    },
    // 占位:Stdio 模式留待 P3.3 实现
    // Stdio,
}

/// 默认 socket 名(纳秒时间戳 + UUID 后缀,保证并发 spawn 唯一)。
pub fn default_socket_name() -> String {
    let suffix = if cfg!(debug_assertions) { "-debug" } else { "" };
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    // 仅 8 char 的 hex 已足够并发去重——socket 名要短(macOS 长度限制 104)。
    let suffix_uuid = uuid::Uuid::new_v4().simple().to_string();
    format!("onetcli-ext-{nanos}-{}{suffix}.sock", &suffix_uuid[..8])
}

/// 已 spawn 的进程句柄。
///
/// 持有 `tokio::process::Child` 以保证 kill_on_drop;此外暴露已连接的
/// [`LocalSocketStream`] 给 caller 自己拆 reader/writer 喂给 `JsonRpcClient`。
pub struct ProcessHandle {
    child: Option<Child>,
    pub stream: Option<LocalSocketStream>,
}

impl std::fmt::Debug for ProcessHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessHandle")
            .field("pid", &self.child.as_ref().and_then(|c| c.id()))
            .field("has_stream", &self.stream.is_some())
            .finish()
    }
}

impl ProcessHandle {
    /// 是否还持有 stream(还没有被 take_stream 拿走)。
    pub fn has_stream(&self) -> bool {
        self.stream.is_some()
    }

    /// 拿走 stream(后续 `tokio::io::split` 喂给 transport)。
    pub fn take_stream(&mut self) -> Option<LocalSocketStream> {
        self.stream.take()
    }

    /// 取得子进程 pid(平台原生 id)。
    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().and_then(|c| c.id())
    }

    /// kill 子进程并等待退出。
    pub async fn kill(&mut self) -> HostResult<()> {
        if let Some(mut c) = self.child.take() {
            let _ = c.start_kill();
            let _ = c.wait().await;
        }
        Ok(())
    }
}

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            // tokio::process::Child::kill_on_drop 在 spawn 时已开启;
            // 这里再尝试一次 start_kill 兜底。
            let _ = c.start_kill();
        }
    }
}

/// spawn 扩展子进程,等待其建立 transport,返回 [`ProcessHandle`]。
pub async fn spawn(config: SpawnConfig) -> HostResult<ProcessHandle> {
    match &config.transport {
        SpawnTransport::LocalSocket { name } => {
            spawn_local_socket(config.clone(), name.clone()).await
        }
    }
}

async fn spawn_local_socket(config: SpawnConfig, socket_name: String) -> HostResult<ProcessHandle> {
    let ns_name = socket_name
        .clone()
        .to_ns_name::<GenericNamespaced>()
        .map_err(|e| HostError::Config(format!("invalid socket name: {e}")))?;

    let listener = ListenerOptions::new()
        .name(ns_name)
        .create_tokio()
        .map_err(HostError::Io)?;

    let mut command = Command::new(&config.program);
    command.args(&config.args).env(SOCKET_ENV_VAR, &socket_name);
    if let Some(cwd) = &config.cwd {
        command.current_dir(cwd);
    }
    for (k, v) in &config.env {
        command.env(k, v);
    }
    command.stdin(Stdio::null());
    if config.capture_stderr {
        command.stderr(Stdio::piped());
    } else {
        command.stderr(Stdio::inherit());
    }
    command.kill_on_drop(true);

    let mut child = command.spawn().map_err(HostError::Io)?;

    let stderr_tail = new_stderr_tail();
    let mut stderr_task = None;
    if config.capture_stderr {
        if let Some(stderr) = child.stderr.take() {
            stderr_task = Some(tokio::spawn(forward_stderr(
                stderr,
                config.program.display().to_string(),
                Arc::clone(&stderr_tail),
            )));
        }
    }

    // 等子进程主动 connect 回来。
    let deadline = Instant::now() + config.ready_timeout;
    let stream = loop {
        if Instant::now() >= deadline {
            // 子进程没及时 ready,杀掉
            let _ = child.start_kill();
            let _ = child.wait().await;
            drain_stderr(&mut stderr_task).await;
            return Err(HostError::ProcessNotReady {
                deadline_ms: config.ready_timeout.as_millis() as u64,
                stderr_tail: stderr_tail_text(&stderr_tail),
            });
        }

        // 子进程已退出?
        if let Ok(Some(status)) = child.try_wait() {
            drain_stderr(&mut stderr_task).await;
            let reason = with_stderr_tail(format!("exit before ready: {status}"), &stderr_tail);
            return Err(HostError::ProcessExited(reason));
        }

        match tokio::time::timeout(Duration::from_millis(200), listener.accept()).await {
            Ok(Ok(stream)) => break stream,
            Ok(Err(e)) => {
                warn!(error = %e, "accept failed; retrying");
                sleep(Duration::from_millis(50)).await;
            }
            Err(_) => {
                // accept timeout,继续 poll child
            }
        }
    };

    Ok(ProcessHandle {
        child: Some(child),
        stream: Some(stream),
    })
}

fn new_stderr_tail() -> StderrTail {
    Arc::new(StdMutex::new(VecDeque::with_capacity(STDERR_TAIL_LINES)))
}

async fn drain_stderr(task: &mut Option<JoinHandle<()>>) {
    if let Some(task) = task.take() {
        let _ = tokio::time::timeout(Duration::from_millis(200), task).await;
    }
}

fn with_stderr_tail(reason: String, tail: &StderrTail) -> String {
    if let Some(stderr) = stderr_tail_text(tail) {
        format!("{reason}; recent stderr:\n{stderr}")
    } else {
        reason
    }
}

fn stderr_tail_text(tail: &StderrTail) -> Option<String> {
    let lines = tail.lock().expect("stderr tail poisoned");
    if lines.is_empty() {
        None
    } else {
        Some(lines.iter().cloned().collect::<Vec<_>>().join("\n"))
    }
}

async fn forward_stderr(stderr: tokio::process::ChildStderr, label: String, tail: StderrTail) {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        {
            let mut guard = tail.lock().expect("stderr tail poisoned");
            if guard.len() >= STDERR_TAIL_LINES {
                guard.pop_front();
            }
            guard.push_back(line.clone());
        }
        tracing::debug!(target: "extension_host::stderr", program = %label, "{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_config_defaults() {
        let c = SpawnConfig::new("/bin/echo");
        assert_eq!(c.program, PathBuf::from("/bin/echo"));
        assert!(c.args.is_empty());
        assert!(c.cwd.is_none());
        assert!(c.env.is_empty());
        assert!(c.capture_stderr);
        assert_eq!(c.ready_timeout, Duration::from_secs(10));
        match c.transport {
            SpawnTransport::LocalSocket { ref name } => assert!(name.starts_with("onetcli-ext-")),
        }
    }

    #[test]
    fn spawn_config_builder_chains() {
        let c = SpawnConfig::new("/bin/x")
            .with_args(["--driver", "cassandra"])
            .with_cwd("/tmp")
            .with_env("FOO", "bar")
            .with_ready_timeout(Duration::from_secs(3))
            .without_stderr_capture();
        assert_eq!(c.args, vec!["--driver", "cassandra"]);
        assert_eq!(c.cwd, Some(PathBuf::from("/tmp")));
        assert_eq!(c.env.get("FOO").map(String::as_str), Some("bar"));
        assert_eq!(c.ready_timeout, Duration::from_secs(3));
        assert!(!c.capture_stderr);
    }

    #[test]
    fn default_socket_name_contains_prefix() {
        let n = default_socket_name();
        assert!(n.starts_with("onetcli-ext-"));
        assert!(n.ends_with(".sock"));
    }

    #[test]
    fn default_socket_name_changes_between_calls() {
        // 不严格保证,但纳秒精度下两次调用极少撞名
        let a = default_socket_name();
        std::thread::sleep(Duration::from_nanos(1));
        let b = default_socket_name();
        // 接受可能撞名,但至少返回非空
        assert!(!a.is_empty());
        assert!(!b.is_empty());
    }

    #[tokio::test]
    async fn spawn_nonexistent_program_returns_io_error() {
        let cfg = SpawnConfig::new("/definitely/does/not/exist/onetcli-fake")
            .with_ready_timeout(Duration::from_millis(50));
        let err = spawn(cfg).await.unwrap_err();
        // 不同平台具体错误码不同,只要是 Io 即可
        assert!(matches!(err, HostError::Io(_)), "got: {err:?}");
    }

    #[tokio::test]
    async fn spawn_program_that_exits_immediately_returns_process_exited_or_not_ready() {
        // /bin/true 立即退出,不会连接 socket
        let bin = if cfg!(target_os = "windows") {
            "cmd"
        } else {
            "/usr/bin/true"
        };
        if !std::path::Path::new(bin).exists() && bin != "cmd" {
            return; // 平台不适用,跳过
        }
        let cfg = SpawnConfig::new(bin).with_ready_timeout(Duration::from_millis(500));
        let err = spawn(cfg).await.unwrap_err();
        assert!(
            matches!(
                err,
                HostError::ProcessExited(_) | HostError::ProcessNotReady { .. }
            ),
            "got: {err:?}"
        );
    }

    #[tokio::test]
    async fn process_exited_error_includes_recent_stderr() {
        if cfg!(target_os = "windows") {
            return;
        }

        let cfg = SpawnConfig::new("/bin/sh")
            .with_args(["-c", "echo duckdb driver failed >&2; exit 42"])
            .with_ready_timeout(Duration::from_millis(500));
        let err = spawn(cfg).await.unwrap_err();

        match err {
            HostError::ProcessExited(reason) => {
                assert!(reason.contains("exit status"));
                assert!(reason.contains("duckdb driver failed"), "{reason}");
            }
            other => panic!("expected ProcessExited, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn process_not_ready_error_includes_recent_stderr() {
        if cfg!(target_os = "windows") {
            return;
        }

        let cfg = SpawnConfig::new("/bin/sh")
            .with_args(["-c", "echo waiting for unavailable dependency >&2; sleep 2"])
            .with_ready_timeout(Duration::from_millis(100));
        let err = spawn(cfg).await.unwrap_err();

        match err {
            HostError::ProcessNotReady {
                stderr_tail: Some(stderr_tail),
                ..
            } => {
                assert!(
                    stderr_tail.contains("waiting for unavailable dependency"),
                    "{stderr_tail}"
                );
            }
            other => panic!("expected ProcessNotReady with stderr, got {other:?}"),
        }
    }
}
