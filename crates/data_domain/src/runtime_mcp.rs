//! 数据领域向公共 MCP 运行时提供的工具集。

use domain_api::{RuntimeMcpProviders, RuntimeMcpSelection};
use gpui::App;
use one_core::storage::{ConnectionRepository, GlobalStorageState};
use tool_runtime::ToolRegistry;

pub struct DataRuntimeMcp;

fn connection_repository(cx: &App) -> Option<std::sync::Arc<ConnectionRepository>> {
    cx.try_global::<GlobalStorageState>()?
        .storage
        .get::<ConnectionRepository>()
}

impl RuntimeMcpProviders for DataRuntimeMcp {
    fn provider_id(&self) -> &'static str {
        "data-runtime-mcp"
    }

    fn runtime_mcp_registries(
        &self,
        cx: &mut App,
        selection: &RuntimeMcpSelection,
    ) -> Vec<ToolRegistry> {
        let mut registries = Vec::new();
        if selection.database {
            if let Some(repo) = connection_repository(cx) {
                registries.push(navop_runtime::database_tools::database_tool_registry(repo));
            } else {
                tracing::warn!("Public MCP database tools enabled without ConnectionRepository");
            }
        }
        if selection.redis {
            registries.push(ToolRegistry::new(crate::mcp::redis_tool_handlers(cx)));
        }
        if selection.mongo {
            registries.push(ToolRegistry::new(crate::mcp::mongo_tool_handlers(cx)));
        }
        registries
    }
}
