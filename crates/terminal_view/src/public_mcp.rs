use gpui::{App, Global};
use public_mcp::registry::{
    ConnectionState as McpConnectionState, PublicMcpRegistry, TerminalConnectionKind as McpKind,
    TerminalExecSessionHandle, TerminalSessionHandle, TerminalSessionSnapshot,
};
use public_mcp::terminal_exec::{TerminalExecCompletion, TerminalExecRequest, TerminalExecResult};
use std::sync::{Arc, Mutex};
use terminal::TerminalInputHandle;
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
    if let Some(input) = terminal.external_input_handle() {
        target_registry.register_terminal_exec(ThreadSafeTerminalExecHandle {
            state: state.clone(),
            input,
        });
    }

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

struct ThreadSafeTerminalExecHandle {
    state: Arc<Mutex<TerminalSessionSnapshot>>,
    input: TerminalInputHandle,
}

impl TerminalExecSessionHandle for ThreadSafeTerminalExecHandle {
    fn snapshot(&self) -> TerminalSessionSnapshot {
        self.state
            .lock()
            .expect("public MCP state lock poisoned")
            .clone()
    }

    fn exec_in_terminal(&self, request: TerminalExecRequest) -> anyhow::Result<TerminalExecResult> {
        let mut input = request.command.clone().into_bytes();
        if request.submit {
            input.push(b'\n');
        }
        self.input.write(input);
        Ok(TerminalExecResult {
            target: request.target,
            command: request.command,
            submitted: request.submit,
            completion: TerminalExecCompletion::SubmittedOnly,
            exit_code: None,
            output: String::new(),
            duration_ms: 0,
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn terminal_exec_handle_writes_command_to_terminal_input() {
        let written = Arc::new(Mutex::new(Vec::new()));
        let sink = written.clone();
        let handle = ThreadSafeTerminalExecHandle {
            state: Arc::new(Mutex::new(TerminalSessionSnapshot {
                session_id: "terminal-1".to_string(),
                connection_id: Some(42),
                title: "terminal".to_string(),
                host_label: "prod-a".to_string(),
                cwd: Some("/root".to_string()),
                rows: 24,
                cols: 120,
                connection_kind: McpKind::Ssh,
                connection_state: McpConnectionState::Connected,
            })),
            input: TerminalInputHandle::new(move |bytes| {
                sink.lock().expect("written lock").push(bytes);
            }),
        };

        let result = handle
            .exec_in_terminal(TerminalExecRequest {
                target: "terminal-1".to_string(),
                command: "df -h".to_string(),
                submit: true,
                wait_for_output: true,
                timeout_ms: None,
            })
            .expect("terminal exec should write input");

        assert_eq!(vec![b"df -h\n".to_vec()], *written.lock().unwrap());
        assert_eq!(TerminalExecCompletion::SubmittedOnly, result.completion);
        assert_eq!(None, result.exit_code);
        assert!(result.output.is_empty());
    }
}
