use public_mcp::permissions::{
    ApprovalDecision, PermissionMode, PublicMcpOperationKind, decide_permission,
};
use public_mcp::registry::{
    ConnectionState, PublicMcpRegistry, RemoteOpsSessionHandle, TerminalConnectionKind,
    TerminalSessionHandle, TerminalSessionSnapshot,
};
use public_mcp::remote_ops::{
    RemoteCommandMode, RemoteCommandStatus, RemoteExecRequest, RemoteExecResult,
    RemoteFileWriteRequest, RemoteFileWriteResult, SessionDiagnosticsRequest,
    SessionDiagnosticsResult,
};
use public_mcp::tools::{PublicMcpToolRegistry, remote_ops_tool_registry};
use serde_json::json;
use std::collections::BTreeMap;
use tool_runtime::{ToolAdapter, ToolContext};

#[derive(Clone)]
struct FakeRemoteSession {
    id: String,
    kind: TerminalConnectionKind,
    state: ConnectionState,
}

impl FakeRemoteSession {
    fn connected_ssh(id: &str) -> Self {
        Self {
            id: id.to_string(),
            kind: TerminalConnectionKind::Ssh,
            state: ConnectionState::Connected,
        }
    }

    fn snapshot_inner(&self) -> TerminalSessionSnapshot {
        TerminalSessionSnapshot {
            session_id: self.id.clone(),
            connection_id: Some(42),
            title: "test".to_string(),
            host_label: "test.host".to_string(),
            cwd: Some("/root".to_string()),
            rows: 24,
            cols: 120,
            connection_kind: self.kind,
            connection_state: self.state.clone(),
        }
    }
}

impl TerminalSessionHandle for FakeRemoteSession {
    fn snapshot(&self) -> TerminalSessionSnapshot {
        self.snapshot_inner()
    }
}

impl RemoteOpsSessionHandle for FakeRemoteSession {
    fn snapshot(&self) -> TerminalSessionSnapshot {
        self.snapshot_inner()
    }

    fn exec(&self, request: RemoteExecRequest) -> anyhow::Result<RemoteExecResult> {
        Ok(RemoteExecResult::foreground(
            RemoteCommandStatus::Exited,
            format!("ran {}\n", request.command),
            String::new(),
            Some(0),
            12,
            false,
        ))
    }

    fn write_file(&self, request: RemoteFileWriteRequest) -> anyhow::Result<RemoteFileWriteResult> {
        Ok(RemoteFileWriteResult {
            path: request.path,
            bytes_written: request.content.len(),
            sha256: "deadbeef".to_string(),
        })
    }

    fn diagnostics(
        &self,
        request: SessionDiagnosticsRequest,
    ) -> anyhow::Result<SessionDiagnosticsResult> {
        Ok(SessionDiagnosticsResult {
            session_id: request.session_id,
            connection_id: Some(42),
            host_label: "test.host".to_string(),
            cwd: Some("/root".to_string()),
            rows: 24,
            cols: 120,
            connection_kind: self.kind,
            state: self.state.clone(),
            last_error: None,
            recoverable: true,
            suggested_action: None,
        })
    }
}

fn registry_with_session() -> PublicMcpRegistry {
    let registry = PublicMcpRegistry::default();
    let session = FakeRemoteSession::connected_ssh("ssh-1");
    registry.register(session.clone());
    registry.register_remote_ops(session);
    registry
}

#[test]
fn remote_ops_tools_are_registered() {
    let tool_registry = PublicMcpToolRegistry::terminal(PublicMcpRegistry::default());
    let names: Vec<String> = tool_registry
        .tools()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();

    assert!(names.contains(&"ssh.exec".to_string()));
    assert!(names.contains(&"ssh.session_diagnostics".to_string()));
    assert!(names.contains(&"ssh.command.poll".to_string()));
    assert!(names.contains(&"ssh.command.output".to_string()));
    assert!(names.contains(&"ssh.command.cancel".to_string()));
    assert!(!names.contains(&"ssh.list_sessions".to_string()));
    assert!(!names.contains(&"ssh.remote_file_write".to_string()));
    assert!(!names.iter().any(|name| name.starts_with("public_mcp.")));
    assert!(!names.iter().any(|name| name == "remote_exec"));
    assert!(!names.contains(&"ssh.remote_exec".to_string()));
    assert!(!names.contains(&"ssh.remote_command_poll".to_string()));
    assert!(!names.contains(&"ssh.remote_command_output".to_string()));
    assert!(!names.contains(&"ssh.remote_command_cancel".to_string()));
}

#[test]
fn ssh_exec_schema_uses_terminal_target_input() {
    let runtime_registry = remote_ops_tool_registry(PublicMcpRegistry::default());
    let exec = runtime_registry
        .list(ToolAdapter::Mcp)
        .into_iter()
        .find(|tool| tool.id == "ssh.exec")
        .expect("ssh.exec should be listed");

    assert_eq!(json!(["target", "command"]), exec.input_schema["required"]);
    assert_eq!("string", exec.input_schema["properties"]["target"]["type"]);
    assert_eq!(
        "string",
        exec.input_schema["properties"]["session_id"]["type"]
    );
    assert!(exec.description.contains("default choice"));
    assert!(exec.description.contains("isolated SSH channel"));
    assert!(exec.description.contains("does not inherit"));
    assert!(exec.description.contains("terminal.exec"));
}

#[test]
fn session_diagnostics_permission_is_read_only() {
    // diagnostics 应在 Deny 模式下也允许
    assert_eq!(
        ApprovalDecision::Allow,
        decide_permission(
            PermissionMode::Deny,
            PublicMcpOperationKind::ReadSessionDiagnostics
        )
    );
}

#[test]
fn remote_exec_permission_respects_deny() {
    assert_eq!(
        ApprovalDecision::Deny,
        decide_permission(
            PermissionMode::Deny,
            PublicMcpOperationKind::ExecuteRemoteCommand
        )
    );
    assert_eq!(
        ApprovalDecision::Allow,
        decide_permission(
            PermissionMode::Allow,
            PublicMcpOperationKind::ExecuteRemoteCommand
        )
    );
}

#[test]
fn remote_file_write_permission_respects_mode() {
    assert_eq!(
        ApprovalDecision::Ask,
        decide_permission(PermissionMode::Ask, PublicMcpOperationKind::WriteRemoteFile)
    );
    assert_eq!(
        ApprovalDecision::Deny,
        decide_permission(
            PermissionMode::Deny,
            PublicMcpOperationKind::WriteRemoteFile
        )
    );
}

#[test]
fn remote_exec_via_registry_returns_structured_result() {
    let registry = registry_with_session();
    let request = RemoteExecRequest {
        session_id: "ssh-1".to_string(),
        command: "pwd".to_string(),
        cwd: None,
        env: BTreeMap::new(),
        timeout_ms: None,
        mode: RemoteCommandMode::Foreground,
    };
    let result = registry
        .remote_exec("ssh-1", request)
        .expect("exec should succeed");

    assert_eq!(RemoteCommandStatus::Exited, result.status);
    assert_eq!(Some(0), result.exit_code);
    assert!(result.stdout.contains("ran pwd"));
}

#[test]
fn ssh_exec_accepts_target_argument() {
    let runtime_registry = remote_ops_tool_registry(registry_with_session());
    let result = futures::executor::block_on(runtime_registry.call(
        "ssh.exec",
        json!({
            "target": "ssh-1",
            "command": "pwd"
        }),
        ToolContext::for_adapter(ToolAdapter::Mcp),
    ))
    .expect("ssh.exec should execute against target");

    assert_eq!(json!("exited"), result.structured_content["status"]);
    assert_eq!(json!(0), result.structured_content["exit_code"]);
    assert!(
        result.structured_content["stdout"]
            .as_str()
            .unwrap_or_default()
            .contains("ran pwd")
    );
}

#[test]
fn ssh_remote_exec_alias_is_rejected() {
    let runtime_registry = remote_ops_tool_registry(registry_with_session());
    let error = futures::executor::block_on(runtime_registry.call(
        "ssh.remote_exec",
        json!({
            "session_id": "ssh-1",
            "command": "pwd"
        }),
        ToolContext::for_adapter(ToolAdapter::Mcp),
    ))
    .expect_err("ssh.remote_exec alias should be unknown");

    assert!(error.to_string().contains("unknown tool"));
}

#[test]
fn session_diagnostics_via_registry_works_for_connected_session() {
    let registry = registry_with_session();
    let result = registry
        .session_diagnostics(SessionDiagnosticsRequest {
            session_id: "ssh-1".to_string(),
        })
        .expect("diagnostics should succeed");

    assert_eq!("ssh-1", result.session_id);
    assert_eq!(ConnectionState::Connected, result.state);
    assert_eq!(TerminalConnectionKind::Ssh, result.connection_kind);
    assert!(result.recoverable);
}

#[test]
fn provider_session_diagnostics_serializes_result() {
    let registry = registry_with_session();
    let runtime_registry = remote_ops_tool_registry(registry);
    let value = serde_json::to_value(
        runtime_registry
            .list(ToolAdapter::Mcp)
            .iter()
            .find(|t| t.id == "ssh.session_diagnostics")
            .map(|t| t.id.clone())
            .unwrap_or_default(),
    )
    .unwrap();
    assert_eq!(json!("ssh.session_diagnostics"), value);
}

#[test]
fn command_lifecycle_via_registry() {
    use public_mcp::remote_ops::{
        RemoteCommandCancelRequest, RemoteCommandOutputRequest, RemoteCommandPollRequest,
        RemoteCommandSignal, RemoteCommandStatus,
    };

    let registry = registry_with_session();
    let store = registry.command_store().clone();

    let (command_id, entry, _cancel_rx) = store.register("ssh-1", "sleep 300");

    // 初始状态 running
    let running = registry
        .remote_command_poll(RemoteCommandPollRequest {
            command_id: command_id.clone(),
        })
        .unwrap();
    assert_eq!(RemoteCommandStatus::Running, running.status);

    // 追加输出并读取
    entry.push_stdout(
        b"step 1
",
    );
    let output = registry
        .remote_command_output(RemoteCommandOutputRequest {
            command_id: command_id.clone(),
            stdout_offset: 0,
            stderr_offset: 0,
            limit_bytes: None,
        })
        .unwrap();
    assert_eq!("step 1\n", output.stdout);

    // 取消
    let cancel = registry
        .remote_command_cancel(RemoteCommandCancelRequest {
            command_id: command_id.clone(),
            signal: RemoteCommandSignal::Sigint,
        })
        .unwrap();
    assert_eq!(RemoteCommandStatus::CancelRequested, cancel.status);

    // 完成后 poll 返回最终状态
    entry.complete(RemoteCommandStatus::Cancelled, Some(130));
    let done = registry
        .remote_command_poll(RemoteCommandPollRequest { command_id })
        .unwrap();
    assert_eq!(RemoteCommandStatus::Cancelled, done.status);
    assert_eq!(Some(130), done.exit_code);
}
