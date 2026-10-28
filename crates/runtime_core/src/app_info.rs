//! 应用元信息工具；三产品共用，不依赖任何连接域。

use tool_runtime::{
    ToolAdapter, ToolAnnotations, ToolContext, ToolDescriptor, ToolFuture, ToolHandler,
    ToolRegistry, ToolResult,
};

pub fn builtin_tool_registry() -> ToolRegistry {
    builtin_tool_registry_with_version(env!("CARGO_PKG_VERSION"))
}

pub fn builtin_tool_registry_with_version(version: &'static str) -> ToolRegistry {
    ToolRegistry::new(vec![std::sync::Arc::new(AppInfoTool { version })])
}

#[derive(Clone)]
struct AppInfoTool {
    version: &'static str,
}

impl ToolHandler for AppInfoTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            id: "onetcli.app_info".to_string(),
            title: "App Info".to_string(),
            description: "Read metadata for the running Navop application, including app name and version. Use this for version checks, diagnostics, or compatibility reporting; it does not inspect remote hosts or saved connections.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
            output_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "version": { "type": "string" }
                },
                "required": ["name", "version"]
            }),
            permissions: Vec::new(),
            mode: tool_runtime::ToolMode::Deterministic,
            adapters: vec![
                ToolAdapter::Mcp,
                ToolAdapter::FunctionCalling,
            ],
            annotations: ToolAnnotations::read_only("App Info"),
        }
    }

    fn call(&self, _input: serde_json::Value, _context: ToolContext) -> ToolFuture {
        let version = self.version;
        Box::pin(async move {
            Ok(ToolResult::structured(serde_json::json!({
                "name": "Navop",
                "version": version
            })))
        })
    }
}
