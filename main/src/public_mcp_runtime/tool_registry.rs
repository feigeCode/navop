use super::{internal_functions, redis, resource_pool};
use gpui::App;
use one_core::settings::ToolExposureToolsetSettings;
use public_mcp::tools::{
    PublicMcpToolProvider, PublicMcpToolRegistry, ToolRuntimeMcpProvider,
    internal_function_tool_registry, remote_ops_tool_registry, terminal_exec_tool_registry,
};
use std::sync::Arc;

pub(super) fn build_tool_registry(
    cx: &mut App,
    toolsets: &ToolExposureToolsetSettings,
) -> anyhow::Result<PublicMcpToolRegistry> {
    let mut providers: Vec<Arc<dyn PublicMcpToolProvider>> = Vec::new();
    let mut runtime_registries = Vec::new();
    if toolsets.terminal {
        if let Some(registry) = terminal_view::public_mcp::registry(cx) {
            if toolsets.terminal_ssh_exec {
                runtime_registries.push(remote_ops_tool_registry(registry.clone()));
            }
            if toolsets.terminal_exec {
                runtime_registries.push(terminal_exec_tool_registry(registry));
            }
        } else {
            tracing::warn!("Public MCP terminal registry is not initialized");
        }
    }
    if toolsets.internal_functions {
        runtime_registries.push(onetcli_runtime::builtin_tool_registry_with_version(env!(
            "CARGO_PKG_VERSION"
        )));
        runtime_registries.push(internal_function_tool_registry(
            internal_functions::definitions(cx),
        ));
    }
    if toolsets.connections {
        if let Some(storage) = cx.try_global::<one_core::storage::GlobalStorageState>() {
            if let Some(repo) = storage
                .storage
                .get::<one_core::storage::ConnectionRepository>()
            {
                let workspace_repo = storage
                    .storage
                    .get::<one_core::storage::WorkspaceRepository>();
                let session_opener = super::connection_sessions::connection_session_opener(cx);
                let save_notifier = super::connection_sessions::connection_save_notifier(cx);
                runtime_registries.push(
                    onetcli_runtime::connections::connection_tool_registry_with_workspaces_and_hooks(
                        repo,
                        workspace_repo.clone(),
                        onetcli_runtime::connections::ConnectionToolHooks::default()
                            .with_session_opener(Some(session_opener))
                            .with_save_notifier(Some(save_notifier)),
                    ),
                );
                if let Some(workspace_repo) = workspace_repo {
                    runtime_registries.push(onetcli_runtime::workspaces::workspace_tool_registry(
                        workspace_repo,
                    ));
                } else {
                    tracing::warn!(
                        "Public MCP connection tools enabled without WorkspaceRepository"
                    );
                }
            } else {
                tracing::warn!("Public MCP connection tools enabled without ConnectionRepository");
            }
        } else {
            tracing::warn!("Public MCP connection tools enabled before storage is initialized");
        }
    }
    if toolsets.sftp {
        if let Some(storage) = cx.try_global::<one_core::storage::GlobalStorageState>() {
            if let Some(repo) = storage
                .storage
                .get::<one_core::storage::ConnectionRepository>()
            {
                runtime_registries.push(onetcli_runtime::sftp_tools::sftp_tool_registry(repo));
            } else {
                tracing::warn!("Public MCP SFTP tools enabled without ConnectionRepository");
            }
        } else {
            tracing::warn!("Public MCP SFTP tools enabled before storage is initialized");
        }
    }
    if toolsets.database {
        if let Some(storage) = cx.try_global::<one_core::storage::GlobalStorageState>() {
            if let Some(repo) = storage
                .storage
                .get::<one_core::storage::ConnectionRepository>()
            {
                runtime_registries.push(onetcli_runtime::database_tools::database_tool_registry(
                    repo,
                ));
            } else {
                tracing::warn!("Public MCP database tools enabled without ConnectionRepository");
            }
        } else {
            tracing::warn!("Public MCP database tools enabled before storage is initialized");
        }
    }
    if toolsets.redis {
        runtime_registries.push(tool_runtime::ToolRegistry::new(redis::redis_tool_handlers(
            cx,
        )));
    }
    if !runtime_registries.is_empty() {
        let mut provider =
            ToolRuntimeMcpProvider::new(tool_runtime::ToolRegistry::merge(runtime_registries)?);
        if let Some(resource_pool_provider) = resource_pool::app_resource_pool_provider(cx) {
            provider = provider.with_resource_pool_provider(resource_pool_provider);
        }
        providers.push(Arc::new(provider));
    }
    if providers.is_empty() {
        tracing::warn!("Public MCP runtime enabled without any tool providers");
    }
    Ok(PublicMcpToolRegistry::try_new(providers)?)
}

#[cfg(test)]
mod tests {
    use super::build_tool_registry;
    use crate::public_mcp_runtime::register_internal_function;
    use gpui::TestAppContext;
    use one_core::connection_notifier::{ConnectionDataEvent, get_notifier};
    use one_core::settings::ToolExposureToolsetSettings;
    use one_core::storage::connection::SqliteConnection;
    use one_core::storage::migration::run_migrations;
    use one_core::storage::traits::Repository;
    use one_core::storage::{
        ConnectionRepository, DatabaseType, DbConnectionConfig, GlobalStorageState, SshAuthMethod,
        SshParams, StorageManager, StoredConnection, WorkspaceRepository,
    };
    use public_mcp::approval::{
        PublicMcpApprovalFuture, PublicMcpApprovalManager, PublicMcpApprovalOutcome,
        PublicMcpApprovalRequest, PublicMcpApprover,
    };
    use public_mcp::permissions::PermissionMode;
    use public_mcp::registry::{
        ConnectionState, TerminalConnectionKind, TerminalExecSessionHandle, TerminalSessionHandle,
        TerminalSessionSnapshot,
    };
    use public_mcp::terminal_exec::{
        TerminalExecCompletion, TerminalExecRequest, TerminalExecResult,
    };
    use public_mcp::tools::{InternalFunctionDefinition, PublicMcpToolContext};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    #[gpui::test]
    fn build_tool_registry_includes_internal_function_tools(cx: &mut TestAppContext) {
        let toolsets = internal_function_toolsets();

        let tools = cx.update(|cx| {
            build_tool_registry(cx, &toolsets)
                .expect("internal function registry should build")
                .tools()
        });

        assert!(
            tools
                .iter()
                .any(|tool| tool.name == "internal_functions.list")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool.name == "internal_functions.call")
        );
    }

    #[gpui::test]
    fn build_tool_registry_includes_redis_tools(cx: &mut TestAppContext) {
        let toolsets = ToolExposureToolsetSettings {
            terminal: false,
            connections: false,
            redis: true,
            ..Default::default()
        };

        let tools = cx.update(|cx| {
            build_tool_registry(cx, &toolsets)
                .expect("redis registry should build")
                .tools()
        });

        assert!(
            tools
                .iter()
                .any(|tool| tool.name == "redis.list_connections")
        );
        assert!(tools.iter().any(|tool| tool.name == "redis.command"));
        assert!(tools.iter().any(|tool| tool.name == "redis.keys"));
        assert!(tools.iter().any(|tool| tool.name == "redis.get"));
        assert!(tools.iter().any(|tool| tool.name == "redis.set"));
    }

    #[gpui::test]
    fn build_tool_registry_includes_connection_tools(cx: &mut TestAppContext) {
        let toolsets = ToolExposureToolsetSettings {
            terminal: false,
            connections: true,
            ..Default::default()
        };

        let tools = cx.update(|cx| {
            register_connection_repository(cx);
            build_tool_registry(cx, &toolsets)
                .expect("connection registry should build")
                .tools()
        });

        assert!(tools.iter().any(|tool| tool.name == "connections.list"));
        assert!(
            tools
                .iter()
                .any(|tool| tool.name == "connections.list_sessions")
        );
        assert!(tools.iter().any(|tool| tool.name == "connections.show"));
        assert!(tools.iter().any(|tool| tool.name == "connections.validate"));
        assert!(tools.iter().any(|tool| tool.name == "connections.find"));
        assert!(tools.iter().any(|tool| tool.name == "connections.save"));
        assert!(tools.iter().any(|tool| tool.name == "connections.delete"));
        assert!(tools.iter().any(|tool| tool.name == "connections.test"));
        assert!(
            tools
                .iter()
                .any(|tool| tool.name == "connections.open_session")
        );
        assert!(tools.iter().any(|tool| tool.name == "workspaces.list"));
        assert!(tools.iter().any(|tool| tool.name == "workspaces.show"));
        assert!(
            !tools
                .iter()
                .any(|tool| tool.name.starts_with("onetcli.connections."))
        );
    }

    #[gpui::test]
    fn connection_save_emits_connection_created_event(cx: &mut TestAppContext) {
        let toolsets = ToolExposureToolsetSettings {
            terminal: false,
            connections: true,
            ..Default::default()
        };
        let events = Arc::new(Mutex::new(Vec::<ConnectionDataEvent>::new()));
        let events_for_subscription = events.clone();

        let (registry, _subscription) = cx.update(|cx| {
            one_core::connection_notifier::init(cx);
            register_connection_repository(cx);
            let notifier = get_notifier(cx).expect("connection notifier should be initialized");
            let subscription = cx.subscribe(&notifier, move |_, event: &ConnectionDataEvent, _| {
                events_for_subscription
                    .lock()
                    .expect("events lock")
                    .push(event.clone());
            });
            (
                build_tool_registry(cx, &toolsets).expect("connection registry should build"),
                subscription,
            )
        });

        let result = futures::executor::block_on(registry.call_tool(
            "connections.save",
            Some(serde_json::Map::from_iter([
                ("kind".to_string(), json!("database")),
                ("database_type".to_string(), json!("MySQL")),
                (
                    "values".to_string(),
                    json!({
                        "name": "created mysql",
                        "host": "10.0.1.20",
                        "username": "app"
                    }),
                ),
            ])),
            PublicMcpToolContext {
                permission_mode: PermissionMode::Allow,
                approver: PublicMcpApprovalManager::new(Arc::new(AlwaysApprove)),
            },
        ))
        .expect("connections.save should create a connection");

        assert_eq!(
            Some(&json!(true)),
            result
                .structured_content
                .as_ref()
                .and_then(|value| value.get("ok"))
        );
        cx.run_until_parked();
        let events = events.lock().expect("events lock");
        assert_eq!(1, events.len());
        match &events[0] {
            ConnectionDataEvent::ConnectionCreated { connection } => {
                assert_eq!("created mysql", connection.name);
            }
            other => panic!("unexpected connection event: {other:?}"),
        }
    }

    #[gpui::test]
    fn build_tool_registry_includes_sftp_tools(cx: &mut TestAppContext) {
        let toolsets = ToolExposureToolsetSettings {
            terminal: false,
            connections: false,
            sftp: true,
            ..Default::default()
        };

        let tools = cx.update(|cx| {
            register_connection_repository(cx);
            build_tool_registry(cx, &toolsets)
                .expect("sftp registry should build")
                .tools()
        });

        assert!(tools.iter().any(|tool| tool.name == "sftp.list"));
        assert!(tools.iter().any(|tool| tool.name == "sftp.read"));
        assert!(tools.iter().any(|tool| tool.name == "sftp.write"));
        assert!(tools.iter().any(|tool| tool.name == "sftp.stat"));
        assert!(tools.iter().any(|tool| tool.name == "sftp.upload"));
        assert!(tools.iter().any(|tool| tool.name == "sftp.download"));
    }

    #[gpui::test]
    fn build_tool_registry_includes_database_tools(cx: &mut TestAppContext) {
        let toolsets = ToolExposureToolsetSettings {
            terminal: false,
            connections: false,
            database: true,
            ..Default::default()
        };

        let tools = cx.update(|cx| {
            register_connection_repository(cx);
            build_tool_registry(cx, &toolsets)
                .expect("database registry should build")
                .tools()
        });

        assert!(tools.iter().any(|tool| tool.name == "db.schema"));
        assert!(tools.iter().any(|tool| tool.name == "db.query"));
        assert!(tools.iter().any(|tool| tool.name == "db.exec"));
    }

    #[gpui::test]
    fn build_tool_registry_resolves_saved_connection_target_alias(cx: &mut TestAppContext) {
        let toolsets = ToolExposureToolsetSettings {
            terminal: false,
            connections: false,
            database: true,
            ..Default::default()
        };

        let registry = cx.update(|cx| {
            let repo = register_connection_repository(cx);
            insert_database_connection(&repo, "prod-db", "127.0.0.1");
            build_tool_registry(cx, &toolsets).expect("database registry should build")
        });

        let error = futures::executor::block_on(registry.call_tool(
            "db.query",
            Some(serde_json::Map::from_iter([
                ("target".to_string(), json!("127.0.0.1")),
                (
                    "sql".to_string(),
                    json!("create table unsafe_write(id integer)"),
                ),
            ])),
            PublicMcpToolContext {
                permission_mode: PermissionMode::Deny,
                approver: Default::default(),
            },
        ))
        .expect_err("resolved target should reach db.query validation");

        assert!(
            error
                .to_string()
                .contains("db.query only accepts query statements"),
            "unexpected error: {error}"
        );
    }

    #[gpui::test]
    fn build_tool_registry_terminal_toolset_includes_terminal_exec(cx: &mut TestAppContext) {
        let toolsets = ToolExposureToolsetSettings {
            terminal: true,
            connections: false,
            internal_functions: false,
            ..Default::default()
        };

        let tools = cx.update(|cx| {
            terminal_view::public_mcp::init(cx);
            build_tool_registry(cx, &toolsets)
                .expect("terminal registry should build")
                .tools()
        });

        assert!(tools.iter().any(|tool| tool.name == "ssh.exec"));
        assert!(tools.iter().any(|tool| tool.name == "terminal.exec"));
    }

    #[gpui::test]
    fn terminal_toolset_can_disable_visible_terminal_exec(cx: &mut TestAppContext) {
        let toolsets = ToolExposureToolsetSettings {
            terminal: true,
            terminal_ssh_exec: true,
            terminal_exec: false,
            connections: false,
            internal_functions: false,
            ..Default::default()
        };

        let tools = cx.update(|cx| {
            terminal_view::public_mcp::init(cx);
            build_tool_registry(cx, &toolsets)
                .expect("terminal registry should build")
                .tools()
        });

        assert!(tools.iter().any(|tool| tool.name == "ssh.exec"));
        assert!(!tools.iter().any(|tool| tool.name == "terminal.exec"));
    }

    #[gpui::test]
    fn terminal_toolset_can_disable_structured_ssh_exec(cx: &mut TestAppContext) {
        let toolsets = ToolExposureToolsetSettings {
            terminal: true,
            terminal_ssh_exec: false,
            terminal_exec: true,
            connections: false,
            internal_functions: false,
            ..Default::default()
        };

        let tools = cx.update(|cx| {
            terminal_view::public_mcp::init(cx);
            build_tool_registry(cx, &toolsets)
                .expect("terminal registry should build")
                .tools()
        });

        assert!(!tools.iter().any(|tool| tool.name == "ssh.exec"));
        assert!(tools.iter().any(|tool| tool.name == "terminal.exec"));
    }

    #[gpui::test]
    fn build_tool_registry_resolves_active_terminal_target_alias(cx: &mut TestAppContext) {
        let toolsets = ToolExposureToolsetSettings {
            terminal: true,
            connections: false,
            internal_functions: false,
            database: true,
            ..Default::default()
        };

        let (registry, terminal) = cx.update(|cx| {
            terminal_view::public_mcp::init(cx);
            let terminal_registry =
                terminal_view::public_mcp::registry(cx).expect("terminal registry should exist");
            let repo = register_connection_repository(cx);
            let connection_id = insert_ssh_connection(&repo, "prod-a", "10.2.4.54");
            let terminal = FakeTerminalSession::new(
                "ssh-terminal-prod-a",
                Some(connection_id),
                "prod-a",
                "root@zn-54:~",
            );
            terminal_registry.register(terminal.clone());
            terminal_registry.register_terminal_exec(terminal.clone());
            insert_database_connection(&repo, "prod-db", "10.2.4.54");
            (
                build_tool_registry(cx, &toolsets).expect("terminal registry should build"),
                terminal,
            )
        });

        for target in ["10.2.4.54", "root@zn-54:~"] {
            let content = call_terminal_exec(&registry, target);
            assert_eq!(json!("ssh-terminal-prod-a"), content["target"]);
        }

        assert_eq!(
            vec!["df -h\n".to_string(), "df -h\n".to_string()],
            terminal.inserted()
        );
    }

    #[gpui::test]
    fn build_tool_registry_uses_live_resource_pool_for_new_terminal_targets(
        cx: &mut TestAppContext,
    ) {
        let toolsets = ToolExposureToolsetSettings {
            terminal: true,
            connections: false,
            internal_functions: false,
            database: true,
            ..Default::default()
        };

        let (registry, terminal, connection_id) = cx.update(|cx| {
            terminal_view::public_mcp::init(cx);
            let terminal_registry =
                terminal_view::public_mcp::registry(cx).expect("terminal registry should exist");
            let repo = register_connection_repository(cx);
            let registry =
                build_tool_registry(cx, &toolsets).expect("terminal registry should build");
            let connection_id = insert_ssh_connection(&repo, "prod-a", "10.2.4.54");
            let terminal = FakeTerminalSession::new(
                "ssh-terminal-prod-a",
                Some(connection_id),
                "prod-a",
                "root@zn-54:~",
            );
            terminal_registry.register(terminal.clone());
            terminal_registry.register_terminal_exec(terminal.clone());
            insert_database_connection(&repo, "prod-db", "10.2.4.54");
            (registry, terminal, connection_id)
        });

        let content = call_terminal_exec(&registry, &connection_id.to_string());

        assert_eq!(json!("ssh-terminal-prod-a"), content["target"]);
        assert_eq!(vec!["df -h\n".to_string()], terminal.inserted());
    }

    #[gpui::test]
    fn build_tool_registry_uses_registered_internal_functions(cx: &mut TestAppContext) {
        let toolsets = internal_function_toolsets();
        let registry = cx.update(|cx| {
            register_internal_function(cx, runtime_status_function());
            build_tool_registry(cx, &toolsets).expect("internal function registry should build")
        });

        let result = futures::executor::block_on(registry.call_tool(
            "internal_functions.list",
            None,
            PublicMcpToolContext {
                permission_mode: PermissionMode::Deny,
                approver: Default::default(),
            },
        ))
        .expect("list tool should run");

        assert_eq!(
            Some(json!({
                "functions": [{
                    "name": "onetcli.runtime_status",
                    "description": "Read the public MCP runtime status.",
                    "read_only": true,
                    "input_schema": {
                        "type": "object"
                    }
                }]
            })),
            result.structured_content
        );
    }

    #[gpui::test]
    fn build_tool_registry_exposes_tool_runtime_app_info(cx: &mut TestAppContext) {
        let toolsets = internal_function_toolsets();
        let registry = cx.update(|cx| {
            build_tool_registry(cx, &toolsets).expect("tool runtime registry should build")
        });

        let tools = registry.tools();
        assert!(tools.iter().any(|tool| tool.name == "onetcli.app_info"));

        let result = futures::executor::block_on(registry.call_tool(
            "onetcli.app_info",
            None,
            PublicMcpToolContext {
                permission_mode: PermissionMode::Deny,
                approver: Default::default(),
            },
        ))
        .expect("app info tool should run");

        assert_eq!(
            Some(json!({
                "name": "onetcli",
                "version": env!("CARGO_PKG_VERSION")
            })),
            result.structured_content
        );
    }

    fn internal_function_toolsets() -> ToolExposureToolsetSettings {
        ToolExposureToolsetSettings {
            terminal: false,
            connections: false,
            internal_functions: true,
            ..Default::default()
        }
    }

    fn runtime_status_function() -> InternalFunctionDefinition {
        InternalFunctionDefinition::read_only(
            "onetcli.runtime_status",
            "Read the public MCP runtime status.",
            |_| async { Ok(json!({ "state": "disabled" })) },
        )
    }

    #[derive(Clone)]
    struct FakeTerminalSession {
        session_id: String,
        connection_id: Option<i64>,
        host_label: String,
        title: String,
        inserted: Arc<Mutex<Vec<String>>>,
    }

    impl FakeTerminalSession {
        fn new(
            session_id: &str,
            connection_id: Option<i64>,
            host_label: &str,
            title: &str,
        ) -> Self {
            Self {
                session_id: session_id.to_string(),
                connection_id,
                host_label: host_label.to_string(),
                title: title.to_string(),
                inserted: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn inserted(&self) -> Vec<String> {
            self.inserted.lock().expect("inserted lock").clone()
        }

        fn snapshot(&self) -> TerminalSessionSnapshot {
            TerminalSessionSnapshot {
                session_id: self.session_id.clone(),
                connection_id: self.connection_id,
                title: self.title.clone(),
                host_label: self.host_label.clone(),
                cwd: Some("/root".to_string()),
                rows: 24,
                cols: 120,
                connection_kind: TerminalConnectionKind::Ssh,
                connection_state: ConnectionState::Connected,
            }
        }
    }

    impl TerminalSessionHandle for FakeTerminalSession {
        fn snapshot(&self) -> TerminalSessionSnapshot {
            self.snapshot()
        }
    }

    impl TerminalExecSessionHandle for FakeTerminalSession {
        fn snapshot(&self) -> TerminalSessionSnapshot {
            self.snapshot()
        }

        fn exec_in_terminal(
            &self,
            request: TerminalExecRequest,
        ) -> anyhow::Result<TerminalExecResult> {
            let suffix = if request.submit { "\n" } else { "" };
            self.inserted
                .lock()
                .expect("inserted lock")
                .push(format!("{}{suffix}", request.command));
            Ok(TerminalExecResult {
                target: request.target,
                command: request.command,
                submitted: request.submit,
                completion: TerminalExecCompletion::SubmittedOnly,
                exit_code: None,
                output: String::new(),
                duration_ms: 0,
            })
        }
    }

    fn call_terminal_exec(
        registry: &public_mcp::tools::PublicMcpToolRegistry,
        target: &str,
    ) -> serde_json::Value {
        futures::executor::block_on(registry.call_tool(
            "terminal.exec",
            Some(serde_json::Map::from_iter([
                ("target".to_string(), json!(target)),
                ("command".to_string(), json!("df -h")),
                ("submit".to_string(), json!(true)),
            ])),
            PublicMcpToolContext {
                permission_mode: PermissionMode::Allow,
                approver: PublicMcpApprovalManager::new(Arc::new(AlwaysApprove)),
            },
        ))
        .expect("terminal target alias should resolve to active terminal session")
        .structured_content
        .expect("terminal.exec should return structured content")
    }

    struct AlwaysApprove;

    impl PublicMcpApprover for AlwaysApprove {
        fn request_approval(&self, _request: PublicMcpApprovalRequest) -> PublicMcpApprovalFuture {
            Box::pin(async { PublicMcpApprovalOutcome::Approved })
        }
    }

    fn register_connection_repository(cx: &mut gpui::App) -> Arc<ConnectionRepository> {
        let storage = StorageManager::new_with_connection(test_connection());
        let repo = ConnectionRepository::new(storage.connection());
        let repo_for_insert = Arc::new(repo.clone());
        storage.register(repo);
        storage.register(WorkspaceRepository::new(storage.connection()));
        cx.set_global(GlobalStorageState { storage });
        repo_for_insert
    }

    fn insert_database_connection(repo: &ConnectionRepository, name: &str, host: &str) {
        let mut connection =
            StoredConnection::new_database(name.to_string(), db_config(host), None);
        repo.insert(&mut connection)
            .expect("database connection should insert");
    }

    fn insert_ssh_connection(repo: &ConnectionRepository, name: &str, host: &str) -> i64 {
        let mut connection = StoredConnection::new_ssh(name.to_string(), ssh_params(host), None);
        repo.insert(&mut connection)
            .expect("ssh connection should insert");
        connection
            .id
            .expect("inserted ssh connection should have id")
    }

    fn db_config(host: &str) -> DbConnectionConfig {
        DbConnectionConfig {
            id: String::new(),
            database_type: DatabaseType::SQLite,
            name: String::new(),
            host: host.to_string(),
            port: 0,
            username: String::new(),
            password: String::new(),
            database: None,
            service_name: None,
            sid: None,
            workspace_id: None,
            proxy: None,
            extra_params: Default::default(),
        }
    }

    fn ssh_params(host: &str) -> SshParams {
        SshParams {
            host: host.to_string(),
            port: 22,
            username: "root".to_string(),
            auth_method: SshAuthMethod::Agent,
            connect_timeout: None,
            keepalive_interval: None,
            keepalive_max: None,
            default_directory: None,
            init_script: None,
            disable_shell_integration: None,
            jump_server: None,
            proxy: None,
        }
    }

    fn test_connection() -> SqliteConnection {
        let conn = SqliteConnection::open_with_pool_size(":memory:", 1)
            .expect("sqlite connection should open");
        conn.with_connection(|db| {
            run_migrations(db)?;
            Ok(())
        })
        .expect("migrations should run");
        conn
    }
}
