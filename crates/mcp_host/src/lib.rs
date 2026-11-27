//! 三个产品共用的 Public MCP 宿主。
//!
//! 领域 crate 只注册“本域提供哪些工具”，产品身份、发现文件、server 生命周期
//! 与工具集开关都由这里统一处理。
//!
//! 全量产品由 `navop_app::public_mcp_runtime` 托管 server；独立产品（navop-shell /
//! navop-data）走这里。两条实现共用 `PublicMcpStartConfig`，并且都会观察
//! `AppSettings`：设置页改了工具集、权限模式或服务器模式之后立即收敛，不需要重启 App。

pub mod config;

use domain_api::AppIdentity;
use gpui::{App, AsyncApp, Global, Subscription};
use one_core::gpui_tokio::Tokio;
use one_core::settings::{AppSettings, ToolExposureToolsetSettings};
use public_mcp::discovery::{PublicMcpMode, public_mcp_discovery_path};
use public_mcp::runtime::PublicMcpRuntime;
use public_mcp::tools::{PublicMcpToolProvider, PublicMcpToolRegistry, ToolRuntimeMcpProvider};
use std::net::SocketAddr;
use std::path::PathBuf;
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
    /// 扩展声明的 AI 工具（plugin-ai-tools.md，默认关闭）。
    Extensions,
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
    /// server 当前实际生效的配置，用于判断设置变化是否需要重启。
    active_config: Option<PublicMcpStartConfig>,
    /// Temporary 模式下的会话级开关；Persistent 模式读 `mcp.server_enabled`。
    session_enabled: bool,
    last_error: Option<String>,
    starting: bool,
    /// 观察 `AppSettings`，让设置页的修改无需重启 App 就能应用。
    settings_subscription: Option<Subscription>,
}

struct GlobalMcpHost(HostState);

impl Global for GlobalMcpHost {}

/// 独立产品的 MCP server 状态。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum McpHostStatus {
    Disabled,
    Starting,
    Running {
        bind_addr: SocketAddr,
        mode: PublicMcpMode,
        discovery_path: PathBuf,
        client_count: usize,
    },
    Failed {
        message: String,
    },
}

/// 启动（或在已启动时收敛）本进程的 MCP server。
///
/// 必须在该产品的 `init` 之后调用：领域工具工厂依赖终端/Redis/Mongo 等全局状态。
/// 重复调用是安全的：配置没变时保持现状，配置变了才重启 server。
pub fn start(cx: &mut App, identity: &'static AppIdentity) -> anyhow::Result<()> {
    let _ = IDENTITY.set(identity);
    ensure_global(cx);
    ensure_settings_observer(cx);
    reconcile(cx);
    Ok(())
}

/// 安装 `AppSettings` 观察者：MCP 设置页改完即生效，不必重启 App。
pub fn ensure_settings_observer(cx: &mut App) {
    ensure_global(cx);
    if cx
        .global::<GlobalMcpHost>()
        .0
        .settings_subscription
        .is_some()
    {
        return;
    }
    let subscription = cx.observe_global::<AppSettings>(reconcile);
    cx.global_mut::<GlobalMcpHost>().0.settings_subscription = Some(subscription);
}

/// 按当前设置收敛 server：该停就停、该重启就重启、只改权限模式时原地生效。
fn reconcile(cx: &mut App) {
    ensure_global(cx);
    let session_enabled = cx.global::<GlobalMcpHost>().0.session_enabled;
    let settings = AppSettings::current(cx);
    let config = PublicMcpStartConfig::from_settings_session_and_env(
        &settings,
        session_enabled,
        PublicMcpEnvOverride::from_env(),
    );
    if cx.global::<GlobalMcpHost>().0.starting {
        // 启动流程结束后会再收敛一次，这里不必排队。
        return;
    }
    if !config.enabled {
        stop(cx);
        return;
    }
    if apply_config_without_restart(cx, &config) {
        return;
    }
    // 先释放旧 runtime（同时清理发现文件），再启动新的，避免两个 server 抢同一个发现文件。
    stop(cx);
    spawn_start(cx, config);
}

/// 只改了权限模式时不必重启 server，直接下发新的审批策略。
fn apply_config_without_restart(cx: &mut App, config: &PublicMcpStartConfig) -> bool {
    let Some(state) = cx.try_global::<GlobalMcpHost>() else {
        return false;
    };
    let Some(active) = state.0.active_config.as_ref() else {
        return false;
    };
    if state.0.runtime.is_none() || active.requires_runtime_restart(config) {
        return false;
    }
    if let Some(runtime) = state.0.runtime.as_ref() {
        runtime.set_permission_mode(config.permission_mode);
    }
    cx.global_mut::<GlobalMcpHost>().0.active_config = Some(config.clone());
    true
}

fn spawn_start(cx: &mut App, config: PublicMcpStartConfig) {
    ensure_global(cx);
    let Some(identity) = IDENTITY.get().copied() else {
        set_error(cx, "MCP host identity is not configured".to_string());
        return;
    };
    let Some(registry) = build_registry(cx, &config.toolsets) else {
        set_error(cx, "no MCP toolsets registered".to_string());
        return;
    };
    let discovery_path = public_mcp_discovery_path();
    let mode = config.mode;
    let permission_mode = config.permission_mode;
    let approval_manager = mcp_approval::approval_manager(cx);
    cx.global_mut::<GlobalMcpHost>().0.starting = true;
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
    cx.spawn(async move |cx: &mut AsyncApp| {
        let result = task.await;
        cx.update(move |cx| finish_start(cx, identity, config, result));
        Ok::<(), anyhow::Error>(())
    })
    .detach();
}

fn finish_start(
    cx: &mut App,
    identity: &'static AppIdentity,
    config: PublicMcpStartConfig,
    result: anyhow::Result<PublicMcpRuntime>,
) {
    ensure_global(cx);
    match result {
        Ok(runtime) => {
            let bind_addr = runtime.bind_addr();
            let path = runtime.discovery_path().clone();
            let state = &mut cx.global_mut::<GlobalMcpHost>().0;
            state.runtime = Some(runtime);
            state.active_config = Some(config);
            state.last_error = None;
            state.starting = false;
            tracing::info!(
                app = identity.mcp_app_name,
                bind_addr = %bind_addr,
                discovery_path = %path.display(),
                "public MCP runtime started"
            );
        }
        Err(error) => {
            // 启动失败不自动重试，等下一次设置变更再试，避免失败重试风暴。
            set_error(cx, error.to_string());
            return;
        }
    }
    // 启动期间设置可能又变了（例如刚打开开关就改权限模式），按最新设置再收敛一次。
    reconcile(cx);
}

/// 打开/关闭当前产品的 Public MCP；按 `Temporary` / `Persistent` 语义写入设置并立即应用。
pub fn set_enabled(cx: &mut App, enabled: bool) -> anyhow::Result<()> {
    ensure_global(cx);
    let mut settings = AppSettings::current(cx);
    let mut session_enabled = cx.global::<GlobalMcpHost>().0.session_enabled;
    config::apply_server_enabled_for_mode(&mut settings, &mut session_enabled, enabled);
    apply_mcp_settings(cx, settings, session_enabled);
    Ok(())
}

/// 切换 `Temporary` / `Persistent`，保留当前的开关状态。
pub fn set_server_mode(
    cx: &mut App,
    mode: one_core::settings::McpServerMode,
) -> anyhow::Result<()> {
    ensure_global(cx);
    let mut settings = AppSettings::current(cx);
    let mut session_enabled = cx.global::<GlobalMcpHost>().0.session_enabled;
    config::apply_server_mode_preserving_enabled(&mut settings, &mut session_enabled, mode);
    apply_mcp_settings(cx, settings, session_enabled);
    Ok(())
}

fn apply_mcp_settings(cx: &mut App, settings: AppSettings, session_enabled: bool) {
    ensure_global(cx);
    cx.global_mut::<GlobalMcpHost>().0.session_enabled = session_enabled;
    settings.save();
    cx.set_global(settings);
    // `set_global` 已经触发观察者；这里再显式收敛一次，保证观察者还没装上时也生效。
    ensure_settings_observer(cx);
    reconcile(cx);
}

/// 当前的 MCP 开关（Temporary 模式返回会话级开关，Persistent 模式返回持久化开关）。
pub fn settings_enabled(cx: &App) -> bool {
    server_enabled(cx)
}

/// 当前是否允许其它客户端调用。
pub fn server_enabled(cx: &App) -> bool {
    let settings = AppSettings::current(cx);
    let session_enabled = cx
        .try_global::<GlobalMcpHost>()
        .map(|state| state.0.session_enabled)
        .unwrap_or(false);
    config::effective_server_enabled(&settings, session_enabled)
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
pub fn set_toolset_enabled(cx: &mut App, toolset: McpToolset, enabled: bool) -> anyhow::Result<()> {
    AppSettings::update_and_save(cx, |settings| {
        set_toolset_fields(&mut settings.tool_exposure.mcp, toolset, enabled);
    });
    ensure_settings_observer(cx);
    reconcile(cx);
    Ok(())
}

fn set_toolset_fields(
    toolsets: &mut ToolExposureToolsetSettings,
    toolset: McpToolset,
    enabled: bool,
) {
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
        McpToolset::Extensions => toolsets.extensions = enabled,
    }
}

/// 停止 server；发现文件由 `PublicMcpRuntime::Drop` 清理。
pub fn stop(cx: &mut App) {
    ensure_global(cx);
    let state = &mut cx.global_mut::<GlobalMcpHost>().0;
    state.runtime = None;
    state.active_config = None;
    state.last_error = None;
    state.starting = false;
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

/// 供设置页展示的 server 状态。
pub fn status(cx: &App) -> McpHostStatus {
    let Some(state) = cx.try_global::<GlobalMcpHost>() else {
        return McpHostStatus::Disabled;
    };
    let state = &state.0;
    if state.starting {
        return McpHostStatus::Starting;
    }
    if let Some(runtime) = state.runtime.as_ref() {
        let mode = state
            .active_config
            .as_ref()
            .map(|config| config.mode)
            .unwrap_or(PublicMcpMode::Temporary);
        return McpHostStatus::Running {
            bind_addr: runtime.bind_addr(),
            mode,
            discovery_path: runtime.discovery_path().clone(),
            client_count: runtime.client_count(),
        };
    }
    match state.last_error.as_ref() {
        Some(message) => McpHostStatus::Failed {
            message: message.clone(),
        },
        None => McpHostStatus::Disabled,
    }
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
        McpToolset::Extensions => toolsets.extensions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_core::settings::McpServerMode;

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

    #[test]
    fn temporary_mode_toggles_session_instead_of_persisted_flag() {
        let mut settings = AppSettings::default();
        assert_eq!(settings.mcp.server_mode, McpServerMode::Temporary);
        let mut session_enabled = false;

        config::apply_server_enabled_for_mode(&mut settings, &mut session_enabled, true);

        assert!(session_enabled);
        assert!(!settings.mcp.server_enabled);
        assert!(config::effective_server_enabled(&settings, session_enabled));
        assert!(!config::effective_server_enabled(&settings, false));
    }

    #[test]
    fn restart_is_only_needed_when_enabled_mode_or_toolsets_change() {
        let mut settings = AppSettings::default();
        settings.mcp.server_mode = McpServerMode::Persistent;
        settings.mcp.server_enabled = true;
        settings.mcp.permission_mode = one_core::settings::McpPermissionMode::Deny;
        let active =
            PublicMcpStartConfig::from_settings_and_env(&settings, PublicMcpEnvOverride::default());

        let mut next_settings = settings.clone();
        next_settings.mcp.permission_mode = one_core::settings::McpPermissionMode::Allow;
        let next = PublicMcpStartConfig::from_settings_and_env(
            &next_settings,
            PublicMcpEnvOverride::default(),
        );
        assert!(!active.requires_runtime_restart(&next));

        let mut toolset_settings = settings.clone();
        toolset_settings.tool_exposure.mcp.notes = !settings.tool_exposure.mcp.notes;
        let next = PublicMcpStartConfig::from_settings_and_env(
            &toolset_settings,
            PublicMcpEnvOverride::default(),
        );
        assert!(active.requires_runtime_restart(&next));
    }
}

#[cfg(test)]
mod host_tests {
    use super::*;
    use gpui::TestAppContext;
    use one_core::settings::McpServerMode;

    // `observe_global` 的注册是延迟生效的（要等一次 effect flush），所以每一步都
    // 用独立的 `cx.update`，模拟真实事件循环里的“下一帧”。
    #[gpui::test]
    fn settings_changes_reconcile_the_host_without_restarting(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let mut settings = AppSettings::default();
            settings.mcp.server_mode = McpServerMode::Persistent;
            settings.mcp.server_enabled = false;
            cx.set_global(settings);
            start(cx, &AppIdentity::FULL).expect("start should succeed");
            assert_eq!(status(cx), McpHostStatus::Disabled);
        });

        // 打开开关：设置观察者必须立刻收敛。本测试进程没有注册任何领域工具集，
        // 所以这里走的是失败分支 —— 但已经证明 reconcile 真的被调用了。
        cx.update(|cx| {
            AppSettings::update_and_save(cx, |settings| settings.mcp.server_enabled = true);
        });
        cx.update(|cx| {
            assert!(server_enabled(cx));
            assert!(
                matches!(status(cx), McpHostStatus::Failed { .. }),
                "status = {:?}",
                status(cx)
            );
        });

        // 关掉开关同样立即生效，不需要重启 App。
        cx.update(|cx| {
            AppSettings::update_and_save(cx, |settings| settings.mcp.server_enabled = false);
        });
        cx.update(|cx| assert_eq!(status(cx), McpHostStatus::Disabled));
    }

    #[gpui::test]
    fn temporary_mode_toggle_keeps_the_persisted_switch_off(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(AppSettings::default());
            start(cx, &AppIdentity::FULL).expect("start should succeed");
            assert!(!server_enabled(cx));
            set_enabled(cx, true).expect("enable");
            assert!(server_enabled(cx));
            // Temporary 模式不写持久化开关。
            assert!(!AppSettings::current(cx).mcp.server_enabled);
            assert!(
                matches!(status(cx), McpHostStatus::Failed { .. }),
                "status = {:?}",
                status(cx)
            );
            set_enabled(cx, false).expect("disable");
            assert!(!server_enabled(cx));
            assert_eq!(status(cx), McpHostStatus::Disabled);
        });
    }
}
