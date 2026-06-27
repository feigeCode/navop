//! Redis 主标签页视图

use std::ops::Deref;

use crate::GlobalRedisState;
use crate::key_value_view::KeyValueView;
use crate::redis_tool_data::RedisToolKind;
use crate::redis_tool_view::RedisToolView;
use crate::redis_tree_event::RedisEventHandler;
use crate::redis_tree_view::{RedisTreeView, RedisTreeViewEvent};
use crate::sidebar::{RedisSidebar, RedisSidebarEvent};
use crate::{RedisNode, RedisNodeType};
use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, Axis, Bounds, Context, Element, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point,
    Render, SharedString, Style, Styled, Subscription, Task, Window, div, px,
};
use gpui_component::{ActiveTheme, Icon, IconName, Sizable, Size, h_flex};
use one_core::gpui_tokio::Tokio;
use one_core::layout::{
    SIDEBAR_DEFAULT_WIDTH, SIDEBAR_MAX_WIDTH, SIDEBAR_MIN_WIDTH, TOOLBAR_WIDTH,
};
use one_core::storage::{ActiveConnections, StoredConnection, Workspace};
use one_core::tab_container::{
    TabContainer, TabContainerEvent, TabContent, TabContentEvent, TabItem,
};
use one_ui::resize_handle::{HandlePlacement, ResizePanel, resize_handle};
use tracing::warn;

const PANEL_MIN_SIZE: Pixels = px(100.0);
const TREE_PANEL_DEFAULT_SIZE: Pixels = px(250.0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResizingPanel {
    TreePanel,
    Sidebar,
}

/// Redis 标签页视图
pub struct RedisTabView {
    /// 连接列表
    connections: Vec<StoredConnection>,
    /// 活跃连接 ID
    active_connection_id: Option<i64>,
    /// 树形视图
    tree_view: Entity<RedisTreeView>,
    /// 标签容器
    tab_container: Entity<TabContainer>,
    /// 侧边栏
    sidebar: Entity<RedisSidebar>,
    /// 键值视图(默认页签)
    _key_value_view: Entity<KeyValueView>,
    /// 通过右键菜单按需打开的工具页签(Info/Memory/SlowLog/Monitor/PubSub/Chart)
    tool_views: Vec<Entity<RedisToolView>>,
    /// 事件处理器
    _event_handler: Entity<RedisEventHandler>,
    /// 工作区信息
    workspace: Option<Workspace>,
    /// 焦点句柄
    focus_handle: FocusHandle,
    /// 订阅句柄
    _subscriptions: Vec<Subscription>,
    /// 树面板大小
    tree_panel_size: Pixels,
    /// 侧边栏面板大小
    sidebar_panel_size: Pixels,
    /// 正在调整大小的面板
    resizing: Option<ResizingPanel>,
    /// 视图边界
    bounds: Bounds<Pixels>,
}

impl RedisTabView {
    pub fn new_with_active_conn(
        workspace: Option<Workspace>,
        connections: Vec<StoredConnection>,
        active_conn_id: Option<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let tree_view = cx.new(|cx| RedisTreeView::new_with_connections(&connections, window, cx));

        let tab_container = cx.new(|cx| TabContainer::new(window, cx));
        let key_value_view = cx.new(|cx| KeyValueView::new(window, cx));
        let sidebar =
            cx.new(|cx| RedisSidebar::new(connections.clone(), active_conn_id, window, cx));

        let active_connection = connections
            .iter()
            .find(|conn| conn.id == active_conn_id)
            .cloned()
            .or_else(|| connections.first().cloned());

        let active_connection_id =
            active_conn_id.or_else(|| active_connection.as_ref().and_then(|conn| conn.id));

        // 默认只创建键值页签,其余工具页签和 CLI 通过右键菜单按需打开
        tab_container.update(cx, |container, cx| {
            container.add_and_activate_tab_with_focus(
                TabItem::new("key-value", "redis", key_value_view.clone()),
                window,
                cx,
            );
        });

        // 事件处理器负责处理树视图事件，包括连接建立后创建 CLI 视图
        let event_handler = cx.new(|cx| {
            RedisEventHandler::new(
                &tree_view,
                tab_container.clone(),
                key_value_view.clone(),
                window,
                cx,
            )
        });

        let mut subscriptions = Vec::new();
        subscriptions.push(Self::subscribe_tree_events(&tree_view, window, cx));
        subscriptions.push(Self::subscribe_tab_events(&tab_container, cx));
        subscriptions.push(
            cx.subscribe(
                &sidebar,
                |_this, _, event: &RedisSidebarEvent, cx| match event {
                    RedisSidebarEvent::PanelChanged | RedisSidebarEvent::AskAi => {
                        cx.notify();
                    }
                },
            ),
        );

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
            sidebar,
            _key_value_view: key_value_view,
            tool_views: Vec::new(),
            _event_handler: event_handler,
            workspace,
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
            tree_panel_size: TREE_PANEL_DEFAULT_SIZE,
            sidebar_panel_size: SIDEBAR_DEFAULT_WIDTH,
            resizing: None,
            bounds: Bounds::default(),
        }
    }

    pub fn new(connection: StoredConnection, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let active_conn_id = connection.id;
        Self::new_with_active_conn(None, vec![connection], active_conn_id, window, cx)
    }

    fn subscribe_tree_events(
        tree_view: &Entity<RedisTreeView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(
            tree_view,
            window,
            |this, tree, event, window, cx| match event {
                RedisTreeViewEvent::NodeSelected { node_id }
                | RedisTreeViewEvent::KeySelected { node_id }
                | RedisTreeViewEvent::ConnectionEstablished { node_id } => {
                    let Some(node) = tree.read(cx).get_node(node_id).cloned() else {
                        return;
                    };
                    let Some((connection_id, db_index)) = Self::node_connection_context(&node)
                    else {
                        return;
                    };

                    this.active_connection_id = connection_id.parse().ok();
                    let should_refresh =
                        matches!(event, RedisTreeViewEvent::ConnectionEstablished { .. });
                    for view in &this.tool_views {
                        view.update(cx, |view, cx| {
                            view.set_connection(Some(connection_id.clone()), db_index);
                            if should_refresh {
                                view.refresh(cx);
                            }
                        });
                    }
                }
                RedisTreeViewEvent::OpenToolView { node_id, kind } => {
                    let Some(node) = tree.read(cx).get_node(node_id).cloned() else {
                        return;
                    };
                    let Some((connection_id, db_index)) = Self::node_connection_context(&node)
                    else {
                        return;
                    };
                    this.open_tool_view(*kind, connection_id, db_index, window, cx);
                }
                _ => {}
            },
        )
    }

    fn subscribe_tab_events(
        tab_container: &Entity<TabContainer>,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe(
            tab_container,
            |this, _container, event: &TabContainerEvent, cx| {
                if let TabContainerEvent::TabClosed { id } = event {
                    let closed_id = id.clone();
                    this.tool_views
                        .retain(|view| view.read(cx).kind.tab_id() != closed_id.as_str());
                }
            },
        )
    }

    fn open_tool_view(
        &mut self,
        kind: RedisToolKind,
        connection_id: String,
        db_index: u8,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab_id = kind.tab_id();

        let existing = self
            .tool_views
            .iter()
            .find(|v| v.read(cx).kind == kind)
            .cloned();

        let view = if let Some(existing) = existing {
            existing.update(cx, |view, cx| {
                view.set_connection(Some(connection_id), db_index);
                view.refresh(cx);
            });
            existing
        } else {
            let new_view =
                cx.new(|cx| RedisToolView::new(kind, Some(connection_id), db_index, window, cx));
            new_view.update(cx, |view, cx| view.refresh(cx));
            self.tool_views.push(new_view.clone());
            new_view
        };

        self.tab_container.update(cx, |container, cx| {
            container.activate_or_add_tab_lazy(
                tab_id,
                move |_window, _cx| TabItem::new(tab_id, "redis", view),
                window,
                cx,
            );
        });
    }

    fn node_connection_context(node: &RedisNode) -> Option<(String, u8)> {
        match node.node_type {
            RedisNodeType::Connection | RedisNodeType::Database(_) | RedisNodeType::Key(_) => {
                Some((node.connection_id.clone(), node.db_index))
            }
            RedisNodeType::Namespace => Some((node.connection_id.clone(), node.db_index)),
            RedisNodeType::LoadMore => None,
        }
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

    fn render_tree_resize_handle(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity().clone();

        resize_handle::<ResizePanel, ResizePanel>("tree-resize-handle", Axis::Horizontal)
            .placement(HandlePlacement::Left)
            .on_drag(ResizePanel, move |info, _, _, cx| {
                cx.stop_propagation();
                view.update(cx, |view, cx| {
                    view.resizing = Some(ResizingPanel::TreePanel);
                    cx.notify();
                });
                cx.new(|_| info.deref().clone())
            })
    }

    fn render_sidebar_resize_handle(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity().clone();

        resize_handle::<ResizePanel, ResizePanel>("sidebar-resize-handle", Axis::Horizontal)
            .placement(HandlePlacement::Right)
            .on_drag(ResizePanel, move |info, _, _, cx| {
                cx.stop_propagation();
                view.update(cx, |view, cx| {
                    view.resizing = Some(ResizingPanel::Sidebar);
                    cx.notify();
                });
                cx.new(|_| info.deref().clone())
            })
    }

    fn resize(
        &mut self,
        mouse_position: Point<Pixels>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(resizing) = self.resizing else {
            return;
        };

        let available_width = self.bounds.size.width;

        match resizing {
            ResizingPanel::TreePanel => {
                let new_size = mouse_position.x - self.bounds.left();
                let sidebar_visible = self.sidebar.read(cx).is_panel_visible();
                let sidebar_width = if sidebar_visible {
                    self.sidebar_panel_size
                } else {
                    TOOLBAR_WIDTH
                };
                let max_size =
                    (available_width - PANEL_MIN_SIZE - sidebar_width).max(PANEL_MIN_SIZE);
                self.tree_panel_size = new_size.clamp(PANEL_MIN_SIZE, max_size);
            }
            ResizingPanel::Sidebar => {
                let new_size = self.bounds.right() - mouse_position.x;
                let max_size = (available_width - self.tree_panel_size - PANEL_MIN_SIZE)
                    .max(SIDEBAR_MIN_WIDTH);
                self.sidebar_panel_size =
                    new_size.clamp(SIDEBAR_MIN_WIDTH, max_size.min(SIDEBAR_MAX_WIDTH));
            }
        }

        cx.notify();
    }

    fn done_resizing(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.resizing = None;
        cx.notify();
    }
}

impl Focusable for RedisTabView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.tab_container.focus_handle(cx)
    }
}

impl EventEmitter<TabContentEvent> for RedisTabView {}

impl TabContent for RedisTabView {
    fn content_key(&self) -> &'static str {
        "Redis"
    }

    fn title(&self, _cx: &App) -> SharedString {
        if let Some(workspace) = &self.workspace {
            workspace.name.clone().into()
        } else {
            self.active_connection()
                .map(|connection| connection.name.clone())
                .unwrap_or_else(|| "Redis".to_string())
                .into()
        }
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        if self.workspace.is_some() {
            Some(IconName::AppsColor.color().color().with_size(Size::Medium))
        } else {
            Some(Icon::new(IconName::Redis).color().with_size(Size::Medium))
        }
    }

    fn closeable(&self, _cx: &App) -> bool {
        true
    }

    fn on_activate(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_active(true, cx);
        });
    }

    fn on_deactivate(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_active(false, cx);
        });
    }

    fn try_close(
        &mut self,
        _tab_id: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        let connections = self.connections.clone();
        let global_state = cx.global::<GlobalRedisState>().clone();

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
                        "Failed to close redis connection {}: {}",
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

impl Render for RedisTabView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().clone();
        let border_color = cx.theme().border;
        let tree_panel_size = self.tree_panel_size;
        let sidebar_visible = self.sidebar.read(cx).is_panel_visible();
        let sidebar_panel_size = self.sidebar_panel_size;

        div()
            .id("redis-tab-view")
            .track_focus(&self.focus_handle)
            .size_full()
            .child(
                h_flex()
                    .size_full()
                    .child(
                        div()
                            .relative()
                            .h_full()
                            .w(tree_panel_size)
                            .flex_shrink_0()
                            .border_r_1()
                            .border_color(border_color)
                            .child(self.tree_view.clone())
                            .child(self.render_tree_resize_handle(window, cx)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .min_w_0()
                            .child(self.tab_container.clone()),
                    )
                    .when(sidebar_visible, |this| {
                        this.child(
                            div()
                                .relative()
                                .h_full()
                                .w(sidebar_panel_size)
                                .flex_shrink_0()
                                .child(self.render_sidebar_resize_handle(window, cx))
                                .child(self.sidebar.clone()),
                        )
                    })
                    .when(!sidebar_visible, |this| this.child(self.sidebar.clone()))
                    .child(ResizeEventHandler { view }),
            )
    }
}

struct ResizeEventHandler {
    view: Entity<RedisTabView>,
}

impl IntoElement for ResizeEventHandler {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for ResizeEventHandler {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        (window.request_layout(Style::default(), None, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let bounds = window.bounds();
        self.view.update(cx, |view, _| {
            view.bounds = Bounds {
                origin: Point::default(),
                size: bounds.size,
            };
        });
    }

    fn paint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.on_mouse_event({
            let view = self.view.clone();
            let resizing = view.read(cx).resizing;
            move |e: &MouseMoveEvent, phase, window, cx| {
                if resizing.is_none() {
                    return;
                }
                if !phase.bubble() {
                    return;
                }
                view.update(cx, |view, cx| view.resize(e.position, window, cx));
            }
        });

        window.on_mouse_event({
            let view = self.view.clone();
            move |_: &MouseUpEvent, phase, window, cx| {
                if phase.bubble() {
                    view.update(cx, |view, cx| view.done_resizing(window, cx));
                }
            }
        });
    }
}
