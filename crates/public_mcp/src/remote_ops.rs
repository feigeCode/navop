use crate::registry::{ConnectionState, TerminalConnectionKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const DEFAULT_FOREGROUND_TIMEOUT_MS: u64 = 30_000;
pub const DEFAULT_OUTPUT_LIMIT_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RemoteCommandMode {
    #[default]
    Foreground,
    Background,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteCommandStatus {
    Running,
    Exited,
    Failed,
    CancelRequested,
    Cancelled,
    TimedOut,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteExecRequest {
    pub session_id: String,
    pub command: String,
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub mode: RemoteCommandMode,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteExecResult {
    pub status: RemoteCommandStatus,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub timed_out: bool,
    pub command_id: Option<String>,
    pub started_at_ms: Option<i64>,
}

impl RemoteExecResult {
    pub fn foreground(
        status: RemoteCommandStatus,
        stdout: String,
        stderr: String,
        exit_code: Option<i32>,
        duration_ms: u64,
        timed_out: bool,
    ) -> Self {
        Self {
            status,
            stdout,
            stderr,
            exit_code,
            duration_ms,
            timed_out,
            command_id: None,
            started_at_ms: None,
        }
    }

    pub fn background(command_id: impl Into<String>, started_at_ms: i64) -> Self {
        Self {
            status: RemoteCommandStatus::Running,
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            duration_ms: 0,
            timed_out: false,
            command_id: Some(command_id.into()),
            started_at_ms: Some(started_at_ms),
        }
    }
}

impl RemoteExecResult {
    /// 命令是否以 exit code 0 正常结束。
    pub fn is_success(&self) -> bool {
        matches!(self.status, RemoteCommandStatus::Exited) && self.exit_code == Some(0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteCommandPollRequest {
    pub command_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteCommandPollResult {
    pub command_id: String,
    pub status: RemoteCommandStatus,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteCommandOutputRequest {
    pub command_id: String,
    #[serde(default)]
    pub stdout_offset: usize,
    #[serde(default)]
    pub stderr_offset: usize,
    pub limit_bytes: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteCommandOutputResult {
    pub command_id: String,
    pub stdout: String,
    pub stderr: String,
    pub next_stdout_offset: usize,
    pub next_stderr_offset: usize,
    pub truncated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteCommandSignal {
    Sigint,
    Sigterm,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteCommandCancelRequest {
    pub command_id: String,
    #[serde(default = "default_cancel_signal")]
    pub signal: RemoteCommandSignal,
}

fn default_cancel_signal() -> RemoteCommandSignal {
    RemoteCommandSignal::Sigint
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteCommandCancelResult {
    pub command_id: String,
    pub status: RemoteCommandStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteFileWriteRequest {
    pub session_id: String,
    pub path: String,
    pub content: String,
    pub mode: Option<u32>,
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteFileWriteResult {
    pub path: String,
    pub bytes_written: usize,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionDiagnosticsRequest {
    pub session_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SessionDiagnosticsResult {
    pub session_id: String,
    pub connection_id: Option<i64>,
    pub host_label: String,
    pub cwd: Option<String>,
    pub rows: usize,
    pub cols: usize,
    pub connection_kind: TerminalConnectionKind,
    pub state: ConnectionState,
    pub last_error: Option<String>,
    pub recoverable: bool,
    pub suggested_action: Option<String>,
}
