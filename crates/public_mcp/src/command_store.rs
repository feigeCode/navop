use crate::remote_ops::{
    RemoteCommandCancelRequest, RemoteCommandCancelResult, RemoteCommandOutputRequest,
    RemoteCommandOutputResult, RemoteCommandPollResult, RemoteCommandStatus,
};
use anyhow::{Result, anyhow};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::watch;
use uuid::Uuid;

/// background 命令的退出信号。drop 它或 send `true` 即通知任务应停止。
pub struct CommandCancelHandle {
    cancel_tx: watch::Sender<bool>,
}

impl CommandCancelHandle {
    pub fn new() -> (Self, watch::Receiver<bool>) {
        let (cancel_tx, cancel_rx) = watch::channel(false);
        (Self { cancel_tx }, cancel_rx)
    }

    /// 请求取消。幂等：多次调用安全。
    pub fn request_cancel(&self) {
        let _ = self.cancel_tx.send(true);
    }

    pub fn is_cancelled(&self) -> bool {
        *self.cancel_tx.borrow()
    }
}

/// 单个 background 命令的运行时状态。由启动命令的执行桥持有可变部分。
pub struct CommandEntry {
    pub command_id: String,
    pub session_id: String,
    pub command: String,
    state: Mutex<CommandState>,
}

struct CommandState {
    status: RemoteCommandStatus,
    exit_code: Option<i32>,
    started: Instant,
    finished: Option<Instant>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    cancel_handle: Option<CommandCancelHandle>,
}

impl CommandEntry {
    fn new(
        command_id: String,
        session_id: String,
        command: String,
        cancel_handle: CommandCancelHandle,
    ) -> Self {
        Self {
            command_id,
            session_id,
            command,
            state: Mutex::new(CommandState {
                status: RemoteCommandStatus::Running,
                exit_code: None,
                started: Instant::now(),
                finished: None,
                stdout: Vec::new(),
                stderr: Vec::new(),
                cancel_handle: Some(cancel_handle),
            }),
        }
    }

    /// 追加输出。由执行桥在收到 channel 数据时调用。
    pub fn push_stdout(&self, data: &[u8]) {
        let mut state = self.state.lock().expect("command state lock poisoned");
        if state.stdout.len() + data.len() <= MAX_COMMAND_BUFFER_BYTES {
            state.stdout.extend_from_slice(data);
        }
    }

    pub fn push_stderr(&self, data: &[u8]) {
        let mut state = self.state.lock().expect("command state lock poisoned");
        if state.stderr.len() + data.len() <= MAX_COMMAND_BUFFER_BYTES {
            state.stderr.extend_from_slice(data);
        }
    }

    /// 标记命令完成。由执行桥在收到 ExitStatus/ExitSignal/EOF 时调用。
    pub fn complete(&self, status: RemoteCommandStatus, exit_code: Option<i32>) {
        let mut state = self.state.lock().expect("command state lock poisoned");
        state.status = status;
        state.exit_code = exit_code;
        state.finished = Some(Instant::now());
        // 完成后不再需要取消句柄。
        state.cancel_handle = None;
    }

    fn snapshot(&self) -> CommandSnapshot {
        let state = self.state.lock().expect("command state lock poisoned");
        let duration_ms = match state.finished {
            Some(end) => end.duration_since(state.started).as_millis() as u64,
            None => state.started.elapsed().as_millis() as u64,
        };
        CommandSnapshot {
            status: state.status,
            exit_code: state.exit_code,
            duration_ms,
            stdout_len: state.stdout.len(),
            stderr_len: state.stderr.len(),
        }
    }

    fn read_output(&self, request: &RemoteCommandOutputRequest) -> CommandOutput {
        let state = self.state.lock().expect("command state lock poisoned");
        let limit = request
            .limit_bytes
            .unwrap_or(crate::remote_ops::DEFAULT_OUTPUT_LIMIT_BYTES);
        let stdout_slice = slice_output(&state.stdout, request.stdout_offset, limit);
        let remaining = limit.saturating_sub(stdout_slice.0.len());
        let stderr_slice = slice_output(&state.stderr, request.stderr_offset, remaining);
        CommandOutput {
            stdout: stdout_slice.0,
            stdout_next_offset: stdout_slice.1,
            stderr: stderr_slice.0,
            stderr_next_offset: stderr_slice.1,
            truncated: stdout_slice.2 || stderr_slice.2,
        }
    }

    fn request_cancel(&self, signal: crate::remote_ops::RemoteCommandSignal) -> bool {
        let mut state = self.state.lock().expect("command state lock poisoned");
        if let Some(handle) = &state.cancel_handle {
            handle.request_cancel();
            state.status = RemoteCommandStatus::CancelRequested;
            let _ = signal;
            true
        } else {
            false
        }
    }
}

struct CommandSnapshot {
    status: RemoteCommandStatus,
    exit_code: Option<i32>,
    duration_ms: u64,
    stdout_len: usize,
    stderr_len: usize,
}

struct CommandOutput {
    stdout: Vec<u8>,
    stdout_next_offset: usize,
    stderr: Vec<u8>,
    stderr_next_offset: usize,
    truncated: bool,
}

/// 单条命令 stdout/stderr 各自的最大缓冲。超出部分从头部滚动丢弃，保证 poll 总能拿到最近输出。
const MAX_COMMAND_BUFFER_BYTES: usize = 8 * 1024 * 1024;

fn slice_output(buffer: &[u8], offset: usize, limit: usize) -> (Vec<u8>, usize, bool) {
    if offset >= buffer.len() {
        return (Vec::new(), buffer.len(), false);
    }
    let end = (offset + limit).min(buffer.len());
    let truncated = end - offset == limit && end < buffer.len();
    (buffer[offset..end].to_vec(), end, truncated)
}

/// 进程级 background 命令注册表。由启动命令的执行桥写入，由 MCP 工具读取/取消。
#[derive(Clone, Default)]
pub struct RemoteCommandStore {
    commands: Arc<Mutex<HashMap<String, Arc<CommandEntry>>>>,
}

impl RemoteCommandStore {
    /// 注册一个新命令，返回它的 entry 和取消句柄。
    /// 调用方应立即 spawn 执行任务，并在任务里持有 entry 以追加输出和标记完成。
    pub fn register(
        &self,
        session_id: &str,
        command: &str,
    ) -> (String, Arc<CommandEntry>, watch::Receiver<bool>) {
        let command_id = format!("cmd_{}", Uuid::new_v4().simple());
        let (cancel_handle, cancel_rx) = CommandCancelHandle::new();
        let entry = Arc::new(CommandEntry::new(
            command_id.clone(),
            session_id.to_string(),
            command.to_string(),
            cancel_handle,
        ));
        self.commands
            .lock()
            .expect("command store lock poisoned")
            .insert(command_id.clone(), entry.clone());
        (command_id, entry, cancel_rx)
    }

    pub fn poll_by_id(&self, command_id: &str) -> Result<RemoteCommandPollResult> {
        let entry = self.entry(command_id)?;
        let snap = entry.snapshot();
        Ok(RemoteCommandPollResult {
            command_id: command_id.to_string(),
            status: snap.status,
            exit_code: snap.exit_code,
            duration_ms: snap.duration_ms,
            stdout_bytes: snap.stdout_len,
            stderr_bytes: snap.stderr_len,
        })
    }

    pub fn output(
        &self,
        request: &RemoteCommandOutputRequest,
    ) -> Result<RemoteCommandOutputResult> {
        let entry = self.entry(&request.command_id)?;
        let out = entry.read_output(request);
        Ok(RemoteCommandOutputResult {
            command_id: request.command_id.clone(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            next_stdout_offset: out.stdout_next_offset,
            next_stderr_offset: out.stderr_next_offset,
            truncated: out.truncated,
        })
    }

    pub fn cancel(
        &self,
        request: &RemoteCommandCancelRequest,
    ) -> Result<RemoteCommandCancelResult> {
        let entry = self.entry(&request.command_id)?;
        let cancelled = entry.request_cancel(request.signal);
        Ok(RemoteCommandCancelResult {
            command_id: request.command_id.clone(),
            status: if cancelled {
                RemoteCommandStatus::CancelRequested
            } else {
                // 命令已结束，返回当前真实状态。
                entry.snapshot().status
            },
        })
    }

    /// 查询命令的原始命令文本，用于审批/审计。
    pub fn command_text(&self, command_id: &str) -> Option<String> {
        self.entry(command_id)
            .ok()
            .map(|entry| entry.command.clone())
    }

    fn entry(&self, command_id: &str) -> Result<Arc<CommandEntry>> {
        self.commands
            .lock()
            .expect("command store lock poisoned")
            .get(command_id)
            .cloned()
            .ok_or_else(|| anyhow!("unknown command: {command_id}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote_ops::{
        RemoteCommandCancelRequest, RemoteCommandOutputRequest, RemoteCommandSignal,
    };

    #[test]
    fn register_returns_unique_command_id() {
        let store = RemoteCommandStore::default();
        let (id1, _, _) = store.register("ssh-1", "pwd");
        let (id2, _, _) = store.register("ssh-1", "ls");
        assert_ne!(id1, id2);
    }

    #[test]
    fn poll_reports_running_then_exited() {
        let store = RemoteCommandStore::default();
        let (id, entry, _cancel_rx) = store.register("ssh-1", "sleep 1");

        let running = store.poll_by_id(&id).unwrap();
        assert_eq!(RemoteCommandStatus::Running, running.status);

        entry.complete(RemoteCommandStatus::Exited, Some(0));

        let done = store.poll_by_id(&id).unwrap();
        assert_eq!(RemoteCommandStatus::Exited, done.status);
        assert_eq!(Some(0), done.exit_code);
    }

    #[test]
    fn output_reads_incrementally_by_offset() {
        let store = RemoteCommandStore::default();
        let (id, entry, _) = store.register("ssh-1", "cat file");

        entry.push_stdout(b"hello ");
        entry.push_stdout(b"world\n");

        let first = store
            .output(&RemoteCommandOutputRequest {
                command_id: id.clone(),
                stdout_offset: 0,
                stderr_offset: 0,
                limit_bytes: Some(5),
            })
            .unwrap();
        assert_eq!("hello", first.stdout);
        assert_eq!(5, first.next_stdout_offset);
        assert!(first.truncated);

        let rest = store
            .output(&RemoteCommandOutputRequest {
                command_id: id,
                stdout_offset: 5,
                stderr_offset: 0,
                limit_bytes: None,
            })
            .unwrap();
        assert_eq!(" world\n", rest.stdout);
        assert!(!rest.truncated);
    }

    #[test]
    fn cancel_marks_running_command() {
        let store = RemoteCommandStore::default();
        let (id, _, _) = store.register("ssh-1", "sleep 300");

        let result = store
            .cancel(&RemoteCommandCancelRequest {
                command_id: id.clone(),
                signal: RemoteCommandSignal::Sigint,
            })
            .unwrap();

        assert_eq!(RemoteCommandStatus::CancelRequested, result.status);

        let poll = store.poll_by_id(&id).unwrap();
        assert_eq!(RemoteCommandStatus::CancelRequested, poll.status);
    }

    #[test]
    fn cancel_completed_command_returns_final_status() {
        let store = RemoteCommandStore::default();
        let (id, entry, _) = store.register("ssh-1", "true");
        entry.complete(RemoteCommandStatus::Exited, Some(0));

        let result = store
            .cancel(&RemoteCommandCancelRequest {
                command_id: id,
                signal: RemoteCommandSignal::Sigterm,
            })
            .unwrap();

        assert_eq!(RemoteCommandStatus::Exited, result.status);
    }

    #[test]
    fn unknown_command_returns_error() {
        let store = RemoteCommandStore::default();
        assert!(store.poll_by_id("missing").is_err());
        assert!(
            store
                .output(&RemoteCommandOutputRequest {
                    command_id: "missing".to_string(),
                    stdout_offset: 0,
                    stderr_offset: 0,
                    limit_bytes: None,
                })
                .is_err()
        );
    }
}
