//! 扩展声明工具（`contributes.tools`）的执行与注册。
//!
//! 设计文档：docs/extension-resource-plugins/plugin-ai-tools.md。
//! 一个扩展工具在 agent 对话里以 `ext_<ns>__<tool_id>` 暴露，执行时
//! 走 provider 的 resource/open + resource/invoke，与扩展页面同一通道。

pub(crate) mod handler;
pub(crate) mod connections;
pub mod env;
mod error;
mod result_decode;
mod tool;

pub use connections::{ConnectionSource, ConnectionStore, RepositoryConnectionSource};
pub use error::{ExtensionToolErrorCode, PROVIDER_ERROR_MAX_CHARS};
pub use tool::{ExtensionTool, public_tool_name};
pub use handler::extension_mcp_tool_registry;

use std::collections::BTreeSet;
use std::sync::Arc;

use agent_runtime::tools::{ToolNameAllocator, ToolRegistry};
use extension_runtime::RegisteredToolContribution;

use crate::universal_plugins::UniversalPluginService;

/// catalog 快照 → 注册表构建结果。
pub struct ExtensionToolRegistrySnapshot {
    pub registry: ToolRegistry,
    /// 公开名 → (extension_id, tool_id)。
    pub index: ExtensionToolIndex,
}

#[derive(Clone, Debug, Default)]
pub struct ExtensionToolIndex {
    entries: Vec<IndexEntry>,
}

#[derive(Clone, Debug)]
struct IndexEntry {
    public_name: String,
    extension_id: String,
    tool_id: String,
}

impl ExtensionToolIndex {
    pub fn lookup(&self, public_name: &str) -> Option<(&str, &str)> {
        self.entries
            .iter()
            .find(|entry| entry.public_name == public_name)
            .map(|entry| (entry.extension_id.as_str(), entry.tool_id.as_str()))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn public_names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|entry| entry.public_name.as_str())
    }
}

/// 从 catalog 快照构建扩展工具注册表。
///
/// - 跳过被 `disabled_extension_tools`（扩展 id）禁用的扩展；
/// - 没有声明工具或 shellApp 缺失的扩展不参与；
/// - 公开名 `ext_<ns>__<id>`，冲突时由 allocator 加后缀。
pub fn build_extension_tool_registry(
    service: &UniversalPluginService,
    connections: Option<ConnectionStore>,
    disabled_extensions: &BTreeSet<String>,
) -> ExtensionToolRegistrySnapshot {
    let catalog = service.catalog_snapshot();
    let mut allocator = ToolNameAllocator::default();
    let mut registry = ToolRegistry::new();
    let mut index = ExtensionToolIndex::default();

    for contribution in catalog.tools() {
        let contribution: RegisteredToolContribution = contribution.clone();
        if disabled_extensions.contains(&contribution.extension_id) {
            continue;
        }
        // 声明了工具但没有 shellApp 的扩展在安装期就会被拒；这里防御性跳过。
        if catalog.shell_app(&contribution.extension_id).is_none() {
            tracing::warn!(
                target: "extension_tools",
                extension = %contribution.extension_id,
                tool = %contribution.id,
                "工具所在扩展没有 shellApp，跳过"
            );
            continue;
        }
        let base = tool::public_tool_name(&contribution.extension_id, &contribution.id);
        let public_name = allocator.allocate(base.as_str());
        let entry = IndexEntry {
            public_name: public_name.as_str().to_string(),
            extension_id: contribution.extension_id.clone(),
            tool_id: contribution.id.clone(),
        };
        let extension_tool = ExtensionTool::new(
            service.clone(),
            connections.clone(),
            contribution,
            public_name,
        );
        registry.register(Arc::new(extension_tool));
        index.entries.push(entry);
    }

    ExtensionToolRegistrySnapshot { registry, index }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_looks_up_by_public_name() {
        let mut index = ExtensionToolIndex::default();
        index.entries.push(IndexEntry {
            public_name: "ext_docker__containers_list".to_string(),
            extension_id: "com.navop.docker".to_string(),
            tool_id: "containers.list".to_string(),
        });
        assert_eq!(
            index.lookup("ext_docker__containers_list"),
            Some(("com.navop.docker", "containers.list"))
        );
        assert_eq!(index.lookup("ext_missing__nope"), None);
        assert_eq!(index.len(), 1);
    }
}
