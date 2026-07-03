use serde_json::{Value, json};
use std::sync::Arc;
use tool_runtime::{
    ToolAdapter, ToolAlias, ToolAnnotations, ToolContext, ToolDescriptor, ToolError, ToolHandler,
    ToolMode, ToolResult,
};

const REDIS_LIST_CONNECTIONS_TOOL: &str = "redis.list_connections";
const REDIS_COMMAND_TOOL: &str = "redis.command";
const REDIS_EXECUTE_COMMAND_ALIAS: &str = "redis.execute_command";

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
    ExecuteCommand,
}

impl RedisToolProvider {
    pub fn new(runtime: Arc<dyn RedisRuntimeProvider>) -> Self {
        Self::handler(runtime, RedisTool::ListConnections)
    }

    pub fn handlers(runtime: Arc<dyn RedisRuntimeProvider>) -> Vec<Arc<dyn ToolHandler>> {
        vec![
            Arc::new(Self::handler(runtime.clone(), RedisTool::ListConnections)),
            Arc::new(Self::handler(runtime, RedisTool::ExecuteCommand)),
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
        self.runtime
            .execute_command(&connection_id, db, &command)
            .await
    }
}

impl ToolHandler for RedisToolProvider {
    fn descriptor(&self) -> ToolDescriptor {
        let (id, title, description, input_schema, annotations) = match self.tool {
            RedisTool::ListConnections => (
                REDIS_LIST_CONNECTIONS_TOOL,
                "List Redis connections",
                "List currently active Redis connections exposed by the running OnetCli app. Use this to discover runtime Redis connection ids before calling Redis-specific tools. This does not list saved profiles; use connections.list for saved Redis connection profiles.",
                json!({
                    "type": "object",
                    "properties": {}
                }),
                ToolAnnotations::read_only("List Redis connections"),
            ),
            RedisTool::ExecuteCommand => (
                REDIS_COMMAND_TOOL,
                "Execute Redis command",
                "Execute one Redis command against an active Redis connection in the running OnetCli app. Use redis.list_connections first to discover connection_id. Pass db to target a specific logical database. The command may mutate Redis data and therefore requires write approval.",
                execute_command_schema("connection_id"),
                ToolAnnotations::mutating("Execute Redis command"),
            ),
        };
        ToolDescriptor {
            id: id.to_string(),
            title: title.to_string(),
            description: description.to_string(),
            input_schema,
            output_schema: json!({ "type": "object" }),
            permissions: Vec::new(),
            mode: ToolMode::Deterministic,
            adapters: vec![ToolAdapter::Mcp, ToolAdapter::FunctionCalling],
            annotations,
        }
    }

    fn aliases(&self) -> Vec<ToolAlias> {
        match self.tool {
            RedisTool::ListConnections => Vec::new(),
            RedisTool::ExecuteCommand => vec![ToolAlias::new(REDIS_EXECUTE_COMMAND_ALIAS)],
        }
    }

    fn call(&self, input: Value, _context: ToolContext) -> tool_runtime::ToolFuture {
        let handler = self.clone();
        Box::pin(async move {
            match handler.tool {
                RedisTool::ListConnections => Ok(handler.list_connections()),
                RedisTool::ExecuteCommand => handler.execute_command(input).await,
            }
        })
    }
}

fn execute_command_schema(connection_field: &'static str) -> Value {
    json!({
        "type": "object",
        "properties": {
            connection_field: {
                "type": "string",
                "description": "Redis connection identifier."
            },
            "command": {
                "type": "string",
                "description": "Single Redis command, for example `PING` or `GET user:1`."
            },
            "db": {
                "type": "integer",
                "minimum": 0,
                "maximum": 255,
                "description": "Optional Redis logical database index."
            }
        },
        "required": [connection_field, "command"]
    })
}

fn required_string(input: &Value, field: &'static str) -> Result<String, ToolError> {
    input
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| ToolError::Failed {
            message: format!("missing required string field `{field}`"),
        })
}

fn optional_u8(input: &Value, field: &'static str) -> Result<Option<u8>, ToolError> {
    match input.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|value| u8::try_from(value).ok())
            .map(Some)
            .ok_or_else(|| ToolError::Failed {
                message: format!("field `{field}` must be an integer from 0 to 255"),
            }),
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
