mod input;
mod schema;

use serde_json::{Value, json};
use std::sync::Arc;
use tool_runtime::{
    ResourceKind, RiskLevel, ToolAdapter, ToolAnnotations, ToolContext, ToolDescriptor, ToolError,
    ToolHandler, ToolMode, ToolResult, ToolTargetSpec,
};

use input::{optional_u8, redis_arg, required_string};
use schema::{command_schema, get_schema, keys_schema, list_connections_schema, set_schema};

const REDIS_LIST_CONNECTIONS_TOOL: &str = "redis.list_connections";
const REDIS_COMMAND_TOOL: &str = "redis.command";
const REDIS_KEYS_TOOL: &str = "redis.keys";
const REDIS_GET_TOOL: &str = "redis.get";
const REDIS_SET_TOOL: &str = "redis.set";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RedisConnectionSnapshot {
    pub connection_id: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RedisCommandExecution {
    pub connection_id: String,
    pub db: Option<u8>,
    pub command: String,
    pub result: Value,
    pub display: String,
}

impl RedisCommandExecution {
    pub fn into_json(self) -> Value {
        json!({
            "connection_id": self.connection_id,
            "db": self.db,
            "command": self.command,
            "result": self.result,
            "display": self.display
        })
    }
}

pub trait RedisConnectionSnapshotProvider: Send + Sync + 'static {
    fn list_connections(&self) -> Vec<RedisConnectionSnapshot>;
}

pub trait RedisCommandExecutionProvider: Send + Sync + 'static {
    fn execute_command(
        &self,
        connection_id: &str,
        db: Option<u8>,
        command: &str,
    ) -> tool_runtime::ToolFuture;
}

pub trait RedisRuntimeProvider:
    RedisConnectionSnapshotProvider + RedisCommandExecutionProvider
{
}

impl<T> RedisRuntimeProvider for T where
    T: RedisConnectionSnapshotProvider + RedisCommandExecutionProvider
{
}

#[derive(Clone)]
pub struct RedisToolProvider {
    runtime: Arc<dyn RedisRuntimeProvider>,
    tool: RedisTool,
}

#[derive(Clone, Copy)]
enum RedisTool {
    ListConnections,
    Command,
    Keys,
    Get,
    Set,
}

impl RedisToolProvider {
    pub fn new(runtime: Arc<dyn RedisRuntimeProvider>) -> Self {
        Self::handler(runtime, RedisTool::ListConnections)
    }

    pub fn handlers(runtime: Arc<dyn RedisRuntimeProvider>) -> Vec<Arc<dyn ToolHandler>> {
        vec![
            Arc::new(Self::handler(runtime.clone(), RedisTool::ListConnections)),
            Arc::new(Self::handler(runtime.clone(), RedisTool::Command)),
            Arc::new(Self::handler(runtime.clone(), RedisTool::Keys)),
            Arc::new(Self::handler(runtime.clone(), RedisTool::Get)),
            Arc::new(Self::handler(runtime, RedisTool::Set)),
        ]
    }

    pub fn empty() -> Vec<Arc<dyn ToolHandler>> {
        Self::handlers(Arc::new(EmptyRedisRuntime))
    }

    fn handler(runtime: Arc<dyn RedisRuntimeProvider>, tool: RedisTool) -> Self {
        Self { runtime, tool }
    }

    fn list_connections(&self) -> ToolResult {
        let mut connections = self.runtime.list_connections();
        connections.sort_by(|left, right| left.connection_id.cmp(&right.connection_id));
        ToolResult::structured(json!({
            "connections": connections
                .into_iter()
                .map(|connection| json!({ "connection_id": connection.connection_id }))
                .collect::<Vec<_>>()
        }))
    }

    async fn execute_command(&self, input: Value) -> Result<ToolResult, ToolError> {
        let connection_id = required_string(&input, "connection_id")?;
        let command = required_string(&input, "command")?;
        let db = optional_u8(&input, "db")?;
        self.call_redis(connection_id, db, command).await
    }

    async fn execute_keys(&self, input: Value) -> Result<ToolResult, ToolError> {
        let connection_id = required_string(&input, "connection_id")?;
        let pattern = required_string(&input, "pattern")?;
        let db = optional_u8(&input, "db")?;
        self.call_redis(connection_id, db, format!("KEYS {}", redis_arg(&pattern)))
            .await
    }

    async fn execute_get(&self, input: Value) -> Result<ToolResult, ToolError> {
        let connection_id = required_string(&input, "connection_id")?;
        let key = required_string(&input, "key")?;
        let db = optional_u8(&input, "db")?;
        self.call_redis(connection_id, db, format!("GET {}", redis_arg(&key)))
            .await
    }

    async fn execute_set(&self, input: Value) -> Result<ToolResult, ToolError> {
        let connection_id = required_string(&input, "connection_id")?;
        let key = required_string(&input, "key")?;
        let value = required_string(&input, "value")?;
        let db = optional_u8(&input, "db")?;
        self.call_redis(
            connection_id,
            db,
            format!("SET {} {}", redis_arg(&key), redis_arg(&value)),
        )
        .await
    }

    async fn call_redis(
        &self,
        connection_id: String,
        db: Option<u8>,
        command: String,
    ) -> Result<ToolResult, ToolError> {
        self.runtime
            .execute_command(&connection_id, db, &command)
            .await
    }
}

impl ToolHandler for RedisToolProvider {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            id: self.tool.id().to_string(),
            title: self.tool.title().to_string(),
            description: self.tool.description().to_string(),
            input_schema: self.tool.input_schema(),
            output_schema: json!({ "type": "object" }),
            permissions: Vec::new(),
            mode: ToolMode::Deterministic,
            adapters: vec![ToolAdapter::Mcp, ToolAdapter::FunctionCalling],
            annotations: self.tool.annotations(),
        }
    }

    fn call(&self, input: Value, _context: ToolContext) -> tool_runtime::ToolFuture {
        let handler = self.clone();
        Box::pin(async move {
            match handler.tool {
                RedisTool::ListConnections => Ok(handler.list_connections()),
                RedisTool::Command => handler.execute_command(input).await,
                RedisTool::Keys => handler.execute_keys(input).await,
                RedisTool::Get => handler.execute_get(input).await,
                RedisTool::Set => handler.execute_set(input).await,
            }
        })
    }

    fn target_spec(&self) -> ToolTargetSpec {
        if matches!(self.tool, RedisTool::ListConnections) {
            return ToolTargetSpec::none();
        }
        ToolTargetSpec::required(vec![ResourceKind::Redis])
    }
}

impl RedisTool {
    fn id(self) -> &'static str {
        match self {
            RedisTool::ListConnections => REDIS_LIST_CONNECTIONS_TOOL,
            RedisTool::Command => REDIS_COMMAND_TOOL,
            RedisTool::Keys => REDIS_KEYS_TOOL,
            RedisTool::Get => REDIS_GET_TOOL,
            RedisTool::Set => REDIS_SET_TOOL,
        }
    }

    fn title(self) -> &'static str {
        match self {
            RedisTool::ListConnections => "List Redis connections",
            RedisTool::Command => "Execute Redis command",
            RedisTool::Keys => "List Redis keys",
            RedisTool::Get => "Get Redis key",
            RedisTool::Set => "Set Redis key",
        }
    }

    fn description(self) -> &'static str {
        match self {
            RedisTool::ListConnections => {
                "List currently active Redis connections exposed by the running Navop app. Use this to discover runtime Redis connection ids before calling Redis-specific tools. This does not list saved profiles; use connections.list for saved Redis connection profiles."
            }
            RedisTool::Command => {
                "Execute one Redis command against an active Redis connection in the running Navop app. Use redis.list_connections first to discover connection_id. Pass db to target a specific logical database. The command may mutate Redis data and therefore requires write approval."
            }
            RedisTool::Keys => {
                "List Redis keys matching a pattern against an active Redis connection in the running Navop app. This is read-only but may be expensive on large databases."
            }
            RedisTool::Get => {
                "Read the string value for one Redis key against an active Redis connection in the running Navop app."
            }
            RedisTool::Set => {
                "Set the string value for one Redis key against an active Redis connection in the running Navop app. This mutates Redis data and requires write approval."
            }
        }
    }

    fn input_schema(self) -> Value {
        match self {
            RedisTool::ListConnections => list_connections_schema(),
            RedisTool::Command => command_schema("connection_id"),
            RedisTool::Keys => keys_schema(),
            RedisTool::Get => get_schema(),
            RedisTool::Set => set_schema(),
        }
    }

    fn annotations(self) -> ToolAnnotations {
        match self {
            RedisTool::ListConnections => ToolAnnotations::read_only(self.title()),
            RedisTool::Command => ToolAnnotations::mutating(self.title()),
            RedisTool::Keys => {
                ToolAnnotations::read_only(self.title()).with_risk(RiskLevel::Medium)
            }
            RedisTool::Get => ToolAnnotations::read_only(self.title()).with_risk(RiskLevel::Low),
            RedisTool::Set => ToolAnnotations::mutating(self.title()),
        }
    }
}

struct EmptyRedisRuntime;

impl RedisConnectionSnapshotProvider for EmptyRedisRuntime {
    fn list_connections(&self) -> Vec<RedisConnectionSnapshot> {
        Vec::new()
    }
}

impl RedisCommandExecutionProvider for EmptyRedisRuntime {
    fn execute_command(
        &self,
        connection_id: &str,
        _db: Option<u8>,
        _command: &str,
    ) -> tool_runtime::ToolFuture {
        let connection_id = connection_id.to_string();
        Box::pin(async move {
            Err(ToolError::Failed {
                message: format!("unknown Redis connection: {connection_id}"),
            })
        })
    }
}
