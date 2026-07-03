mod config;
mod connection_sessions;
mod internal_functions;
mod redis;
mod session;
mod status;
mod tool_registry;

#[cfg(test)]
mod agent_db_registry_tests;

pub use config::{PublicMcpEnvOverride, PublicMcpStartConfig};
pub use session::{mcp_server_enabled, set_mcp_server_enabled, set_mcp_server_mode};
pub use status::PublicMcpRuntimeStatus;

#[cfg_attr(
    target_os = "windows",
    expect(dead_code, reason = "Windows 暂时禁用 CLI 入口，后续拆分独立 CLI")
)]
pub(crate) fn cli_tool_registry() -> anyhow::Result<tool_runtime::ToolRegistry> {
    let storage = one_core::storage::StorageManager::new()?;
    let repo = std::sync::Arc::new(one_core::storage::ConnectionRepository::new(
        storage.connection(),
    ));
    Ok(onetcli_runtime::tool_registry_with_version(
        repo,
        env!("CARGO_PKG_VERSION"),
    )?)
}

use gpui::{App, AsyncApp, Global, Subscription};
use one_core::gpui_tokio::Tokio;
use one_core::settings::AppSettings;
use public_mcp::approval::PublicMcpApprovalManager;
use public_mcp::discovery::{
    public_mcp_discovery_path, read_discovery, remove_discovery, write_discovery,
};
use public_mcp::runtime::PublicMcpRuntime;
use public_mcp::tools::InternalFunctionDefinition;
use std::path::PathBuf;
use tool_registry::build_tool_registry;

pub struct GlobalPublicMcpRuntime {
    runtime: Option<PublicMcpRuntime>,
    active_config: Option<PublicMcpStartConfig>,
    status: PublicMcpRuntimeStatus,
    generation: u64,
    session_enabled: bool,
    _settings_subscription: Subscription,
}

impl Global for GlobalPublicMcpRuntime {}

impl Drop for GlobalPublicMcpRuntime {
    fn drop(&mut self) {
        if self.runtime.is_some() {
            let _ = remove_discovery(&public_mcp_discovery_path());
            tracing::debug!("Public MCP runtime stopped");
        }
    }
}

pub fn init(cx: &mut App) {
    let discovery_path = public_mcp_discovery_path();
    let _ = remove_discovery(&discovery_path);
    ai_chat_view::set_plan_tool_registry_provider(cx, agent_runtime_tool_registry);
    internal_functions::ensure_registry(cx);
    for definition in internal_functions::builtin_definitions() {
        register_internal_function(cx, definition);
    }
    let settings_subscription = cx.observe_global::<AppSettings>(reconcile_runtime);
    cx.set_global(GlobalPublicMcpRuntime {
        runtime: None,
        active_config: None,
        status: PublicMcpRuntimeStatus::Disabled,
        generation: 0,
        session_enabled: false,
        _settings_subscription: settings_subscription,
    });
    reconcile_runtime(cx);
}

pub fn runtime_status(cx: &App) -> PublicMcpRuntimeStatus {
    cx.try_global::<GlobalPublicMcpRuntime>()
        .map(|state| {
            let client_count = state
                .runtime
                .as_ref()
                .map(PublicMcpRuntime::client_count)
                .unwrap_or_default();
            state.status.clone().with_client_count(client_count)
        })
        .unwrap_or_default()
}

pub fn register_internal_function(cx: &mut App, definition: InternalFunctionDefinition) {
    internal_functions::register_internal_function(cx, definition);
}

pub fn agent_runtime_tool_registry(cx: &mut App) -> anyhow::Result<agent_runtime::ToolRegistry> {
    let settings = AppSettings::current(cx);
    let session_enabled = session::runtime_session_enabled(cx);
    let config = PublicMcpStartConfig::from_settings_session_and_env(
        &settings,
        session_enabled,
        PublicMcpEnvOverride::from_env(),
    );
    let mut agent_toolsets = config.toolsets.clone();
    let agent_database_enabled = agent_toolsets.database;
    let agent_redis_enabled = agent_toolsets.redis;
    let agent_sftp_enabled = agent_toolsets.sftp;
    agent_toolsets.database = false;
    agent_toolsets.redis = false;
    agent_toolsets.sftp = false;
    let registry = build_tool_registry(cx, &agent_toolsets)?;
    let mut agent_registry = public_mcp::tools::agent_runtime_tool_registry(
        registry,
        config.permission_mode,
        build_approval_manager(cx),
    );
    if agent_database_enabled {
        if let Some(repo) = connection_repository(cx) {
            let runtime_db_registry = onetcli_runtime::database_tools::database_tool_registry(repo);
            let runtime_agent_db_registry = agent_runtime::tools::tool_runtime_agent_tool_registry(
                runtime_db_registry,
                tool_runtime::ToolAdapter::FunctionCalling,
            );
            agent_registry.extend(runtime_agent_db_registry);
        } else {
            tracing::warn!("Agent database tools enabled without ConnectionRepository");
        }
    }
    if agent_redis_enabled {
        register_runtime_redis_tools(cx, &mut agent_registry);
    }
    if agent_sftp_enabled {
        if let Some(repo) = connection_repository(cx) {
            register_runtime_sftp_tools(repo, &mut agent_registry);
        } else {
            tracing::warn!("Agent SSH/SFTP tools enabled without ConnectionRepository");
        }
    }
    Ok(agent_registry)
}

fn register_runtime_redis_tools(cx: &App, agent_registry: &mut agent_runtime::ToolRegistry) {
    let Some(repo) = connection_repository(cx) else {
        tracing::warn!("Agent Redis runtime tools enabled without ConnectionRepository");
        return;
    };
    let runtime_redis_registry = onetcli_runtime::redis_tools::redis_tool_registry(repo);
    let runtime_agent_redis_registry = agent_runtime::tools::tool_runtime_agent_tool_registry(
        runtime_redis_registry,
        tool_runtime::ToolAdapter::FunctionCalling,
    );
    agent_registry.extend(runtime_agent_redis_registry);
}

fn register_runtime_sftp_tools(
    repo: std::sync::Arc<one_core::storage::ConnectionRepository>,
    agent_registry: &mut agent_runtime::ToolRegistry,
) {
    let runtime_sftp_registry = onetcli_runtime::sftp_tools::sftp_tool_registry(repo);
    let runtime_agent_sftp_registry = agent_runtime::tools::tool_runtime_agent_tool_registry(
        runtime_sftp_registry,
        tool_runtime::ToolAdapter::FunctionCalling,
    );
    agent_registry.extend(runtime_agent_sftp_registry);
}

fn connection_repository(
    cx: &App,
) -> Option<std::sync::Arc<one_core::storage::ConnectionRepository>> {
    cx.try_global::<one_core::storage::GlobalStorageState>()?
        .storage
        .get::<one_core::storage::ConnectionRepository>()
}

fn reconcile_runtime(cx: &mut App) {
    let settings = AppSettings::current(cx);
    let session_enabled = session::runtime_session_enabled(cx);
    let config = PublicMcpStartConfig::from_settings_session_and_env(
        &settings,
        session_enabled,
        PublicMcpEnvOverride::from_env(),
    );
    if !config.enabled {
        stop_runtime(cx);
        return;
    }
    if update_active_runtime_without_restart(cx, &config) {
        return;
    }
    start_runtime(cx, config);
}

fn update_active_runtime_without_restart(cx: &mut App, config: &PublicMcpStartConfig) -> bool {
    let Some(state) = cx.try_global::<GlobalPublicMcpRuntime>() else {
        return false;
    };
    let Some(active_config) = state.active_config.as_ref() else {
        return false;
    };
    if active_config.requires_runtime_restart(config) || state.runtime.is_none() {
        return false;
    }

    let state = cx.global_mut::<GlobalPublicMcpRuntime>();
    if let Some(runtime) = state.runtime.as_ref() {
        runtime.set_permission_mode(config.permission_mode);
    }
    state.active_config = Some(config.clone());
    true
}

fn start_runtime(cx: &mut App, config: PublicMcpStartConfig) {
    let tool_registry = match build_tool_registry(cx, &config.toolsets) {
        Ok(registry) => registry,
        Err(error) => {
            let generation = next_generation(cx);
            record_runtime_failure(cx, generation, error.to_string());
            tracing::warn!(error = %error, "Invalid Public MCP tool registry");
            return;
        }
    };
    let approval_manager = build_approval_manager(cx);
    let generation = next_generation(cx);
    let task_config = config.clone();
    let staged_discovery_path = staged_discovery_path(generation);
    let task = Tokio::spawn_result(cx, async move {
        PublicMcpRuntime::start_with_tool_registry_and_approval(
            tool_registry,
            task_config.mode,
            task_config.permission_mode,
            staged_discovery_path,
            approval_manager,
        )
        .await
    });
    cx.spawn(async move |cx: &mut AsyncApp| {
        match task.await {
            Ok(runtime) => {
                let bind_addr = runtime.bind_addr();
                let canonical_discovery_path = public_mcp_discovery_path();
                let activated =
                    cx.update(move |cx| activate_runtime(cx, generation, config, runtime));
                if activated {
                    tracing::info!(
                        bind_addr = %bind_addr,
                        discovery_path = %canonical_discovery_path.display(),
                        "Public MCP runtime started"
                    );
                }
            }
            Err(error) => {
                let message = error.to_string();
                let _ = cx.update(move |cx| record_runtime_failure(cx, generation, message));
                tracing::warn!(error = %error, "Failed to start Public MCP runtime");
            }
        }
        Ok::<(), anyhow::Error>(())
    })
    .detach();
}

fn build_approval_manager(cx: &App) -> PublicMcpApprovalManager {
    crate::public_mcp_approval::approval_manager(cx)
}

fn next_generation(cx: &mut App) -> u64 {
    let state = cx.global_mut::<GlobalPublicMcpRuntime>();
    state.generation += 1;
    state.runtime = None;
    state.active_config = None;
    state.status = state.status.clone().starting(state.generation);
    let _ = remove_discovery(&public_mcp_discovery_path());
    state.generation
}

fn activate_runtime(
    cx: &mut App,
    generation: u64,
    config: PublicMcpStartConfig,
    runtime: PublicMcpRuntime,
) -> bool {
    let state = cx.global_mut::<GlobalPublicMcpRuntime>();
    if state.generation != generation {
        return false;
    }
    if let Err(error) = publish_runtime_discovery(&runtime) {
        tracing::warn!(error = %error, "Failed to publish Public MCP discovery");
        state
            .status
            .try_set_failed(generation, format!("Failed to publish discovery: {error}"));
        return false;
    }
    let bind_addr = runtime.bind_addr();
    let discovery_path = public_mcp_discovery_path();
    state.status =
        state
            .status
            .clone()
            .running(generation, bind_addr, config.mode, discovery_path, 0);
    state.active_config = Some(config);
    state.runtime = Some(runtime);
    true
}

fn record_runtime_failure(cx: &mut App, generation: u64, message: String) {
    let state = cx.global_mut::<GlobalPublicMcpRuntime>();
    state.status.try_set_failed(generation, message);
}

fn stop_runtime(cx: &mut App) {
    if !cx.has_global::<GlobalPublicMcpRuntime>() {
        return;
    }
    let state = cx.global_mut::<GlobalPublicMcpRuntime>();
    state.generation += 1;
    state.runtime = None;
    state.active_config = None;
    state.status = state.status.clone().disabled();
    let _ = remove_discovery(&public_mcp_discovery_path());
}

fn publish_runtime_discovery(runtime: &PublicMcpRuntime) -> anyhow::Result<()> {
    let document = read_discovery(runtime.discovery_path())?;
    write_discovery(&public_mcp_discovery_path(), &document)?;
    Ok(())
}

fn staged_discovery_path(generation: u64) -> PathBuf {
    public_mcp_discovery_path().with_file_name(format!("public-mcp-{generation}.staging.json"))
}

#[cfg(test)]
mod tests {
    use super::{
        GlobalPublicMcpRuntime, PublicMcpRuntimeStatus, next_generation, staged_discovery_path,
    };
    use gpui::{Subscription, TestAppContext};
    use public_mcp::discovery::public_mcp_discovery_path;
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn staged_discovery_path_stays_next_to_canonical_discovery() {
        let canonical = public_mcp_discovery_path();
        let staged = staged_discovery_path(42);

        assert_ne!(canonical, staged);
        assert_eq!(canonical.parent(), staged.parent());
        assert_eq!(
            Some("public-mcp-42.staging.json"),
            staged.file_name().and_then(|name| name.to_str())
        );
    }

    #[gpui::test]
    fn runtime_status_updates_notify_global_observers(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(GlobalPublicMcpRuntime {
                runtime: None,
                active_config: None,
                status: PublicMcpRuntimeStatus::Disabled,
                generation: 0,
                session_enabled: false,
                _settings_subscription: Subscription::new(|| {}),
            });
        });

        let notifications = Rc::new(Cell::new(0));
        let observer_notifications = notifications.clone();
        let _subscription = cx.update(|cx| {
            cx.observe_global::<GlobalPublicMcpRuntime>(move |_| {
                observer_notifications.set(observer_notifications.get() + 1);
            })
        });
        cx.run_until_parked();
        notifications.set(0);

        cx.update(|cx| {
            next_generation(cx);
        });
        cx.run_until_parked();

        assert_eq!(1, notifications.get());
    }
}
