//! MongoDB 主标签页视图

use std::ops::Deref;

use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, Axis, Bounds, Context, Element, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point,
    Render, SharedString, Style, Styled, Subscription, Task, Window, div, px,
};
use gpui_component::{ActiveTheme, Icon, IconName, Sizable, Size, h_flex};
use one_core::gpui_tokio::Tokio;
use one_core::sidebar_contribution::{
    SidebarContribution, SidebarPanelChrome, SidebarPanelId, SidebarPanelPolicy, SidebarPanelSize,
    SidebarPanelStyle, SidebarPlacement, SidebarPlacementSet,
};
use one_core::storage::{ActiveConnections, StoredConnection, Workspace};
use one_core::tab_container::{TabContainer, TabContent, TabContentEvent, TabItem};
use one_ui::resize_handle::{HandlePlacement, ResizePanel, resize_handle};
use tracing::warn;

use crate::GlobalMongoState;
use crate::collection_view::CollectionView;
use crate::mongo_tree_event::MongoEventHandler;
use crate::mongo_tree_view::MongoTreeView;
use crate::sidebar::{MongoSidebar, MongoSidebarEvent};
use one_core::layout::{
    SIDEBAR_DEFAULT_WIDTH, SIDEBAR_MAX_WIDTH, SIDEBAR_MIN_WIDTH, TOOLBAR_WIDTH,
};

const PANEL_MIN_SIZE: Pixels = px(100.0);
const TREE_PANEL_DEFAULT_SIZE: Pixels = px(250.0);

fn mongo_tools_sidebar_policy() -> SidebarPanelPolicy {
    SidebarPanelPolicy {
        hideable: true,
        movable: true,
        allowed_placements: SidebarPlacementSet::all(),
        initially_visible: true,
    }
}

fn mongo_tools_sidebar_size(panel_visible: bool, panel_size: Pixels) -> SidebarPanelSize {
    if panel_visible {
        SidebarPanelSize {
            side_width: Some(panel_size + TOOLBAR_WIDTH),
            bottom_height: Some(panel_size),
        }
    } else {
        SidebarPanelSize {
            side_width: Some(TOOLBAR_WIDTH),
            bottom_height: Some(TOOLBAR_WIDTH),
        }
    }
}

fn mongo_tools_sidebar_chrome() -> SidebarPanelChrome {
    SidebarPanelChrome::HostNoHeader
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResizingPanel {
    TreePanel,
    Sidebar,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MongoSidebarRenderMode {
    Embedded,
    External,
}

/// MongoDB 标签页视图
pub struct MongoTabView {
    connections: Vec<StoredConnection>,
    active_connection_id: Option<i64>,
    tree_view: Entity<MongoTreeView>,
    tab_container: Entity<TabContainer>,
    sidebar: Entity<MongoSidebar>,
    _event_handler: Entity<MongoEventHandler>,
    workspace: Option<Workspace>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
    tree_panel_size: Pixels,
    sidebar_panel_size: Pixels,
    sidebar_render_mode: MongoSidebarRenderMode,
    resizing: Option<ResizingPanel>,
    bounds: Bounds<Pixels>,
}

impl MongoTabView {
    pub fn new_with_active_conn(
        workspace: Option<Workspace>,
        connections: Vec<StoredConnection>,
        active_connection_id: Option<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let tree_view = cx.new(|cx| MongoTreeView::new_with_connections(&connections, window, cx));
        let tab_container = cx.new(|cx| TabContainer::new(window, cx));
        let collection_view = cx.new(|cx| CollectionView::new(window, cx));
        let sidebar =
            cx.new(|cx| MongoSidebar::new(connections.clone(), active_connection_id, window, cx));

        tab_container.update(cx, |container, cx| {
            let view = collection_view.clone();
            let tab = TabItem::new("mongo-documents", "mongodb", view);
            container.add_and_activate_tab_with_focus(tab, window, cx);
        });

        let event_handler = cx.new(|cx| {
            MongoEventHandler::new(
                &tree_view,
                tab_container.clone(),
                collection_view.clone(),
                window,
                cx,
            )
        });

        let mut subscriptions = Vec::new();
        subscriptions.push(
            cx.subscribe(
                &sidebar,
                |_this, _, event: &MongoSidebarEvent, cx| match event {
                    MongoSidebarEvent::PanelChanged | MongoSidebarEvent::AskAi => {
                        cx.notify();
                    }
                },
            ),
        );

        let active_connection = connections
            .iter()
            .find(|connection| connection.id == active_connection_id)
            .cloned()
            .or_else(|| connections.first().cloned());
        let resolved_active_connection_id = active_connection_id.or_else(|| {
            active_connection
                .as_ref()
                .and_then(|connection| connection.id)
        });

        if let Some(active_connection_id) = resolved_active_connection_id {
            tree_view.update(cx, |tree_view, cx| {
                tree_view.active_connection(active_connection_id.to_string(), cx);
            });
        }

        Self {
            connections,
            active_connection_id: resolved_active_connection_id,
            tree_view,
            tab_container,
            sidebar,
            _event_handler: event_handler,
            workspace,
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
            tree_panel_size: TREE_PANEL_DEFAULT_SIZE,
            sidebar_panel_size: SIDEBAR_DEFAULT_WIDTH,
            sidebar_render_mode: MongoSidebarRenderMode::Embedded,
            resizing: None,
            bounds: Bounds::default(),
        }
    }

    pub fn with_external_sidebar(mut self) -> Self {
        self.sidebar_render_mode = MongoSidebarRenderMode::External;
        self
    }

    fn active_connection(&self) -> Option<&StoredConnection> {
        self.active_connection_id.and_then(|id| {
            self.connections
                .iter()
                .find(|connection| connection.id == Some(id))
        })
    }

    fn render_tree_resize_handle(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity().clone();

        resize_handle::<ResizePanel, ResizePanel>("mongo-tree-resize-handle", Axis::Horizontal)
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

        resize_handle::<ResizePanel, ResizePanel>("mongo-sidebar-resize-handle", Axis::Horizontal)
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

impl Focusable for MongoTabView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.tab_container.focus_handle(cx)
    }
}

impl EventEmitter<TabContentEvent> for MongoTabView {}

impl TabContent for MongoTabView {
    fn content_key(&self) -> &'static str {
        "MongoDB"
    }

    fn title(&self, _cx: &App) -> SharedString {
        if let Some(workspace) = &self.workspace {
            workspace.name.clone().into()
        } else {
            self.active_connection()
                .map(|connection| connection.name.clone())
                .unwrap_or_else(|| "MongoDB".to_string())
                .into()
        }
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        if self.workspace.is_some() {
            Some(IconName::AppsColor.color().with_size(Size::Medium))
        } else {
            Some(Icon::new(IconName::MongoDB).color().with_size(Size::Medium))
        }
    }

    fn closeable(&self, _cx: &App) -> bool {
        true
    }

    fn sidebar_contributions(&self, _cx: &App) -> Vec<SidebarContribution> {
        if self.sidebar_render_mode != MongoSidebarRenderMode::External {
            return Vec::new();
        }

        vec![
            SidebarContribution {
                id: SidebarPanelId::new(self.tree_view.entity_id(), "mongodb.tree"),
                title: SharedString::from("MongoDB"),
                icon: IconName::MongoDB,
                view: self.tree_view.clone().into(),
                default_placement: SidebarPlacement::Left,
                policy: SidebarPanelPolicy {
                    hideable: false,
                    movable: true,
                    allowed_placements: SidebarPlacementSet::left_right(),
                    initially_visible: true,
                },
                style: SidebarPanelStyle::default(),
                size: SidebarPanelSize {
                    side_width: Some(self.tree_panel_size),
                    bottom_height: None,
                },
                chrome: SidebarPanelChrome::HostNoHeader,
                actions: Default::default(),
            },
            SidebarContribution {
                id: SidebarPanelId::new(self.sidebar.entity_id(), "mongodb.sidebar"),
                title: SharedString::from("MongoDB Tools"),
                icon: IconName::Bot,
                view: self.sidebar.clone().into(),
                default_placement: SidebarPlacement::Right,
                policy: mongo_tools_sidebar_policy(),
                style: SidebarPanelStyle::default(),
                size: mongo_tools_sidebar_size(
                    self.sidebar.read(_cx).is_panel_visible(),
                    self.sidebar_panel_size,
                ),
                chrome: mongo_tools_sidebar_chrome(),
                actions: Default::default(),
            },
        ]
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
        let global_state = cx.global::<GlobalMongoState>().clone();

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
                            .map_err(|e| anyhow::anyhow!("{}", e))
                    }
                })
                .await;

                if let Err(error) = result {
                    warn!(
                        "Failed to close mongodb connection {}: {}",
                        connection_id, error
                    );
                }

                if let Ok(connection_id_value) = connection_id.parse::<i64>() {
                    let _ = cx.update(|cx| {
                        cx.global_mut::<ActiveConnections>()
                            .remove(connection_id_value);
                    });
                }
            }

            true
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mongo_tools_sidebar_keeps_toolbar_visible_by_default() {
        let policy = mongo_tools_sidebar_policy();

        assert!(policy.hideable);
        assert!(policy.initially_visible);
    }

    #[test]
    fn mongo_tools_sidebar_uses_toolbar_width_until_panel_opens() {
        let collapsed = mongo_tools_sidebar_size(false, SIDEBAR_DEFAULT_WIDTH);
        let expanded = mongo_tools_sidebar_size(true, SIDEBAR_DEFAULT_WIDTH);

        assert_eq!(Some(TOOLBAR_WIDTH), collapsed.side_width);
        assert_eq!(Some(TOOLBAR_WIDTH), collapsed.bottom_height);
        assert_eq!(
            Some(SIDEBAR_DEFAULT_WIDTH + TOOLBAR_WIDTH),
            expanded.side_width
        );
        assert_eq!(Some(SIDEBAR_DEFAULT_WIDTH), expanded.bottom_height);
    }

    #[test]
    fn mongo_tools_sidebar_does_not_render_host_header() {
        assert_eq!(
            SidebarPanelChrome::HostNoHeader,
            mongo_tools_sidebar_chrome()
        );
    }
}

impl Render for MongoTabView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().clone();
        let border_color = cx.theme().border;
        let tree_panel_size = self.tree_panel_size;
        let sidebar_visible = self.sidebar.read(cx).is_panel_visible();
        let sidebar_panel_size = self.sidebar_panel_size;

        if self.sidebar_render_mode == MongoSidebarRenderMode::External {
            return div()
                .id("mongodb-tab-view")
                .track_focus(&self.focus_handle)
                .size_full()
                .child(self.tab_container.clone());
        }

        div()
            .id("mongodb-tab-view")
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
    view: Entity<MongoTabView>,
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
        _cx: &mut App,
    ) -> Self::PrepaintState {
        let bounds = window.bounds();
        self.view.update(_cx, |view, _| {
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
