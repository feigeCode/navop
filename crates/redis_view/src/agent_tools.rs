use crate::manager::GlobalRedisState;
use crate::types::RedisValue;
use agent_runtime::{
    ResourceContext, ResourceKind, RiskLevel, ToolError, ToolName, ToolObservation, ToolRegistry,
    ToolSpec,
    tools::{ObservationData, Tool, ToolInvocation},
};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose};
use gpui::App;
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Clone)]
struct AgentRedisTool {
    state: Option<GlobalRedisState>,
}

pub fn register_agent_redis_tools(cx: &mut App, registry: &mut ToolRegistry) -> anyhow::Result<()> {
    let state = cx.try_global::<GlobalRedisState>().cloned();
    if state.is_none() {
        tracing::warn!("Agent Redis tools enabled before Redis state is initialized");
    }
    registry.register(Arc::new(AgentRedisTool { state }));
    Ok(())
}

#[async_trait]
impl Tool for AgentRedisTool {
    fn name(&self) -> ToolName {
        ToolName::new("redis_execute_command")
    }

    fn spec(&self, resources: &ResourceContext) -> ToolSpec {
        let suffix = current_context_suffix(resources);
        ToolSpec::new(
            self.name(),
            format!(
                "Execute one Redis command against the current Agent Redis context. This always requires user approval before execution.{suffix}"
            ),
            json!({
                "type": "object",
                "properties": {
                    "connection": {
                        "type": "string",
                        "description": "Optional Redis connection id. Defaults to the current Agent Redis resource id."
                    },
                    "command": {
                        "type": "string",
                        "description": "Single Redis command, for example `PING`, `GET user:1`, or `SET user:1 Ada`."
                    },
                    "db": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": 255,
                        "description": "Optional Redis logical database index. Defaults to the current Agent Redis db scope when present."
                    }
                },
                "required": ["command"]
            }),
        )
        .with_risk(RiskLevel::High)
    }

    async fn execute(&self, invocation: ToolInvocation) -> Result<ToolObservation, ToolError> {
        let state = self
            .state
            .clone()
            .ok_or_else(|| ToolError::Execution("Redis runtime state is not initialized".into()))?;
        let target = resolve_redis_target(&invocation)?;
        let command = required_str(&invocation.arguments, "command")?;
        let connection = state.get_connection(&target.connection_id).ok_or_else(|| {
            ToolError::MissingResource(format!(
                "unknown Redis connection: {}",
                target.connection_id
            ))
        })?;
        let guard = connection.read().await;
        let value = match target.db {
            Some(db) => guard.execute_command_in_db(db, &command).await,
            None => guard.execute_command(&command).await,
        }
        .map_err(|error| ToolError::Execution(error.to_string()))?;
        Ok(ToolObservation::success(
            invocation.call_id,
            invocation.tool_name,
            "Redis command executed",
            ObservationData::Json(json!({
                "connection": target.connection_id,
                "db": target.db,
                "command": command,
                "display": value.to_display_string(),
                "result": redis_value_json(value),
            })),
        ))
    }
}

struct RedisTarget {
    connection_id: String,
    db: Option<u8>,
}

fn resolve_redis_target(invocation: &ToolInvocation) -> Result<RedisTarget, ToolError> {
    let resource = invocation.target_resource();
    if let Some(resource) = resource
        && resource.kind != ResourceKind::Redis
    {
        return Err(ToolError::MissingResource(format!(
            "current Agent resource is not a Redis connection: {}",
            resource.id
        )));
    }
    let connection_id = invocation
        .arg_str("connection")
        .map(ToString::to_string)
        .or_else(|| resource.map(|item| item.id.to_string()))
        .ok_or_else(|| {
            ToolError::MissingResource(
                "please select a Redis connection in the Redis sidebar first".into(),
            )
        })?;
    let db = optional_u8(&invocation.arguments, "db")?
        .or_else(|| scope_value(resource, "db").and_then(|value| value.parse::<u8>().ok()));
    Ok(RedisTarget { connection_id, db })
}

fn current_context_suffix(resources: &ResourceContext) -> String {
    let Some(resource) = resources.current() else {
        return String::new();
    };
    if resource.kind != ResourceKind::Redis {
        return String::new();
    }
    format!(
        " Defaults: connection={}, db={}.",
        resource.id,
        scope_value(Some(resource), "db").unwrap_or_else(|| "<none>".into())
    )
}

fn scope_value(resource: Option<&agent_runtime::ResourceRef>, key: &str) -> Option<String> {
    resource.and_then(|resource| {
        resource
            .scopes
            .iter()
            .find(|scope| scope.key == key)
            .map(|scope| scope.value.clone())
    })
}

fn required_str(input: &Value, key: &str) -> Result<String, ToolError> {
    input
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            ToolError::InvalidArguments(format!("missing required string field `{key}`"))
        })
}

fn optional_u8(input: &Value, key: &str) -> Result<Option<u8>, ToolError> {
    match input.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|value| u8::try_from(value).ok())
            .map(Some)
            .ok_or_else(|| {
                ToolError::InvalidArguments(format!(
                    "field `{key}` must be an integer from 0 to 255"
                ))
            }),
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
