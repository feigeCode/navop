use base64::{Engine as _, engine::general_purpose};
use one_core::storage::traits::Repository;
use one_core::storage::{ConnectionRepository, ConnectionType, RedisMode, RedisParams};
use redis_client::aio::{ConnectionManager, ConnectionManagerConfig};
use redis_client::{Client, ConnectionAddr, ConnectionInfo, RedisConnectionInfo};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tool_runtime::{
    ToolAdapter, ToolAnnotations, ToolContext, ToolDescriptor, ToolError, ToolHandler, ToolMode,
    ToolRegistry, ToolResult,
};

const REDIS_EXECUTE_COMMAND_TOOL: &str = "redis.execute_command";
const DEFAULT_TIMEOUT_SECS: u64 = 10;
const MAX_DB_INDEX: u64 = 255;

#[derive(Clone)]
struct RedisExecuteCommandTool {
    repo: Arc<ConnectionRepository>,
}

pub fn redis_tool_registry(repo: Arc<ConnectionRepository>) -> ToolRegistry {
    ToolRegistry::new(vec![Arc::new(RedisExecuteCommandTool { repo })])
}

impl RedisExecuteCommandTool {
    async fn execute(&self, input: Value) -> Result<ToolResult, ToolError> {
        let connection = required_str(&input, "connection")?;
        let command = required_str(&input, "command")?;
        let mut params = self.redis_params(&connection)?;
        let db = optional_u8(&input, "db")?.unwrap_or(params.db_index);
        params.db_index = db;
        let parts = parse_command_args(&command);
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

impl ToolHandler for RedisExecuteCommandTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            id: REDIS_EXECUTE_COMMAND_TOOL.to_string(),
            title: "Execute Redis command".to_string(),
            description: "Execute one Redis command through a saved Redis connection. The connection argument accepts a saved connection id or exact saved connection name. Pass db to target a specific logical database. The command may mutate Redis data and therefore requires --allow-write when called through onetcli tool call.".to_string(),
            input_schema: execute_schema(),
            output_schema: json!({ "type": "object" }),
            permissions: Vec::new(),
            mode: ToolMode::Deterministic,
            adapters: vec![
                ToolAdapter::Mcp,
                ToolAdapter::FunctionCalling,
                ToolAdapter::Cli,
            ],
            annotations: ToolAnnotations::mutating("Execute Redis command"),
        }
    }

    fn call(&self, input: Value, _context: ToolContext) -> tool_runtime::ToolFuture {
        let handler = self.clone();
        Box::pin(async move { handler.execute(input).await })
    }
}

struct RedisCommandOutput {
    value: Value,
    display: String,
}

async fn run_command(
    params: RedisParams,
    parts: &[String],
) -> Result<RedisCommandOutput, ToolError> {
    let mut conn = open_connection(&params).await?;
    let mut cmd = redis_client::cmd(parts[0].as_str());
    for arg in &parts[1..] {
        cmd.arg(arg.as_str());
    }
    let value = cmd
        .query_async::<redis_client::Value>(&mut conn)
        .await
        .map_err(tool_error)?;
    Ok(redis_value_output(value))
}

async fn open_connection(params: &RedisParams) -> Result<ConnectionManager, ToolError> {
    reject_unsupported_redis_config(params)?;
    let client = Client::open(connection_info(params)).map_err(tool_error)?;
    let timeout = Duration::from_secs(params.connect_timeout.unwrap_or(DEFAULT_TIMEOUT_SECS));
    let config = ConnectionManagerConfig::new()
        .set_connection_timeout(timeout)
        .set_response_timeout(timeout)
        .set_number_of_retries(1)
        .set_max_delay(500);
    ConnectionManager::new_with_config(client, config)
        .await
        .map_err(tool_error)
}

fn connection_info(params: &RedisParams) -> ConnectionInfo {
    let addr = if params.use_tls {
        ConnectionAddr::TcpTls {
            host: params.host.clone(),
            port: params.port,
            insecure: false,
            tls_params: None,
        }
    } else {
        ConnectionAddr::Tcp(params.host.clone(), params.port)
    };
    ConnectionInfo {
        addr,
        redis: RedisConnectionInfo {
            db: i64::from(params.db_index),
            username: params.username.clone(),
            password: params.password.clone(),
            ..Default::default()
        },
    }
}

fn reject_unsupported_redis_config(params: &RedisParams) -> Result<(), ToolError> {
    if params
        .ssh_tunnel
        .as_ref()
        .is_some_and(|tunnel| tunnel.enabled)
    {
        return Err(ToolError::Failed {
            message: "redis.execute_command does not support Redis SSH tunnels in onetcli CLI yet"
                .to_string(),
        });
    }
    if params.mode != RedisMode::Standalone {
        return Err(ToolError::Failed {
            message: "redis.execute_command currently supports standalone Redis connections"
                .to_string(),
        });
    }
    Ok(())
}

fn redis_value_output(value: redis_client::Value) -> RedisCommandOutput {
    match value {
        redis_client::Value::Nil => output(json!({ "type": "nil", "value": null }), "(nil)"),
        redis_client::Value::Int(value) => {
            output(json!({ "type": "integer", "value": value }), value)
        }
        redis_client::Value::BulkString(bytes) => bytes_output(bytes),
        redis_client::Value::Array(items) => array_output(items),
        redis_client::Value::SimpleString(value) => {
            output(json!({ "type": "status", "value": value }), value)
        }
        redis_client::Value::Okay => output(json!({ "type": "status", "value": "OK" }), "OK"),
        redis_client::Value::Double(value) => {
            output(json!({ "type": "float", "value": value }), value)
        }
        redis_client::Value::Boolean(value) => {
            let integer = if value { 1 } else { 0 };
            output(json!({ "type": "integer", "value": integer }), integer)
        }
        _ => output(json!({ "type": "nil", "value": null }), "(nil)"),
    }
}

fn bytes_output(bytes: Vec<u8>) -> RedisCommandOutput {
    match String::from_utf8(bytes.clone()) {
        Ok(value) => output(json!({ "type": "string", "value": value }), value),
        Err(_) => RedisCommandOutput {
            value: json!({
                "type": "binary",
                "base64": general_purpose::STANDARD.encode(&bytes),
                "bytes": bytes.len()
            }),
            display: format!("<binary: {} bytes>", bytes.len()),
        },
    }
}

fn array_output(items: Vec<redis_client::Value>) -> RedisCommandOutput {
    let outputs = items
        .into_iter()
        .map(redis_value_output)
        .collect::<Vec<_>>();
    let display = format!(
        "[{}]",
        outputs
            .iter()
            .map(|item| item.display.clone())
            .collect::<Vec<_>>()
            .join(", ")
    );
    RedisCommandOutput {
        value: json!({
            "type": "array",
            "value": outputs.into_iter().map(|item| item.value).collect::<Vec<_>>()
        }),
        display,
    }
}

fn output(value: Value, display: impl ToString) -> RedisCommandOutput {
    RedisCommandOutput {
        value,
        display: display.to_string(),
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

fn execute_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "connection": {
                "type": "string",
                "description": "Saved Redis connection id or exact saved connection name."
            },
            "command": {
                "type": "string",
                "description": "Single Redis command, for example `PING` or `GET user:1`."
            },
            "db": {
                "type": "integer",
                "minimum": 0,
                "maximum": MAX_DB_INDEX,
                "description": "Optional Redis logical database index."
            }
        },
        "required": ["connection", "command"]
    })
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

fn required_str(input: &Value, field: &'static str) -> Result<String, ToolError> {
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
            .filter(|value| *value <= MAX_DB_INDEX)
            .map(|value| value as u8)
            .map(Some)
            .ok_or_else(|| ToolError::Failed {
                message: format!("field `{field}` must be an integer from 0 to {MAX_DB_INDEX}"),
            }),
    }
}

fn parse_command_args(command: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    for ch in command.chars() {
        if escaped {
            current.push(match (in_single || in_double, ch) {
                (true, 'n') => '\n',
                (true, 'r') => '\r',
                (true, 't') => '\t',
                _ => ch,
            });
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            value if value.is_whitespace() && !in_single && !in_double => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            value => current.push(value),
        }
    }
    if !current.is_empty() {
        args.push(current);
    }
    args
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
