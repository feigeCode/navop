use base64::{Engine as _, engine::general_purpose};
use one_core::storage::{RedisMode, RedisParams};
use redis_client::aio::{ConnectionManager, ConnectionManagerConfig};
use redis_client::{Client, ConnectionAddr, ConnectionInfo, RedisConnectionInfo};
use serde_json::{Value, json};
use std::time::Duration;
use tool_runtime::ToolError;

const DEFAULT_TIMEOUT_SECS: u64 = 10;

pub(super) struct RedisCommandOutput {
    pub value: Value,
    pub display: String,
}

pub(super) async fn run_command(
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
            message: "redis.command does not support Redis SSH tunnels in onetcli CLI yet"
                .to_string(),
        });
    }
    if params.mode != RedisMode::Standalone {
        return Err(ToolError::Failed {
            message: "redis.command currently supports standalone Redis connections".to_string(),
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

fn tool_error(error: impl std::fmt::Display) -> ToolError {
    ToolError::Failed {
        message: error.to_string(),
    }
}
