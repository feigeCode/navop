use serde_json::json;
use std::sync::Arc;
use tool_runtime::{
    ToolAdapter, ToolAnnotations, ToolContext, ToolDescriptor, ToolHandler, ToolMode, ToolResult,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RedisConnectionSnapshot {
    pub connection_id: String,
}

pub trait RedisConnectionSnapshotProvider: Send + Sync + 'static {
    fn list_connections(&self) -> Vec<RedisConnectionSnapshot>;
}

#[derive(Clone)]
pub struct RedisToolProvider {
    snapshots: Arc<dyn RedisConnectionSnapshotProvider>,
}

impl RedisToolProvider {
    pub fn new(snapshots: Arc<dyn RedisConnectionSnapshotProvider>) -> Self {
        Self { snapshots }
    }

    pub fn empty() -> Self {
        Self::new(Arc::new(EmptyRedisSnapshots))
    }

    fn list_connections(&self) -> ToolResult {
        let mut connections = self.snapshots.list_connections();
        connections.sort_by(|left, right| left.connection_id.cmp(&right.connection_id));
        ToolResult::structured(json!({
            "connections": connections
                .into_iter()
                .map(|connection| json!({ "connection_id": connection.connection_id }))
                .collect::<Vec<_>>()
        }))
    }
}

impl ToolHandler for RedisToolProvider {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            id: "redis.list_connections".to_string(),
            title: "List Redis connections".to_string(),
            description: "List currently active Redis connections exposed by the running OnetCli app. Use this to discover runtime Redis connection ids before calling Redis-specific tools. This does not list saved profiles; use connections.list for saved Redis connection profiles.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
            output_schema: json!({ "type": "object" }),
            permissions: Vec::new(),
            mode: ToolMode::Deterministic,
            adapters: vec![ToolAdapter::Mcp, ToolAdapter::FunctionCalling],
            annotations: ToolAnnotations::read_only("List Redis connections"),
        }
    }

    fn call(&self, _input: serde_json::Value, _context: ToolContext) -> tool_runtime::ToolFuture {
        let result = self.list_connections();
        Box::pin(async move { Ok(result) })
    }
}

struct EmptyRedisSnapshots;

impl RedisConnectionSnapshotProvider for EmptyRedisSnapshots {
    fn list_connections(&self) -> Vec<RedisConnectionSnapshot> {
        Vec::new()
    }
}
