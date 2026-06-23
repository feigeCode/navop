use anyhow::{Result, anyhow};
use public_mcp::command_store::RemoteCommandStore;
use public_mcp::registry::{ConnectionState, RemoteOpsSessionHandle, TerminalSessionSnapshot};
use public_mcp::remote_ops::{
    RemoteCommandMode, RemoteCommandStatus, RemoteExecRequest, RemoteExecResult,
    RemoteFileWriteRequest, RemoteFileWriteResult, SessionDiagnosticsRequest,
    SessionDiagnosticsResult,
};
use sha2::{Digest, Sha256};
use ssh::{ChannelEvent, SshChannel, SshSessionManager};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::runtime::Handle;

/// SSH exec 收集 stdout/stderr 的最大缓冲。超出后截断并标记（避免单次 exec OOM）。
const MAX_EXEC_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
/// SSH channel 事件的默认 exec 超时，单位毫秒。
const DEFAULT_TIMEOUT_MS: u64 = 30_000;

/// 默认非交互环境，避免 pager 进入交互模式导致自动化卡住。
fn default_env() -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    env.insert("SYSTEMD_PAGER".to_string(), "cat".to_string());
    env.insert("PAGER".to_string(), "cat".to_string());
    env.insert("LESS".to_string(), "-F -X".to_string());
    env
}

/// 合并默认环境与调用方提供的环境（调用方优先）。
fn merged_env(caller_env: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut env = default_env();
    for (key, value) in caller_env {
        env.insert(key.clone(), value.clone());
    }
    env
}

/// 把调用方 cwd/env 组装成 shell 前缀，拼到命令前面，确保 exec 在指定上下文执行。
fn build_full_command(request: &RemoteExecRequest) -> String {
    let mut prefix = String::new();
    if let Some(cwd) = &request.cwd {
        prefix.push_str(&format!("cd {cwd} && "));
    }
    for (key, value) in &request.env {
        // shell 安全引用 value；此处用单引号包裹并对内部单引号转义。
        let escaped = value.replace('\'', "'\"'\"'");
        prefix.push_str(&format!("{key}='{escaped}' "));
    }
    format!("{prefix}{}", request.command)
}

/// 在当前 tokio runtime 上同步执行一个 async future。
///
/// MCP tool 调用本身运行在 multi-thread tokio runtime 的 worker 线程上，
/// 因此 `block_in_place` + `Handle::current().block_on` 可安全阻塞而不死锁。
fn block_on_async<F>(future: F) -> Result<F::Output>
where
    F: std::future::Future + Send,
    F::Output: Send,
{
    let handle = Handle::try_current().map_err(|error| anyhow!(error.to_string()))?;
    let output = tokio::task::block_in_place(|| handle.block_on(future));
    Ok(output)
}

/// terminal-view 侧的远程操作桥。持有共享 SSH session manager 与会话快照。
/// `state` 可与 terminal bridge handle 共享，使一次 refresh 同时更新两者。
pub struct SshRemoteOpsHandle {
    session_manager: Arc<SshSessionManager>,
    state: Arc<Mutex<TerminalSessionSnapshot>>,
    command_store: RemoteCommandStore,
}

impl SshRemoteOpsHandle {
    pub fn new(
        session_manager: Arc<SshSessionManager>,
        snapshot: TerminalSessionSnapshot,
        command_store: RemoteCommandStore,
    ) -> Self {
        Self {
            session_manager,
            state: Arc::new(Mutex::new(snapshot)),
            command_store,
        }
    }

    /// 用外部共享 state 构造，使 refresh 与 terminal handle 同步。
    pub fn with_shared_state(
        session_manager: Arc<SshSessionManager>,
        state: Arc<Mutex<TerminalSessionSnapshot>>,
        command_store: RemoteCommandStore,
    ) -> Self {
        Self {
            session_manager,
            state,
            command_store,
        }
    }

    pub fn refresh(&self, snapshot: TerminalSessionSnapshot) {
        *self.state.lock().expect("remote ops state lock poisoned") = snapshot;
    }

    /// 启动 background 命令：注册到 command store，spawn tokio task 异步执行，
    /// 立即返回 command_id。poll/output/cancel 由 command store 驱动。
    fn spawn_background(
        &self,
        full_command: String,
        env: BTreeMap<String, String>,
        timeout_ms: u64,
    ) -> Result<RemoteExecResult> {
        let session_id = self
            .state
            .lock()
            .expect("remote ops state lock poisoned")
            .session_id
            .clone();
        let (command_id, entry, mut cancel_rx) =
            self.command_store.register(&session_id, &full_command);

        let session_manager = self.session_manager.clone();
        let entry_clone = entry.clone();
        tokio::spawn(async move {
            let result = run_exec_with_cancel(
                &session_manager,
                &full_command,
                &env,
                timeout_ms,
                &mut cancel_rx,
            )
            .await;

            match result {
                Ok(exec_result) => {
                    entry_clone.push_stdout(exec_result.stdout.as_bytes());
                    entry_clone.push_stderr(exec_result.stderr.as_bytes());
                    entry_clone.complete(exec_result.status, exec_result.exit_code);
                }
                Err(error) => {
                    entry_clone.push_stderr(error.to_string().as_bytes());
                    entry_clone.complete(RemoteCommandStatus::Failed, None);
                }
            }
        });

        let started_at_ms = chrono::Utc::now().timestamp_millis();
        Ok(RemoteExecResult::background(command_id, started_at_ms))
    }
}

impl RemoteOpsSessionHandle for SshRemoteOpsHandle {
    fn snapshot(&self) -> TerminalSessionSnapshot {
        self.state
            .lock()
            .expect("remote ops state lock poisoned")
            .clone()
    }

    fn exec(&self, request: RemoteExecRequest) -> Result<RemoteExecResult> {
        let timeout_ms = request.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
        let full_command = build_full_command(&request);
        let env = merged_env(&request.env);

        if matches!(request.mode, RemoteCommandMode::Background) {
            return self.spawn_background(full_command, env, timeout_ms);
        }

        let result = block_on_async(async {
            run_exec(&self.session_manager, &full_command, &env, timeout_ms).await
        })??;

        Ok(result)
    }

    fn write_file(&self, request: RemoteFileWriteRequest) -> Result<RemoteFileWriteResult> {
        // 第一版用 base64 经 exec 写文件，避免依赖额外 SFTP 子系统 channel。
        // 后续可切到 SFTP 通道以获得更好的语义（mkdir、stat）。
        let bytes = request.content.as_bytes();
        let sha256 = sha256_hex(bytes);
        let content_b64 = base64_encode(bytes);
        let mode_arg = request
            .mode
            .map(|mode| {
                format!(
                    " && chmod {mode:o} {quote}",
                    quote = shell_quote(&request.path)
                )
            })
            .unwrap_or_default();
        let overwrite_guard = if request.overwrite {
            String::new()
        } else {
            format!(
                "test -e {quote} && exit 17; ",
                quote = shell_quote(&request.path)
            )
        };
        let command = format!(
            "{overwrite_guard}umask 077 && printf '%s' '{b64}' | base64 -d > {quote}{mode_arg}",
            b64 = content_b64,
            quote = shell_quote(&request.path),
        );

        let exec_request = RemoteExecRequest {
            session_id: String::new(),
            command,
            cwd: None,
            env: BTreeMap::new(),
            timeout_ms: Some(DEFAULT_TIMEOUT_MS),
            mode: public_mcp::remote_ops::RemoteCommandMode::Foreground,
        };

        let result = self.exec(exec_request)?;
        if result.exit_code == Some(17) {
            return Err(anyhow!("file already exists: {}", request.path));
        }
        if !result.is_success() {
            return Err(anyhow!(
                "remote file write failed (exit {:?}): {}",
                result.exit_code,
                result.stderr
            ));
        }

        Ok(RemoteFileWriteResult {
            path: request.path,
            bytes_written: bytes.len(),
            sha256,
        })
    }

    fn diagnostics(&self, request: SessionDiagnosticsRequest) -> Result<SessionDiagnosticsResult> {
        let snapshot = self.snapshot();
        let (recoverable, suggested_action) = match &snapshot.connection_state {
            ConnectionState::Connected => (true, None),
            ConnectionState::Connecting => (true, Some("wait_for_connection".to_string())),
            ConnectionState::Disconnected { .. } => {
                (true, Some("reconnect_in_onetcli".to_string()))
            }
        };
        let last_error = match &snapshot.connection_state {
            ConnectionState::Disconnected { error } => error.clone(),
            _ => None,
        };
        Ok(SessionDiagnosticsResult {
            session_id: request.session_id,
            connection_id: snapshot.connection_id,
            host_label: snapshot.host_label,
            cwd: snapshot.cwd,
            rows: snapshot.rows,
            cols: snapshot.cols,
            connection_kind: snapshot.connection_kind,
            state: snapshot.connection_state,
            last_error,
            recoverable,
            suggested_action,
        })
    }
}

/// 在 SSH session 上执行单条命令，收集完整 stdout/stderr/exit code。
async fn run_exec(
    session_manager: &SshSessionManager,
    command: &str,
    env: &BTreeMap<String, String>,
    timeout_ms: u64,
) -> Result<RemoteExecResult> {
    let started = Instant::now();
    let mut channel = session_manager.open_channel().await?;

    // 先设置环境变量，再 exec。set_env 在 exec 前发送，命令执行时即可继承。
    for (key, value) in env {
        let _ = channel.set_env(key, value).await;
    }
    channel.exec(command).await?;

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_code: Option<i32> = None;
    let mut timed_out = false;

    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        tokio::select! {
            biased;
            _ = tokio::time::sleep_until(deadline), if exit_code.is_none() => {
                timed_out = true;
                let _ = channel.eof().await;
                let _ = channel.close().await;
                break;
            }
            event = channel.recv() => {
                match event {
                    Some(ChannelEvent::Data(data)) => {
                        if stdout.len() + data.len() <= MAX_EXEC_OUTPUT_BYTES {
                            stdout.extend_from_slice(&data);
                        }
                    }
                    Some(ChannelEvent::ExtendedData { ext, data }) => {
                        if ext == 1 && stderr.len() + data.len() <= MAX_EXEC_OUTPUT_BYTES {
                            stderr.extend_from_slice(&data);
                        }
                    }
                    Some(ChannelEvent::ExitStatus(code)) => {
                        exit_code = Some(code as i32);
                    }
                    Some(ChannelEvent::ExitSignal { signal_name, error_message }) => {
                        exit_code = Some(130);
                        if !error_message.is_empty() {
                            stderr.extend_from_slice(error_message.as_bytes());
                        }
                        if !signal_name.is_empty() {
                            stderr.extend_from_slice(format!("\nsignal: {signal_name}").as_bytes());
                        }
                    }
                    Some(ChannelEvent::Eof) | Some(ChannelEvent::Close) | None => {
                        break;
                    }

                }
            }
        }
    }

    let _ = channel.eof().await;
    let _ = channel.close().await;

    let status = if timed_out {
        RemoteCommandStatus::TimedOut
    } else if exit_code.map(|code| code == 0).unwrap_or(false) {
        RemoteCommandStatus::Exited
    } else if exit_code.is_some() {
        RemoteCommandStatus::Failed
    } else {
        RemoteCommandStatus::Exited
    };

    let stdout = String::from_utf8_lossy(&stdout).into_owned();
    let stderr = String::from_utf8_lossy(&stderr).into_owned();

    Ok(RemoteExecResult::foreground(
        status,
        stdout,
        stderr,
        exit_code,
        started.elapsed().as_millis() as u64,
        timed_out,
    ))
}

/// background 命令执行：与 `run_exec` 共用 SSH channel 收集逻辑，
/// 额外监听取消信号并实时把输出 push 到 command entry。
async fn run_exec_with_cancel(
    session_manager: &SshSessionManager,
    command: &str,
    env: &BTreeMap<String, String>,
    timeout_ms: u64,
    cancel_rx: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<RemoteExecResult> {
    let started = Instant::now();
    let mut channel = session_manager.open_channel().await?;

    for (key, value) in env {
        let _ = channel.set_env(key, value).await;
    }
    channel.exec(command).await?;

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_code: Option<i32> = None;
    let mut timed_out = false;
    let mut cancelled = false;

    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        tokio::select! {
            biased;
            cancelled_flag = cancel_rx.changed() => {
                if cancelled_flag.is_ok() && *cancel_rx.borrow() {
                    cancelled = true;
                    let _ = channel.eof().await;
                    let _ = channel.close().await;
                    break;
                }
            }
            _ = tokio::time::sleep_until(deadline), if exit_code.is_none() => {
                timed_out = true;
                let _ = channel.eof().await;
                let _ = channel.close().await;
                break;
            }
            event = channel.recv() => {
                match event {
                    Some(ChannelEvent::Data(data)) => {
                        if stdout.len() + data.len() <= MAX_EXEC_OUTPUT_BYTES {
                            stdout.extend_from_slice(&data);
                        }
                    }
                    Some(ChannelEvent::ExtendedData { ext, data }) => {
                        if ext == 1 && stderr.len() + data.len() <= MAX_EXEC_OUTPUT_BYTES {
                            stderr.extend_from_slice(&data);
                        }
                    }
                    Some(ChannelEvent::ExitStatus(code)) => {
                        exit_code = Some(code as i32);
                    }
                    Some(ChannelEvent::ExitSignal { signal_name, error_message }) => {
                        exit_code = Some(130);
                        if !error_message.is_empty() {
                            stderr.extend_from_slice(error_message.as_bytes());
                        }
                        if !signal_name.is_empty() {
                            stderr.extend_from_slice(format!("\nsignal: {signal_name}").as_bytes());
                        }
                    }
                    Some(ChannelEvent::Eof) | Some(ChannelEvent::Close) | None => {
                        break;
                    }
                }
            }
        }
    }

    let _ = channel.eof().await;
    let _ = channel.close().await;

    let status = if cancelled {
        RemoteCommandStatus::Cancelled
    } else if timed_out {
        RemoteCommandStatus::TimedOut
    } else if exit_code.map(|code| code == 0).unwrap_or(false) {
        RemoteCommandStatus::Exited
    } else if exit_code.is_some() {
        RemoteCommandStatus::Failed
    } else {
        RemoteCommandStatus::Exited
    };

    let stdout = String::from_utf8_lossy(&stdout).into_owned();
    let stderr = String::from_utf8_lossy(&stderr).into_owned();

    Ok(RemoteExecResult::foreground(
        status,
        stdout,
        stderr,
        exit_code,
        started.elapsed().as_millis() as u64,
        timed_out,
    ))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 轻量 base64 编码，避免引入额外依赖。
fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((triple >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(triple & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// 用单引号包裹路径并转义内部单引号，用于 shell 命令拼接。
fn shell_quote(path: &str) -> String {
    let escaped = path.replace('\'', "'\"'\"'");
    format!("'{escaped}'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merged_env_allows_caller_override() {
        let mut caller = BTreeMap::new();
        caller.insert("PAGER".to_string(), "less".to_string());
        let env = merged_env(&caller);
        assert_eq!("less", env["PAGER"]);
        assert_eq!("cat", env["SYSTEMD_PAGER"]);
    }

    #[test]
    fn build_full_command_prepends_cwd_and_env() {
        let mut env = BTreeMap::new();
        env.insert("FOO".to_string(), "bar".to_string());
        let request = RemoteExecRequest {
            session_id: String::new(),
            command: "echo hi".to_string(),
            cwd: Some("/data".to_string()),
            env,
            timeout_ms: None,
            mode: public_mcp::remote_ops::RemoteCommandMode::Foreground,
        };
        let full = build_full_command(&request);
        assert!(full.starts_with("cd /data && "));
        assert!(full.contains("FOO='bar'"));
        assert!(full.ends_with("echo hi"));
    }

    #[test]
    fn sha256_matches_known_vector() {
        assert_eq!(
            "dffd6021bb2bd5b0af676290809ec3a53191dd81c7f70a4b28688a362182986f",
            sha256_hex(b"Hello, World!")
        );
    }

    #[test]
    fn base64_encode_matches_standard() {
        assert_eq!("aGVsbG8=", base64_encode(b"hello"));
        assert_eq!("Zm9vYmFy", base64_encode(b"foobar"));
        assert_eq!("YQ==", base64_encode(b"a"));
    }

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!("'it'\"'\"'s'", shell_quote("it's"));
    }
}
