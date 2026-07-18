use one_core::settings::ToolExposureToolsetSettings;
use serde_json::json;
use std::sync::Arc;
use tool_runtime::{
    ToolAdapter, ToolAnnotations, ToolContext, ToolDescriptor, ToolHandler, ToolMode, ToolResult,
};

const RUNTIME_STATUS_TOOL: &str = "navop.runtime_status";

pub(super) fn runtime_status_tool_registry(
    toolsets: &ToolExposureToolsetSettings,
) -> tool_runtime::ToolRegistry {
    tool_runtime::ToolRegistry::new(vec![Arc::new(RuntimeStatusTool {
        toolsets: toolsets.clone(),
    })])
}

#[derive(Clone)]
struct RuntimeStatusTool {
    toolsets: ToolExposureToolsetSettings,
}

impl ToolHandler for RuntimeStatusTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            id: RUNTIME_STATUS_TOOL.to_string(),
            title: "Navop runtime status".to_string(),
            description: "Read the running Navop application's public MCP compatibility, permission mode, and authoritative Tool Exposure group states. Use tools/list for the actual available tool names and schemas; do not infer tools from the CLI package or Skill.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
            output_schema: json!({ "type": "object" }),
            permissions: Vec::new(),
            mode: ToolMode::Deterministic,
            adapters: vec![ToolAdapter::Mcp, ToolAdapter::FunctionCalling],
            annotations: ToolAnnotations::read_only("Navop runtime status"),
        }
    }

    fn call(&self, _input: serde_json::Value, _context: ToolContext) -> tool_runtime::ToolFuture {
        let toolsets = self.toolsets.clone();
        Box::pin(async move {
            Ok(ToolResult::structured(json!({
                "contractVersion": 1,
                "app": {
                    "name": "Navop",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "toolDiscovery": {
                    "source": "tools/list",
                    "schemaSource": "tools/list",
                    "dynamic": true
                },
                "settingsPath": "Settings > General > Tool Exposure",
                "toolGroups": [
                    group("terminal", toolsets.terminal),
                    group("ssh", toolsets.terminal && toolsets.terminal_ssh_exec),
                    group("terminal_exec", toolsets.terminal && toolsets.terminal_exec),
                    group("connections", toolsets.connections),
                    group("sftp", toolsets.sftp),
                    group("database", toolsets.database),
                    group("redis", toolsets.redis),
                    group("mongodb", toolsets.mongo),
                    group("internal_functions", toolsets.internal_functions)
                ]
            })))
        })
    }
}

fn group(id: &str, enabled: bool) -> serde_json::Value {
    json!({ "id": id, "enabled": enabled })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn runtime_status_reports_authoritative_exposure_and_discovery_contract() {
        let mut toolsets = ToolExposureToolsetSettings::public_mcp_default();
        toolsets.redis = true;
        let registry = runtime_status_tool_registry(&toolsets);
        let result = registry
            .call(
                RUNTIME_STATUS_TOOL,
                json!({}),
                ToolContext::for_adapter(ToolAdapter::Mcp),
            )
            .await
            .expect("runtime status should succeed")
            .structured_content;

        assert_eq!(Some(1), result["contractVersion"].as_u64());
        assert_eq!(
            Some("tools/list"),
            result["toolDiscovery"]["source"].as_str()
        );
        assert!(result["toolDiscovery"]["dynamic"].as_bool().unwrap());
        assert!(
            result["toolGroups"]
                .as_array()
                .unwrap()
                .iter()
                .any(|group| group == &json!({ "id": "redis", "enabled": true }))
        );
        assert!(
            result["toolGroups"]
                .as_array()
                .unwrap()
                .iter()
                .any(|group| group == &json!({ "id": "mongodb", "enabled": false }))
        );
    }
}
