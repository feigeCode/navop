use public_mcp::permissions::PermissionMode;
use public_mcp::tools::{
    PublicMcpToolContext, PublicMcpToolRegistry, RedisConnectionSnapshot,
    RedisConnectionSnapshotProvider, RedisToolProvider, ToolRuntimeMcpProvider,
};
use serde_json::json;
use std::sync::Arc;
use tool_runtime::ToolRegistry;

#[tokio::test]
async fn redis_provider_lists_runtime_connections() {
    let runtime_registry = ToolRegistry::new(vec![Arc::new(RedisToolProvider::new(Arc::new(
        FakeRedisSnapshots {
            connections: vec![
                RedisConnectionSnapshot {
                    connection_id: "redis-b".to_string(),
                },
                RedisConnectionSnapshot {
                    connection_id: "redis-a".to_string(),
                },
            ],
        },
    )))]);
    let registry = PublicMcpToolRegistry::new(vec![Arc::new(ToolRuntimeMcpProvider::new(
        runtime_registry,
    ))]);

    let result = registry
        .call_tool(
            "public_mcp.redis.list_connections",
            None,
            PublicMcpToolContext {
                permission_mode: PermissionMode::Deny,
                approver: Default::default(),
            },
        )
        .await
        .expect("redis list connections tool should run");

    assert_eq!(
        Some(json!({
            "connections": [
                { "connection_id": "redis-a" },
                { "connection_id": "redis-b" }
            ]
        })),
        result.structured_content
    );
}

#[derive(Clone)]
struct FakeRedisSnapshots {
    connections: Vec<RedisConnectionSnapshot>,
}

impl RedisConnectionSnapshotProvider for FakeRedisSnapshots {
    fn list_connections(&self) -> Vec<RedisConnectionSnapshot> {
        self.connections.clone()
    }
}
