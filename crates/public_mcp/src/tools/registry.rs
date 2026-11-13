use super::{
    PublicMcpToolContext, PublicMcpToolProvider, ToolRuntimeMcpProvider, remote_ops_tool_registry,
    terminal_control_tool_registry, terminal_exec_tool_registry, terminal_read_tool_registry,
    terminal_write_command_tool_registry, terminal_write_keys_tool_registry,
};
use crate::registry::PublicMcpRegistry;
use agent_runtime::tools::{ToolName, ToolNameAllocator};
use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, JsonObject, Tool},
};
use std::{collections::BTreeSet, error::Error, fmt, sync::Arc};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicMcpToolRegistryError {
    duplicate_tool_names: Vec<String>,
    duplicate_runtime_tool_ids: Vec<String>,
}

impl PublicMcpToolRegistryError {
    pub fn duplicate_tool_names(&self) -> Vec<String> {
        self.duplicate_tool_names.clone()
    }

    pub fn duplicate_runtime_tool_ids(&self) -> Vec<String> {
        self.duplicate_runtime_tool_ids.clone()
    }
}

impl fmt::Display for PublicMcpToolRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.duplicate_tool_names.is_empty() {
            return write!(
                formatter,
                "duplicate public MCP tool names: {}",
                self.duplicate_tool_names.join(", ")
            );
        }
        write!(
            formatter,
            "duplicate terminal runtime tool ids: {}",
            self.duplicate_runtime_tool_ids.join(", ")
        )
    }
}

impl Error for PublicMcpToolRegistryError {}

impl From<tool_runtime::ToolRegistryError> for PublicMcpToolRegistryError {
    fn from(error: tool_runtime::ToolRegistryError) -> Self {
        Self {
            duplicate_tool_names: Vec::new(),
            duplicate_runtime_tool_ids: error.duplicate_tool_ids(),
        }
    }
}

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
                duplicate_runtime_tool_ids: Vec::new(),
            });
        }
        Ok(Self {
            providers: Arc::new(providers),
        })
    }

    pub fn terminal(registry: PublicMcpRegistry) -> Result<Self, PublicMcpToolRegistryError> {
        Self::from_runtime_registries(vec![
            remote_ops_tool_registry(registry.clone()),
            terminal_read_tool_registry(registry.clone()),
            terminal_exec_tool_registry(registry.clone()),
            terminal_control_tool_registry(registry.clone()),
            terminal_write_command_tool_registry(registry.clone()),
            terminal_write_keys_tool_registry(registry),
        ])
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

    /// 面向 MCP 客户端的工具清单：名字已换成函数调用安全的形式。
    ///
    /// Grok 等 OpenAI 兼容客户端只接受 `[A-Za-z0-9_-]`，而 Navop 工具 id 用 `.`
    /// 分层（`ssh.command.poll`）；带点的名字会被客户端在注册阶段整批丢弃，于是
    /// 「连接成功了却查不到任何工具」（issue #194）。
    ///
    /// 这里复用内置 agent 路径（[`super::agent_runtime_tool_registry`]）同一套
    /// 规则、同一个「先按内部 id 排序」的顺序，因此同一个工具在 ACP 与 MCP 两端
    /// 拿到同一个名字，冲突消解也可复现。
    pub fn client_tools(&self) -> Vec<Tool> {
        let mut tools = self.tools();
        tools.sort_by(|left, right| left.name.cmp(&right.name));
        let mut allocator = ToolNameAllocator::default();
        for tool in tools.iter_mut() {
            tool.name = allocator
                .allocate(tool.name.as_ref())
                .as_str()
                .to_string()
                .into();
        }
        tools
    }

    /// 按客户端工具名取工具；返回对象的 `name` 与 [`Self::client_tools`] 一致。
    pub fn client_tool(&self, name: &str) -> Option<Tool> {
        let canonical = self.resolve_client_tool_name(name)?;
        let client_name = self
            .client_tool_names()
            .into_iter()
            .find(|(candidate, _)| candidate == &canonical)?
            .1;
        let mut tool = self.get_tool(&canonical)?;
        tool.name = client_name.as_str().to_string().into();
        Some(tool)
    }

    /// 把客户端送来的名字解析回内部工具 id。
    ///
    /// 先按客户端名匹配，再回退到内部 id：仍允许 `.` 的客户端（Claude 等）可以
    /// 照旧用 `ssh.command.poll` 调用，不必跟着改名。
    pub fn resolve_client_tool_name(&self, name: &str) -> Option<String> {
        let names = self.client_tool_names();
        names
            .iter()
            .find(|(_, client)| client.as_str() == name)
            .or_else(|| names.iter().find(|(canonical, _)| canonical == name))
            .map(|(canonical, _)| canonical.clone())
    }

    /// 内部工具 id 与客户端函数名的对照表，按内部 id 排序。
    fn client_tool_names(&self) -> Vec<(String, ToolName)> {
        let mut canonical_names: Vec<String> = self
            .tools()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect();
        canonical_names.sort();
        let mut allocator = ToolNameAllocator::default();
        canonical_names
            .into_iter()
            .map(|canonical| {
                let client_name = allocator.allocate(&canonical);
                (canonical, client_name)
            })
            .collect()
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

    pub async fn call_tool_with_cancellation(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
        context: PublicMcpToolContext,
        cancellation: CancellationToken,
    ) -> Result<CallToolResult, McpError> {
        for provider in self.providers.iter() {
            if let Some(result) = provider.call_tool_with_cancellation(
                name,
                arguments.clone(),
                context.clone(),
                cancellation.clone(),
            ) {
                return result.await;
            }
        }
        Err(McpError::invalid_params(
            format!("unknown public MCP tool: {name}"),
            None,
        ))
    }

    fn from_runtime_registries(
        registries: Vec<tool_runtime::ToolRegistry>,
    ) -> Result<Self, PublicMcpToolRegistryError> {
        let runtime_registry = tool_runtime::ToolRegistry::merge(registries)?;
        Self::try_new(vec![Arc::new(ToolRuntimeMcpProvider::new(
            runtime_registry,
        ))])
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::PublicMcpToolFuture;

    /// 只提供静态工具清单的 stub；本模块的用例只关心名字，不关心执行。
    struct StaticToolProvider {
        tools: Vec<Tool>,
    }

    impl StaticToolProvider {
        fn new(names: &[&str]) -> Self {
            Self {
                tools: names
                    .iter()
                    .map(|name| Tool::new(name.to_string(), "stub tool", JsonObject::new()))
                    .collect(),
            }
        }
    }

    impl PublicMcpToolProvider for StaticToolProvider {
        fn tools(&self) -> Vec<Tool> {
            self.tools.clone()
        }

        fn call_tool(
            &self,
            _name: &str,
            _arguments: Option<JsonObject>,
            _context: PublicMcpToolContext,
        ) -> Option<PublicMcpToolFuture> {
            None
        }
    }

    fn client_names(registry: &PublicMcpToolRegistry) -> Vec<String> {
        registry
            .client_tools()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect()
    }

    #[test]
    fn terminal_runtime_duplicates_return_error_instead_of_panicking() {
        let registry = PublicMcpRegistry::default();
        let error = match PublicMcpToolRegistry::from_runtime_registries(vec![
            remote_ops_tool_registry(registry.clone()),
            remote_ops_tool_registry(registry),
        ]) {
            Ok(_) => panic!("duplicate terminal runtime tools must fail closed"),
            Err(error) => error,
        };

        assert!(!error.duplicate_runtime_tool_ids().is_empty());
        assert!(
            error
                .to_string()
                .contains("duplicate terminal runtime tool ids")
        );
    }

    #[test]
    fn client_tools_sanitize_dotted_tool_ids() {
        let registry = PublicMcpToolRegistry::new(vec![Arc::new(StaticToolProvider::new(&[
            "ssh.command.poll",
            "terminal.write_keys",
            "ssh.exec",
        ]))]);

        assert_eq!(
            vec![
                "ssh_command_poll".to_string(),
                "ssh_exec".to_string(),
                "terminal_write_keys".to_string(),
            ],
            client_names(&registry)
        );
    }

    #[test]
    fn client_tool_names_disambiguate_sanitized_collisions() {
        // `sample.echo` 与 `sample_echo` 净化后同名，必须各自拿到唯一的名字。
        let registry = PublicMcpToolRegistry::new(vec![Arc::new(StaticToolProvider::new(&[
            "sample_echo",
            "sample.echo",
        ]))]);

        // 对照表按内部 id 排序，`.`(0x2E) 在 `_`(0x5F) 之前 ⇒ 带点的先占基础名。
        assert_eq!(
            vec!["sample_echo".to_string(), "sample_echo_2".to_string()],
            client_names(&registry)
        );
        assert_eq!(
            Some("sample.echo".to_string()),
            registry.resolve_client_tool_name("sample_echo")
        );
        assert_eq!(
            Some("sample_echo".to_string()),
            registry.resolve_client_tool_name("sample_echo_2")
        );
    }

    #[test]
    fn resolve_client_tool_name_accepts_the_internal_id_too() {
        let registry = PublicMcpToolRegistry::new(vec![Arc::new(StaticToolProvider::new(&[
            "ssh.command.poll",
        ]))]);

        // 仍允许 `.` 的客户端照旧用内部 id 调用。
        assert_eq!(
            Some("ssh.command.poll".to_string()),
            registry.resolve_client_tool_name("ssh.command.poll")
        );
        assert_eq!(
            Some("ssh.command.poll".to_string()),
            registry.resolve_client_tool_name("ssh_command_poll")
        );
        assert_eq!(
            None,
            registry.resolve_client_tool_name("ssh_command_unknown")
        );
    }

    #[test]
    fn client_tool_reports_the_sanitized_name() {
        let registry = PublicMcpToolRegistry::new(vec![Arc::new(StaticToolProvider::new(&[
            "ssh.command.poll",
        ]))]);

        let tool = registry
            .client_tool("ssh_command_poll")
            .expect("client name should resolve");
        assert_eq!("ssh_command_poll", tool.name);

        // 内部 id 也能取到，但返回的仍是客户端侧的名字，与 tools/list 保持一致。
        let tool = registry
            .client_tool("ssh.command.poll")
            .expect("internal id should resolve");
        assert_eq!("ssh_command_poll", tool.name);

        assert!(registry.client_tool("ssh_command_unknown").is_none());
    }
}
