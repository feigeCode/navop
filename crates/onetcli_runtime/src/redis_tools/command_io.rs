use base64::Engine as _;
use extension_host::NativeDriverRegistry;
use one_core::storage::{RedisMode, RedisParams};
use redis_runtime::{RedisConnectionConfig, RedisConnectionMode, RedisValue};
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
    let manifest = redis_manifest()?;
    let config = RedisConnectionConfig {
        id: String::new(),
        name: String::new(),
        host: params.host,
        port: params.port,
        password: params.password,
        username: params.username,
        db_index: params.db_index,
        use_tls: params.use_tls,
        timeout: params.connect_timeout.unwrap_or(10),
        mode: RedisConnectionMode::Standalone,
        ssh_tunnel: params.ssh_tunnel,
    };
    let connection = redis_runtime::IpcRedisConnection::start(&manifest, config)
        .await
        .map_err(tool_error)?;
    let value = connection
        .command_bytes(
            Some(params.db_index),
            parts.iter().map(|part| part.as_bytes().to_vec()).collect(),
        )
        .await
        .map_err(tool_error)?;
    connection.shutdown().await;
    Ok(redis_value_output(value))
}

fn redis_manifest() -> Result<extension_host::NativeDriverManifest, ToolError> {
    let root = one_core::storage::manager::get_config_dir()
        .map_err(tool_error)?
        .join("extensions")
        .join("database_drivers");
    let registry = NativeDriverRegistry::load_from_dir(&root).map_err(tool_error)?;
    registry
        .find("redis", "redis")
        .ok_or_else(|| ToolError::Failed {
            message: "Redis native driver is not installed".into(),
        })
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
            message: "Redis IPC provider currently supports standalone Redis".into(),
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
