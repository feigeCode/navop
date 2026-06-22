use super::{PublicMcpToolContext, PublicMcpToolFuture, PublicMcpToolProvider};
use crate::approval::PublicMcpApprovalOutcome;
use crate::permissions::{ApprovalDecision, PublicMcpOperationKind, decide_permission};
use crate::registry::PublicMcpRegistry;
use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, JsonObject, Tool, ToolAnnotations},
};
use serde_json::{Value, json};
use std::{borrow::Cow, sync::Arc};

#[derive(Clone)]
pub struct TerminalToolProvider {
    registry: PublicMcpRegistry,
}

impl TerminalToolProvider {
    pub fn new(registry: PublicMcpRegistry) -> Self {
        Self { registry }
    }

    fn list_sessions(&self) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::structured(json!({
            "sessions": self.registry.list_sessions()
        })))
    }

    fn terminal_snapshot(&self, arguments: Option<JsonObject>) -> Result<CallToolResult, McpError> {
        let session_id = required_string(arguments.as_ref(), "session_id")?;
        let snapshot = self
            .registry
            .terminal_snapshot(session_id)
            .map_err(internal_error)?;
        Ok(CallToolResult::structured(
            serde_json::to_value(snapshot).map_err(internal_error)?,
        ))
    }

    async fn terminal_write(
        &self,
        arguments: Option<JsonObject>,
        context: PublicMcpToolContext,
    ) -> Result<CallToolResult, McpError> {
        match decide_permission(
            context.permission_mode,
            PublicMcpOperationKind::WriteTerminal,
        ) {
            ApprovalDecision::Allow => self.write_terminal(arguments.as_ref()),
            ApprovalDecision::Ask => self.ask_then_write(arguments, context).await,
            ApprovalDecision::Deny => Ok(permission_denied_result(
                "terminal write denied by permission mode",
            )),
        }
    }

    fn write_terminal(&self, arguments: Option<&JsonObject>) -> Result<CallToolResult, McpError> {
        let (session_id, input) = terminal_write_args(arguments)?;
        self.registry
            .write_terminal(&session_id, &input)
            .map_err(internal_error)?;
        Ok(CallToolResult::structured(json!({ "ok": true })))
    }

    async fn ask_then_write(
        &self,
        arguments: Option<JsonObject>,
        context: PublicMcpToolContext,
    ) -> Result<CallToolResult, McpError> {
        let (session_id, input) = terminal_write_args(arguments.as_ref())?;
        let outcome = context
            .request_approval(
                PublicMcpOperationKind::WriteTerminal,
                "public_mcp.terminal_write",
                format!("Write to terminal session {session_id}"),
                json!({
                    "session_id": session_id,
                    "input_preview": input.chars().take(120).collect::<String>()
                }),
            )
            .await;

        match outcome {
            PublicMcpApprovalOutcome::Approved => self.write_terminal(arguments.as_ref()),
            PublicMcpApprovalOutcome::Denied { reason } => {
                Ok(permission_denied_result(reason.unwrap_or_else(|| {
                    "terminal write denied by approval".to_string()
                })))
            }
        }
    }
}

impl PublicMcpToolProvider for TerminalToolProvider {
    fn tools(&self) -> Vec<Tool> {
        terminal_tools()
    }

    fn call_tool(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
        context: PublicMcpToolContext,
    ) -> Option<PublicMcpToolFuture> {
        match name {
            "public_mcp.list_sessions" => {
                let result = self.list_sessions();
                Some(Box::pin(async move { result }))
            }
            "public_mcp.terminal_snapshot" => {
                let result = self.terminal_snapshot(arguments);
                Some(Box::pin(async move { result }))
            }
            "public_mcp.terminal_write" => {
                let provider = self.clone();
                Some(Box::pin(async move {
                    provider.terminal_write(arguments, context).await
                }))
            }
            _ => None,
        }
    }
}

fn terminal_tools() -> Vec<Tool> {
    vec![
        Tool::new(
            "public_mcp.list_sessions",
            "List currently connected SSH terminal sessions exposed by OnetCli.",
            object_schema([]),
        )
        .with_annotations(read_only_annotations("List terminal sessions")),
        Tool::new(
            "public_mcp.terminal_snapshot",
            "Read the visible text of an exposed SSH terminal session.",
            object_schema([("session_id", string_schema())]),
        )
        .with_annotations(read_only_annotations("Read terminal snapshot")),
        Tool::new(
            "public_mcp.terminal_write",
            "Write raw input to an exposed SSH terminal session.",
            object_schema([("session_id", string_schema()), ("input", string_schema())]),
        )
        .with_annotations(
            ToolAnnotations::with_title("Write terminal input")
                .read_only(false)
                .destructive(true)
                .idempotent(false)
                .open_world(true),
        ),
    ]
}

fn read_only_annotations(title: &str) -> ToolAnnotations {
    ToolAnnotations::with_title(title)
        .read_only(true)
        .destructive(false)
        .idempotent(true)
        .open_world(false)
}

fn object_schema(properties: impl IntoIterator<Item = (&'static str, Value)>) -> Arc<JsonObject> {
    let required = properties
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect::<JsonObject>();
    let required_names = required
        .keys()
        .cloned()
        .map(Value::String)
        .collect::<Vec<_>>();
    let mut schema = JsonObject::new();
    schema.insert("type".to_string(), Value::String("object".to_string()));
    schema.insert("properties".to_string(), Value::Object(required));
    if !required_names.is_empty() {
        schema.insert("required".to_string(), Value::Array(required_names));
    }
    Arc::new(schema)
}

fn string_schema() -> Value {
    json!({ "type": "string" })
}

fn required_string<'a>(
    arguments: Option<&'a JsonObject>,
    field: &'static str,
) -> Result<&'a str, McpError> {
    arguments
        .and_then(|args| args.get(field))
        .and_then(Value::as_str)
        .ok_or_else(|| McpError::invalid_params(format!("missing string argument: {field}"), None))
}

fn terminal_write_args(arguments: Option<&JsonObject>) -> Result<(String, String), McpError> {
    Ok((
        required_string(arguments, "session_id")?.to_string(),
        required_string(arguments, "input")?.to_string(),
    ))
}

fn permission_denied_result(message: impl Into<String>) -> CallToolResult {
    CallToolResult::structured_error(json!({
        "code": "permission_denied",
        "message": message.into()
    }))
}

fn internal_error(error: impl std::fmt::Display) -> McpError {
    McpError::internal_error(Cow::Owned(error.to_string()), None)
}
