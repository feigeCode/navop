use super::{connections, internal_functions, redis, tool_runtime_tools};
use gpui::App;
use one_core::settings::McpToolsetSettings;
use public_mcp::tools::{
    PublicMcpToolProvider, PublicMcpToolRegistry, ToolRuntimeMcpProvider,
    internal_function_tool_registry, remote_ops_tool_registry,
};
use std::sync::Arc;

pub(super) fn build_tool_registry(
    cx: &App,
    toolsets: &McpToolsetSettings,
) -> anyhow::Result<PublicMcpToolRegistry> {
    let mut providers: Vec<Arc<dyn PublicMcpToolProvider>> = Vec::new();
    if toolsets.terminal {
        if let Some(registry) = terminal_view::public_mcp::registry(cx) {
            providers.push(Arc::new(ToolRuntimeMcpProvider::new(
                remote_ops_tool_registry(registry),
            )));
        } else {
            tracing::warn!("Public MCP terminal registry is not initialized");
        }
    }
    if toolsets.internal_functions {
        providers.push(Arc::new(ToolRuntimeMcpProvider::new(
            tool_runtime_tools::registry(),
        )));
        providers.push(Arc::new(ToolRuntimeMcpProvider::new(
            internal_function_tool_registry(internal_functions::definitions(cx)),
        )));
    }
    if toolsets.database {
        if let Some(storage) = cx.try_global::<one_core::storage::GlobalStorageState>() {
            if let Some(repo) = storage
                .storage
                .get::<one_core::storage::ConnectionRepository>()
            {
                providers.push(Arc::new(ToolRuntimeMcpProvider::new(
                    connections::connection_tool_registry(repo),
                )));
            } else {
                tracing::warn!("Public MCP connection tools enabled without ConnectionRepository");
            }
        } else {
            tracing::warn!("Public MCP connection tools enabled before storage is initialized");
        }
    }
    if toolsets.redis {
        providers.push(Arc::new(ToolRuntimeMcpProvider::new(
            tool_runtime::ToolRegistry::new(vec![Arc::new(redis::redis_tool_provider(cx))]),
        )));
    }
    if providers.is_empty() {
        tracing::warn!("Public MCP runtime enabled without any tool providers");
    }
    Ok(PublicMcpToolRegistry::try_new(providers)?)
}

#[cfg(test)]
mod tests {
    use super::build_tool_registry;
    use crate::public_mcp_runtime::register_internal_function;
    use gpui::TestAppContext;
    use one_core::settings::McpToolsetSettings;
    use public_mcp::permissions::PermissionMode;
    use public_mcp::tools::{InternalFunctionDefinition, PublicMcpToolContext};
    use serde_json::json;

    #[gpui::test]
    fn build_tool_registry_includes_internal_function_tools(cx: &mut TestAppContext) {
        let toolsets = internal_function_toolsets();

        let tools = cx.update(|cx| {
            build_tool_registry(cx, &toolsets)
                .expect("internal function registry should build")
                .tools()
        });

        assert!(
            tools
                .iter()
                .any(|tool| tool.name == "public_mcp.internal_functions.list")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool.name == "public_mcp.internal_functions.call")
        );
    }

    #[gpui::test]
    fn build_tool_registry_includes_redis_tools(cx: &mut TestAppContext) {
        let toolsets = McpToolsetSettings {
            terminal: false,
            redis: true,
            ..Default::default()
        };

        let tools = cx.update(|cx| {
            build_tool_registry(cx, &toolsets)
                .expect("redis registry should build")
                .tools()
        });

        assert!(
            tools
                .iter()
                .any(|tool| tool.name == "public_mcp.redis.list_connections")
        );
    }

    #[gpui::test]
    fn build_tool_registry_uses_registered_internal_functions(cx: &mut TestAppContext) {
        let toolsets = internal_function_toolsets();
        let registry = cx.update(|cx| {
            register_internal_function(cx, runtime_status_function());
            build_tool_registry(cx, &toolsets).expect("internal function registry should build")
        });

        let result = futures::executor::block_on(registry.call_tool(
            "public_mcp.internal_functions.list",
            None,
            PublicMcpToolContext {
                permission_mode: PermissionMode::Deny,
                approver: Default::default(),
            },
        ))
        .expect("list tool should run");

        assert_eq!(
            Some(json!({
                "functions": [{
                    "name": "onetcli.runtime_status",
                    "description": "Read the public MCP runtime status.",
                    "read_only": true,
                    "input_schema": {
                        "type": "object"
                    }
                }]
            })),
            result.structured_content
        );
    }

    #[gpui::test]
    fn build_tool_registry_exposes_tool_runtime_app_info(cx: &mut TestAppContext) {
        let toolsets = internal_function_toolsets();
        let registry = cx.update(|cx| {
            build_tool_registry(cx, &toolsets).expect("tool runtime registry should build")
        });

        let tools = registry.tools();
        assert!(tools.iter().any(|tool| tool.name == "onetcli.app_info"));

        let result = futures::executor::block_on(registry.call_tool(
            "onetcli.app_info",
            None,
            PublicMcpToolContext {
                permission_mode: PermissionMode::Deny,
                approver: Default::default(),
            },
        ))
        .expect("app info tool should run");

        assert_eq!(
            Some(json!({
                "name": "onetcli",
                "version": env!("CARGO_PKG_VERSION")
            })),
            result.structured_content
        );
    }

    fn internal_function_toolsets() -> McpToolsetSettings {
        McpToolsetSettings {
            terminal: false,
            internal_functions: true,
            ..Default::default()
        }
    }

    fn runtime_status_function() -> InternalFunctionDefinition {
        InternalFunctionDefinition::read_only(
            "onetcli.runtime_status",
            "Read the public MCP runtime status.",
            |_| async { Ok(json!({ "state": "disabled" })) },
        )
    }
}
