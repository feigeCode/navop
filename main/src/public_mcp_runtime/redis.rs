use base64::{Engine as _, engine::general_purpose};
use gpui::App;
use public_mcp::tools::{
    RedisCommandExecution, RedisCommandExecutionProvider, RedisConnectionSnapshot,
    RedisConnectionSnapshotProvider, RedisToolProvider,
};
use redis_view::{GlobalRedisState, RedisValue};
use serde_json::{Value, json};
use std::sync::Arc;
use tool_runtime::{ToolError, ToolHandler, ToolResult};

pub(super) fn redis_tool_handlers(cx: &App) -> Vec<Arc<dyn ToolHandler>> {
    match cx.try_global::<GlobalRedisState>().cloned() {
        Some(state) => RedisToolProvider::handlers(Arc::new(RedisRuntimeSnapshots { state })),
        None => {
            tracing::warn!("Public MCP Redis toolset enabled before Redis state is initialized");
            RedisToolProvider::empty()
        }
    }
}

struct RedisRuntimeSnapshots {
    state: GlobalRedisState,
}

impl RedisConnectionSnapshotProvider for RedisRuntimeSnapshots {
    fn list_connections(&self) -> Vec<RedisConnectionSnapshot> {
        self.state
            .connection_ids()
            .into_iter()
            .map(|connection_id| RedisConnectionSnapshot { connection_id })
            .collect()
    }
}

impl RedisCommandExecutionProvider for RedisRuntimeSnapshots {
    fn execute_command(
        &self,
        connection_id: &str,
        db: Option<u8>,
        command: &str,
    ) -> tool_runtime::ToolFuture {
        let state = self.state.clone();
        let connection_id = connection_id.to_string();
        let command = command.to_string();
        Box::pin(async move {
            let connection =
                state
                    .get_connection(&connection_id)
                    .ok_or_else(|| ToolError::Failed {
                        message: format!("unknown Redis connection: {connection_id}"),
                    })?;
            let guard = connection.read().await;
            let value = match db {
                Some(db) => guard.execute_command_in_db(db, &command).await,
                None => guard.execute_command(&command).await,
            }
            .map_err(|error| ToolError::Failed {
                message: error.to_string(),
            })?;
            Ok(ToolResult::structured(
                RedisCommandExecution {
                    connection_id,
                    db,
                    command,
                    display: value.to_display_string(),
                    result: redis_value_json(value),
                }
                .into_json(),
            ))
        })
    }
}

fn redis_value_json(value: RedisValue) -> Value {
    match value {
        RedisValue::Nil => json!({ "type": "nil", "value": null }),
        RedisValue::String(value) => json!({ "type": "string", "value": value }),
        RedisValue::Integer(value) => json!({ "type": "integer", "value": value }),
        RedisValue::Float(value) => json!({ "type": "float", "value": value }),
        RedisValue::Status(value) => json!({ "type": "status", "value": value }),
        RedisValue::Error(value) => json!({ "type": "error", "value": value }),
        RedisValue::Binary(value) => json!({
            "type": "binary",
            "base64": general_purpose::STANDARD.encode(&value),
            "bytes": value.len()
        }),
        RedisValue::Bulk(items) => json!({
            "type": "array",
            "value": items.into_iter().map(redis_value_json).collect::<Vec<_>>()
        }),
    }
}
