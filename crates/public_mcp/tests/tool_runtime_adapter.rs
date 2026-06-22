use std::sync::Arc;

use public_mcp::{
    permissions::PermissionMode,
    tools::{PublicMcpToolContext, PublicMcpToolRegistry, ToolRuntimeMcpProvider},
};
use serde_json::json;
use tool_runtime::{
    ToolAdapter, ToolAnnotations, ToolContext, ToolDescriptor, ToolHandler, ToolMode, ToolRegistry,
    ToolResult,
};

#[test]
fn tool_runtime_provider_exposes_mcp_tools_and_dispatches_calls() {
    let provider = ToolRuntimeMcpProvider::new(ToolRegistry::new(vec![Arc::new(RuntimeEchoTool)]));
    let registry = PublicMcpToolRegistry::new(vec![Arc::new(provider)]);

    let tools = registry.tools();
    assert_eq!(1, tools.len());
    assert_eq!("example.echo", tools[0].name);
    assert_eq!(Some("Echo input"), tools[0].description.as_deref());

    let result = futures::executor::block_on(registry.call_tool(
        "example.echo",
        Some(rmcp::model::JsonObject::from_iter([(
            "message".to_string(),
            json!("hello"),
        )])),
        PublicMcpToolContext {
            permission_mode: PermissionMode::Deny,
            approver: Default::default(),
        },
    ))
    .expect("tool runtime MCP provider should dispatch call");

    assert_eq!(
        Some(json!({ "message": "hello" })),
        result.structured_content
    );
}

#[derive(Clone)]
struct RuntimeEchoTool;

impl ToolHandler for RuntimeEchoTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            id: "example.echo".to_string(),
            title: "Echo".to_string(),
            description: "Echo input".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string" }
                },
                "required": ["message"]
            }),
            output_schema: json!({ "type": "object" }),
            permissions: Vec::new(),
            mode: ToolMode::Deterministic,
            adapters: vec![ToolAdapter::Mcp],
            annotations: ToolAnnotations::read_only("Echo"),
        }
    }

    fn call(&self, input: serde_json::Value, _context: ToolContext) -> tool_runtime::ToolFuture {
        Box::pin(async move { Ok(ToolResult::structured(input)) })
    }
}
