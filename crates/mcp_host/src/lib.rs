//! 三个产品共用的 Public MCP 宿主。
//!
//! 领域 crate 只注册“本域提供哪些工具”，产品身份、发现文件、server 生命周期
//! 与工具集开关都由这里统一处理。

pub mod config;

use domain_api::AppIdentity;
use gpui::{App, AsyncApp, Global};
use one_core::gpui_tokio::Tokio;
use one_core::settings::{AppSettings, ToolExposureToolsetSettings};
use public_mcp::discovery::public_mcp_discovery_path;
use public_mcp::runtime::PublicMcpRuntime;
use public_mcp::tools::{PublicMcpToolProvider, PublicMcpToolRegistry, ToolRuntimeMcpProvider};
use std::sync::{Arc, Mutex, OnceLock};

pub use config::{PublicMcpEnvOverride, PublicMcpStartConfig};

/// 一个领域可以提供的工具集。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpToolset {
    Terminal,
    Sftp,
    Database,
    Redis,
    Mongo,
    Connections,
}

type ProviderFactory = Arc<dyn Fn(&App) -> Vec<Arc<dyn PublicMcpToolProvider>> + Send + Sync>;

struct Registration {
    toolset: McpToolset,
    factory: ProviderFactory,
}

static REGISTRATIONS: OnceLock<Mutex<Vec<Registration>>> = OnceLock::new();
static IDENTITY: OnceLock<&'static AppIdentity> = OnceLock::new();

fn registrations() -> &'static Mutex<Vec<Registration>> {
    REGISTRATIONS.get_or_init(|| Mutex::new(Vec::new()))
}

/// 注册一个领域的工具集工厂。
pub fn register_toolset(toolset: McpToolset, factory: ProviderFactory) {
    if let Ok(mut entries) = registrations().lock() {
        if entries.iter().any(|entry| entry.toolset == toolset) {
            return;
        }
        entries.push(Registration { toolset, factory });
    }
}

/// 已注册的工具集，供状态页与测试使用。
pub fn registered_toolsets() -> Vec<McpToolset> {
    registrations()
        .lock()
        .map(|entries| entries.iter().map(|entry| entry.toolset).collect())
        .unwrap_or_default()
}

/// 当前进程的连接仓库；独立产品的工具工厂都从这里取连接。
pub fn connection_repository(cx: &App) -> Option<Arc<one_core::storage::ConnectionRepository>> {
    cx.try_global::<one_core::storage::GlobalStorageState>()?
        .storage
        .get::<one_core::storage::ConnectionRepository>()
}

pub fn workspace_repository(cx: &App) -> Option<Arc<one_core::storage::WorkspaceRepository>> {
    cx.try_global::<one_core::storage::GlobalStorageState>()?
        .storage
        .get::<one_core::storage::WorkspaceRepository>()
}

/// 把任意 `ToolRegistry` 包装成 Public MCP provider。
pub fn runtime_provider(registry: tool_runtime::ToolRegistry) -> Arc<dyn PublicMcpToolProvider> {
    Arc::new(ToolRuntimeMcpProvider::new(registry))
}

#[derive(Default)]
struct HostState {
    runtime: Option<PublicMcpRuntime>,
    last_error: Option<String>,
    starting: bool,
}

struct GlobalMcpHost(HostState);

impl Global for GlobalMcpHost {}

/// 按产品设置启动（或停止）本进程的 MCP server。
///
/// 必须在产品的 `init` 之后调用：领域工具工厂依赖终端/Redis/Mongo 等全局状态。
pub fn start(cx: &mut App, identity: &'static AppIdentity) -> anyhow::Result<()> {
    let _ = IDENTITY.set(identity);
    let settings = AppSettings::current(cx);
    let config = PublicMcpStartConfig::from_settings_session_and_env(
        &settings,
        false,
        PublicMcpEnvOverride::from_env(),
    );
    if !config.enabled {
        stop(cx);
        return Ok(());
    }
    if is_running(cx) || is_starting(cx) {
        return Ok(());
    }
    ensure_global(cx);
    cx.global_mut::<GlobalMcpHost>().0.starting = true;

    let Some(registry) = build_registry(cx, &config.toolsets) else {
        set_error(cx, "no MCP toolsets registered".to_string());
        return Ok(());
    };
    let discovery_path = public_mcp_discovery_path();
    let mode = config.mode;
    let permission_mode = config.permission_mode;
    let approval_manager = mcp_approval::approval_manager(cx);
    let task = Tokio::spawn_result(cx, async move {
        PublicMcpRuntime::start_with_tool_registry_and_approval(
            registry,
            mode,
            permission_mode,
            discovery_path,
            approval_manager,
        )
        .await
    });
    ensure_global(cx);
    cx.spawn(async move |cx: &mut AsyncApp| {
        match task.await {
            Ok(runtime) => {
                let bind_addr = runtime.bind_addr();
                let path = runtime.discovery_path().clone();
                cx.update(move |cx| {
                    ensure_global(cx);
                    let state = &mut cx.global_mut::<GlobalMcpHost>().0;
                    state.runtime = Some(runtime);
                    state.last_error = None;
                    state.starting = false;
                    tracing::info!(
                        app = identity.mcp_app_name,
                        bind_addr = %bind_addr,
                        discovery_path = %path.display(),
                        "public MCP runtime started"
                    );
                });
            }
            Err(error) => {
                let message = error.to_string();
                cx.update(move |cx| set_error(cx, message));
            }
        }
        Ok::<(), anyhow::Error>(())
    })
    .detach();
    Ok(())
}

/// 打开/关闭当前产品的 Public MCP；设置持久化后立即应用。
pub fn set_enabled(cx: &mut App, enabled: bool) -> anyhow::Result<()> {
    AppSettings::update_and_save(cx, |settings| {
        settings.mcp.server_enabled = enabled;
    });
    let Some(identity) = IDENTITY.get().copied() else {
        return Ok(());
    };
    start(cx, identity)
}

/// 设置里的 MCP 开关，用于状态展示。
pub fn settings_enabled(cx: &App) -> bool {
    AppSettings::current(cx).mcp.server_enabled
}

/// 首次启动独立产品时，把本产品的核心工具集写入设置。
/// 之后用户在设置里关掉的工具集不会被再次打开。
pub fn enable_default_toolsets(cx: &mut App, toolsets: &[McpToolset]) {
    if AppSettings::current(cx).mcp.toolsets_initialized {
        return;
    }
    AppSettings::update_and_save(cx, |settings| {
        for toolset in toolsets {
            set_toolset_fields(&mut settings.tool_exposure.mcp, *toolset, true);
        }
        settings.mcp.toolsets_initialized = true;
    });
}

/// 读取当前设置里的工具集开关。
pub fn current_toolset_enabled(cx: &App, toolset: McpToolset) -> bool {
    toolset_enabled(toolset, &AppSettings::current(cx).tool_exposure.mcp)
}

/// 修改单个工具集并重启 server。
pub fn set_toolset_enabled(
    cx: &mut App,
    toolset: McpToolset,
    enabled: bool,
) -> anyhow::Result<()> {
    AppSettings::update_and_save(cx, |settings| {
        set_toolset_fields(&mut settings.tool_exposure.mcp, toolset, enabled);
    });
    let Some(identity) = IDENTITY.get().copied() else {
        return Ok(());
    };
    start(cx, identity)
}

fn set_toolset_fields(toolsets: &mut ToolExposureToolsetSettings, toolset: McpToolset, enabled: bool) {
    match toolset {
        McpToolset::Terminal => {
            toolsets.terminal = enabled;
            toolsets.terminal_ssh_exec = enabled;
            toolsets.terminal_exec = enabled;
        }
        McpToolset::Sftp => toolsets.sftp = enabled,
        McpToolset::Database => toolsets.database = enabled,
        McpToolset::Redis => toolsets.redis = enabled,
        McpToolset::Mongo => toolsets.mongo = enabled,
        McpToolset::Connections => toolsets.connections = enabled,
    }
}

/// 停止 server；发现文件由 `PublicMcpRuntime::Drop` 清理。
pub fn stop(cx: &mut App) {
    ensure_global(cx);
    let state = &mut cx.global_mut::<GlobalMcpHost>().0;
    state.runtime = None;
    state.starting = false;
}

fn is_starting(cx: &App) -> bool {
    cx.try_global::<GlobalMcpHost>()
        .map(|state| state.0.starting)
        .unwrap_or(false)
}

pub fn is_running(cx: &App) -> bool {
    cx.try_global::<GlobalMcpHost>()
        .and_then(|state| state.0.runtime.as_ref())
        .is_some()
}

pub fn client_count(cx: &App) -> usize {
    cx.try_global::<GlobalMcpHost>()
        .and_then(|state| state.0.runtime.as_ref())
        .map(PublicMcpRuntime::client_count)
        .unwrap_or(0)
}

pub fn last_error(cx: &App) -> Option<String> {
    cx.try_global::<GlobalMcpHost>()
        .and_then(|state| state.0.last_error.clone())
}

fn ensure_global(cx: &mut App) {
    if !cx.has_global::<GlobalMcpHost>() {
        cx.set_global(GlobalMcpHost(HostState::default()));
    }
}

fn set_error(cx: &mut App, message: String) {
    ensure_global(cx);
    let state = &mut cx.global_mut::<GlobalMcpHost>().0;
    tracing::warn!(error = %message, "public MCP runtime failed");
    state.last_error = Some(message);
    state.starting = false;
}

fn build_registry(
    cx: &App,
    toolsets: &ToolExposureToolsetSettings,
) -> Option<PublicMcpToolRegistry> {
    let mut providers = Vec::new();
    if let Ok(entries) = registrations().lock() {
        for entry in entries.iter() {
            if toolset_enabled(entry.toolset, toolsets) {
                providers.extend((entry.factory)(cx));
            }
        }
    }
    if providers.is_empty() {
        return None;
    }
    let registry = PublicMcpToolRegistry::try_new(providers).ok()?;
    tracing::info!(
        tools = registry.tools().len(),
        "public MCP tool registry assembled"
    );
    Some(registry)
}

pub fn toolset_enabled(toolset: McpToolset, toolsets: &ToolExposureToolsetSettings) -> bool {
    match toolset {
        McpToolset::Terminal => {
            toolsets.terminal || toolsets.terminal_ssh_exec || toolsets.terminal_exec
        }
        McpToolset::Sftp => toolsets.sftp,
        McpToolset::Database => toolsets.database,
        McpToolset::Redis => toolsets.redis,
        McpToolset::Mongo => toolsets.mongo,
        McpToolset::Connections => toolsets.connections,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_toolset_matches_any_terminal_flag() {
        let mut toolsets = ToolExposureToolsetSettings::public_mcp_default();
        toolsets.terminal = false;
        toolsets.terminal_ssh_exec = true;
        toolsets.terminal_exec = false;
        assert!(toolset_enabled(McpToolset::Terminal, &toolsets));
        toolsets.terminal_ssh_exec = false;
        assert!(!toolset_enabled(McpToolset::Terminal, &toolsets));
    }

    #[test]
    fn data_toolsets_are_independent() {
        let mut toolsets = ToolExposureToolsetSettings::public_mcp_default();
        assert!(!toolset_enabled(McpToolset::Database, &toolsets));
        toolsets.database = true;
        assert!(toolset_enabled(McpToolset::Database, &toolsets));
        assert!(!toolset_enabled(McpToolset::Sftp, &toolsets));
    }
}
