use crate::command_store::RemoteCommandStore;
use crate::remote_ops::{
    RemoteCommandCancelRequest, RemoteCommandCancelResult, RemoteCommandOutputRequest,
    RemoteCommandOutputResult, RemoteCommandPollRequest, RemoteCommandPollResult,
    RemoteExecRequest, RemoteExecResult, RemoteFileWriteRequest, RemoteFileWriteResult,
    SessionDiagnosticsRequest, SessionDiagnosticsResult,
};
use anyhow::{Result, anyhow};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalConnectionKind {
    Local,
    Ssh,
    Serial,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Connected,
    Connecting,
    Disconnected { error: Option<String> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TerminalSessionSnapshot {
    pub session_id: String,
    pub connection_id: Option<i64>,
    pub title: String,
    pub host_label: String,
    pub cwd: Option<String>,
    pub rows: usize,
    pub cols: usize,
    pub connection_kind: TerminalConnectionKind,
    pub connection_state: ConnectionState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PublicMcpSessionInfo {
    pub session_id: String,
    pub connection_id: Option<i64>,
    pub title: String,
    pub host_label: String,
    pub cwd: Option<String>,
    pub rows: usize,
    pub cols: usize,
    pub connected: bool,
}

pub trait TerminalSessionHandle: Send + Sync + 'static {
    fn snapshot(&self) -> TerminalSessionSnapshot;
}

/// 结构化远程操作能力。由可执行非交互命令的活动 SSH 会话实现。
/// 与 `TerminalSessionHandle` 分离，使 terminal 桥接与远程执行通道可独立测试与演进。
///
/// 第一版只覆盖 foreground exec、file write 与 session diagnostics。
/// background command lifecycle（poll/output/cancel）由独立 command registry 支持，
/// 待 terminal-view SSH 执行桥落地后补齐。
pub trait RemoteOpsSessionHandle: Send + Sync + 'static {
    fn snapshot(&self) -> TerminalSessionSnapshot;
    fn exec(&self, request: RemoteExecRequest) -> Result<RemoteExecResult>;
    fn write_file(&self, request: RemoteFileWriteRequest) -> Result<RemoteFileWriteResult>;
    fn diagnostics(&self, request: SessionDiagnosticsRequest) -> Result<SessionDiagnosticsResult>;
}

#[derive(Clone, Default)]
pub struct PublicMcpRegistry {
    sessions: Arc<Mutex<HashMap<String, Arc<dyn TerminalSessionHandle>>>>,
    remote_ops_sessions: Arc<Mutex<HashMap<String, Arc<dyn RemoteOpsSessionHandle>>>>,
    command_store: RemoteCommandStore,
}

impl PublicMcpRegistry {
    pub fn register(&self, handle: impl TerminalSessionHandle) {
        let snapshot = handle.snapshot();
        self.sessions
            .lock()
            .expect("public MCP registry lock poisoned")
            .insert(snapshot.session_id, Arc::new(handle));
    }

    pub fn unregister(&self, session_id: &str) {
        self.sessions
            .lock()
            .expect("public MCP registry lock poisoned")
            .remove(session_id);
    }

    pub fn list_sessions(&self) -> Vec<PublicMcpSessionInfo> {
        let sessions = self
            .sessions
            .lock()
            .expect("public MCP registry lock poisoned")
            .values()
            .cloned()
            .collect::<Vec<_>>();

        sessions
            .iter()
            .filter_map(|handle| exposed_session_info(handle.snapshot()))
            .collect()
    }

    /// 注册一个可执行结构化远程操作的会话。session_id 必须与同名 terminal session 对应。
    pub fn register_remote_ops(&self, handle: impl RemoteOpsSessionHandle) {
        let snapshot = handle.snapshot();
        self.remote_ops_sessions
            .lock()
            .expect("public MCP registry lock poisoned")
            .insert(snapshot.session_id, Arc::new(handle));
    }

    pub fn unregister_remote_ops(&self, session_id: &str) {
        self.remote_ops_sessions
            .lock()
            .expect("public MCP registry lock poisoned")
            .remove(session_id);
    }

    pub fn remote_exec(
        &self,
        session_id: &str,
        request: RemoteExecRequest,
    ) -> Result<RemoteExecResult> {
        let handle = self.remote_ops_handle(session_id)?;
        ensure_exposed_session(&handle.snapshot())?;
        handle.exec(request)
    }

    pub fn remote_file_write(
        &self,
        session_id: &str,
        request: RemoteFileWriteRequest,
    ) -> Result<RemoteFileWriteResult> {
        let handle = self.remote_ops_handle(session_id)?;
        ensure_exposed_session(&handle.snapshot())?;
        handle.write_file(request)
    }

    /// background 命令存储。执行桥用它注册命令，MCP 工具用它 poll/output/cancel。
    pub fn command_store(&self) -> &RemoteCommandStore {
        &self.command_store
    }

    pub fn remote_command_poll(
        &self,
        request: RemoteCommandPollRequest,
    ) -> Result<RemoteCommandPollResult> {
        self.command_store.poll_by_id(&request.command_id)
    }

    pub fn remote_command_output(
        &self,
        request: RemoteCommandOutputRequest,
    ) -> Result<RemoteCommandOutputResult> {
        self.command_store.output(&request)
    }

    pub fn remote_command_cancel(
        &self,
        request: RemoteCommandCancelRequest,
    ) -> Result<RemoteCommandCancelResult> {
        self.command_store.cancel(&request)
    }

    /// 会话诊断。已知会话返回结构化诊断结果；未知会话返回 unknown_session 错误。
    pub fn session_diagnostics(
        &self,
        request: SessionDiagnosticsRequest,
    ) -> Result<SessionDiagnosticsResult> {
        match self
            .remote_ops_sessions
            .lock()
            .expect("public MCP registry lock poisoned")
            .get(&request.session_id)
            .cloned()
        {
            Some(handle) => handle.diagnostics(request),
            None => {
                // 回退到 terminal session，至少能给出 connected 状态。
                match self.handle(&request.session_id) {
                    Ok(handle) => Ok(diagnostics_from_snapshot(handle.snapshot())),
                    Err(_) => Err(anyhow!(unknown_session_error(&request.session_id))),
                }
            }
        }
    }

    fn remote_ops_handle(&self, session_id: &str) -> Result<Arc<dyn RemoteOpsSessionHandle>> {
        self.remote_ops_sessions
            .lock()
            .expect("public MCP registry lock poisoned")
            .get(session_id)
            .cloned()
            .ok_or_else(|| anyhow!(unknown_session_error(session_id)))
    }

    fn handle(&self, session_id: &str) -> Result<Arc<dyn TerminalSessionHandle>> {
        self.sessions
            .lock()
            .expect("public MCP registry lock poisoned")
            .get(session_id)
            .cloned()
            .ok_or_else(|| anyhow!("unknown public MCP terminal session: {session_id}"))
    }
}

fn exposed_session_info(snapshot: TerminalSessionSnapshot) -> Option<PublicMcpSessionInfo> {
    if !is_exposed(&snapshot) {
        return None;
    }

    Some(PublicMcpSessionInfo {
        session_id: snapshot.session_id,
        connection_id: snapshot.connection_id,
        title: snapshot.title,
        host_label: snapshot.host_label,
        cwd: snapshot.cwd,
        rows: snapshot.rows,
        cols: snapshot.cols,
        connected: true,
    })
}

fn ensure_exposed_session(snapshot: &TerminalSessionSnapshot) -> Result<()> {
    if is_exposed(snapshot) {
        return Ok(());
    }
    Err(anyhow!(
        "terminal session is not an exposed connected SSH session"
    ))
}

fn is_exposed(snapshot: &TerminalSessionSnapshot) -> bool {
    snapshot.connection_kind == TerminalConnectionKind::Ssh
        && matches!(snapshot.connection_state, ConnectionState::Connected)
}

fn diagnostics_from_snapshot(snapshot: TerminalSessionSnapshot) -> SessionDiagnosticsResult {
    let connection_state = snapshot.connection_state.clone();
    let (recoverable, suggested_action) = match &connection_state {
        ConnectionState::Connected => (true, None),
        ConnectionState::Connecting => (true, Some("wait_for_connection".to_string())),
        ConnectionState::Disconnected { .. } => (true, Some("reconnect_in_onetcli".to_string())),
    };
    let last_error = match &connection_state {
        ConnectionState::Disconnected { error } => error.clone(),
        _ => None,
    };

    SessionDiagnosticsResult {
        session_id: snapshot.session_id,
        connection_id: snapshot.connection_id,
        host_label: snapshot.host_label,
        cwd: snapshot.cwd,
        rows: snapshot.rows,
        cols: snapshot.cols,
        connection_kind: snapshot.connection_kind,
        state: connection_state,
        last_error,
        recoverable,
        suggested_action,
    }
}

fn unknown_session_error(session_id: &str) -> String {
    format!("unknown public MCP session: {session_id}")
}
