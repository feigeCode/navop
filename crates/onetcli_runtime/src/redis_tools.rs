mod command_io;
mod input;
mod schema;

use one_core::storage::traits::Repository;
use one_core::storage::{ConnectionRepository, ConnectionType, RedisMode, RedisParams};
use serde_json::{Value, json};
use std::sync::Arc;
use tool_runtime::{
    RiskLevel, ToolAdapter, ToolAnnotations, ToolContext, ToolDescriptor, ToolError, ToolHandler,
    ToolMode, ToolRegistry, ToolResult,
};

use command_io::run_command;
use input::{optional_u8, parse_command_args, required_str};
use schema::{command_schema, get_schema, keys_schema, set_schema};

const REDIS_COMMAND_TOOL: &str = "redis.command";
const REDIS_KEYS_TOOL: &str = "redis.keys";
const REDIS_GET_TOOL: &str = "redis.get";
const REDIS_SET_TOOL: &str = "redis.set";

#[derive(Clone, Copy)]
enum RedisTool {
    Command,
    Keys,
    Get,
    Set,
}

#[derive(Clone)]
struct RedisToolHandler {
    repo: Arc<ConnectionRepository>,
    tool: RedisTool,
}

pub fn redis_tool_registry(repo: Arc<ConnectionRepository>) -> ToolRegistry {
    let handlers = [
        RedisTool::Command,
        RedisTool::Keys,
        RedisTool::Get,
        RedisTool::Set,
    ]
    .into_iter()
    .map(|tool| Arc::new(RedisToolHandler::new(repo.clone(), tool)) as Arc<dyn ToolHandler>)
    .collect();
    ToolRegistry::new(handlers)
}

impl RedisToolHandler {
    fn new(repo: Arc<ConnectionRepository>, tool: RedisTool) -> Self {
        Self { repo, tool }
    }

    async fn execute(&self, input: Value) -> Result<ToolResult, ToolError> {
        match self.tool {
            RedisTool::Command => self.execute_command(input).await,
            RedisTool::Keys => self.execute_keys(input).await,
            RedisTool::Get => self.execute_get(input).await,
            RedisTool::Set => self.execute_set(input).await,
        }
    }

    async fn execute_command(&self, input: Value) -> Result<ToolResult, ToolError> {
        let connection = required_str(&input, "connection")?;
        let command = required_str(&input, "command")?;
        let db = optional_u8(&input, "db")?;
        self.execute_parts(
            connection,
            db,
            command.clone(),
            parse_command_args(&command),
        )
        .await
    }

    async fn execute_keys(&self, input: Value) -> Result<ToolResult, ToolError> {
        let connection = required_str(&input, "connection")?;
        let pattern = required_str(&input, "pattern")?;
        let db = optional_u8(&input, "db")?;
        self.execute_parts(
            connection,
            db,
            format!("KEYS {pattern}"),
            vec!["KEYS".into(), pattern],
        )
        .await
    }

    async fn execute_get(&self, input: Value) -> Result<ToolResult, ToolError> {
        let connection = required_str(&input, "connection")?;
        let key = required_str(&input, "key")?;
        let db = optional_u8(&input, "db")?;
        self.execute_parts(
            connection,
            db,
            format!("GET {key}"),
            vec!["GET".into(), key],
        )
        .await
    }

    async fn execute_set(&self, input: Value) -> Result<ToolResult, ToolError> {
        let connection = required_str(&input, "connection")?;
        let key = required_str(&input, "key")?;
        let value = required_str(&input, "value")?;
        let db = optional_u8(&input, "db")?;
        self.execute_parts(
            connection,
            db,
            format!("SET {key} <value>"),
            vec!["SET".into(), key, value],
        )
        .await
    }

    async fn execute_parts(
        &self,
        connection: String,
        db_override: Option<u8>,
        command: String,
        parts: Vec<String>,
    ) -> Result<ToolResult, ToolError> {
        let mut params = self.redis_params(&connection)?;
        let db = db_override.unwrap_or(params.db_index);
        params.db_index = db;
        validate_command(&parts, db, params.mode.clone())?;
        let result = run_command(params, &parts).await?;

        Ok(ToolResult::structured(json!({
            "connection": connection,
            "db": db,
            "command": command,
            "result": result.value,
            "display": result.display
        })))
    }

    fn redis_params(&self, connection: &str) -> Result<RedisParams, ToolError> {
        let stored = find_connection(&self.repo, connection)?;
        if stored.connection_type != ConnectionType::Redis {
            return Err(ToolError::Failed {
                message: format!("connection is not redis: {connection}"),
            });
        }
        stored.to_redis_params().map_err(tool_error)
    }
}

impl ToolHandler for RedisToolHandler {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            id: self.tool.id().to_string(),
            title: self.tool.title().to_string(),
            description: self.tool.description().to_string(),
            input_schema: self.tool.input_schema(),
            output_schema: json!({ "type": "object" }),
            permissions: Vec::new(),
            mode: ToolMode::Deterministic,
            adapters: vec![
                ToolAdapter::Mcp,
                ToolAdapter::FunctionCalling,
                ToolAdapter::Cli,
            ],
            annotations: self.tool.annotations(),
        }
    }

    fn call(&self, input: Value, _context: ToolContext) -> tool_runtime::ToolFuture {
        let handler = self.clone();
        Box::pin(async move { handler.execute(input).await })
    }
}

impl RedisTool {
    fn id(self) -> &'static str {
        match self {
            RedisTool::Command => REDIS_COMMAND_TOOL,
            RedisTool::Keys => REDIS_KEYS_TOOL,
            RedisTool::Get => REDIS_GET_TOOL,
            RedisTool::Set => REDIS_SET_TOOL,
        }
    }

    fn title(self) -> &'static str {
        match self {
            RedisTool::Command => "Execute Redis command",
            RedisTool::Keys => "List Redis keys",
            RedisTool::Get => "Get Redis key",
            RedisTool::Set => "Set Redis key",
        }
    }

    fn description(self) -> &'static str {
        match self {
            RedisTool::Command => {
                "Execute one Redis command through a saved Redis connection. The connection argument accepts a saved connection id or exact saved connection name. Pass db to target a specific logical database. The command may mutate Redis data and therefore requires --allow-write when called through onetcli tool call."
            }
            RedisTool::Keys => {
                "List Redis keys matching a pattern through a saved Redis connection. This is read-only but may be expensive on large databases."
            }
            RedisTool::Get => {
                "Read the string value for one Redis key through a saved Redis connection."
            }
            RedisTool::Set => {
                "Set the string value for one Redis key through a saved Redis connection. This mutates Redis data and requires --allow-write when called through onetcli tool call."
            }
        }
    }

    fn input_schema(self) -> Value {
        match self {
            RedisTool::Command => command_schema(),
            RedisTool::Keys => keys_schema(),
            RedisTool::Get => get_schema(),
            RedisTool::Set => set_schema(),
        }
    }

    fn annotations(self) -> ToolAnnotations {
        match self {
            RedisTool::Command => ToolAnnotations::mutating(self.title()),
            RedisTool::Keys => {
                ToolAnnotations::read_only(self.title()).with_risk(RiskLevel::Medium)
            }
            RedisTool::Get => ToolAnnotations::read_only(self.title()).with_risk(RiskLevel::Low),
            RedisTool::Set => ToolAnnotations::mutating(self.title()),
        }
    }
}

fn find_connection(
    repo: &ConnectionRepository,
    connection: &str,
) -> Result<one_core::storage::StoredConnection, ToolError> {
    if let Ok(id) = connection.parse::<i64>() {
        return repo
            .get(id)
            .map_err(tool_error)?
            .ok_or_else(|| unknown_connection(connection));
    }
    repo.list()
        .map_err(tool_error)?
        .into_iter()
        .find(|stored| stored.name == connection)
        .ok_or_else(|| unknown_connection(connection))
}

fn validate_command(parts: &[String], db: u8, mode: RedisMode) -> Result<(), ToolError> {
    if parts.is_empty() {
        return Err(ToolError::Failed {
            message: "missing Redis command".to_string(),
        });
    }
    if parts[0].eq_ignore_ascii_case("SELECT") {
        return Err(ToolError::Failed {
            message: "SELECT is not supported; pass db instead".to_string(),
        });
    }
    if mode == RedisMode::Cluster && db != 0 {
        return Err(ToolError::Failed {
            message: "Redis Cluster only supports database 0".to_string(),
        });
    }
    Ok(())
}

fn unknown_connection(connection: &str) -> ToolError {
    ToolError::Failed {
        message: format!("unknown Redis connection: {connection}"),
    }
}

fn tool_error(error: impl std::fmt::Display) -> ToolError {
    ToolError::Failed {
        message: error.to_string(),
    }
}
