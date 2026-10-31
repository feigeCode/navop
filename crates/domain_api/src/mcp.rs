//! 公共 MCP 宿主的领域工具集装配契约。
//!
//! 应用外壳（`navop_app`）不感知任何领域 crate：它把需要装配的工具集开关交给本契约，
//! 由各领域包（`shell_domain` / `data_domain`）返回自己的 `ToolRegistry`。

use gpui::App;
use std::sync::{Arc, Mutex, OnceLock};
use tool_runtime::ToolRegistry;

/// 需要装配的运行时工具集开关。
#[derive(Clone, Copy, Debug, Default)]
pub struct RuntimeMcpSelection {
    pub database: bool,
    pub redis: bool,
    pub mongo: bool,
    pub terminal: bool,
    pub terminal_ssh_exec: bool,
    pub terminal_exec: bool,
    pub terminal_control: bool,
    pub terminal_read: bool,
    pub terminal_write_keys: bool,
    pub sftp: bool,
}

/// 领域包对公共 MCP 运行时提供的工具集。
pub trait RuntimeMcpProviders: Send + Sync {
    /// 提供方标识，用于避免重复注册（进程内注册是幂等的）。
    fn provider_id(&self) -> &'static str {
        "runtime-mcp"
    }

    /// 按选择装配本领域的工具集；无关字段由实现方忽略。
    fn runtime_mcp_registries(
        &self,
        cx: &mut App,
        selection: &RuntimeMcpSelection,
    ) -> Vec<ToolRegistry> {
        let _ = (cx, selection);
        Vec::new()
    }
}

fn providers() -> &'static Mutex<Vec<Arc<dyn RuntimeMcpProviders>>> {
    static PROVIDERS: OnceLock<Mutex<Vec<Arc<dyn RuntimeMcpProviders>>>> = OnceLock::new();
    PROVIDERS.get_or_init(Default::default)
}

/// 由领域包在 `Domain::register` 中调用。
pub fn register_runtime_mcp_provider(provider: Arc<dyn RuntimeMcpProviders>) {
    let mut providers = providers()
        .lock()
        .expect("runtime mcp providers mutex poisoned");
    if providers
        .iter()
        .any(|existing| existing.provider_id() == provider.provider_id())
    {
        return;
    }
    providers.push(provider);
}

/// 汇总所有领域包的工具集。
pub fn runtime_mcp_registries(cx: &mut App, selection: &RuntimeMcpSelection) -> Vec<ToolRegistry> {
    let providers = providers()
        .lock()
        .expect("runtime mcp providers mutex poisoned")
        .clone();
    providers
        .iter()
        .flat_map(|provider| provider.runtime_mcp_registries(cx, selection))
        .collect()
}
