mod config;
mod connection_sessions;
mod diagnostics;
mod internal_functions;
mod mongo;
mod redis;
mod resource_pool;
mod session;
mod status;
mod tool_registry;

#[cfg(test)]
mod agent_db_registry_tests;

pub use config::{PublicMcpEnvOverride, PublicMcpStartConfig};
pub use session::{mcp_server_enabled, set_mcp_server_enabled, set_mcp_server_mode};
pub use status::PublicMcpRuntimeStatus;

use agent_runtime::ToolExecutionMode;
use gpui::{App, AsyncApp, Global, Subscription};
use one_core::gpui_tokio::Tokio;
use one_core::settings::{AppSettings, McpPermissionMode};
use public_mcp::approval::PublicMcpApprovalManager;
use public_mcp::discovery::{
    legacy_public_mcp_discovery_path, public_mcp_discovery_path, read_discovery, remove_discovery,
    write_discovery,
};
use public_mcp::runtime::PublicMcpRuntime;
use public_mcp::tools::InternalFunctionDefinition;
use rust_i18n::t;
use std::path::PathBuf;
use tool_registry::{build_agent_tool_registry, build_tool_registry};

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
            let _ = remove_discovery(&legacy_public_mcp_discovery_path());
            tracing::debug!("Public MCP runtime stopped");
        }
    }
}

pub fn init(cx: &mut App) {
    let discovery_path = public_mcp_discovery_path();
    let _ = remove_discovery(&discovery_path);
    let _ = remove_discovery(&legacy_public_mcp_discovery_path());
    ai_chat_view::set_plan_tool_registry_provider(cx, agent_runtime_tool_registry);
    ai_chat_view::set_acp_tool_mode_provider(
        cx,
        |cx| {
            let settings = AppSettings::current(cx);
            let config = PublicMcpStartConfig::from_settings_session_and_env(
                &settings,
                session::runtime_session_enabled(cx),
                PublicMcpEnvOverride::from_env(),
            );
            Some(tool_execution_mode_for_permission(config.permission_mode))
        },
        |cx, mode| {
            if let Some(override_mode) = PublicMcpEnvOverride::from_env().permission_mode {
                anyhow::bail!(
                    "{}",
                    t!(
                        "Settings.General.Mcp.acp_permission_env_override",
                        mode = format!("{override_mode:?}")
                    )
                );
            }
            let permission_mode = mcp_permission_mode_for_tool_execution(mode);
            AppSettings::update_and_save(cx, |settings| {
                settings.mcp.permission_mode = permission_mode;
            });
            Ok(())
        },
    );
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

fn tool_execution_mode_for_permission(
    mode: public_mcp::permissions::PermissionMode,
) -> ToolExecutionMode {
    match mode {
        public_mcp::permissions::PermissionMode::Deny => ToolExecutionMode::ReadOnly,
        public_mcp::permissions::PermissionMode::Ask => ToolExecutionMode::Manual,
        public_mcp::permissions::PermissionMode::Allow => ToolExecutionMode::Auto,
    }
}

fn mcp_permission_mode_for_tool_execution(mode: ToolExecutionMode) -> McpPermissionMode {
    match mode {
        ToolExecutionMode::ReadOnly => McpPermissionMode::Deny,
        ToolExecutionMode::Manual => McpPermissionMode::Ask,
        ToolExecutionMode::Auto => McpPermissionMode::Allow,
    }
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
    let mut agent_toolsets = settings.tool_exposure.agent.clone();
    let agent_database_enabled = agent_toolsets.database;
    let agent_redis_enabled = agent_toolsets.redis;
    let agent_sftp_enabled = agent_toolsets.sftp;
    agent_toolsets.database = false;
    agent_toolsets.redis = false;
    agent_toolsets.sftp = false;
    let registry = build_agent_tool_registry(cx, &agent_toolsets)?;
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
    let _ = remove_discovery(&legacy_public_mcp_discovery_path());
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
    let _ = remove_discovery(&legacy_public_mcp_discovery_path());
}

fn publish_runtime_discovery(runtime: &PublicMcpRuntime) -> anyhow::Result<()> {
    publish_discovery_documents(
        runtime.discovery_path(),
        &public_mcp_discovery_path(),
        &legacy_public_mcp_discovery_path(),
    )
}

fn publish_discovery_documents(
    source: &std::path::Path,
    canonical: &std::path::Path,
    legacy: &std::path::Path,
) -> anyhow::Result<()> {
    let document = read_discovery(source)?;
    write_discovery(canonical, &document)?;
    write_discovery(legacy, &document.legacy_compatible())?;
    Ok(())
}

fn staged_discovery_path(generation: u64) -> PathBuf {
    public_mcp_discovery_path().with_file_name(format!("public-mcp-{generation}.staging.json"))
}

#[cfg(test)]
mod tests {
    use super::{
        GlobalPublicMcpRuntime, PublicMcpRuntimeStatus, mcp_permission_mode_for_tool_execution,
        next_generation, publish_discovery_documents, staged_discovery_path,
        tool_execution_mode_for_permission,
    };
    use agent_runtime::ToolExecutionMode;
    use gpui::{Subscription, TestAppContext};
    use one_core::settings::McpPermissionMode;
    use public_mcp::discovery::public_mcp_discovery_path;
    use public_mcp::permissions::PermissionMode;
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

    #[test]
    fn publishing_discovery_keeps_legacy_installed_helpers_working() {
        use public_mcp::discovery::{
            DiscoveryDocument, PublicMcpMode, read_discovery, write_discovery,
        };
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};

        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("staged.json");
        let canonical = dir.path().join("navop/public-mcp.json");
        let legacy = dir.path().join("onetcli/public-mcp.json");
        let document = DiscoveryDocument::new(
            1,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 49152),
            "a".repeat(64),
            PublicMcpMode::Persistent,
        );
        write_discovery(&source, &document).unwrap();

        publish_discovery_documents(&source, &canonical, &legacy).unwrap();

        let current = read_discovery(&canonical).unwrap();
        let old = read_discovery(&legacy).unwrap();
        assert_eq!("navop", current.app);
        assert_eq!("onetcli", old.app);
        assert_eq!(current.port, old.port);
        assert_eq!(current.token, old.token);
    }

    #[test]
    fn acp_tool_modes_round_trip_to_mcp_permission_profiles() {
        assert_eq!(
            ToolExecutionMode::ReadOnly,
            tool_execution_mode_for_permission(PermissionMode::Deny)
        );
        assert_eq!(
            ToolExecutionMode::Manual,
            tool_execution_mode_for_permission(PermissionMode::Ask)
        );
        assert_eq!(
            ToolExecutionMode::Auto,
            tool_execution_mode_for_permission(PermissionMode::Allow)
        );

        assert_eq!(
            McpPermissionMode::Deny,
            mcp_permission_mode_for_tool_execution(ToolExecutionMode::ReadOnly)
        );
        assert_eq!(
            McpPermissionMode::Ask,
            mcp_permission_mode_for_tool_execution(ToolExecutionMode::Manual)
        );
        assert_eq!(
            McpPermissionMode::Allow,
            mcp_permission_mode_for_tool_execution(ToolExecutionMode::Auto)
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
