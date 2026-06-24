use super::{PublicMcpToolContext, PublicMcpToolFuture, PublicMcpToolProvider};
use crate::approval::PublicMcpApprovalOutcome;
use crate::permissions::{ApprovalDecision, PublicMcpOperationKind, decide_permission};
use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, JsonObject, Tool, ToolAnnotations},
};
use serde_json::{Value, json};
use std::sync::Arc;
use tool_runtime::{
    ToolAdapter, ToolAnnotations as RuntimeToolAnnotations, ToolContext, ToolDescriptor, ToolError,
    ToolRegistry, ToolResult,
};

#[derive(Clone)]
pub struct ToolRuntimeMcpProvider {
    registry: ToolRegistry,
}

impl ToolRuntimeMcpProvider {
    pub fn new(registry: ToolRegistry) -> Self {
        Self { registry }
    }
}

impl PublicMcpToolProvider for ToolRuntimeMcpProvider {
    fn tools(&self) -> Vec<Tool> {
        self.registry
            .list(ToolAdapter::Mcp)
            .into_iter()
            .map(runtime_tool_to_mcp_tool)
            .collect()
    }

    fn call_tool(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
        context: PublicMcpToolContext,
    ) -> Option<PublicMcpToolFuture> {
        let descriptor = self.registry.get(name, ToolAdapter::Mcp)?;
        let registry = self.registry.clone();
        let name = name.to_string();
        let input = Value::Object(arguments.unwrap_or_default());
        Some(Box::pin(async move {
            let call_annotations = registry
                .call_annotations(&name, ToolAdapter::Mcp, &input)
                .unwrap_or_else(|| descriptor.annotations.clone());
            call_runtime_tool(registry, descriptor, call_annotations, name, input, context).await
        }))
    }
}

async fn call_runtime_tool(
    registry: ToolRegistry,
    descriptor: ToolDescriptor,
    call_annotations: RuntimeToolAnnotations,
    name: String,
    input: Value,
    context: PublicMcpToolContext,
) -> Result<CallToolResult, McpError> {
    if call_annotations.read_only {
        return run_runtime_tool(registry, name, input).await;
    }

    match decide_permission(
        context.permission_mode,
        PublicMcpOperationKind::CallToolRuntimeTool,
    ) {
        ApprovalDecision::Allow => run_runtime_tool(registry, name, input).await,
        ApprovalDecision::Ask => {
            ask_then_run_runtime_tool(registry, descriptor, name, input, context).await
        }
        ApprovalDecision::Deny => Ok(permission_denied_result(
            "tool runtime call denied by permission mode",
        )),
    }
}

async fn ask_then_run_runtime_tool(
    registry: ToolRegistry,
    descriptor: ToolDescriptor,
    name: String,
    input: Value,
    context: PublicMcpToolContext,
) -> Result<CallToolResult, McpError> {
    let outcome = context
        .request_approval(
            PublicMcpOperationKind::CallToolRuntimeTool,
            name.clone(),
            format!("Call {}", descriptor.title),
            json!({
                "tool": name,
                "arguments": redact_secrets(input.clone()),
            }),
        )
        .await;

    match outcome {
        PublicMcpApprovalOutcome::Approved => run_runtime_tool(registry, name, input).await,
        PublicMcpApprovalOutcome::Denied { reason } => {
            Ok(permission_denied_result(reason.unwrap_or_else(|| {
                "tool runtime call denied by approval".to_string()
            })))
        }
    }
}

async fn run_runtime_tool(
    registry: ToolRegistry,
    name: String,
    input: Value,
) -> Result<CallToolResult, McpError> {
    registry
        .call(&name, input, ToolContext::for_adapter(ToolAdapter::Mcp))
        .await
        .map(runtime_result_to_mcp_result)
        .map_err(runtime_error_to_mcp_error)
}

fn runtime_tool_to_mcp_tool(descriptor: ToolDescriptor) -> Tool {
    Tool::new(
        descriptor.id,
        descriptor.description,
        schema_object(descriptor.input_schema),
    )
    .with_annotations(runtime_annotations_to_mcp_annotations(
        descriptor.annotations,
    ))
}

fn schema_object(schema: Value) -> Arc<JsonObject> {
    match schema {
        Value::Object(object) => Arc::new(object),
        _ => {
            let mut object = JsonObject::new();
            object.insert("type".to_string(), Value::String("object".to_string()));
            Arc::new(object)
        }
    }
}

fn runtime_annotations_to_mcp_annotations(
    annotations: tool_runtime::ToolAnnotations,
) -> ToolAnnotations {
    ToolAnnotations::with_title(annotations.title)
        .read_only(annotations.read_only)
        .destructive(annotations.destructive)
        .idempotent(annotations.idempotent)
        .open_world(annotations.open_world)
}

fn runtime_result_to_mcp_result(result: ToolResult) -> CallToolResult {
    CallToolResult::structured(result.structured_content)
}

fn runtime_error_to_mcp_error(error: ToolError) -> McpError {
    match error {
        ToolError::UnknownTool { id } => {
            McpError::invalid_params(format!("unknown tool: {id}"), None)
        }
        ToolError::UnsupportedAdapter { id, adapter } => McpError::invalid_params(
            format!("tool `{id}` is not exposed for adapter {adapter:?}"),
            None,
        ),
        ToolError::Failed { message } => McpError::internal_error(message, None),
    }
}

fn permission_denied_result(message: impl Into<String>) -> CallToolResult {
    CallToolResult::structured_error(json!({
        "code": "permission_denied",
        "message": message.into()
    }))
}

fn redact_secrets(value: Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .map(|(key, value)| {
                    if is_secret_key(&key) {
                        (key, Value::String("<redacted>".to_string()))
                    } else {
                        (key, redact_secrets(value))
                    }
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(redact_secrets).collect()),
        value => value,
    }
}

fn is_secret_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase();
    normalized.contains("password")
        || normalized.contains("passphrase")
        || normalized.contains("secret")
        || normalized.contains("token")
        || normalized.contains("private_key")
}
