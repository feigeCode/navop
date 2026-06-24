pub mod cli_host;
pub mod connections;

pub fn builtin_tool_registry() -> tool_runtime::ToolRegistry {
    builtin_tool_registry_with_version(env!("CARGO_PKG_VERSION"))
}

pub fn builtin_tool_registry_with_version(version: &'static str) -> tool_runtime::ToolRegistry {
    tool_runtime::ToolRegistry::new(vec![std::sync::Arc::new(AppInfoTool { version })])
}

pub fn tool_registry(
    repo: std::sync::Arc<one_core::storage::ConnectionRepository>,
) -> Result<tool_runtime::ToolRegistry, tool_runtime::ToolRegistryError> {
    tool_registry_with_version(repo, env!("CARGO_PKG_VERSION"))
}

pub fn tool_registry_with_version(
    repo: std::sync::Arc<one_core::storage::ConnectionRepository>,
    version: &'static str,
) -> Result<tool_runtime::ToolRegistry, tool_runtime::ToolRegistryError> {
    tool_runtime::ToolRegistry::merge(vec![
        builtin_tool_registry_with_version(version),
        connections::connection_tool_registry(repo),
    ])
}

#[derive(Clone)]
struct AppInfoTool {
    version: &'static str,
}

impl tool_runtime::ToolHandler for AppInfoTool {
    fn descriptor(&self) -> tool_runtime::ToolDescriptor {
        tool_runtime::ToolDescriptor {
            id: "onetcli.app_info".to_string(),
            title: "App Info".to_string(),
            description: "Read OnetCli app metadata.".to_string(),
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
                tool_runtime::ToolAdapter::Mcp,
                tool_runtime::ToolAdapter::FunctionCalling,
                tool_runtime::ToolAdapter::Cli,
            ],
            annotations: tool_runtime::ToolAnnotations::read_only("App Info"),
        }
    }

    fn call(
        &self,
        _input: serde_json::Value,
        _context: tool_runtime::ToolContext,
    ) -> tool_runtime::ToolFuture {
        let version = self.version;
        Box::pin(async move {
            Ok(tool_runtime::ToolResult::structured(serde_json::json!({
                "name": "onetcli",
                "version": version
            })))
        })
    }
}
