use public_mcp::permissions::PermissionMode;
use public_mcp::protocol::PublicMcpServer;
use public_mcp::registry::{
    ConnectionState, PublicMcpRegistry, TerminalConnectionKind, TerminalSessionHandle,
    TerminalSessionSnapshot,
};
use public_mcp::server::LoopbackMcpServer;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::time::{Duration, timeout};

#[derive(Clone)]
struct FakeTerminal {
    writes: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl TerminalSessionHandle for FakeTerminal {
    fn snapshot(&self) -> TerminalSessionSnapshot {
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

    fn visible_text(&self) -> anyhow::Result<String> {
        Ok("hello from terminal".to_string())
    }

    fn write_external_input(&self, data: &[u8]) -> anyhow::Result<()> {
        self.writes.lock().unwrap().push(data.to_vec());
        Ok(())
    }
}

#[tokio::test]
async fn tools_call_list_sessions_returns_connected_sessions() {
    let registry = PublicMcpRegistry::default();
    registry.register(FakeTerminal {
        writes: Arc::new(Mutex::new(Vec::new())),
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
async fn terminal_write_rejects_when_permission_mode_denies_writes() {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let registry = PublicMcpRegistry::default();
    registry.register(FakeTerminal {
        writes: writes.clone(),
    });
    let mut client = TestClient::connect(registry, PermissionMode::Deny).await;

    let response = client
        .request(json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "public_mcp.terminal_write",
                "arguments": {
                    "session_id": "ssh-1",
                    "input": "date\n"
                }
            }
        }))
        .await;

    assert_eq!(
        "permission_denied",
        response["result"]["structuredContent"]["code"]
    );
    assert_eq!(json!(true), response["result"]["isError"]);
    assert!(writes.lock().unwrap().is_empty());
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
    _server: LoopbackMcpServer,
    reader: BufReader<tokio::net::tcp::OwnedReadHalf>,
    writer: tokio::net::tcp::OwnedWriteHalf,
}

impl TestClient {
    async fn connect(registry: PublicMcpRegistry, mode: PermissionMode) -> Self {
        let protocol = PublicMcpServer::new(registry, mode);
        let server = LoopbackMcpServer::bind(protocol, "correct-token".to_string())
            .await
            .unwrap();
        let stream = TcpStream::connect(server.bind_addr()).await.unwrap();
        let (read_half, mut writer) = stream.into_split();
        writer.write_all(b"correct-token\n").await.unwrap();
        let mut client = Self {
            _server: server,
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
        timeout(Duration::from_millis(100), self.next_response())
            .await
            .ok()
    }
}
