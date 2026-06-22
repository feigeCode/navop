use super::{PublicMcpToolContext, PublicMcpToolFuture, PublicMcpToolProvider};
use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, JsonObject, Tool, ToolAnnotations},
};
use serde_json::Value;
use std::sync::Arc;
use tool_runtime::{ToolAdapter, ToolContext, ToolDescriptor, ToolError, ToolRegistry, ToolResult};

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
        _context: PublicMcpToolContext,
    ) -> Option<PublicMcpToolFuture> {
        if self.registry.get(name, ToolAdapter::Mcp).is_none() {
            return None;
        }
        let registry = self.registry.clone();
        let name = name.to_string();
        let input = Value::Object(arguments.unwrap_or_default());
        Some(Box::pin(async move {
            registry
                .call(&name, input, ToolContext::for_adapter(ToolAdapter::Mcp))
                .await
                .map(runtime_result_to_mcp_result)
                .map_err(runtime_error_to_mcp_error)
        }))
    }
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
