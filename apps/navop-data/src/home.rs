use domain_api::{DatabaseTabMode, HomeContext, NotifyLevel, TabHost};
use gpui::{App, Context, Entity, Subscription, Window};
use gpui_component::WindowExt as _;
use one_core::settings::{AppSettings, DatabaseOpenMode};
use one_core::storage::traits::Repository;
use one_core::storage::{
    ConnectionRepository, ConnectionType, GlobalStorageState, StoredConnection, Workspace,
    WorkspaceRepository,
};
use one_core::tab_container::{TabContainer, TabItem, TabOpenMode};
use port_forwarding::runtime::PortForwardingRuntime;
use std::sync::Arc;

mod render;

pub struct DataHome {
    tab_container: Entity<TabContainer>,
    connections: Vec<StoredConnection>,
    workspaces: Vec<Workspace>,
    subscriptions: Vec<Subscription>,
}

impl DataHome {
    pub fn new(tab_container: Entity<TabContainer>, cx: &mut Context<Self>) -> Self {
        let storage = cx.global::<GlobalStorageState>().storage.clone();
        let connections = storage
            .get::<ConnectionRepository>()
            .and_then(|repo| repo.list().ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|connection| {
                matches!(
                    connection.connection_type,
                    ConnectionType::Database | ConnectionType::Redis | ConnectionType::MongoDB
                )
            })
            .collect();
        let workspaces = storage
            .get::<WorkspaceRepository>()
            .and_then(|repo| repo.list().ok())
            .unwrap_or_default();
        Self {
            tab_container,
            connections,
            workspaces,
            subscriptions: Vec::new(),
        }
    }

    fn open_connection(&mut self, connection: StoredConnection, window: &mut Window, cx: &mut App) {
        let kind = connection.connection_type;
        if !app_kit::registry().open_connection(
            &connection,
            TabOpenMode::Activate,
            self,
            window,
            cx,
        ) {
            let message = match kind {
                ConnectionType::Database => "该数据库连接暂不可用：可能缺少驱动，或连接参数无效",
                _ => "该连接类型尚未接入 navop-data",
            };
            self.notify(NotifyLevel::Warning, message, window, cx);
        }
    }
}

impl TabHost for DataHome {
    fn open_tab(
        &mut self,
        item: TabItem,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let tab_container = self.tab_container.clone();
        let tab_id = item.id();
        window.defer(cx, move |window, cx| {
            tab_container.update(cx, |container, cx| {
                if !container.activate_tab_by_id(tab_id.as_ref(), window, cx) {
                    container.add_tab_with_mode(item, mode, window, cx);
                }
            });
        });
        true
    }

    fn open_tab_with_focus(&mut self, item: TabItem, window: &mut Window, cx: &mut App) -> bool {
        let tab_container = self.tab_container.clone();
        window.defer(cx, move |window, cx| {
            tab_container.update(cx, |container, cx| {
                container.add_and_activate_tab_with_focus(item, window, cx)
            });
        });
        true
    }

    fn open_tab_lazy(
        &mut self,
        id: String,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut App,
        build: Box<dyn FnOnce(&mut Window, &mut App) -> TabItem>,
    ) -> bool {
        let tab_container = self.tab_container.clone();
        window.defer(cx, move |window, cx| {
            tab_container.update(cx, |container, cx| {
                container.activate_or_add_tab_lazy_with_mode(
                    id,
                    mode,
                    move |window, cx| build(window, cx),
                    window,
                    cx,
                );
            });
        });
        true
    }
}

impl HomeContext for DataHome {
    fn connections(&self) -> &[StoredConnection] {
        &self.connections
    }

    fn workspace(&self, id: i64) -> Option<Workspace> {
        self.workspaces
            .iter()
            .find(|workspace| workspace.id == Some(id))
            .cloned()
    }

    fn sort_connections(&self, connections: &mut [StoredConnection], cx: &App) {
        let order = AppSettings::global(cx).connection_sort_order;
        crate::sort::sort_connections(connections, order);
    }

    fn database_open_mode(&self, cx: &App) -> DatabaseTabMode {
        match AppSettings::global(cx).database_open_mode {
            DatabaseOpenMode::Single => DatabaseTabMode::Single,
            DatabaseOpenMode::Workspace => DatabaseTabMode::Workspace,
        }
    }

    fn next_tab_index(&self, _base_title: &str, cx: &App) -> Option<usize> {
        Some(self.tab_container.read(cx).tabs().len())
    }

    fn keep_subscription(&mut self, subscription: Subscription) {
        self.subscriptions.push(subscription);
    }

    fn resolve_connection_credentials(
        &mut self,
        connection: &StoredConnection,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<StoredConnection> {
        let storage = cx.global::<GlobalStorageState>().storage.clone();
        let Some(repository) = storage.get::<ConnectionRepository>() else {
            self.notify(NotifyLevel::Error, "连接仓库未初始化", window, cx);
            return None;
        };
        match repository.resolve_runtime_connection(connection) {
            Ok(connection) => Some(connection),
            Err(error) => {
                self.notify(
                    NotifyLevel::Error,
                    &format!("无法解析连接凭据：{error:#}"),
                    window,
                    cx,
                );
                None
            }
        }
    }

    fn port_forwarding_runtime(&self) -> Option<Arc<tokio::sync::Mutex<PortForwardingRuntime>>> {
        None
    }

    fn notify(&mut self, level: NotifyLevel, message: &str, window: &mut Window, cx: &mut App) {
        let notification = match level {
            NotifyLevel::Info => gpui_component::notification::Notification::info(message),
            NotifyLevel::Warning => gpui_component::notification::Notification::warning(message),
            NotifyLevel::Error => gpui_component::notification::Notification::error(message),
        };
        window.push_notification(notification, cx);
    }
}
