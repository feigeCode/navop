use crate::remote_ops::{
    RemoteCommandCancelRequest, RemoteCommandCancelResult, RemoteCommandOutputRequest,
    RemoteCommandOutputResult, RemoteCommandPollResult, RemoteCommandStatus,
};
use anyhow::{Result, anyhow};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::{Notify, watch};
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
    stdout_base_offset: usize,
    stdout_snapshot: Vec<u8>,
    stderr: Vec<u8>,
    stderr_base_offset: usize,
    cancel_handle: Option<CommandCancelHandle>,
    finished_notify: Arc<Notify>,
}

impl CommandEntry {
    fn new(
        command_id: String,
        session_id: String,
        command: String,
        cancel_handle: Option<CommandCancelHandle>,
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
                stdout_base_offset: 0,
                stdout_snapshot: Vec::new(),
                stderr: Vec::new(),
                stderr_base_offset: 0,
                cancel_handle,
                finished_notify: Arc::new(Notify::new()),
            }),
        }
    }

    /// 追加输出。由执行桥在收到 channel 数据时调用。
    pub fn push_stdout(&self, data: &[u8]) {
        let mut state = self.state.lock().expect("command state lock poisoned");
        let discarded = append_bounded_tail(&mut state.stdout, data, MAX_COMMAND_BUFFER_BYTES);
        state.stdout_base_offset = state.stdout_base_offset.saturating_add(discarded);
    }

    pub fn push_stderr(&self, data: &[u8]) {
        let mut state = self.state.lock().expect("command state lock poisoned");
        let discarded = append_bounded_tail(&mut state.stderr, data, MAX_COMMAND_BUFFER_BYTES);
        state.stderr_base_offset = state.stderr_base_offset.saturating_add(discarded);
    }

    /// Incorporates a cumulative stdout snapshot without rewinding the public
    /// stream offsets already returned to clients.
    ///
    /// Terminal observers publish a bounded, sanitized snapshot on every
    /// update. Before that capture buffer fills, snapshots normally extend the
    /// previous snapshot. After it rolls, the new snapshot overlaps the suffix
    /// of the previous one. Only the newly observed suffix is appended to this
    /// entry's independently bounded stream.
    pub fn replace_stdout(&self, data: &[u8]) {
        let mut state = self.state.lock().expect("command state lock poisoned");
        let append_from = snapshot_append_offset(&state.stdout_snapshot, data);
        let discarded = append_bounded_tail(
            &mut state.stdout,
            &data[append_from..],
            MAX_COMMAND_BUFFER_BYTES,
        );
        state.stdout_base_offset = state.stdout_base_offset.saturating_add(discarded);
        state.stdout_snapshot.clear();
        state.stdout_snapshot.extend_from_slice(data);
    }

    /// 标记命令完成。由执行桥在收到 ExitStatus/ExitSignal/EOF 时调用。
    pub fn complete(&self, status: RemoteCommandStatus, exit_code: Option<i32>) {
        let mut state = self.state.lock().expect("command state lock poisoned");
        state.status = status;
        state.exit_code = exit_code;
        state.finished = Some(Instant::now());
        // 完成后不再需要取消句柄。
        state.cancel_handle = None;
        state.finished_notify.notify_waiters();
    }

    pub async fn wait_for_completion(&self, timeout: std::time::Duration) -> bool {
        if self.is_finished() {
            return true;
        }
        let notify = {
            self.state
                .lock()
                .expect("command state lock poisoned")
                .finished_notify
                .clone()
        };
        tokio::time::timeout(timeout, async {
            loop {
                let notified = notify.notified();
                if self.is_finished() {
                    return;
                }
                notified.await;
            }
        })
        .await
        .is_ok()
    }

    fn is_finished(&self) -> bool {
        self.state
            .lock()
            .expect("command state lock poisoned")
            .finished
            .is_some()
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
            stdout_len: state.stdout_base_offset.saturating_add(state.stdout.len()),
            stderr_len: state.stderr_base_offset.saturating_add(state.stderr.len()),
        }
    }

    fn read_output(&self, request: &RemoteCommandOutputRequest) -> CommandOutput {
        let state = self.state.lock().expect("command state lock poisoned");
        let limit = request
            .limit_bytes
            .unwrap_or(crate::remote_ops::DEFAULT_OUTPUT_LIMIT_BYTES);
        let stdout_slice = slice_output(
            &state.stdout,
            state.stdout_base_offset,
            request.stdout_offset,
            limit,
        );
        let remaining = limit.saturating_sub(stdout_slice.bytes.len());
        let stderr_slice = slice_output(
            &state.stderr,
            state.stderr_base_offset,
            request.stderr_offset,
            remaining,
        );
        CommandOutput {
            stdout: stdout_slice.bytes,
            stdout_start_offset: stdout_slice.start_offset,
            stdout_next_offset: stdout_slice.next_offset,
            stdout_discarded_bytes: state.stdout_base_offset,
            stderr: stderr_slice.bytes,
            stderr_start_offset: stderr_slice.start_offset,
            stderr_next_offset: stderr_slice.next_offset,
            stderr_discarded_bytes: state.stderr_base_offset,
            truncated: stdout_slice.truncated || stderr_slice.truncated,
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
    stdout_start_offset: usize,
    stdout_next_offset: usize,
    stdout_discarded_bytes: usize,
    stderr: Vec<u8>,
    stderr_start_offset: usize,
    stderr_next_offset: usize,
    stderr_discarded_bytes: usize,
    truncated: bool,
}

/// 单条命令 stdout/stderr 各自的最大缓冲。超出部分从头部滚动丢弃，保证 poll 总能拿到最近输出。
const MAX_COMMAND_BUFFER_BYTES: usize = 8 * 1024 * 1024;

fn append_bounded_tail(buffer: &mut Vec<u8>, data: &[u8], max_bytes: usize) -> usize {
    if max_bytes == 0 {
        let discarded = buffer.len().saturating_add(data.len());
        buffer.clear();
        return discarded;
    }
    if data.len() >= max_bytes {
        let discarded = buffer
            .len()
            .saturating_add(data.len().saturating_sub(max_bytes));
        buffer.clear();
        buffer.extend_from_slice(&data[data.len() - max_bytes..]);
        return discarded;
    }

    let overflow = buffer
        .len()
        .saturating_add(data.len())
        .saturating_sub(max_bytes);
    if overflow > 0 {
        buffer.drain(..overflow);
    }
    buffer.extend_from_slice(data);
    overflow
}

#[cfg(test)]
fn replace_bounded_tail(buffer: &mut Vec<u8>, data: &[u8], max_bytes: usize) -> usize {
    buffer.clear();
    let discarded = data.len().saturating_sub(max_bytes);
    if max_bytes > 0 {
        buffer.extend_from_slice(&data[discarded..]);
    }
    discarded
}

fn snapshot_append_offset(previous: &[u8], next: &[u8]) -> usize {
    if previous.is_empty() {
        return 0;
    }
    if next.starts_with(previous) {
        return previous.len();
    }
    if previous.starts_with(next) {
        return next.len();
    }
    longest_suffix_prefix_overlap(previous, next)
}

fn longest_suffix_prefix_overlap(previous: &[u8], next: &[u8]) -> usize {
    if next.is_empty() {
        return 0;
    }

    let mut prefix = vec![0; next.len()];
    for index in 1..next.len() {
        let mut matched = prefix[index - 1];
        while matched > 0 && next[index] != next[matched] {
            matched = prefix[matched - 1];
        }
        if next[index] == next[matched] {
            matched += 1;
        }
        prefix[index] = matched;
    }

    let mut matched = 0;
    for (index, byte) in previous.iter().enumerate() {
        while matched > 0 && *byte != next[matched] {
            matched = prefix[matched - 1];
        }
        if *byte == next[matched] {
            matched += 1;
        }
        if matched == next.len() && index + 1 != previous.len() {
            matched = prefix[matched - 1];
        }
    }
    matched
}

struct OutputSlice {
    bytes: Vec<u8>,
    start_offset: usize,
    next_offset: usize,
    truncated: bool,
}

fn slice_output(
    buffer: &[u8],
    base_offset: usize,
    requested_offset: usize,
    limit: usize,
) -> OutputSlice {
    let stream_end = base_offset.saturating_add(buffer.len());
    let start_offset = requested_offset.clamp(base_offset, stream_end);
    let start = start_offset.saturating_sub(base_offset);
    let end = start.saturating_add(limit).min(buffer.len());
    let missed_evicted_output = requested_offset < base_offset;
    let has_more_buffered_output = end < buffer.len();
    OutputSlice {
        bytes: buffer[start..end].to_vec(),
        start_offset,
        next_offset: base_offset.saturating_add(end),
        truncated: missed_evicted_output || has_more_buffered_output,
    }
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
            Some(cancel_handle),
        ));
        self.commands
            .lock()
            .expect("command store lock poisoned")
            .insert(command_id.clone(), entry.clone());
        (command_id, entry, cancel_rx)
    }

    pub fn register_observed(
        &self,
        session_id: &str,
        command: &str,
    ) -> (String, Arc<CommandEntry>) {
        let command_id = format!("cmd_{}", Uuid::new_v4().simple());
        let entry = Arc::new(CommandEntry::new(
            command_id.clone(),
            session_id.to_string(),
            command.to_string(),
            None,
        ));
        self.commands
            .lock()
            .expect("command store lock poisoned")
            .insert(command_id.clone(), entry.clone());
        (command_id, entry)
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
            stdout_start_offset: out.stdout_start_offset,
            stderr_start_offset: out.stderr_start_offset,
            next_stdout_offset: out.stdout_next_offset,
            next_stderr_offset: out.stderr_next_offset,
            stdout_discarded_bytes: out.stdout_discarded_bytes,
            stderr_discarded_bytes: out.stderr_discarded_bytes,
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

    pub fn remove(&self, command_id: &str) {
        self.commands
            .lock()
            .expect("command store lock poisoned")
            .remove(command_id);
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
    fn bounded_tail_rolls_old_bytes_when_append_crosses_limit() {
        let mut buffer = b"abcdef".to_vec();

        let discarded = append_bounded_tail(&mut buffer, b"ghij", 8);

        assert_eq!(b"cdefghij", buffer.as_slice());
        assert_eq!(2, discarded);
    }

    #[test]
    fn bounded_tail_keeps_end_of_an_oversized_chunk() {
        let mut buffer = b"old".to_vec();

        let discarded = append_bounded_tail(&mut buffer, b"0123456789", 6);

        assert_eq!(b"456789", buffer.as_slice());
        assert_eq!(7, discarded);
    }

    #[test]
    fn replace_bounded_tail_keeps_newest_bytes() {
        let mut buffer = b"old".to_vec();

        let base_offset = replace_bounded_tail(&mut buffer, b"0123456789", 6);

        assert_eq!(b"456789", buffer.as_slice());
        assert_eq!(4, base_offset);
    }

    #[test]
    fn rolling_output_offsets_remain_absolute_after_eviction() {
        let entry = CommandEntry::new(
            "cmd-1".to_string(),
            "ssh-1".to_string(),
            "cat".to_string(),
            None,
        );
        let prefix = vec![b'a'; MAX_COMMAND_BUFFER_BYTES - 2];
        entry.push_stdout(&prefix);
        entry.push_stdout(b"BCDE");

        let output = entry.read_output(&RemoteCommandOutputRequest {
            command_id: "cmd-1".to_string(),
            stdout_offset: 0,
            stderr_offset: 0,
            limit_bytes: Some(MAX_COMMAND_BUFFER_BYTES),
        });

        assert_eq!(2, output.stdout_start_offset);
        assert_eq!(MAX_COMMAND_BUFFER_BYTES + 2, output.stdout_next_offset);
        assert_eq!(2, output.stdout_discarded_bytes);
        assert_eq!(MAX_COMMAND_BUFFER_BYTES, output.stdout.len());
        assert_eq!(b"BCDE", &output.stdout[output.stdout.len() - 4..]);
        assert!(output.truncated);
    }

    #[test]
    fn old_absolute_offset_resynchronizes_to_retained_tail() {
        let mut buffer = b"abcdef".to_vec();
        let base_offset = append_bounded_tail(&mut buffer, b"ghij", 8);

        let output = slice_output(&buffer, base_offset, 0, 3);

        assert_eq!(b"cde", output.bytes.as_slice());
        assert_eq!(2, output.start_offset);
        assert_eq!(5, output.next_offset);
        assert!(output.truncated);
    }

    #[test]
    fn offset_beyond_stream_end_returns_absolute_end() {
        let output = slice_output(b"cdefghij", 2, 100, 8);

        assert!(output.bytes.is_empty());
        assert_eq!(10, output.start_offset);
        assert_eq!(10, output.next_offset);
        assert!(!output.truncated);
    }

    #[test]
    fn cumulative_snapshot_appends_only_new_output() {
        let entry = CommandEntry::new(
            "cmd-1".to_string(),
            "ssh-1".to_string(),
            "echo".to_string(),
            None,
        );
        entry.replace_stdout(b"first");
        entry.replace_stdout(b"first second");

        let output = entry.read_output(&RemoteCommandOutputRequest {
            command_id: "cmd-1".to_string(),
            stdout_offset: 0,
            stderr_offset: 0,
            limit_bytes: Some(MAX_COMMAND_BUFFER_BYTES),
        });

        assert_eq!(b"first second", output.stdout.as_slice());
        assert_eq!(0, output.stdout_start_offset);
        assert_eq!(12, output.stdout_next_offset);
        assert_eq!(0, output.stdout_discarded_bytes);
    }

    #[test]
    fn rolling_snapshot_overlap_keeps_offsets_monotonic_without_duplicates() {
        let entry = CommandEntry::new(
            "cmd-1".to_string(),
            "ssh-1".to_string(),
            "stream".to_string(),
            None,
        );
        entry.replace_stdout(b"abcdefgh");
        entry.replace_stdout(b"defghijk");

        let output = entry.read_output(&RemoteCommandOutputRequest {
            command_id: "cmd-1".to_string(),
            stdout_offset: 0,
            stderr_offset: 0,
            limit_bytes: Some(MAX_COMMAND_BUFFER_BYTES),
        });

        assert_eq!(b"abcdefghijk", output.stdout.as_slice());
        assert_eq!(0, output.stdout_start_offset);
        assert_eq!(11, output.stdout_next_offset);
        assert_eq!(0, output.stdout_discarded_bytes);
    }

    #[test]
    fn rewritten_shorter_snapshot_does_not_replay_existing_prefix() {
        let entry = CommandEntry::new(
            "cmd-1".to_string(),
            "ssh-1".to_string(),
            "stream".to_string(),
            None,
        );
        entry.replace_stdout(b"stable transient");
        entry.replace_stdout(b"stable");

        let output = entry.read_output(&RemoteCommandOutputRequest {
            command_id: "cmd-1".to_string(),
            stdout_offset: 0,
            stderr_offset: 0,
            limit_bytes: Some(MAX_COMMAND_BUFFER_BYTES),
        });

        assert_eq!(b"stable transient", output.stdout.as_slice());
        assert_eq!(16, output.stdout_next_offset);
    }

    #[test]
    fn oversized_replacement_reports_snapshot_tail_base() {
        let entry = CommandEntry::new(
            "cmd-1".to_string(),
            "ssh-1".to_string(),
            "cat".to_string(),
            None,
        );
        let output_bytes = vec![b'x'; MAX_COMMAND_BUFFER_BYTES + 7];
        entry.replace_stdout(&output_bytes);

        let output = entry.read_output(&RemoteCommandOutputRequest {
            command_id: "cmd-1".to_string(),
            stdout_offset: 0,
            stderr_offset: 0,
            limit_bytes: Some(MAX_COMMAND_BUFFER_BYTES),
        });

        assert_eq!(7, output.stdout_start_offset);
        assert_eq!(MAX_COMMAND_BUFFER_BYTES + 7, output.stdout_next_offset);
        assert_eq!(7, output.stdout_discarded_bytes);
        assert_eq!(MAX_COMMAND_BUFFER_BYTES, output.stdout.len());
        assert!(output.truncated);
    }

    #[test]
    fn poll_byte_totals_are_monotonic_across_rolling_eviction() {
        let store = RemoteCommandStore::default();
        let (id, entry, _) = store.register("ssh-1", "cat");
        let prefix = vec![b'a'; MAX_COMMAND_BUFFER_BYTES];
        entry.push_stdout(&prefix);
        let first = store.poll_by_id(&id).unwrap();

        entry.push_stdout(b"more");
        let second = store.poll_by_id(&id).unwrap();

        assert_eq!(MAX_COMMAND_BUFFER_BYTES, first.stdout_bytes);
        assert_eq!(MAX_COMMAND_BUFFER_BYTES + 4, second.stdout_bytes);
    }

    #[test]
    fn stdout_and_stderr_use_the_same_rolling_tail_policy() {
        let entry = CommandEntry::new(
            "cmd-1".to_string(),
            "ssh-1".to_string(),
            "echo".to_string(),
            None,
        );
        let first = vec![b'a'; MAX_COMMAND_BUFFER_BYTES - 2];
        entry.push_stdout(&first);
        entry.push_stdout(b"BCDE");
        entry.push_stderr(&first);
        entry.push_stderr(b"BCDE");

        let state = entry.state.lock().expect("command state lock poisoned");
        assert_eq!(MAX_COMMAND_BUFFER_BYTES, state.stdout.len());
        assert_eq!(MAX_COMMAND_BUFFER_BYTES, state.stderr.len());
        assert_eq!(b"BCDE", &state.stdout[state.stdout.len() - 4..]);
        assert_eq!(b"BCDE", &state.stderr[state.stderr.len() - 4..]);
    }

    #[test]
    fn stdout_and_stderr_report_independent_absolute_offsets() {
        let entry = CommandEntry::new(
            "cmd-1".to_string(),
            "ssh-1".to_string(),
            "mixed-output".to_string(),
            None,
        );
        {
            let mut state = entry.state.lock().expect("command state lock poisoned");
            state.stdout = b"stdout-tail".to_vec();
            state.stdout_base_offset = 11;
            state.stderr = b"err-tail".to_vec();
            state.stderr_base_offset = 3;
        }

        let output = entry.read_output(&RemoteCommandOutputRequest {
            command_id: "cmd-1".to_string(),
            stdout_offset: 0,
            stderr_offset: 0,
            limit_bytes: Some(64),
        });

        assert_eq!(b"stdout-tail", output.stdout.as_slice());
        assert_eq!(11, output.stdout_start_offset);
        assert_eq!(22, output.stdout_next_offset);
        assert_eq!(11, output.stdout_discarded_bytes);
        assert_eq!(b"err-tail", output.stderr.as_slice());
        assert_eq!(3, output.stderr_start_offset);
        assert_eq!(11, output.stderr_next_offset);
        assert_eq!(3, output.stderr_discarded_bytes);
        assert!(output.truncated);
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

    #[tokio::test]
    async fn observed_command_can_be_read_after_wait_timeout() {
        let store = RemoteCommandStore::default();
        let (id, entry) = store.register_observed("ssh-1", "sleep 1");

        assert!(
            !entry
                .wait_for_completion(std::time::Duration::from_millis(1))
                .await
        );
        entry.push_stdout(b"still running\n");
        let output = store
            .output(&RemoteCommandOutputRequest {
                command_id: id.clone(),
                stdout_offset: 0,
                stderr_offset: 0,
                limit_bytes: None,
            })
            .unwrap();
        assert_eq!("still running\n", output.stdout);
        assert_eq!(
            RemoteCommandStatus::Running,
            store.poll_by_id(&id).unwrap().status
        );

        entry.complete(RemoteCommandStatus::Exited, Some(0));
        assert!(
            entry
                .wait_for_completion(std::time::Duration::from_secs(1))
                .await
        );
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
