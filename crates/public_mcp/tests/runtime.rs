use public_mcp::discovery::{PublicMcpMode, read_discovery};
use public_mcp::launcher::connect_to_runtime;
use public_mcp::permissions::PermissionMode;
use public_mcp::registry::{
    ConnectionState, PublicMcpRegistry, RemoteOpsSessionHandle, TerminalConnectionKind,
    TerminalSessionHandle, TerminalSessionSnapshot,
};
use public_mcp::remote_ops::{
    RemoteCommandStatus, RemoteExecRequest, RemoteExecResult, RemoteFileWriteRequest,
    RemoteFileWriteResult, SessionDiagnosticsRequest, SessionDiagnosticsResult,
};
use public_mcp::runtime::PublicMcpRuntime;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::time::{Duration, sleep};

#[tokio::test]
async fn runtime_writes_and_removes_discovery_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("public-mcp.json");

    {
        let runtime = PublicMcpRuntime::start_with_discovery_path(
            PublicMcpRegistry::default(),
            PublicMcpMode::Temporary,
            PermissionMode::Deny,
            path.clone(),
        )
        .await
        .unwrap();
        let discovery = read_discovery(&path).unwrap();

        assert_eq!(runtime.bind_addr().port(), discovery.port);
        assert_eq!("127.0.0.1", discovery.host);
        assert_eq!(64, discovery.token.len());
    }

    assert!(!path.exists());
}

#[tokio::test]
async fn runtime_exposes_active_client_count() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("public-mcp.json");
    let runtime = PublicMcpRuntime::start_with_discovery_path(
        PublicMcpRegistry::default(),
        PublicMcpMode::Temporary,
        PermissionMode::Allow,
        path.clone(),
    )
    .await
    .unwrap();
    let discovery = read_discovery(&path).unwrap();

    let stream = connect_to_runtime(&discovery).await.unwrap();
    wait_for_client_count(&runtime, 1).await;

    drop(stream);
    wait_for_client_count(&runtime, 0).await;
}

#[tokio::test]
async fn runtime_updates_permission_mode_for_existing_client_connection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("public-mcp.json");
    let executed_commands = Arc::new(Mutex::new(Vec::new()));
    let registry = PublicMcpRegistry::default();
    let session = FakeRemoteSession {
        executed_commands: executed_commands.clone(),
    };
    registry.register(session.clone());
    registry.register_remote_ops(session);
    let runtime = PublicMcpRuntime::start_with_discovery_path(
        registry,
        PublicMcpMode::Temporary,
        PermissionMode::Deny,
        path.clone(),
    )
    .await
    .unwrap();
    let discovery = read_discovery(&path).unwrap();
    let mut client = TestClient::connect(&discovery).await;

    let denied = client.remote_exec(20, "pwd").await;

    assert_eq!(
        "permission_denied",
        denied["result"]["structuredContent"]["code"]
    );
    runtime.set_permission_mode(PermissionMode::Allow);

    let allowed = client.remote_exec(21, "pwd").await;

    assert_eq!(0, allowed["result"]["structuredContent"]["exit_code"]);
    assert_eq!(vec!["pwd".to_string()], *executed_commands.lock().unwrap());
}

async fn wait_for_client_count(runtime: &PublicMcpRuntime, expected: usize) {
    for _ in 0..20 {
        if runtime.client_count() == expected {
            return;
        }
        sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(expected, runtime.client_count());
}

#[derive(Clone)]
struct FakeRemoteSession {
    executed_commands: Arc<Mutex<Vec<String>>>,
}

fn fake_snapshot() -> TerminalSessionSnapshot {
    TerminalSessionSnapshot {
        session_id: "ssh-1".to_string(),
        connection_id: Some(7),
        title: "test shell".to_string(),
        host_label: "test".to_string(),
        cwd: Some("/tmp".to_string()),
        rows: 24,
        cols: 80,
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
            "/tmp\n".to_string(),
            String::new(),
            Some(0),
            3,
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
            connection_id: Some(7),
            host_label: "test".to_string(),
            cwd: Some("/tmp".to_string()),
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

struct TestClient {
    reader: BufReader<tokio::io::ReadHalf<TcpStream>>,
    writer: tokio::io::WriteHalf<TcpStream>,
}

impl TestClient {
    async fn connect(discovery: &public_mcp::discovery::DiscoveryDocument) -> Self {
        let stream = connect_to_runtime(discovery).await.unwrap();
        let (read_half, writer) = tokio::io::split(stream);
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
        self.write_json(json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }))
        .await;
    }

    async fn remote_exec(&mut self, id: i64, command: &str) -> Value {
        self.request(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {
                "name": "ssh.remote_exec",
                "arguments": {
                    "session_id": "ssh-1",
                    "command": command
                }
            }
        }))
        .await
    }

    async fn request(&mut self, message: Value) -> Value {
        self.write_json(message).await;
        let mut line = String::new();
        self.reader.read_line(&mut line).await.unwrap();
        serde_json::from_str(&line).unwrap()
    }

    async fn write_json(&mut self, message: Value) {
        self.writer
            .write_all(serde_json::to_string(&message).unwrap().as_bytes())
            .await
            .unwrap();
        self.writer.write_all(b"\n").await.unwrap();
    }
}
