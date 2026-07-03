use super::{internal_functions, redis, resource_pool};
use gpui::App;
use one_core::settings::McpToolsetSettings;
use public_mcp::tools::{
    PublicMcpToolProvider, PublicMcpToolRegistry, ToolRuntimeMcpProvider,
    internal_function_tool_registry, remote_ops_tool_registry, terminal_exec_tool_registry,
};
use std::sync::Arc;

pub(super) fn build_tool_registry(
    cx: &mut App,
    toolsets: &McpToolsetSettings,
) -> anyhow::Result<PublicMcpToolRegistry> {
    let mut providers: Vec<Arc<dyn PublicMcpToolProvider>> = Vec::new();
    let mut runtime_registries = Vec::new();
    if toolsets.terminal {
        if let Some(registry) = terminal_view::public_mcp::registry(cx) {
            runtime_registries.push(remote_ops_tool_registry(registry.clone()));
            runtime_registries.push(terminal_exec_tool_registry(registry));
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
                runtime_registries.push(
                    onetcli_runtime::connections::connection_tool_registry_with_workspaces_and_session_opener(
                        repo,
                        workspace_repo.clone(),
                        Some(session_opener),
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
        if let Some(pool) =
            resource_pool::app_resource_pool(cx)?.filter(|pool| !pool.resources.is_empty())
        {
            provider = provider.with_resource_pool(pool);
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
    use one_core::settings::McpToolsetSettings;
    use one_core::storage::connection::SqliteConnection;
    use one_core::storage::migration::run_migrations;
    use one_core::storage::traits::Repository;
    use one_core::storage::{
        ConnectionRepository, DatabaseType, DbConnectionConfig, GlobalStorageState, StorageManager,
        StoredConnection, WorkspaceRepository,
    };
    use public_mcp::permissions::PermissionMode;
    use public_mcp::tools::{InternalFunctionDefinition, PublicMcpToolContext};
    use serde_json::json;
    use std::sync::Arc;

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
        let toolsets = McpToolsetSettings {
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
        let toolsets = McpToolsetSettings {
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
        assert!(tools.iter().any(|tool| tool.name == "connections.show"));
        assert!(tools.iter().any(|tool| tool.name == "connections.validate"));
        assert!(tools.iter().any(|tool| tool.name == "connections.find"));
        assert!(tools.iter().any(|tool| tool.name == "connections.update"));
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
    fn build_tool_registry_includes_sftp_tools(cx: &mut TestAppContext) {
        let toolsets = McpToolsetSettings {
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
        let toolsets = McpToolsetSettings {
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
        let toolsets = McpToolsetSettings {
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
        let toolsets = McpToolsetSettings {
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

    fn internal_function_toolsets() -> McpToolsetSettings {
        McpToolsetSettings {
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
            extra_params: Default::default(),
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
