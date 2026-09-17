use base64::Engine as _;
use one_core::storage::{RedisMode, RedisParams};
use redis_runtime::{RedisConnection, RedisConnectionConfig, RedisConnectionMode, RedisValue};
use serde_json::{Value, json};
use tool_runtime::ToolError;

/// 单次 `redis.keys` 扫描返回的键数量上限。
const MAX_SCAN_KEYS: usize = 1000;
/// SCAN 每次游标迭代的 COUNT。
const SCAN_BATCH: usize = 200;

pub(super) struct RedisCommandOutput {
    pub value: Value,
    pub display: String,
    pub truncated: bool,
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

/// `redis.keys` 的受限实现：用 SCAN 分批扫描，最多返回 `MAX_SCAN_KEYS` 个键，
/// 避免 `KEYS` 在大库上阻塞服务器并一次性把全部键名拉进内存。
pub(super) async fn run_scan_keys(
    params: RedisParams,
    pattern: String,
) -> Result<RedisCommandOutput, ToolError> {
    reject_unsupported_redis_config(&params)?;
    let db = params.db_index;
    let config = redis_connection_config(&params);
    let mut connection = redis_runtime::RedisConnectionImpl::new(config);
    connection.connect().await.map_err(tool_error)?;

    let mut keys: Vec<RedisValue> = Vec::new();
    let mut cursor: u64 = 0;
    let mut truncated = false;
    loop {
        let parts = vec![
            "SCAN".to_string(),
            cursor.to_string(),
            "MATCH".to_string(),
            pattern.clone(),
            "COUNT".to_string(),
            SCAN_BATCH.to_string(),
        ];
        let reply = connection
            .command_parts_in_db(db, &parts)
            .await
            .map_err(tool_error)?;
        let (next, batch) = parse_scan_reply(reply)?;
        for key in batch {
            if keys.len() >= MAX_SCAN_KEYS {
                truncated = true;
                break;
            }
            keys.push(key);
        }
        cursor = next;
        if truncated || cursor == 0 {
            break;
        }
    }
    let _ = connection.disconnect().await;

    let value = RedisValue::Bulk(keys);
    let display = value.to_display_string();
    Ok(RedisCommandOutput {
        value: redis_value_json(value),
        display,
        truncated,
    })
}

/// 解析 `SCAN` 的 `[cursor, [keys...]]` 回复；键保留原始字节形态。
fn parse_scan_reply(value: RedisValue) -> Result<(u64, Vec<RedisValue>), ToolError> {
    let RedisValue::Bulk(mut items) = value else {
        return Err(ToolError::Failed {
            message: "unexpected SCAN reply shape".into(),
        });
    };
    if items.len() != 2 {
        return Err(ToolError::Failed {
            message: "unexpected SCAN reply arity".into(),
        });
    }
    let keys = match items.pop() {
        Some(RedisValue::Bulk(keys)) => keys,
        _ => {
            return Err(ToolError::Failed {
                message: "unexpected SCAN keys shape".into(),
            });
        }
    };
    let cursor = match items.pop() {
        Some(RedisValue::String(cursor)) => cursor.parse::<u64>().unwrap_or(0),
        Some(RedisValue::Integer(cursor)) => cursor.max(0) as u64,
        _ => 0,
    };
    Ok((cursor, keys))
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
    RedisCommandOutput {
        value,
        display,
        truncated: false,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_reply_parses_cursor_and_preserves_binary_keys() {
        let reply = RedisValue::Bulk(vec![
            RedisValue::String("42".to_string()),
            RedisValue::Bulk(vec![
                RedisValue::String("plain".to_string()),
                RedisValue::Binary(vec![0, 0xff]),
            ]),
        ]);

        let (cursor, keys) = parse_scan_reply(reply).expect("scan reply should parse");

        assert_eq!(42, cursor);
        assert_eq!(2, keys.len());
        assert!(matches!(&keys[1], RedisValue::Binary(bytes) if bytes == &vec![0, 0xff]));
    }

    #[test]
    fn scan_reply_rejects_unexpected_shapes() {
        assert!(parse_scan_reply(RedisValue::Nil).is_err());
        assert!(parse_scan_reply(RedisValue::Bulk(vec![RedisValue::String("1".into())])).is_err());
    }
}
