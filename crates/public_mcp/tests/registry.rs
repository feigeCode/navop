use public_mcp::registry::{
    ConnectionState, PublicMcpRegistry, TerminalConnectionKind, TerminalSessionHandle,
    TerminalSessionSnapshot,
};

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

    fn visible_text(&self) -> anyhow::Result<String> {
        Ok(format!("visible {}", self.id))
    }

    fn write_external_input(&self, _data: &[u8]) -> anyhow::Result<()> {
        Ok(())
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
