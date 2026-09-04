//! MQTT 主标签页视图
//!
//! 布局:左侧连接树(固定宽度 ~260,不做拖拽分隔)+ 右侧 TabContainer,
//! 默认打开一个 SubscribeView 操作页(tab id "mqtt-messages")。
//!
//! 事件联动:
//! - 树 ConnectionEstablished/ConnectionClosed -> SubscribeView 启停消息流
//! - 树 SubscriptionAdded/SubscriptionRemoved -> SubscribeView 刷新订阅列表
//! - SubscribeView SubscriptionsChanged -> 树刷新订阅子节点

use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, Styled, Subscription, Task, Window, div, px,
};
use gpui_component::{ActiveTheme, Icon, IconName, IconSize, Sizable, h_flex};
use one_core::gpui_tokio::Tokio;
use one_core::storage::{ActiveConnections, StoredConnection, Workspace};
use one_core::tab_container::{TabContainer, TabContent, TabContentEvent, TabItem};
use tracing::warn;

use crate::manager::GlobalMqttState;
use crate::mqtt_tree_view::{MqttTreeView, MqttTreeViewEvent};
use crate::subscribe_view::{MqttSubscribeView, MqttSubscribeViewEvent};

/// 左侧树面板固定宽度
const TREE_PANEL_WIDTH: f32 = 260.0;

/// MQTT 标签页视图
pub struct MqttTabView {
    /// 连接列表
    connections: Vec<StoredConnection>,
    /// 活跃连接 ID
    active_connection_id: Option<i64>,
    /// 连接树
    tree_view: Entity<MqttTreeView>,
    /// 标签容器
    tab_container: Entity<TabContainer>,
    /// 默认操作页(订阅/消息/发布)
    subscribe_view: Entity<MqttSubscribeView>,
    /// 工作区信息
    workspace: Option<Workspace>,
    /// 焦点句柄
    focus_handle: FocusHandle,
    /// 订阅句柄
    _subscriptions: Vec<Subscription>,
}

impl MqttTabView {
    pub fn new_with_active_conn(
        workspace: Option<Workspace>,
        connections: Vec<StoredConnection>,
        active_conn_id: Option<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let tree_view = cx.new(|cx| MqttTreeView::new_with_connections(&connections, window, cx));
        let tab_container =
            cx.new(|cx| TabContainer::new(window, cx).with_background_task_panel(false));

        let active_connection = connections
            .iter()
            .find(|connection| connection.id == active_conn_id)
            .cloned()
            .or_else(|| connections.first().cloned());
        let active_connection_id =
            active_conn_id.or_else(|| active_connection.as_ref().and_then(|conn| conn.id));

        // 默认操作页绑定活跃连接
        let subscribe_connection = active_connection.clone().unwrap_or_else(|| {
            StoredConnection::new_mqtt("MQTT".to_string(), Default::default(), None)
        });
        let subscribe_view = cx.new(|cx| MqttSubscribeView::new(subscribe_connection, window, cx));

        // 默认页签:mqtt-messages
        tab_container.update(cx, |container, cx| {
            container.add_and_activate_tab_with_focus(
                TabItem::new("mqtt-messages", "mqtt", subscribe_view.clone()),
                window,
                cx,
            );
        });

        let mut subscriptions = Vec::new();
        // 树事件 -> 操作页联动
        subscriptions.push(cx.subscribe_in(
            &tree_view,
            window,
            |this, tree, event: &MqttTreeViewEvent, _window, cx| match event {
                MqttTreeViewEvent::ConnectionEstablished { connection_id } => {
                    let same_binding =
                        this.subscribe_view.read(cx).connection_id() == connection_id.as_str();
                    if same_binding {
                        this.subscribe_view.update(cx, |view, cx| {
                            view.start_message_stream(cx);
                            view.refresh_subscriptions(cx);
                        });
                    } else if let Some(stored) =
                        tree.read(cx).get_stored_connection(connection_id).cloned()
                    {
                        this.subscribe_view.update(cx, |view, cx| {
                            view.bind_connection(connection_id.clone(), stored, cx);
                            view.start_message_stream(cx);
                        });
                    }
                }
                MqttTreeViewEvent::ConnectionClosed { connection_id } => {
                    if this.subscribe_view.read(cx).connection_id() == connection_id.as_str() {
                        this.subscribe_view.update(cx, |view, cx| {
                            view.stop_message_stream(cx);
                        });
                    }
                }
                MqttTreeViewEvent::SubscriptionAdded { connection_id, .. }
                | MqttTreeViewEvent::SubscriptionRemoved { connection_id, .. } => {
                    if this.subscribe_view.read(cx).connection_id() == connection_id.as_str() {
                        this.subscribe_view.update(cx, |view, cx| {
                            view.refresh_subscriptions(cx);
                        });
                    }
                }
            },
        ));
        // 操作页订阅变化 -> 树刷新
        subscriptions.push(cx.subscribe(
            &subscribe_view,
            |this, _view, event: &MqttSubscribeViewEvent, cx| {
                let MqttSubscribeViewEvent::SubscriptionsChanged { connection_id } = event;
                let tree = this.tree_view.clone();
                let connection_id = connection_id.clone();
                tree.update(cx, |tree, cx| tree.load_subscriptions(connection_id, cx));
            },
        ));

        // 激活连接:选中并自动连接
        if let Some(active_connection_id) = active_connection_id {
            tree_view.update(cx, |tree_view, cx| {
                tree_view.active_connection(active_connection_id.to_string(), cx);
            });
        }

        Self {
            connections,
            active_connection_id,
            tree_view,
            tab_container,
            subscribe_view,
            workspace,
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// 便捷构造:单连接直接打开
    pub fn new(connection: StoredConnection, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let active_conn_id = connection.id;
        Self::new_with_active_conn(None, vec![connection], active_conn_id, window, cx)
    }

    fn active_connection(&self) -> Option<&StoredConnection> {
        if let Some(active_conn_id) = self.active_connection_id {
            self.connections
                .iter()
                .find(|conn| conn.id == Some(active_conn_id))
                .or_else(|| self.connections.first())
        } else {
            self.connections.first()
        }
    }
}

impl Focusable for MqttTabView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.tab_container.focus_handle(cx)
    }
}

impl EventEmitter<TabContentEvent> for MqttTabView {}

impl TabContent for MqttTabView {
    fn content_key(&self) -> &'static str {
        "MQTT"
    }

    fn title(&self, _cx: &App) -> SharedString {
        if let Some(workspace) = &self.workspace {
            workspace.name.clone().into()
        } else {
            self.active_connection()
                .map(|connection| connection.name.clone())
                .unwrap_or_else(|| "MQTT".to_string())
                .into()
        }
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        if self.workspace.is_some() {
            Some(
                Icon::new(IconName::AppsColor)
                    .with_size(IconSize::Medium)
                    .color(),
            )
        } else {
            Some(
                Icon::default()
                    .path(one_core::storage::NAVOP_MQTT_COLOR_ICON)
                    .color()
                    .with_size(IconSize::Medium),
            )
        }
    }

    fn closeable(&self, _cx: &App) -> bool {
        true
    }

    fn try_close(
        &mut self,
        _tab_id: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        let connections = self.connections.clone();
        let global_state = cx.global::<GlobalMqttState>().clone();

        cx.spawn(async move |_this, cx: &mut gpui::AsyncApp| {
            for connection in &connections {
                let connection_id = connection.id.map(|id| id.to_string()).unwrap_or_default();
                if connection_id.is_empty() {
                    continue;
                }

                let connection_id_clone = connection_id.clone();
                let result = Tokio::spawn_result(cx, {
                    let global_state = global_state.clone();
                    async move {
                        global_state
                            .remove_connection(&connection_id_clone)
                            .await
                            .map_err(anyhow::Error::new)
                    }
                })
                .await;

                if let Err(error) = result {
                    warn!(
                        "Failed to close mqtt connection {}: {}",
                        connection_id, error
                    );
                }
            }
            let _ = cx.update(|cx| {
                let global_state = cx.global_mut::<ActiveConnections>();
                for connection in &connections {
                    if let Some(id) = connection.id {
                        global_state.remove(id);
                    }
                }
            });
            true
        })
    }
}

impl Render for MqttTabView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let border_color = cx.theme().border;

        div()
            .id("mqtt-tab-view")
            .track_focus(&self.focus_handle)
            .size_full()
            .child(
                h_flex()
                    .size_full()
                    .child(
                        div()
                            .h_full()
                            .w(px(TREE_PANEL_WIDTH))
                            .flex_shrink_0()
                            .border_r_1()
                            .border_color(border_color)
                            .child(self.tree_view.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .min_w_0()
                            .child(self.tab_container.clone()),
                    ),
            )
    }
}
