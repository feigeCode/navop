use domain_api::{DatabaseTabMode, HomeContext, NotifyLevel, TabHost};
use gpui::{App, AppContext as _, Context, Entity, Subscription, Window};
use gpui_component::WindowExt as _;
use gpui_component::input::{InputEvent, InputState};
use one_core::connection_notifier::{ConnectionDataEvent, get_notifier};
use one_core::settings::{AppSettings, DatabaseOpenMode};
use one_core::storage::traits::Repository;
use one_core::storage::{
    ConnectionRepository, ConnectionType, GlobalStorageState, StoredConnection, Workspace,
    WorkspaceRepository,
};
use one_core::tab_container::{TabContainer, TabItem, TabOpenMode};
use port_forwarding::runtime::PortForwardingRuntime;
use std::sync::Arc;

mod actions;
mod dialogs;
mod render;
mod sidebar;

pub struct DataHome {
    tab_container: Entity<TabContainer>,
    connections: Vec<StoredConnection>,
    workspaces: Vec<Workspace>,
    /// 侧栏搜索框；输入变化时整体重绘，渲染时读当前文本过滤列表。
    search_input: Entity<InputState>,
    subscriptions: Vec<Subscription>,
}

impl DataHome {
    pub fn new(
        tab_container: Entity<TabContainer>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("搜索连接")
                .clean_on_escape()
        });
        // 搜索框只负责把变更同步成一次重绘，过滤逻辑在渲染期读取最新文本。
        cx.subscribe_in(&search_input, window, |_, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        let mut this = Self {
            tab_container,
            connections: Vec::new(),
            workspaces: Vec::new(),
            search_input,
            subscriptions: Vec::new(),
        };
        this.reload_data(cx);
        this.subscribe_connection_events(cx);
        this
    }

    /// 从全局存储重新加载连接与工作区，保留 data 的数据库类类型过滤。
    fn reload_data(&mut self, cx: &mut Context<Self>) {
        let storage = cx.global::<GlobalStorageState>().storage.clone();
        self.connections = storage
            .get::<ConnectionRepository>()
            .and_then(|repo| repo.list().ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|connection| is_data_connection(connection.connection_type))
            .collect();
        self.workspaces = storage
            .get::<WorkspaceRepository>()
            .and_then(|repo| repo.list().ok())
            .unwrap_or_default();
        cx.notify();
    }

    /// 订阅全局连接数据事件：连接或工作区变化后重新加载列表。
    fn subscribe_connection_events(&mut self, cx: &mut Context<Self>) {
        let Some(notifier) = get_notifier(cx) else {
            return;
        };
        let subscription = cx.subscribe(&notifier, |this, _, event: &ConnectionDataEvent, cx| {
            if is_connection_data_changed(event) {
                this.reload_data(cx);
            }
        });
        self.subscriptions.push(subscription);
    }
}

/// data 只展示数据库类连接，其余类型留给 navop-shell。
fn is_data_connection(kind: ConnectionType) -> bool {
    matches!(
        kind,
        ConnectionType::Database | ConnectionType::Redis | ConnectionType::MongoDB
    )
}

/// 与列表刷新相关的连接/工作区事件；其余事件（凭据、Schema 等）不触发重载。
fn is_connection_data_changed(event: &ConnectionDataEvent) -> bool {
    matches!(
        event,
        ConnectionDataEvent::ConnectionCreated { .. }
            | ConnectionDataEvent::ConnectionUpdated { .. }
            | ConnectionDataEvent::ConnectionDeleted { .. }
            | ConnectionDataEvent::WorkspaceCreated { .. }
            | ConnectionDataEvent::WorkspaceUpdated { .. }
            | ConnectionDataEvent::WorkspaceDeleted { .. }
    )
}

/// 名称搜索：空查询返回全部，否则大小写不敏感的包含匹配（`query` 已转小写）。
pub(super) fn matches_query(name: &str, query: &str) -> bool {
    query.is_empty() || name.to_lowercase().contains(query)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_is_case_insensitive_contains() {
        assert!(matches_query("Prod-MySQL", "prod"));
        assert!(matches_query("Prod-MySQL", "sql"));
        assert!(!matches_query("Prod-MySQL", "redis"));
        assert!(matches_query("任意名称", ""));
    }

    #[test]
    fn data_keeps_database_connections_only() {
        assert!(is_data_connection(ConnectionType::Database));
        assert!(is_data_connection(ConnectionType::MongoDB));
        assert!(!is_data_connection(ConnectionType::SshSftp));
        assert!(!is_data_connection(ConnectionType::Rdp));
    }
}
