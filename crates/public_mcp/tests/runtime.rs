use public_mcp::discovery::{PublicMcpMode, read_discovery};
use public_mcp::launcher::connect_to_runtime;
use public_mcp::permissions::PermissionMode;
use public_mcp::registry::PublicMcpRegistry;
use public_mcp::runtime::PublicMcpRuntime;
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

async fn wait_for_client_count(runtime: &PublicMcpRuntime, expected: usize) {
    for _ in 0..20 {
        if runtime.client_count() == expected {
            return;
        }
        sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(expected, runtime.client_count());
}
