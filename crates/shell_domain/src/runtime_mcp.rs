//! Shell 领域向公共 MCP 运行时提供的工具集。

use domain_api::{RuntimeMcpProviders, RuntimeMcpSelection};
use gpui::App;
use one_core::storage::{ConnectionRepository, GlobalStorageState};
use public_mcp::tools::{
    remote_ops_tool_registry, terminal_control_tool_registry, terminal_exec_tool_registry,
    terminal_read_tool_registry, terminal_write_keys_tool_registry,
};
use tool_runtime::ToolRegistry;

pub struct ShellRuntimeMcp;

fn connection_repository(cx: &App) -> Option<std::sync::Arc<ConnectionRepository>> {
    cx.try_global::<GlobalStorageState>()?
        .storage
        .get::<ConnectionRepository>()
}

impl RuntimeMcpProviders for ShellRuntimeMcp {
    fn provider_id(&self) -> &'static str {
        "shell-runtime-mcp"
    }

    fn runtime_mcp_registries(
        &self,
        cx: &mut App,
        selection: &RuntimeMcpSelection,
    ) -> Vec<ToolRegistry> {
        let mut registries = Vec::new();
        if selection.sftp {
            if let Some(repo) = connection_repository(cx) {
                registries.push(navop_runtime::sftp_tools::sftp_tool_registry(repo));
            } else {
                tracing::warn!("Public MCP SFTP tools enabled without ConnectionRepository");
            }
        }
        if selection.terminal {
            match terminal_view::public_mcp::registry(cx) {
                Some(registry) => {
                    if selection.terminal_ssh_exec {
                        registries.push(remote_ops_tool_registry(registry.clone()));
                    }
                    if selection.terminal_exec {
                        registries.push(terminal_exec_tool_registry(registry.clone()));
                        registries.push(terminal_read_tool_registry(registry.clone()));
                        registries.push(terminal_control_tool_registry(registry.clone()));
                        registries.push(terminal_write_keys_tool_registry(registry));
                    }
                }
                None => tracing::warn!("Public MCP terminal registry is not initialized"),
            }
        }
        registries
    }
}
