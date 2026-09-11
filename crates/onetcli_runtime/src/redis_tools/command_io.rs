use base64::Engine as _;
use one_core::storage::{RedisMode, RedisParams};
use redis_runtime::{RedisConnection, RedisConnectionConfig, RedisConnectionMode, RedisValue};
use serde_json::{Value, json};
use tool_runtime::ToolError;

pub(super) struct RedisCommandOutput {
    pub value: Value,
    pub display: String,
}

pub(super) async fn run_command(
    params: RedisParams,
    parts: &[String],
) -> Result<RedisCommandOutput, ToolError> {
    reject_unsupported_redis_config(&params)?;
    let db = params.db_index;
    let config = redis_connection_config(&params);
    let value = execute_command(config, db, parts).await?;
    Ok(redis_value_output(value))
}

fn redis_connection_config(params: &RedisParams) -> RedisConnectionConfig {
    RedisConnectionConfig {
        id: String::new(),
        name: String::new(),
        host: params.host.clone(),
        port: params.port,
        password: params.password.clone(),
        username: params.username.clone(),
        db_index: params.db_index,
        use_tls: params.use_tls,
        timeout: params.connect_timeout.unwrap_or(10),
        mode: RedisConnectionMode::Standalone,
        ssh_tunnel: params.ssh_tunnel.clone(),
    }
}

/// 内嵌 redis-rs 后端：直接在主进程内建连执行。
async fn execute_command(
    config: RedisConnectionConfig,
    db: u8,
    parts: &[String],
) -> Result<RedisValue, ToolError> {
    let mut connection = redis_runtime::RedisConnectionImpl::new(config);
    connection.connect().await.map_err(tool_error)?;
    let result = connection.command_parts_in_db(db, parts).await;
    let _ = connection.disconnect().await;
    result.map_err(tool_error)
}

fn reject_unsupported_redis_config(params: &RedisParams) -> Result<(), ToolError> {
    if params
        .ssh_tunnel
        .as_ref()
        .is_some_and(|tunnel| tunnel.enabled)
    {
        return Err(ToolError::Failed {
            message: "Redis SSH tunnel requires host-side tunnel setup".into(),
        });
    }
    if params.mode != RedisMode::Standalone {
        return Err(ToolError::Failed {
            message: "Redis provider currently supports standalone Redis".into(),
        });
    }
    Ok(())
}

fn redis_value_output(value: RedisValue) -> RedisCommandOutput {
    let display = value.to_display_string();
    let value = redis_value_json(value);
    RedisCommandOutput { value, display }
}

fn redis_value_json(value: RedisValue) -> Value {
    match value {
        RedisValue::Nil => json!({"type":"nil","value":null}),
        RedisValue::String(value) => json!({"type":"string","value":value}),
        RedisValue::Integer(value) => json!({"type":"integer","value":value}),
        RedisValue::Float(value) => json!({"type":"float","value":value}),
        RedisValue::Status(value) => json!({"type":"status","value":value}),
        RedisValue::Error(value) => json!({"type":"error","value":value}),
        RedisValue::Binary(value) => json!({
            "type":"binary",
            "base64":base64::engine::general_purpose::STANDARD.encode(&value),
            "bytes":value.len()
        }),
        RedisValue::Bulk(values) => json!({
            "type":"array",
            "value":values.into_iter().map(redis_value_json).collect::<Vec<_>>()
        }),
    }
}

fn tool_error(error: impl std::fmt::Display) -> ToolError {
    ToolError::Failed {
        message: error.to_string(),
    }
}
