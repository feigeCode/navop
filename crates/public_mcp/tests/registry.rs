use public_mcp::registry::{
    ConnectionState, PublicMcpRegistry, RemoteOpsSessionHandle, TerminalConnectionKind,
    TerminalSessionHandle, TerminalSessionSnapshot,
};
use public_mcp::remote_ops::{
    RemoteCommandMode, RemoteExecRequest, RemoteExecResult, RemoteFileWriteRequest,
    RemoteFileWriteResult, SessionDiagnosticsRequest, SessionDiagnosticsResult,
};
use std::collections::BTreeMap;

#[derive(Clone)]
struct FakeTerminal {
    id: String,
    kind: TerminalConnectionKind,
    state: ConnectionState,
}

impl TerminalSessionHandle for FakeTerminal {
    fn snapshot(&self) -> TerminalSessionSnapshot {
        TerminalSessionSnapshot {
            session_id: self.id.clone(),
            connection_id: Some(7),
            title: format!("session {}", self.id),
            host_label: "example.test".to_string(),
            cwd: Some("/home/app".to_string()),
            rows: 24,
            cols: 80,
            connection_kind: self.kind,
            connection_state: self.state.clone(),
        }
    }
}

#[test]
fn list_sessions_only_exposes_connected_ssh_sessions() {
    let registry = PublicMcpRegistry::default();
    registry.register(FakeTerminal {
        id: "ssh-ready".to_string(),
        kind: TerminalConnectionKind::Ssh,
        state: ConnectionState::Connected,
    });
    registry.register(FakeTerminal {
        id: "local-ready".to_string(),
        kind: TerminalConnectionKind::Local,
        state: ConnectionState::Connected,
    });
    registry.register(FakeTerminal {
        id: "ssh-connecting".to_string(),
        kind: TerminalConnectionKind::Ssh,
        state: ConnectionState::Connecting,
    });

    let sessions = registry.list_sessions();

    assert_eq!(1, sessions.len());
    assert_eq!("ssh-ready", sessions[0].session_id);
    assert_eq!("example.test", sessions[0].host_label);
}

#[test]
fn unregister_removes_session() {
    let registry = PublicMcpRegistry::default();
    registry.register(FakeTerminal {
        id: "ssh-ready".to_string(),
        kind: TerminalConnectionKind::Ssh,
        state: ConnectionState::Connected,
    });

    registry.unregister("ssh-ready");

    assert!(registry.list_sessions().is_empty());
}

#[derive(Clone)]
struct FakeRemoteOps {
    terminal: FakeTerminal,
}

impl RemoteOpsSessionHandle for FakeRemoteOps {
    fn snapshot(&self) -> TerminalSessionSnapshot {
        self.terminal.snapshot()
    }

    fn exec(&self, request: RemoteExecRequest) -> anyhow::Result<RemoteExecResult> {
        Ok(RemoteExecResult::foreground(
            public_mcp::remote_ops::RemoteCommandStatus::Exited,
            format!("ran: {} in {:?}", request.command, request.cwd),
            String::new(),
            Some(0),
            10,
            false,
        ))
    }

    fn write_file(
        &self,
        _request: RemoteFileWriteRequest,
    ) -> anyhow::Result<RemoteFileWriteResult> {
        Ok(RemoteFileWriteResult {
            path: "/tmp/x".to_string(),
            bytes_written: 3,
            sha256: "abc".to_string(),
        })
    }

    fn diagnostics(
        &self,
        request: SessionDiagnosticsRequest,
    ) -> anyhow::Result<SessionDiagnosticsResult> {
        Ok(SessionDiagnosticsResult {
            session_id: request.session_id,
            connection_id: Some(7),
            host_label: "example.test".to_string(),
            cwd: Some("/home/app".to_string()),
            rows: 24,
            cols: 80,
            connection_kind: TerminalConnectionKind::Ssh,
            state: ConnectionState::Connected,
            last_error: None,
            recoverable: true,
            suggested_action: None,
        })
    }
}

fn fake_exec_request(session: &str) -> RemoteExecRequest {
    RemoteExecRequest {
        session_id: session.to_string(),
        command: "pwd".to_string(),
        cwd: None,
        env: BTreeMap::new(),
        timeout_ms: None,
        mode: RemoteCommandMode::Foreground,
    }
}

#[test]
fn remote_exec_accepts_connected_ssh_session() {
    let registry = PublicMcpRegistry::default();
    registry.register_remote_ops(FakeRemoteOps {
        terminal: FakeTerminal {
            id: "ssh-ready".to_string(),
            kind: TerminalConnectionKind::Ssh,
            state: ConnectionState::Connected,
        },
    });

    let result = registry
        .remote_exec("ssh-ready", fake_exec_request("ssh-ready"))
        .expect("connected SSH session should accept remote exec");

    assert_eq!(Some(0), result.exit_code);
    assert_eq!(
        public_mcp::remote_ops::RemoteCommandStatus::Exited,
        result.status
    );
}

#[test]
fn remote_exec_rejects_local_session() {
    let registry = PublicMcpRegistry::default();
    registry.register_remote_ops(FakeRemoteOps {
        terminal: FakeTerminal {
            id: "local-ready".to_string(),
            kind: TerminalConnectionKind::Local,
            state: ConnectionState::Connected,
        },
    });

    let err = registry
        .remote_exec("local-ready", fake_exec_request("local-ready"))
        .expect_err("local session should reject remote exec");

    assert!(err.to_string().contains("exposed connected SSH session"));
}

#[test]
fn session_diagnostics_reports_unknown_session_error() {
    let registry = PublicMcpRegistry::default();

    let err = registry
        .session_diagnostics(SessionDiagnosticsRequest {
            session_id: "missing".to_string(),
        })
        .expect_err("unknown session should error");

    assert!(
        err.to_string()
            .contains("unknown public MCP session: missing")
    );
}

#[test]
fn session_diagnostics_recovers_from_terminal_registry() {
    let registry = PublicMcpRegistry::default();
    registry.register(FakeTerminal {
        id: "ssh-only-terminal".to_string(),
        kind: TerminalConnectionKind::Ssh,
        state: ConnectionState::Connected,
    });

    let result = registry
        .session_diagnostics(SessionDiagnosticsRequest {
            session_id: "ssh-only-terminal".to_string(),
        })
        .expect("terminal-only session should still produce diagnostics");

    assert_eq!("ssh-only-terminal", result.session_id);
    assert_eq!(ConnectionState::Connected, result.state);
    assert_eq!(TerminalConnectionKind::Ssh, result.connection_kind);
    assert!(result.recoverable);
}
