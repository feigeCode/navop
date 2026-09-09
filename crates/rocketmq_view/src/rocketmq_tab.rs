//! RocketMQ 主标签页视图
//!
//! 布局:左侧连接树(固定宽度 ~260,不做拖拽分隔)+ 右侧 TabContainer。
//!
//! 事件联动:
//! - 树 ConnectionEstablished -> 追加标准四页管理页签组(概览/Topic/订阅组/消息查询)
//! - 树 ConnectionClosed -> 移除该连接的全部标准页签

use std::collections::HashSet;

use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, Styled, Subscription, Task, Window, div, px,
};
use gpui_component::{ActiveTheme, Icon, IconName, IconSize, Sizable, h_flex};
use middleware_view::MiddlewarePages;
use one_core::gpui_tokio::Tokio;
use one_core::storage::{ActiveConnections, StoredConnection, Workspace};
use one_core::tab_container::{TabContainer, TabContent, TabContentEvent};
use tracing::warn;

use crate::manager::GlobalRocketmqState;
use crate::rocketmq_tree_view::{RocketmqTreeView, RocketmqTreeViewEvent};

/// 左侧树面板固定宽度
const TREE_PANEL_WIDTH: f32 = 260.0;

/// RocketMQ 标签页视图
pub struct RocketmqTabView {
    /// 连接列表
    connections: Vec<StoredConnection>,
    /// 活跃连接 ID
    active_connection_id: Option<i64>,
    /// 连接树
    tree_view: Entity<RocketmqTreeView>,
    /// 标签容器
    tab_container: Entity<TabContainer>,
    /// 已追加标准管理页签组的连接 ID(幂等防重)
    middleware_tab_conns: HashSet<String>,
    /// 工作区信息
    workspace: Option<Workspace>,
    /// 焦点句柄
    focus_handle: FocusHandle,
    /// 订阅句柄
    _subscriptions: Vec<Subscription>,
}

impl RocketmqTabView {
    /// 创建 RocketMQ 主标签页视图
    ///
    /// - `workspace`:工作区信息(工作区聚合模式下用于页签标题与连接过滤)
    /// - `connections`:待展示的连接列表(进入左侧连接树)
    /// - `active_conn_id`:打开时激活并自动连接的连接 ID
    /// - `window`/`cx`:GPUI 窗口与上下文
    ///
    /// 返回的视图左侧为连接树,右侧为 TabContainer;
    /// 连接建立后自动追加标准四页管理页签组。
    pub fn new_with_active_conn(
        workspace: Option<Workspace>,
        connections: Vec<StoredConnection>,
        active_conn_id: Option<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let tree_view =
            cx.new(|cx| RocketmqTreeView::new_with_connections(&connections, window, cx));
        let tab_container =
            cx.new(|cx| TabContainer::new(window, cx).with_background_task_panel(false));

        let active_connection = connections
            .iter()
            .find(|connection| connection.id == active_conn_id)
            .cloned()
            .or_else(|| connections.first().cloned());
        let active_connection_id =
            active_conn_id.or_else(|| active_connection.as_ref().and_then(|conn| conn.id));

        let mut subscriptions = Vec::new();
        // 树事件 -> 页签联动
        subscriptions.push(cx.subscribe_in(
            &tree_view,
            window,
            |this, _tree, event: &RocketmqTreeViewEvent, window, cx| match event {
                RocketmqTreeViewEvent::ConnectionEstablished { connection_id } => {
                    // 连接建立后追加标准管理页签组(RocketMQ 能力位全开,四页齐备)
                    this.add_middleware_tabs(connection_id, window, cx);
                }
                RocketmqTreeViewEvent::ConnectionClosed { connection_id } => {
                    // 连接断开后移除该连接的标准管理页签组
                    this.remove_middleware_tabs(connection_id, window, cx);
                }
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
            middleware_tab_conns: HashSet::new(),
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

    /// 连接建立后追加标准管理页签组(概览/Topic/订阅组/消息查询,按能力位生成)
    ///
    /// 页签持有该连接的 MiddlewareAdmin 句柄;幂等:同一连接只追加一次。
    fn add_middleware_tabs(
        &mut self,
        connection_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if connection_id.is_empty() || self.middleware_tab_conns.contains(connection_id) {
            return;
        }
        let Some(handle) = cx
            .global::<GlobalRocketmqState>()
            .get_admin_handle(connection_id)
        else {
            return;
        };
        let tabs = MiddlewarePages::new(handle, connection_id, window, cx);
        if tabs.is_empty() {
            return;
        }
        self.tab_container.update(cx, |container, cx| {
            for tab in tabs {
                container.add_tab(tab, cx);
            }
        });
        self.middleware_tab_conns.insert(connection_id.to_string());
    }

    /// 连接断开后移除该连接的标准管理页签组
    fn remove_middleware_tabs(
        &mut self,
        connection_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.middleware_tab_conns.remove(connection_id) {
            return;
        }
        self.tab_container.update(cx, |container, cx| {
            for suffix in ["overview", "topics", "groups", "messages"] {
                let tab_id = format!("{connection_id}-{suffix}");
                container.close_tab_by_id(&tab_id, window, cx).detach();
            }
        });
    }
}

impl Focusable for RocketmqTabView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.tab_container.focus_handle(cx)
    }
}

impl EventEmitter<TabContentEvent> for RocketmqTabView {}

impl TabContent for RocketmqTabView {
    fn content_key(&self) -> &'static str {
        "RocketMQ"
    }

    fn title(&self, _cx: &App) -> SharedString {
        if let Some(workspace) = &self.workspace {
            workspace.name.clone().into()
        } else {
            self.active_connection()
                .map(|connection| connection.name.clone())
                .unwrap_or_else(|| "RocketMQ".to_string())
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
                    .path(one_core::storage::NAVOP_ROCKETMQ_COLOR_ICON)
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
        let global_state = cx.global::<GlobalRocketmqState>().clone();

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

                if let Err(err) = result {
                    warn!(
                        "Failed to close rocketmq connection {}: {}",
                        connection_id, err
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

impl Render for RocketmqTabView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let border_color = cx.theme().border;

        div()
            .id("rocketmq-tab-view")
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
