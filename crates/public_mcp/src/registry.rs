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
    fn visible_text(&self) -> Result<String>;
    fn write_external_input(&self, data: &[u8]) -> Result<()>;
}

#[derive(Clone, Default)]
pub struct PublicMcpRegistry {
    sessions: Arc<Mutex<HashMap<String, Arc<dyn TerminalSessionHandle>>>>,
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

    pub fn terminal_snapshot(&self, session_id: &str) -> Result<TerminalSnapshot> {
        let handle = self.handle(session_id)?;
        let snapshot = handle.snapshot();
        ensure_exposed_session(&snapshot)?;
        Ok(TerminalSnapshot {
            session: exposed_session_info(snapshot).expect("validated exposed session"),
            visible_text: handle.visible_text()?,
        })
    }

    pub fn write_terminal(&self, session_id: &str, input: &str) -> Result<()> {
        let handle = self.handle(session_id)?;
        ensure_exposed_session(&handle.snapshot())?;
        handle.write_external_input(input.as_bytes())
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TerminalSnapshot {
    pub session: PublicMcpSessionInfo,
    pub visible_text: String,
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
