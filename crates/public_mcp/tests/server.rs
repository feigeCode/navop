use public_mcp::permissions::PermissionMode;
use public_mcp::protocol::PublicMcpServer;
use public_mcp::registry::PublicMcpRegistry;
use public_mcp::server::LoopbackMcpServer;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::time::{Duration, sleep};

#[tokio::test]
async fn loopback_server_rejects_wrong_token_before_json_rpc() {
    let protocol = PublicMcpServer::new(PublicMcpRegistry::default(), PermissionMode::Allow);
    let server = LoopbackMcpServer::bind(protocol, "correct-token".to_string())
        .await
        .unwrap();
    let mut stream = TcpStream::connect(server.bind_addr()).await.unwrap();

    stream.write_all(b"wrong-token\n").await.unwrap();

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let read = reader.read_line(&mut line).await.unwrap_or(0);
    assert_eq!(0, read);
}

#[tokio::test]
async fn loopback_server_handles_json_rpc_after_token_handshake() {
    let protocol = PublicMcpServer::new(PublicMcpRegistry::default(), PermissionMode::Allow);
    let server = LoopbackMcpServer::bind(protocol, "correct-token".to_string())
        .await
        .unwrap();
    let mut stream = TcpStream::connect(server.bind_addr()).await.unwrap();

    stream.write_all(b"correct-token\n").await.unwrap();
    stream
        .write_all(
            br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test-client","version":"0.0.1"}}}"#,
        )
        .await
        .unwrap();
    stream.write_all(b"\n").await.unwrap();

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    let response: serde_json::Value = serde_json::from_str(&line).unwrap();

    assert_eq!(json!(1), response["id"]);
    assert_eq!(
        "onetcli-public-mcp",
        response["result"]["serverInfo"]["name"]
    );
    assert_eq!("2025-11-25", response["result"]["protocolVersion"]);
}

#[tokio::test]
async fn loopback_server_tracks_active_client_connections() {
    let protocol = PublicMcpServer::new(PublicMcpRegistry::default(), PermissionMode::Allow);
    let server = LoopbackMcpServer::bind(protocol, "correct-token".to_string())
        .await
        .unwrap();

    let stream = TcpStream::connect(server.bind_addr()).await.unwrap();
    wait_for_client_count(&server, 1).await;

    drop(stream);
    wait_for_client_count(&server, 0).await;
}

async fn wait_for_client_count(server: &LoopbackMcpServer, expected: usize) {
    for _ in 0..20 {
        if server.client_count() == expected {
            return;
        }
        sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(expected, server.client_count());
}
