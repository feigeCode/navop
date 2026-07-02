use gpui::{App, Global};
use public_mcp::registry::{
    ConnectionState as McpConnectionState, PublicMcpRegistry, TerminalConnectionKind as McpKind,
    TerminalSessionHandle, TerminalSessionSnapshot,
};
use std::sync::{Arc, Mutex};
use terminal::terminal::{ConnectionState, Terminal, TerminalConnectionKind};
use uuid::Uuid;

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
            registry.unregister_remote_ops(&self.session_id);
            registry.unregister_terminal_exec(&self.session_id);
        }
    }
}

pub fn register_terminal(terminal: &Terminal, cx: &App) -> Option<TerminalPublicMcpRegistration> {
    if terminal.connection_kind() != TerminalConnectionKind::Ssh {
        return None;
    }

    let connection_id = terminal.connection_id()?;
    let session_id = format!("ssh-terminal-{connection_id}-{}", Uuid::new_v4());
    let state = Arc::new(Mutex::new(snapshot_for_terminal(
        session_id.clone(),
        terminal,
    )));
    let target_registry = registry(cx)?;
    target_registry.register(ThreadSafeTerminalHandle {
        state: state.clone(),
    });

    // 注册结构化远程操作桥。remote ops 与 terminal handle 共享同一份 state，一次 refresh 同步两者。
    if let Some(session_manager) = terminal.ssh_session_manager() {
        let command_store = target_registry.command_store().clone();
        let remote_ops = crate::public_mcp_remote_ops::SshRemoteOpsHandle::with_shared_state(
            session_manager.clone(),
            state.clone(),
            command_store,
        );
        target_registry.register_remote_ops(remote_ops);
    }

    Some(TerminalPublicMcpRegistration { session_id, state })
}

struct ThreadSafeTerminalHandle {
    state: Arc<Mutex<TerminalSessionSnapshot>>,
}

impl TerminalSessionHandle for ThreadSafeTerminalHandle {
    fn snapshot(&self) -> TerminalSessionSnapshot {
        self.state
            .lock()
            .expect("public MCP state lock poisoned")
            .clone()
    }
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
        .or_else(|| {
            terminal
                .ssh_config()
                .map(|config| config.ssh_config.host.as_str())
        })
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
