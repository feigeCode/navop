use public_mcp::discovery::{DiscoveryDocument, PublicMcpMode, read_discovery, write_discovery};
use public_mcp::launcher::{connect_to_runtime, load_discovery, parse_discovery_path_arg};
use public_mcp::permissions::PermissionMode;
use public_mcp::registry::PublicMcpRegistry;
use public_mcp::runtime::PublicMcpRuntime;
use serde_json::json;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio::time::{Duration, timeout};

#[test]
fn missing_discovery_reports_that_mcp_must_be_enabled() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("public-mcp.json");

    let error = load_discovery(&path).unwrap_err().to_string();

    assert!(error.contains("enable MCP"));
    assert!(error.contains(path.to_string_lossy().as_ref()));
}

#[test]
fn helper_args_parse_explicit_discovery_path() {
    let path = parse_discovery_path_arg(["--discovery", "/tmp/onetcli/public-mcp.json"])
        .unwrap()
        .unwrap();

    assert_eq!(
        std::path::PathBuf::from("/tmp/onetcli/public-mcp.json"),
        path
    );
}

#[test]
fn helper_args_reject_missing_discovery_path() {
    let error = parse_discovery_path_arg(["--discovery"])
        .unwrap_err()
        .to_string();

    assert!(error.contains("requires a path"));
}

#[test]
fn helper_args_reject_unknown_arguments() {
    let error = parse_discovery_path_arg(["--socket", "/tmp/mcp.sock"])
        .unwrap_err()
        .to_string();

    assert!(error.contains("unknown argument"));
}

#[tokio::test]
async fn launcher_rejects_discovery_that_does_not_point_to_loopback() {
    let discovery = discovery_document(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10)), 49152),
        valid_token(),
    );

    let error = connect_to_runtime(&discovery)
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("loopback"));
}

#[tokio::test]
async fn launcher_rejects_discovery_for_another_app() {
    let mut discovery = discovery_document(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 49152),
        valid_token(),
    );
    discovery.app = "other-app".to_string();

    let error = connect_to_runtime(&discovery)
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("unexpected app"));
}

#[tokio::test]
async fn launcher_rejects_discovery_with_invalid_token_format() {
    let discovery = discovery_document(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 49152),
        "not-a-valid-token".to_string(),
    );

    let error = connect_to_runtime(&discovery)
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("invalid token"));
}

#[tokio::test]
async fn launcher_reports_stale_discovery_when_runtime_port_is_closed() {
    let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let discovery = discovery_document(addr, valid_token());

    let error = connect_to_runtime(&discovery)
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("failed to connect"));
    assert!(error.contains("discovery may be stale"));
    assert!(error.contains("Start OnetCli and enable MCP"));
}

#[tokio::test]
async fn launcher_connects_from_discovery_and_completes_initialize() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("public-mcp.json");
    let _runtime = PublicMcpRuntime::start_with_discovery_path(
        PublicMcpRegistry::default(),
        PublicMcpMode::Temporary,
        PermissionMode::Allow,
        path.clone(),
    )
    .await
    .unwrap();
    let discovery = read_discovery(&path).unwrap();
    let stream = connect_to_runtime(&discovery).await.unwrap();
    let mut reader = BufReader::new(stream);

    reader
        .get_mut()
        .write_all(
            br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test-client","version":"0.0.1"}}}"#,
        )
        .await
        .unwrap();
    reader.get_mut().write_all(b"\n").await.unwrap();

    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    let response: serde_json::Value = serde_json::from_str(&line).unwrap();

    assert_eq!(json!(1), response["id"]);
    assert_eq!(
        "onetcli-public-mcp",
        response["result"]["serverInfo"]["name"]
    );
}

#[tokio::test]
async fn stdio_helper_binary_bridges_initialize_over_discovery() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("public-mcp.json");
    let _runtime = PublicMcpRuntime::start_with_discovery_path(
        PublicMcpRegistry::default(),
        PublicMcpMode::Temporary,
        PermissionMode::Allow,
        path.clone(),
    )
    .await
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_onetcli-public-mcp"))
        .arg("--discovery")
        .arg(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);

    stdin
        .write_all(
            br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"stdio-helper-test","version":"0.0.1"}}}"#,
        )
        .await
        .unwrap();
    stdin.write_all(b"\n").await.unwrap();

    let mut line = String::new();
    timeout(Duration::from_secs(5), reader.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    let response: serde_json::Value = serde_json::from_str(&line).unwrap();

    assert_eq!(json!(1), response["id"]);
    assert_eq!(
        "onetcli-public-mcp",
        response["result"]["serverInfo"]["name"]
    );

    let _ = child.kill().await;
    let _ = child.wait().await;
}

#[tokio::test]
async fn stdio_helper_binary_reports_stale_discovery_on_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("public-mcp.json");
    let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    write_discovery(&path, &discovery_document(addr, valid_token())).unwrap();

    let output = timeout(
        Duration::from_secs(5),
        Command::new(env!("CARGO_BIN_EXE_onetcli-public-mcp"))
            .arg("--discovery")
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(stderr.contains("onetcli-public-mcp:"));
    assert!(stderr.contains("discovery may be stale"));
    assert!(stderr.contains("Start OnetCli and enable MCP"));
}

fn discovery_document(bind_addr: SocketAddr, token: String) -> DiscoveryDocument {
    DiscoveryDocument::new(
        std::process::id(),
        bind_addr,
        token,
        PublicMcpMode::Temporary,
    )
}

fn valid_token() -> String {
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string()
}
