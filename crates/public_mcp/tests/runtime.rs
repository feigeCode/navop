use public_mcp::discovery::{PublicMcpMode, read_discovery};
use public_mcp::permissions::PermissionMode;
use public_mcp::registry::PublicMcpRegistry;
use public_mcp::runtime::PublicMcpRuntime;

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
