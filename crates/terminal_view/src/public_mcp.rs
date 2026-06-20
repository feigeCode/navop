use anyhow::{Result, anyhow};
use gpui::{App, Global};
use public_mcp::registry::{
    ConnectionState as McpConnectionState, PublicMcpRegistry, TerminalConnectionKind as McpKind,
    TerminalSessionHandle, TerminalSessionSnapshot,
};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;
use terminal::terminal::{ConnectionState, Terminal, TerminalConnectionKind};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use uuid::Uuid;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);

pub struct GlobalPublicMcpRegistry(pub PublicMcpRegistry);

impl Global for GlobalPublicMcpRegistry {}

pub fn init(cx: &mut App) {
    if cx.try_global::<GlobalPublicMcpRegistry>().is_none() {
        cx.set_global(GlobalPublicMcpRegistry(PublicMcpRegistry::default()));
    }
}

pub fn registry(cx: &App) -> Option<PublicMcpRegistry> {
    cx.try_global::<GlobalPublicMcpRegistry>()
        .map(|global| global.0.clone())
}

pub struct TerminalPublicMcpRegistration {
    session_id: String,
    state: Arc<Mutex<TerminalSessionSnapshot>>,
}

impl TerminalPublicMcpRegistration {
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn refresh(&self, terminal: &Terminal) {
        let mut state = self.state.lock().expect("public MCP state lock poisoned");
        *state = snapshot_for_terminal(self.session_id.clone(), terminal);
    }

    pub fn unregister(&self, cx: &App) {
        if let Some(registry) = registry(cx) {
            registry.unregister(&self.session_id);
        }
    }
}

pub enum TerminalPublicMcpCommand {
    VisibleText(mpsc::Sender<Result<String, String>>),
    Write(Vec<u8>, mpsc::Sender<Result<(), String>>),
}

pub fn register_terminal(
    terminal: &Terminal,
    cx: &App,
) -> Option<(
    TerminalPublicMcpRegistration,
    UnboundedReceiver<TerminalPublicMcpCommand>,
)> {
    if terminal.connection_kind() != TerminalConnectionKind::Ssh {
        return None;
    }

    let connection_id = terminal.connection_id()?;
    let session_id = format!("ssh-terminal-{connection_id}-{}", Uuid::new_v4());
    let state = Arc::new(Mutex::new(snapshot_for_terminal(
        session_id.clone(),
        terminal,
    )));
    let (command_tx, command_rx) = unbounded_channel();
    let handle = ThreadSafeTerminalHandle {
        state: state.clone(),
        command_tx,
    };
    registry(cx)?.register(handle);

    Some((
        TerminalPublicMcpRegistration { session_id, state },
        command_rx,
    ))
}

struct ThreadSafeTerminalHandle {
    state: Arc<Mutex<TerminalSessionSnapshot>>,
    command_tx: UnboundedSender<TerminalPublicMcpCommand>,
}

impl TerminalSessionHandle for ThreadSafeTerminalHandle {
    fn snapshot(&self) -> TerminalSessionSnapshot {
        self.state
            .lock()
            .expect("public MCP state lock poisoned")
            .clone()
    }

    fn visible_text(&self) -> Result<String> {
        let (tx, rx) = mpsc::channel();
        self.command_tx
            .send(TerminalPublicMcpCommand::VisibleText(tx))
            .map_err(|_| anyhow!("terminal MCP command channel closed"))?;
        recv_response(rx)
    }

    fn write_external_input(&self, data: &[u8]) -> Result<()> {
        let (tx, rx) = mpsc::channel();
        self.command_tx
            .send(TerminalPublicMcpCommand::Write(data.to_vec(), tx))
            .map_err(|_| anyhow!("terminal MCP command channel closed"))?;
        recv_response(rx)
    }
}

fn recv_response<T>(rx: mpsc::Receiver<Result<T, String>>) -> Result<T> {
    rx.recv_timeout(COMMAND_TIMEOUT)
        .map_err(|_| anyhow!("terminal MCP command timed out"))?
        .map_err(|message| anyhow!(message))
}

fn snapshot_for_terminal(session_id: String, terminal: &Terminal) -> TerminalSessionSnapshot {
    TerminalSessionSnapshot {
        session_id,
        connection_id: terminal.connection_id(),
        title: terminal.title().to_string(),
        host_label: host_label(terminal),
        cwd: terminal.current_working_dir().map(str::to_string),
        rows: terminal.rows(),
        cols: terminal.cols(),
        connection_kind: map_kind(terminal.connection_kind()),
        connection_state: map_state(terminal.connection_state()),
    }
}

fn host_label(terminal: &Terminal) -> String {
    terminal
        .connection_name()
        .or_else(|| terminal.ssh_config().map(|config| config.ssh_config.host.as_str()))
        .unwrap_or("ssh terminal")
        .to_string()
}

fn map_kind(kind: TerminalConnectionKind) -> McpKind {
    match kind {
        TerminalConnectionKind::Local => McpKind::Local,
        TerminalConnectionKind::Ssh => McpKind::Ssh,
        TerminalConnectionKind::Serial => McpKind::Serial,
    }
}

fn map_state(state: &ConnectionState) -> McpConnectionState {
    match state {
        ConnectionState::Connected => McpConnectionState::Connected,
        ConnectionState::Connecting => McpConnectionState::Connecting,
        ConnectionState::Disconnected { error } => McpConnectionState::Disconnected {
            error: error.clone(),
        },
    }
}
