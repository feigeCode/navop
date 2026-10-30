//! `navopData` 的领域实现（骨架）。
//!
//! 首个迁入的是 Redis 单连接打开路径；工作区分组仍留在 `main`，
//! 等 `HomeContext` 暴露当前工作区后再迁。Database/Mongo 随后加入。

use db_view::database_tab::DatabaseTabView;
use domain_api::{
    ConnectionOpener, DatabaseTabMode, Domain, DomainId, DomainRegistry, HomeContext,
};
use extension_runtime::database_driver_install::{
    DriverRequirement, database_driver_meets_requirement, prompt_install_database_driver_app,
    required_driver_for_config,
};
use gpui::{App, AppContext, Window};
use mongodb_view::MongoTabView;
use one_core::storage::{
    ConnectionRepository, ConnectionType, GlobalStorageState, StoredConnection,
};
use one_core::tab_container::{TabItem, TabOpenMode};
use redis_view::RedisTabView;
use std::sync::Arc;

pub mod connection_forms;
pub mod mcp;
pub mod settings;
mod tab_restore;
pub mod active_resources;
mod visuals;

pub struct DataDomain;

fn register_mcp_toolsets() {
    mcp_host::register_toolset(
        mcp_host::McpToolset::Database,
        Arc::new(|cx: &App| {
            let Some(repo) = mcp_host::connection_repository(cx) else {
                return Vec::new();
            };
            vec![mcp_host::runtime_provider(
                runtime_database::database_tools::database_tool_registry(repo),
            )]
        }),
    );
    mcp_host::register_toolset(
        mcp_host::McpToolset::Redis,
        Arc::new(|cx: &App| {
            let handlers = mcp::redis_tool_handlers(cx);
            if handlers.is_empty() {
                return Vec::new();
            }
            vec![mcp_host::runtime_provider(tool_runtime::ToolRegistry::new(
                handlers,
            ))]
        }),
    );
    mcp_host::register_toolset(
        mcp_host::McpToolset::Connections,
        Arc::new(|cx: &App| {
            let Some(repo) = mcp_host::connection_repository(cx) else {
                return Vec::new();
            };
            let workspaces = mcp_host::workspace_repository(cx);
            vec![mcp_host::runtime_provider(
                navop_runtime::connections::connection_tool_registry_with_workspaces(
                    repo, workspaces,
                ),
            )]
        }),
    );
    mcp_host::register_toolset(
        mcp_host::McpToolset::Mongo,
        Arc::new(|cx: &App| {
            let handlers = mcp::mongo_tool_handlers(cx);
            if handlers.is_empty() {
                return Vec::new();
            }
            vec![mcp_host::runtime_provider(tool_runtime::ToolRegistry::new(
                handlers,
            ))]
        }),
    );
}

impl Domain for DataDomain {
    fn id(&self) -> DomainId {
        DomainId::DATABASE
    }

    /// 数据领域的子系统初始化：数据库缓存、SQL 编辑器、Redis/Mongo 视图、
    /// 全局数据库状态与“询问 AI”通知器。
    fn refresh_keybindings(&self, cx: &mut App) {
        db_view::search_shortcut::refresh_keybindings(cx);
        redis_view::refresh_keybindings(cx);
        // 必须排在笔记之后：笔记的 `secondary-/` 是无 context 的全局绑定，
        // 同深度后注册者胜，SQL 编辑器的带 context 绑定要靠后登记（#290）。
        db_view::sql_editor_view::refresh_keybindings(cx);
    }

    fn init(&self, cx: &mut App) -> anyhow::Result<()> {
        db::ipc::set_host_version(app_kit::update::current_version())
            .map_err(|error| anyhow::anyhow!("invalid product version: {error}"))?;
        app_kit::visuals::set_connection_visuals(Arc::new(visuals::DataConnectionVisuals::new()));
        db::init_cache(cx);
        if let Some(cache) = cx.try_global::<db::GlobalNodeCache>() {
            cache.start_cleanup_task(cx);
        }
        db_view::search_shortcut::init(cx);
        db_view::sql_editor_view::init(cx);
        redis_view::init(cx);
        mongodb_view::init(cx);
        let storage_state = cx.global::<GlobalStorageState>();
        let connection_repository = storage_state.storage.get::<ConnectionRepository>();
        let db_state = db::GlobalDbState::with_connection_repository(connection_repository);
        db_state.start_cleanup_task(cx);
        cx.set_global(db_state);
        db_view::init_ask_ai_notifier(cx);
        Ok(())
    }

    fn register(&self, registry: &mut DomainRegistry) {
        registry.register_active_resource_provider(Arc::new(
            active_resources::RedisActiveResourceProvider,
        ));
        registry.register_connection_opener(ConnectionType::Redis, Arc::new(RedisConnectionOpener));
        registry.register_connection_opener(
            ConnectionType::Database,
            Arc::new(DatabaseConnectionOpener),
        );
        registry
            .register_connection_opener(ConnectionType::MongoDB, Arc::new(MongoConnectionOpener));
        // 新建连接向导：数据库 / Redis / MongoDB 三个表单由本领域装配。
        registry.register_form_provider(Arc::new(connection_forms::DataConnectionFormProvider));
        // 设置页：数据库打开方式、表格与查询相关设置。
        registry.register_settings_provider(Arc::new(settings::DataSettingsProvider));
        // Tab 会话恢复：把上次关闭时的 Database / Redis / MongoDB 标签重建出来。
        registry.register_tab_restorer(Arc::new(tab_restore::DataTabRestorer));
        register_mcp_toolsets();
    }
}

struct RedisConnectionOpener;

impl RedisConnectionOpener {
    fn supports_type(connection_type: ConnectionType) -> bool {
        connection_type == ConnectionType::Redis
    }
}

impl ConnectionOpener for RedisConnectionOpener {
    fn open(
        &self,
        connection: &StoredConnection,
        mode: TabOpenMode,
        home: &mut dyn HomeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if !Self::supports_type(connection.connection_type) {
            return false;
        }
        let connection = connection.clone();
        let connection_id = connection.id;
        let (tab_id, connections, workspace) = match connection.workspace_id {
            Some(workspace_id) => {
                let mut connections: Vec<StoredConnection> = home
                    .connections()
                    .iter()
                    .filter(|candidate| {
                        candidate.connection_type == ConnectionType::Redis
                            && candidate.workspace_id == Some(workspace_id)
                    })
                    .cloned()
                    .collect();
                if connections.is_empty() {
                    connections.push(connection);
                }
                home.sort_connections(&mut connections, cx);
                (
                    format!("workspace-redis-tab-{workspace_id}"),
                    connections,
                    home.workspace(workspace_id),
                )
            }
            None => (
                format!("redis-{}", connection_id.unwrap_or(0)),
                vec![connection],
                None,
            ),
        };
        let item_id = tab_id.clone();
        home.open_tab_lazy(
            tab_id,
            mode,
            window,
            cx,
            Box::new(move |window, cx| {
                let view = cx.new(|cx| {
                    RedisTabView::new_with_active_conn(
                        workspace,
                        connections,
                        connection_id,
                        window,
                        cx,
                    )
                    .with_external_sidebar()
                });
                TabItem::from_dyn(item_id, "redis", Arc::new(view))
            }),
        )
    }
}

struct DatabaseConnectionOpener;

impl DatabaseConnectionOpener {
    fn supports_type(connection_type: ConnectionType) -> bool {
        connection_type == ConnectionType::Database
    }
}

impl ConnectionOpener for DatabaseConnectionOpener {
    fn open(
        &self,
        connection: &StoredConnection,
        mode: TabOpenMode,
        home: &mut dyn HomeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if !Self::supports_type(connection.connection_type) {
            return false;
        }
        let Ok(config) = connection.to_db_connection() else {
            return false;
        };
        match required_driver_for_config(&config) {
            DriverRequirement::NotRequired => {}
            DriverRequirement::InvalidConfig { message } => {
                home.notify(domain_api::NotifyLevel::Error, &message, window, cx);
                return true;
            }
            DriverRequirement::Required {
                driver_id,
                minimum_version,
            } => {
                if !database_driver_meets_requirement(&driver_id, minimum_version.as_deref()) {
                    prompt_install_database_driver_app(
                        cx,
                        window,
                        driver_id,
                        connection.name.clone(),
                    );
                    return true;
                }
            }
        }

        let open_mode = home.database_open_mode(cx);
        let active_connection_id = connection.id;
        let (tab_id, connections, workspace) = match (open_mode, connection.workspace_id) {
            (DatabaseTabMode::Workspace, Some(workspace_id)) => {
                let mut connections: Vec<StoredConnection> = home
                    .connections()
                    .iter()
                    .filter(|candidate| {
                        candidate.connection_type == ConnectionType::Database
                            && candidate.workspace_id == Some(workspace_id)
                    })
                    .cloned()
                    .collect();
                if connections.is_empty() {
                    connections.push(connection.clone());
                }
                home.sort_connections(&mut connections, cx);
                (
                    format!("workspace-database-tab-{workspace_id}"),
                    connections,
                    home.workspace(workspace_id),
                )
            }
            _ => (
                format!("database-tab-{}", active_connection_id.unwrap_or(0)),
                vec![connection.clone()],
                None,
            ),
        };

        let item_id = tab_id.clone();
        home.open_tab_lazy(
            tab_id,
            mode,
            window,
            cx,
            Box::new(move |window, cx| {
                let view = cx.new(|cx| {
                    DatabaseTabView::new_with_active_conn(
                        workspace,
                        connections,
                        active_connection_id,
                        window,
                        cx,
                    )
                    .with_external_sidebar()
                });
                TabItem::from_dyn(item_id, "home", Arc::new(view))
            }),
        )
    }
}

struct MongoConnectionOpener;

impl MongoConnectionOpener {
    fn supports_type(connection_type: ConnectionType) -> bool {
        connection_type == ConnectionType::MongoDB
    }
}

impl ConnectionOpener for MongoConnectionOpener {
    fn open(
        &self,
        connection: &StoredConnection,
        mode: TabOpenMode,
        home: &mut dyn HomeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if !Self::supports_type(connection.connection_type) {
            return false;
        }

        let active_connection_id = connection.id;
        let (tab_id, connections, workspace) =
            match (home.database_open_mode(cx), connection.workspace_id) {
                (DatabaseTabMode::Workspace, Some(workspace_id)) => {
                    let mut connections: Vec<StoredConnection> = home
                        .connections()
                        .iter()
                        .filter(|candidate| {
                            candidate.connection_type == ConnectionType::MongoDB
                                && candidate.workspace_id == Some(workspace_id)
                        })
                        .cloned()
                        .collect();
                    if connections.is_empty() {
                        connections.push(connection.clone());
                    }
                    home.sort_connections(&mut connections, cx);
                    (
                        format!("workspace-mongodb-tab-{workspace_id}"),
                        connections,
                        home.workspace(workspace_id),
                    )
                }
                _ => (
                    format!("mongodb-{}", active_connection_id.unwrap_or(0)),
                    vec![connection.clone()],
                    None,
                ),
            };

        let item_id = tab_id.clone();
        home.open_tab_lazy(
            tab_id,
            mode,
            window,
            cx,
            Box::new(move |window, cx| {
                let view = cx.new(|cx| {
                    MongoTabView::new_with_active_conn(
                        workspace,
                        connections,
                        active_connection_id,
                        window,
                        cx,
                    )
                    .with_external_sidebar()
                });
                TabItem::from_dyn(item_id, "mongodb", Arc::new(view))
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_domain_registers_its_tab_restorer() {
        let mut registry = DomainRegistry::default();
        DataDomain.register(&mut registry);
        assert_eq!(1, registry.tab_restorers().len());
        assert_eq!(
            vec!["Database", "Redis", "MongoDB"],
            registry.tab_restorers()[0].content_keys()
        );
    }

    #[test]
    fn data_domain_has_stable_id() {
        assert_eq!(DataDomain.id(), DomainId::DATABASE);
    }

    #[test]
    fn redis_opener_accepts_only_redis_type() {
        assert!(RedisConnectionOpener::supports_type(ConnectionType::Redis));
        assert!(!RedisConnectionOpener::supports_type(
            ConnectionType::Database
        ));
    }

    #[test]
    fn database_opener_accepts_only_database_type() {
        assert!(DatabaseConnectionOpener::supports_type(
            ConnectionType::Database
        ));
        assert!(!DatabaseConnectionOpener::supports_type(
            ConnectionType::Redis
        ));
    }

    #[test]
    fn data_domain_registers_database_redis_mongo_and_connections_mcp_toolsets() {
        let mut registry = DomainRegistry::default();
        DataDomain.register(&mut registry);
        let toolsets = mcp_host::registered_toolsets();
        assert!(toolsets.contains(&mcp_host::McpToolset::Database));
        assert!(toolsets.contains(&mcp_host::McpToolset::Redis));
        assert!(toolsets.contains(&mcp_host::McpToolset::Mongo));
        assert!(toolsets.contains(&mcp_host::McpToolset::Connections));
    }

    #[test]
    fn mongo_opener_accepts_only_mongodb_type() {
        assert!(MongoConnectionOpener::supports_type(
            ConnectionType::MongoDB
        ));
        assert!(!MongoConnectionOpener::supports_type(
            ConnectionType::Database
        ));
    }
}
