use super::{
    PublicMcpToolContext, PublicMcpToolProvider, ToolRuntimeMcpProvider, remote_ops_tool_registry,
    terminal_exec_tool_registry,
};
use crate::registry::PublicMcpRegistry;
use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, JsonObject, Tool},
};
use std::{collections::BTreeSet, error::Error, fmt, sync::Arc};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicMcpToolRegistryError {
    duplicate_tool_names: Vec<String>,
}

impl PublicMcpToolRegistryError {
    pub fn duplicate_tool_names(&self) -> Vec<String> {
        self.duplicate_tool_names.clone()
    }
}

impl fmt::Display for PublicMcpToolRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "duplicate public MCP tool names: {}",
            self.duplicate_tool_names.join(", ")
        )
    }
}

impl Error for PublicMcpToolRegistryError {}

#[derive(Clone, Default)]
pub struct PublicMcpToolRegistry {
    providers: Arc<Vec<Arc<dyn PublicMcpToolProvider>>>,
}

impl PublicMcpToolRegistry {
    pub fn new(providers: Vec<Arc<dyn PublicMcpToolProvider>>) -> Self {
        Self::try_new(providers).expect("public MCP tool names must be unique")
    }

    pub fn try_new(
        providers: Vec<Arc<dyn PublicMcpToolProvider>>,
    ) -> Result<Self, PublicMcpToolRegistryError> {
        let duplicate_tool_names = duplicate_tool_names(&providers);
        if !duplicate_tool_names.is_empty() {
            return Err(PublicMcpToolRegistryError {
                duplicate_tool_names,
            });
        }
        Ok(Self {
            providers: Arc::new(providers),
        })
    }

    pub fn terminal(registry: PublicMcpRegistry) -> Self {
        let runtime_registry = tool_runtime::ToolRegistry::merge(vec![
            remote_ops_tool_registry(registry.clone()),
            terminal_exec_tool_registry(registry),
        ])
        .expect("terminal runtime tool names must be unique");
        Self::new(vec![Arc::new(ToolRuntimeMcpProvider::new(
            runtime_registry,
        ))])
    }

    pub fn tools(&self) -> Vec<Tool> {
        self.providers
            .iter()
            .flat_map(|provider| provider.tools())
            .collect()
    }

    pub fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tools().into_iter().find(|tool| tool.name == name)
    }

    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
        context: PublicMcpToolContext,
    ) -> Result<CallToolResult, McpError> {
        for provider in self.providers.iter() {
            if let Some(result) = provider.call_tool(name, arguments.clone(), context.clone()) {
                return result.await;
            }
        }
        Err(McpError::invalid_params(
            format!("unknown public MCP tool: {name}"),
            None,
        ))
    }
}

fn duplicate_tool_names(providers: &[Arc<dyn PublicMcpToolProvider>]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut duplicates = BTreeSet::new();
    for provider in providers {
        for tool in provider.tools() {
            if !seen.insert(tool.name.to_string()) {
                duplicates.insert(tool.name.to_string());
            }
        }
    }
    duplicates.into_iter().collect()
}
