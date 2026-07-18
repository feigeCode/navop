use anyhow::{Result, anyhow};
use public_mcp::command_store::{CommandEntry, RemoteCommandStore};
use public_mcp::registry::{ConnectionState, RemoteOpsSessionHandle, TerminalSessionSnapshot};
use public_mcp::remote_ops::{
    RemoteCommandMode, RemoteCommandOutputRequest, RemoteCommandStatus, RemoteExecRequest,
    RemoteExecResult, RemoteFileWriteRequest, RemoteFileWriteResult, SessionDiagnosticsRequest,
    SessionDiagnosticsResult,
};
use sha2::{Digest, Sha256};
use ssh::{ChannelEvent, SshChannel, SshSessionManager};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::runtime::Handle;

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

struct SpawnedRemoteCommand {
    command_id: String,
    entry: Arc<CommandEntry>,
    started_at_ms: i64,
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

    fn spawn_command(
        &self,
        full_command: String,
        env: BTreeMap<String, String>,
        hard_timeout_ms: Option<u64>,
    ) -> Result<SpawnedRemoteCommand> {
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
                hard_timeout_ms,
                &mut cancel_rx,
                &entry_clone,
            )
            .await;

            match result {
                Ok((status, exit_code)) => {
                    entry_clone.complete(status, exit_code);
                }
                Err(error) => {
                    entry_clone.push_stderr(error.to_string().as_bytes());
                    entry_clone.complete(RemoteCommandStatus::Failed, None);
                }
            }
        });

        let started_at_ms = chrono::Utc::now().timestamp_millis();
        Ok(SpawnedRemoteCommand {
            command_id,
            entry,
            started_at_ms,
        })
    }

    fn spawn_background(
        &self,
        full_command: String,
        env: BTreeMap<String, String>,
        timeout_ms: u64,
    ) -> Result<RemoteExecResult> {
        let spawned = self.spawn_command(full_command, env, Some(timeout_ms))?;
        Ok(RemoteExecResult::background(
            spawned.command_id,
            spawned.started_at_ms,
        ))
    }
}

fn command_result(
    command_store: &RemoteCommandStore,
    command_id: &str,
    started_at_ms: i64,
    timed_out: bool,
) -> Result<RemoteExecResult> {
    let poll = command_store.poll_by_id(command_id)?;
    let output = command_store.output(&RemoteCommandOutputRequest {
        command_id: command_id.to_string(),
        stdout_offset: 0,
        stderr_offset: 0,
        limit_bytes: Some(usize::MAX),
    })?;
    Ok(RemoteExecResult {
        status: poll.status,
        stdout: output.stdout,
        stderr: output.stderr,
        exit_code: poll.exit_code,
        duration_ms: poll.duration_ms,
        timed_out,
        command_id: timed_out.then(|| command_id.to_string()),
        started_at_ms: timed_out.then_some(started_at_ms),
    })
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

        let spawned = self.spawn_command(full_command, env, None)?;
        let completed = block_on_async(
            spawned
                .entry
                .wait_for_completion(Duration::from_millis(timeout_ms)),
        )?;
        command_result(
            &self.command_store,
            &spawned.command_id,
            spawned.started_at_ms,
            !completed,
        )
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

        let full_command = build_full_command(&exec_request);
        let env = merged_env(&exec_request.env);
        let spawned = self.spawn_command(full_command, env, Some(DEFAULT_TIMEOUT_MS))?;
        let completed = block_on_async(spawned.entry.wait_for_completion(Duration::from_millis(
            DEFAULT_TIMEOUT_MS.saturating_add(1_000),
        )))?;
        let result = command_result(
            &self.command_store,
            &spawned.command_id,
            spawned.started_at_ms,
            !completed,
        )?;
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

/// SSH channel 收集逻辑。每个数据事件立即写入 command entry，
/// 因此 background 与 foreground 超时脱离后的命令都能增量读取输出。
async fn run_exec_with_cancel(
    session_manager: &SshSessionManager,
    command: &str,
    env: &BTreeMap<String, String>,
    hard_timeout_ms: Option<u64>,
    cancel_rx: &mut tokio::sync::watch::Receiver<bool>,
    entry: &CommandEntry,
) -> Result<(RemoteCommandStatus, Option<i32>)> {
    let mut channel = session_manager.open_channel().await?;

    for (key, value) in env {
        let _ = channel.set_env(key, value).await;
    }
    channel.exec(command).await?;

    let mut exit_code: Option<i32> = None;
    let mut cancelled = false;
    let mut timed_out = false;

    let deadline = hard_timeout_ms
        .map(|timeout_ms| tokio::time::Instant::now() + Duration::from_millis(timeout_ms));
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
            _ = async {
                if let Some(deadline) = deadline {
                    tokio::time::sleep_until(deadline).await;
                } else {
                    std::future::pending::<()>().await;
                }
            }, if exit_code.is_none() => {
                timed_out = true;
                let _ = channel.eof().await;
                let _ = channel.close().await;
                break;
            }
            event = channel.recv() => {
                match event {
                    Some(ChannelEvent::Data(data)) => {
                        entry.push_stdout(&data);
                    }
                    Some(ChannelEvent::ExtendedData { ext, data }) => {
                        if ext == 1 {
                            entry.push_stderr(&data);
                        }
                    }
                    Some(ChannelEvent::ExitStatus(code)) => {
                        exit_code = Some(code as i32);
                    }
                    Some(ChannelEvent::ExitSignal { signal_name, error_message }) => {
                        exit_code = Some(130);
                        if !error_message.is_empty() {
                            entry.push_stderr(error_message.as_bytes());
                        }
                        if !signal_name.is_empty() {
                            entry.push_stderr(format!("\nsignal: {signal_name}").as_bytes());
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

    Ok((status, exit_code))
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

    #[tokio::test]
    async fn timed_out_foreground_result_keeps_incremental_command_output() {
        let store = RemoteCommandStore::default();
        let (id, entry) = store.register_observed("ssh-1", "long-command");
        entry.push_stdout(b"first\n");
        let result = command_result(&store, &id, 123, true).expect("tracked result");

        assert_eq!(RemoteCommandStatus::Running, result.status);
        assert_eq!(Some(id), result.command_id);
        assert!(result.timed_out);
        assert_eq!("first\n", result.stdout);
    }
}
