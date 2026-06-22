use super::channel::channel_approver;
use public_mcp::approval::PublicMcpApprovalManager;
use public_mcp::permissions::PermissionMode;
use public_mcp::protocol::PublicMcpServer;
use public_mcp::registry::{
    ConnectionState, PublicMcpRegistry, TerminalConnectionKind, TerminalSessionHandle,
    TerminalSessionSnapshot,
};
use public_mcp::server::serve_on_stream;
use public_mcp::tools::PublicMcpToolRegistry;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

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
        Ok(String::new())
    }

    fn write_external_input(&self, data: &[u8]) -> anyhow::Result<()> {
        self.writes.lock().unwrap().push(data.to_vec());
        Ok(())
    }
}

#[tokio::test]
async fn channel_approval_bridges_mcp_terminal_write_until_resolved() {
    let writes = Arc::new(Mutex::new(Vec::new()));
    let registry = PublicMcpRegistry::default();
    registry.register(FakeTerminal {
        writes: writes.clone(),
    });
    let (approver, mut receiver) = channel_approver(Duration::from_secs(10));
    let protocol = PublicMcpServer::with_tool_registry_and_approval(
        PublicMcpToolRegistry::terminal(registry),
        PermissionMode::Ask,
        PublicMcpApprovalManager::new(Arc::new(approver)),
    );
    let mut client = TestClient::connect(protocol).await;

    let request = tokio::spawn(async move {
        client
            .request(json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "public_mcp.terminal_write",
                    "arguments": {
                        "session_id": "ssh-1",
                        "input": "date\n"
                    }
                }
            }))
            .await
    });

    let envelope = receiver.recv().await.expect("approval should be queued");
    assert_eq!("public_mcp.terminal_write", envelope.request.tool_name);
    assert_eq!("Write to terminal session ssh-1", envelope.request.summary);
    envelope.approve();

    let response = request.await.expect("MCP call should complete");
    assert_eq!(json!(true), response["result"]["structuredContent"]["ok"]);
    assert_eq!(vec![b"date\n".to_vec()], *writes.lock().unwrap());
}

struct TestClient {
    reader: BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>,
    writer: tokio::io::WriteHalf<tokio::io::DuplexStream>,
}

impl TestClient {
    async fn connect(protocol: PublicMcpServer) -> Self {
        let (client_stream, server_stream) = tokio::io::duplex(8 * 1024);
        tokio::spawn(async move {
            let _ = serve_on_stream(server_stream, protocol, "correct-token").await;
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
                    "clientInfo": { "name": "approval-test", "version": "0.0.1" }
                }
            }))
            .await;
        self.write_json(json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }))
        .await;
    }

    async fn request(&mut self, message: Value) -> Value {
        self.write_json(message).await;
        self.next_response().await
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
}
