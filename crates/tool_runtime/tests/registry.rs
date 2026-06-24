use std::sync::Arc;

use serde_json::json;
use tool_runtime::{
    ToolAdapter, ToolAnnotations, ToolContext, ToolDescriptor, ToolError, ToolHandler, ToolMode,
    ToolRegistry, ToolResult,
};

#[test]
fn registry_rejects_duplicate_tool_ids() {
    let result = ToolRegistry::try_new(vec![
        Arc::new(EchoHandler::new("example.echo")),
        Arc::new(EchoHandler::new("example.echo")),
    ]);

    let error = result.expect_err("duplicate tool id should be rejected");

    assert_eq!(vec!["example.echo"], error.duplicate_tool_ids());
}

#[test]
fn registry_lists_tools_for_requested_adapter_only() {
    let registry = ToolRegistry::new(vec![
        Arc::new(EchoHandler::new("example.mcp").with_adapters(vec![ToolAdapter::Mcp])),
        Arc::new(EchoHandler::new("example.cli").with_adapters(vec![ToolAdapter::Cli])),
    ]);

    let tools = registry.list(ToolAdapter::Mcp);

    assert_eq!(vec!["example.mcp"], tool_ids(tools));
}

#[test]
fn registry_merges_multiple_registries() {
    let first = ToolRegistry::new(vec![Arc::new(EchoHandler::new("example.first"))]);
    let second = ToolRegistry::new(vec![Arc::new(EchoHandler::new("example.second"))]);

    let registry = ToolRegistry::merge(vec![first, second]).expect("registries should merge");
    let ids = tool_ids(registry.list(ToolAdapter::Mcp));

    assert_eq!(vec!["example.first", "example.second"], ids);
}

#[test]
fn registry_merge_rejects_duplicate_tool_ids() {
    let first = ToolRegistry::new(vec![Arc::new(EchoHandler::new("example.same"))]);
    let second = ToolRegistry::new(vec![Arc::new(EchoHandler::new("example.same"))]);

    let error = ToolRegistry::merge(vec![first, second]).expect_err("duplicates should fail");

    assert_eq!(vec!["example.same"], error.duplicate_tool_ids());
}

#[test]
fn registry_dispatches_calls_to_matching_handler() {
    let registry = ToolRegistry::new(vec![Arc::new(EchoHandler::new("example.echo"))]);

    let result = futures::executor::block_on(registry.call(
        "example.echo",
        json!({ "message": "hello" }),
        ToolContext::for_adapter(ToolAdapter::Mcp),
    ))
    .expect("tool call should succeed");

    assert_eq!(json!({ "message": "hello" }), result.structured_content);
}

#[test]
fn registry_reports_unknown_tool() {
    let registry = ToolRegistry::default();

    let error = futures::executor::block_on(registry.call(
        "missing.tool",
        json!({}),
        ToolContext::for_adapter(ToolAdapter::Mcp),
    ))
    .expect_err("unknown tool should fail");

    assert!(matches!(error, ToolError::UnknownTool { .. }));
}

fn tool_ids(tools: Vec<ToolDescriptor>) -> Vec<String> {
    tools.into_iter().map(|tool| tool.id).collect()
}

#[derive(Clone)]
struct EchoHandler {
    descriptor: ToolDescriptor,
}

impl EchoHandler {
    fn new(id: &str) -> Self {
        Self {
            descriptor: ToolDescriptor {
                id: id.to_string(),
                title: "Echo".to_string(),
                description: "Echo input".to_string(),
                input_schema: json!({ "type": "object" }),
                output_schema: json!({ "type": "object" }),
                permissions: Vec::new(),
                mode: ToolMode::Deterministic,
                adapters: vec![ToolAdapter::Mcp],
                annotations: ToolAnnotations::read_only("Echo"),
            },
        }
    }

    fn with_adapters(mut self, adapters: Vec<ToolAdapter>) -> Self {
        self.descriptor.adapters = adapters;
        self
    }
}

impl ToolHandler for EchoHandler {
    fn descriptor(&self) -> ToolDescriptor {
        self.descriptor.clone()
    }

    fn call(&self, input: serde_json::Value, _context: ToolContext) -> tool_runtime::ToolFuture {
        Box::pin(async move { Ok(ToolResult::structured(input)) })
    }
}
