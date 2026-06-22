use serde_json::json;
use std::sync::Arc;
use tool_runtime::{
    ToolAdapter, ToolAnnotations, ToolContext, ToolDescriptor, ToolHandler, ToolMode, ToolRegistry,
    ToolResult,
};

pub(super) fn registry() -> ToolRegistry {
    ToolRegistry::new(vec![Arc::new(AppInfoTool)])
}

#[derive(Clone)]
struct AppInfoTool;

impl ToolHandler for AppInfoTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            id: "onetcli.app_info".to_string(),
            title: "App Info".to_string(),
            description: "Read OnetCli app metadata.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
            output_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "version": { "type": "string" }
                },
                "required": ["name", "version"]
            }),
            permissions: Vec::new(),
            mode: ToolMode::Deterministic,
            adapters: vec![ToolAdapter::Mcp],
            annotations: ToolAnnotations::read_only("App Info"),
        }
    }

    fn call(&self, _input: serde_json::Value, _context: ToolContext) -> tool_runtime::ToolFuture {
        Box::pin(async {
            Ok(ToolResult::structured(json!({
                "name": "onetcli",
                "version": env!("CARGO_PKG_VERSION")
            })))
        })
    }
}
