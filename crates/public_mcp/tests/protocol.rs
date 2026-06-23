use public_mcp::approval::{
    PublicMcpApprovalFuture, PublicMcpApprovalManager, PublicMcpApprovalOutcome,
    PublicMcpApprovalRequest, PublicMcpApprover,
};
use public_mcp::permissions::PermissionMode;
use public_mcp::protocol::PublicMcpServer;
use public_mcp::registry::{
    ConnectionState, PublicMcpRegistry, RemoteOpsSessionHandle, TerminalConnectionKind,
    TerminalSessionHandle, TerminalSessionSnapshot,
};
use public_mcp::remote_ops::{
    RemoteCommandStatus, RemoteExecRequest, RemoteExecResult, RemoteFileWriteRequest,
    RemoteFileWriteResult, SessionDiagnosticsRequest, SessionDiagnosticsResult,
};
use public_mcp::server::serve_on_stream;
use public_mcp::tools::PublicMcpToolRegistry;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[derive(Clone)]
struct FakeRemoteSession {
    executed_commands: Arc<Mutex<Vec<String>>>,
}

fn fake_snapshot() -> TerminalSessionSnapshot {
    TerminalSessionSnapshot {
        session_id: "ssh-1".to_string(),
        connection_id: Some(42),
        title: "prod shell".to_string(),
        host_label: "prod.example".to_string(),
        cwd: Some("/srv/app".to_string()),
        rows: 30,
        cols: 120,
        connection_kind: TerminalConnectionKind::Ssh,
        connection_state: ConnectionState::Connected,
    }
}

impl TerminalSessionHandle for FakeRemoteSession {
    fn snapshot(&self) -> TerminalSessionSnapshot {
        fake_snapshot()
    }
}

impl RemoteOpsSessionHandle for FakeRemoteSession {
    fn snapshot(&self) -> TerminalSessionSnapshot {
        fake_snapshot()
    }

    fn exec(&self, request: RemoteExecRequest) -> anyhow::Result<RemoteExecResult> {
        self.executed_commands
            .lock()
            .unwrap()
            .push(request.command.clone());
        Ok(RemoteExecResult::foreground(
            RemoteCommandStatus::Exited,
            "/srv/app\n".to_string(),
            String::new(),
            Some(0),
            5,
            false,
        ))
    }

    fn write_file(&self, request: RemoteFileWriteRequest) -> anyhow::Result<RemoteFileWriteResult> {
        Ok(RemoteFileWriteResult {
            path: request.path,
            bytes_written: request.content.len(),
            sha256: "abc".to_string(),
        })
    }

    fn diagnostics(
        &self,
        request: SessionDiagnosticsRequest,
    ) -> anyhow::Result<SessionDiagnosticsResult> {
        Ok(SessionDiagnosticsResult {
            session_id: request.session_id,
            connection_id: Some(42),
            host_label: "prod.example".to_string(),
            cwd: Some("/srv/app".to_string()),
            rows: 30,
            cols: 120,
            connection_kind: TerminalConnectionKind::Ssh,
            state: ConnectionState::Connected,
            last_error: None,
            recoverable: true,
            suggested_action: None,
        })
    }
}

#[derive(Clone)]
struct FixedApprover {
    outcome: PublicMcpApprovalOutcome,
    requests: Arc<Mutex<Vec<PublicMcpApprovalRequest>>>,
}

impl FixedApprover {
    fn new(outcome: PublicMcpApprovalOutcome) -> Self {
        Self {
            outcome,
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn requests(&self) -> Vec<PublicMcpApprovalRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl PublicMcpApprover for FixedApprover {
    fn request_approval(&self, request: PublicMcpApprovalRequest) -> PublicMcpApprovalFuture {
        self.requests.lock().unwrap().push(request);
        let outcome = self.outcome.clone();
        Box::pin(async move { outcome })
    }
}

#[tokio::test]
async fn tools_call_list_sessions_returns_connected_sessions() {
    let registry = PublicMcpRegistry::default();
    registry.register(FakeRemoteSession {
        executed_commands: Arc::new(Mutex::new(Vec::new())),
    });
    let mut client = TestClient::connect(registry, PermissionMode::Allow).await;

    let response = client
        .request(json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "public_mcp.list_sessions",
                "arguments": {}
            }
        }))
        .await;

    assert_eq!(json!(2), response["id"]);
    assert_eq!(
        "ssh-1",
        response["result"]["structuredContent"]["sessions"][0]["session_id"]
    );
}

#[tokio::test]
async fn remote_exec_rejects_when_permission_mode_denies() {
    let executed_commands = Arc::new(Mutex::new(Vec::new()));
    let registry = PublicMcpRegistry::default();
    let session = FakeRemoteSession {
        executed_commands: executed_commands.clone(),
    };
    registry.register(session.clone());
    registry.register_remote_ops(session);
    let mut client = TestClient::connect(registry, PermissionMode::Deny).await;

    let response = client
        .request(json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "public_mcp.remote_exec",
                "arguments": {
                    "session_id": "ssh-1",
                    "command": "pwd"
                }
            }
        }))
        .await;

    assert_eq!(
        "permission_denied",
        response["result"]["structuredContent"]["code"]
    );
    assert_eq!(json!(true), response["result"]["isError"]);
    assert!(executed_commands.lock().unwrap().is_empty());
}

#[tokio::test]
async fn remote_exec_uses_updated_permission_mode() {
    let executed_commands = Arc::new(Mutex::new(Vec::new()));
    let registry = PublicMcpRegistry::default();
    let session = FakeRemoteSession {
        executed_commands: executed_commands.clone(),
    };
    registry.register(session.clone());
    registry.register_remote_ops(session);
    let permission = public_mcp::protocol::SharedPermissionMode::new(PermissionMode::Deny);
    let protocol = PublicMcpServer::with_shared_permission(
        PublicMcpToolRegistry::terminal(registry),
        permission.clone(),
    );
    let mut client = TestClient::connect_protocol(protocol).await;

    let denied = client
        .request(json!({
            "jsonrpc": "2.0",
            "id": 30,
            "method": "tools/call",
            "params": {
                "name": "public_mcp.remote_exec",
                "arguments": {
                    "session_id": "ssh-1",
                    "command": "pwd"
                }
            }
        }))
        .await;

    assert_eq!(
        "permission_denied",
        denied["result"]["structuredContent"]["code"]
    );
    permission.set(PermissionMode::Allow);

    let allowed = client
        .request(json!({
            "jsonrpc": "2.0",
            "id": 31,
            "method": "tools/call",
            "params": {
                "name": "public_mcp.remote_exec",
                "arguments": {
                    "session_id": "ssh-1",
                    "command": "pwd"
                }
            }
        }))
        .await;

    assert_eq!(0, allowed["result"]["structuredContent"]["exit_code"]);
    assert_eq!(vec!["pwd".to_string()], *executed_commands.lock().unwrap());
}

#[tokio::test]
async fn remote_exec_asks_and_runs_when_approved() {
    let executed_commands = Arc::new(Mutex::new(Vec::new()));
    let registry = PublicMcpRegistry::default();
    let session = FakeRemoteSession {
        executed_commands: executed_commands.clone(),
    };
    registry.register(session.clone());
    registry.register_remote_ops(session);
    let approver = Arc::new(FixedApprover::new(PublicMcpApprovalOutcome::Approved));
    let mut client = TestClient::connect_with_approval(
        registry,
        PermissionMode::Ask,
        PublicMcpApprovalManager::new(approver.clone()),
    )
    .await;

    let response = client
        .request(json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "public_mcp.remote_exec",
                "arguments": {
                    "session_id": "ssh-1",
                    "command": "pwd"
                }
            }
        }))
        .await;

    assert_eq!(0, response["result"]["structuredContent"]["exit_code"]);
    assert_eq!(vec!["pwd".to_string()], *executed_commands.lock().unwrap());
    let requests = approver.requests();
    assert_eq!(1, requests.len());
    assert_eq!("public_mcp.remote_exec", requests[0].tool_name);
}

#[tokio::test]
async fn remote_exec_asks_and_does_not_run_when_denied() {
    let executed_commands = Arc::new(Mutex::new(Vec::new()));
    let registry = PublicMcpRegistry::default();
    let session = FakeRemoteSession {
        executed_commands: executed_commands.clone(),
    };
    registry.register(session.clone());
    registry.register_remote_ops(session);
    let approver = Arc::new(FixedApprover::new(PublicMcpApprovalOutcome::Denied {
        reason: Some("operator denied".to_string()),
    }));
    let mut client = TestClient::connect_with_approval(
        registry,
        PermissionMode::Ask,
        PublicMcpApprovalManager::new(approver.clone()),
    )
    .await;

    let response = client
        .request(json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "public_mcp.remote_exec",
                "arguments": {
                    "session_id": "ssh-1",
                    "command": "pwd"
                }
            }
        }))
        .await;

    assert_eq!(
        "permission_denied",
        response["result"]["structuredContent"]["code"]
    );
    assert_eq!(
        "operator denied",
        response["result"]["structuredContent"]["message"]
    );
    assert_eq!(json!(true), response["result"]["isError"]);
    assert!(executed_commands.lock().unwrap().is_empty());
    assert_eq!(1, approver.requests().len());
}

#[tokio::test]
async fn initialized_notification_does_not_produce_response() {
    let mut client = TestClient::connect(PublicMcpRegistry::default(), PermissionMode::Allow).await;

    client
        .notify(json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }))
        .await;

    assert!(client.next_response_timeout().await.is_none());
}

struct TestClient {
    reader: BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>,
    writer: tokio::io::WriteHalf<tokio::io::DuplexStream>,
}

impl TestClient {
    async fn connect(registry: PublicMcpRegistry, mode: PermissionMode) -> Self {
        let protocol = PublicMcpServer::new(registry, mode);
        Self::connect_protocol(protocol).await
    }

    async fn connect_with_approval(
        registry: PublicMcpRegistry,
        mode: PermissionMode,
        approval_manager: PublicMcpApprovalManager,
    ) -> Self {
        let protocol = PublicMcpServer::with_tool_registry_and_approval(
            PublicMcpToolRegistry::terminal(registry),
            mode,
            approval_manager,
        );
        Self::connect_protocol(protocol).await
    }

    async fn connect_protocol(protocol: PublicMcpServer) -> Self {
        // 用 in-memory duplex 驱动 token 校验 + rmcp 服务,避免依赖真实 TCP 绑定。
        let (client_stream, server_stream) = tokio::io::duplex(8 * 1024);
        tokio::spawn(async move {
            if let Err(error) = serve_on_stream(server_stream, protocol, "correct-token").await {
                tracing::debug!(?error, "test public MCP stream closed");
            }
        });

        let (read_half, mut writer) = tokio::io::split(client_stream);
        writer.write_all(b"correct-token\n").await.unwrap();
        let mut client = Self {
            reader: BufReader::new(read_half),
            writer,
        };
        client.initialize().await;
        client
    }

    async fn initialize(&mut self) {
        let _ = self
            .request(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-11-25",
                    "capabilities": {},
                    "clientInfo": { "name": "test-client", "version": "0.0.1" }
                }
            }))
            .await;
        self.notify(json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }))
        .await;
    }

    async fn request(&mut self, message: Value) -> Value {
        self.write_json(message).await;
        self.next_response().await
    }

    async fn notify(&mut self, message: Value) {
        self.write_json(message).await;
    }

    async fn write_json(&mut self, message: Value) {
        self.writer
            .write_all(serde_json::to_string(&message).unwrap().as_bytes())
            .await
            .unwrap();
        self.writer.write_all(b"\n").await.unwrap();
    }

    async fn next_response(&mut self) -> Value {
        let mut line = String::new();
        self.reader.read_line(&mut line).await.unwrap();
        serde_json::from_str(&line).unwrap()
    }

    async fn next_response_timeout(&mut self) -> Option<Value> {
        match tokio::time::timeout(std::time::Duration::from_millis(300), self.next_response())
            .await
        {
            Ok(value) => Some(value),
            Err(_) => None,
        }
    }
}
