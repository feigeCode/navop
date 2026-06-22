use super::{PublicMcpToolContext, PublicMcpToolFuture, PublicMcpToolProvider};
use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, JsonObject, Tool, ToolAnnotations},
};
use serde_json::{Value, json};
use std::sync::Arc;

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

    fn list_connections(&self) -> Result<CallToolResult, McpError> {
        let mut connections = self.snapshots.list_connections();
        connections.sort_by(|left, right| left.connection_id.cmp(&right.connection_id));
        Ok(CallToolResult::structured(json!({
            "connections": connections
                .into_iter()
                .map(|connection| json!({ "connection_id": connection.connection_id }))
                .collect::<Vec<_>>()
        })))
    }
}

impl PublicMcpToolProvider for RedisToolProvider {
    fn tools(&self) -> Vec<Tool> {
        redis_tools()
    }

    fn call_tool(
        &self,
        name: &str,
        _arguments: Option<JsonObject>,
        _context: PublicMcpToolContext,
    ) -> Option<PublicMcpToolFuture> {
        match name {
            "public_mcp.redis.list_connections" => {
                let result = self.list_connections();
                Some(Box::pin(async move { result }))
            }
            _ => None,
        }
    }
}

fn redis_tools() -> Vec<Tool> {
    vec![
        Tool::new(
            "public_mcp.redis.list_connections",
            "List active Redis connections exposed by OnetCli.",
            object_schema([]),
        )
        .with_annotations(read_only_annotations("List Redis connections")),
    ]
}

fn read_only_annotations(title: &str) -> ToolAnnotations {
    ToolAnnotations::with_title(title)
        .read_only(true)
        .destructive(false)
        .idempotent(true)
        .open_world(false)
}

fn object_schema(properties: impl IntoIterator<Item = (&'static str, Value)>) -> Arc<JsonObject> {
    let required = properties
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect::<JsonObject>();
    let mut schema = JsonObject::new();
    schema.insert("type".to_string(), Value::String("object".to_string()));
    schema.insert("properties".to_string(), Value::Object(required));
    Arc::new(schema)
}

struct EmptyRedisSnapshots;

impl RedisConnectionSnapshotProvider for EmptyRedisSnapshots {
    fn list_connections(&self) -> Vec<RedisConnectionSnapshot> {
        Vec::new()
    }
}
