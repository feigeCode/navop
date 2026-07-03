use crate::layout::TOOLBAR_WIDTH;
use crate::sidebar_contribution::{
    SidebarContribution, SidebarPanelChrome, SidebarPanelId, SidebarPanelPolicy, SidebarPlacement,
    sidebar_panel_renders_header,
};
use gpui::prelude::FluentBuilder;
use gpui::{
    Anchor, AnyElement, AnyView, App, AppContext as _, Bounds, Context, Decorations, DragMoveEvent,
    Element, ElementId, Empty, Entity, EntityId, EventEmitter, FocusHandle, Focusable,
    GlobalElementId, InspectorElementId, InteractiveElement, IntoElement, LayoutId, MouseButton,
    MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, Render, RenderOnce, SharedString,
    Style, Styled, Subscription, Task, Window, WindowControlArea, div, px, relative,
};
use gpui::{ScrollHandle, StatefulInteractiveElement as _};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::list::{List, ListDelegate, ListState};
use gpui_component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_component::popover::Popover;
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, IndexPath, InteractiveElementExt as _, Placement,
    Selectable, Sizable, Size, h_flex, v_flex,
};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

const SIDEBAR_PANEL_MIN_SIZE: Pixels = px(120.0);
const SIDEBAR_CENTER_MIN_SIZE: Pixels = px(160.0);
const SIDEBAR_SIDE_DEFAULT_WIDTH: Pixels = px(320.0);
const SIDEBAR_BOTTOM_DEFAULT_HEIGHT: Pixels = px(260.0);
const SIDEBAR_HANDLE_PADDING: Pixels = px(4.0);
const SIDEBAR_HANDLE_SIZE: Pixels = px(1.0);

// ============================================================================
// TabContainer Events
// ============================================================================

pub(crate) fn split_command_enabled(
    container_split_enabled: bool,
    tab_can_split: bool,
    source_tab_count: usize,
) -> bool {
    container_split_enabled && tab_can_split && source_tab_count > 1
}

pub(crate) fn active_content_can_split_for_layout(
    pinned_tab_active: bool,
    pinned_tab_can_split: Option<bool>,
    active_tab_can_split: Option<bool>,
) -> bool {
    if pinned_tab_active {
        pinned_tab_can_split.unwrap_or(false)
    } else {
        active_tab_can_split.unwrap_or(false)
    }
}

pub(crate) fn sidebar_panel_initial_visibility(policy: SidebarPanelPolicy) -> bool {
    policy.initially_visible || !policy.hideable
}

pub(crate) fn sidebar_panel_uses_exclusive_slot(chrome: SidebarPanelChrome) -> bool {
    chrome != SidebarPanelChrome::None
}

pub(crate) fn sidebar_panel_allows_resize(
    chrome: SidebarPanelChrome,
    side_width: Option<Pixels>,
    bottom_height: Option<Pixels>,
) -> bool {
    if chrome == SidebarPanelChrome::None {
        return false;
    }

    side_width.is_some_and(|width| width > TOOLBAR_WIDTH)
        || bottom_height.is_some_and(|height| height > TOOLBAR_WIDTH)
}

pub(crate) fn sidebar_panel_allows_size_override(base_size: Option<Pixels>) -> bool {
    base_size.is_none_or(|size| size > TOOLBAR_WIDTH)
}

pub(crate) fn sidebar_panel_should_hide_for_exclusive_target(
    visible: bool,
    placement: SidebarPlacement,
    hideable: bool,
    chrome: SidebarPanelChrome,
    target_placement: SidebarPlacement,
) -> bool {
    visible
        && placement == target_placement
        && hideable
        && sidebar_panel_uses_exclusive_slot(chrome)
}

pub(crate) fn sidebar_panel_blocks_exclusive_target(
    visible: bool,
    placement: SidebarPlacement,
    hideable: bool,
    chrome: SidebarPanelChrome,
    target_placement: SidebarPlacement,
) -> bool {
    visible
        && placement == target_placement
        && !hideable
        && sidebar_panel_uses_exclusive_slot(chrome)
}

/// Events emitted by TabContent
#[derive(Debug, Clone)]
pub enum TabContentEvent {
    /// Tab state changed
    StateChanged,
}

/// Events emitted by TabContainer
#[derive(Debug, Clone)]
pub enum TabContainerEvent {
    /// Layout has changed (tabs added, removed, reordered, or active index changed)
    LayoutChanged,
    /// A tab was activated
    TabActivated { index: usize, id: String },
    /// A tab was closed
    TabClosed { id: String },
    /// 请求分屏：将 `source` pane 中 `tab_index` 处的 tab
    /// 移动到当前 pane 的 `placement` 方向新建的 pane 中（由上层 SplitTabContainer 处理）
    SplitRequested {
        placement: Placement,
        source: Entity<TabContainer>,
        tab_index: usize,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SidebarPanelOverride {
    visible: bool,
    placement: SidebarPlacement,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct SidebarPanelSizeOverride {
    side_width: Option<Pixels>,
    bottom_height: Option<Pixels>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SidebarResizeTarget {
    id: SidebarPanelId,
    placement: SidebarPlacement,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ResolvedSidebarPanelState {
    visible: bool,
    placement: SidebarPlacement,
}

#[derive(Clone)]
struct ResolvedSidebarContribution {
    contribution: SidebarContribution,
    placement: SidebarPlacement,
    visible: bool,
}

// ============================================================================
// State Serialization Structures
// ============================================================================

/// Serializable state for TabContainer
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TabContainerState {
    /// Version for compatibility checking
    #[serde(default)]
    pub version: Option<usize>,
    /// All tab states
    pub tabs: Vec<TabItemState>,
    /// Currently active tab index
    pub active_index: usize,
    /// Container UI configuration
    #[serde(default)]
    pub config: TabContainerConfig,
}

impl Default for TabContainerState {
    fn default() -> Self {
        Self {
            version: Some(1),
            tabs: Vec::new(),
            active_index: 0,
            config: TabContainerConfig::default(),
        }
    }
}

/// Serializable state for a single tab
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TabItemState {
    /// Unique tab ID
    pub id: SharedString,
    /// Tab From
    pub from: SharedString,
    /// Tab key
    pub key: SharedString,
    /// Tab-level structured metadata for cross-view navigation.
    #[serde(default)]
    pub metadata: HashMap<String, String>,
    /// Tab-specific data (customized by each content type)
    #[serde(default)]
    pub data: serde_json::Value,
}

/// UI configuration for TabContainer
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TabContainerConfig {
    /// Tab size: "xsmall", "small", "medium", "large"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    /// Left padding in pixels
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_padding: Option<f32>,
    /// Top padding in pixels
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_padding: Option<f32>,
}

// ============================================================================
// TabContent Trait - Static Type Interface (like Panel)
// ============================================================================

/// Trait that defines tab content behavior.
/// Implement this on your Entity type (like Panel).
/// Requires: Render + Focusable + EventEmitter<TabContentEvent>
#[allow(unused_variables)]
pub trait TabContent: EventEmitter<TabContentEvent> + Render + Focusable {
    /// Unique key for this content type (used for serialization)
    fn content_key(&self) -> &'static str;

    /// Get the tab title
    fn title(&self, cx: &App) -> SharedString;

    /// Get optional icon for the tab
    fn icon(&self, cx: &App) -> Option<Icon> {
        None
    }

    /// Check if tab can be closed
    fn closeable(&self, cx: &App) -> bool {
        true
    }

    /// Called when tab becomes active
    fn on_activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {}

    /// Called when tab becomes inactive
    fn on_deactivate(&mut self, window: &mut Window, cx: &mut Context<Self>) {}

    /// Try to close this tab. Returns a Task that resolves to true if close succeeded.
    fn try_close(
        &mut self,
        tab_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        Task::ready(true)
    }

    /// Get tab's preferred width size
    fn width_size(&self, cx: &App) -> Option<Size> {
        None
    }

    /// Dump tab state to serializable data
    fn dump(&self, cx: &App) -> serde_json::Value {
        serde_json::Value::Null
    }

    /// Sidebar panels contributed by this tab when it is the active tab.
    fn sidebar_contributions(&self, cx: &App) -> Vec<SidebarContribution> {
        Vec::new()
    }

    /// Whether this tab may be split into a new pane.
    fn can_split(&self, cx: &App) -> bool {
        false
    }
}

// ============================================================================
// TabContentView Trait - Dynamic Type Interface (like PanelView)
// ============================================================================

/// Dynamic trait object interface for TabContent.
/// This allows storing different TabContent types in a single collection.
#[allow(unused_variables)]
pub trait TabContentView: 'static + Send + Sync {
    fn content_key(&self, cx: &App) -> &'static str;
    fn content_id(&self, cx: &App) -> EntityId;
    fn title(&self, cx: &App) -> SharedString;
    fn icon(&self, cx: &App) -> Option<Icon>;
    fn closeable(&self, cx: &App) -> bool;
    fn on_activate(&self, window: &mut Window, cx: &mut App);
    fn on_deactivate(&self, window: &mut Window, cx: &mut App);
    fn try_close(&self, tab_id: &str, window: &mut Window, cx: &mut App) -> Task<bool>;
    fn width_size(&self, cx: &App) -> Option<Size>;
    fn focus_handle(&self, cx: &App) -> FocusHandle;
    fn view(&self) -> AnyView;
    fn dump(&self, cx: &App) -> serde_json::Value;
    fn sidebar_contributions(&self, cx: &App) -> Vec<SidebarContribution>;
    fn can_split(&self, cx: &App) -> bool;
    fn subscribe_events(&self, window: &mut Window, cx: &mut Context<TabContainer>)
    -> Subscription;
}

/// Blanket implementation: Entity<T: TabContent> automatically implements TabContentView
impl<T: TabContent> TabContentView for Entity<T> {
    fn content_key(&self, cx: &App) -> &'static str {
        self.read(cx).content_key()
    }

    fn content_id(&self, _cx: &App) -> EntityId {
        self.entity_id()
    }

    fn title(&self, cx: &App) -> SharedString {
        self.read(cx).title(cx)
    }

    fn icon(&self, cx: &App) -> Option<Icon> {
        self.read(cx).icon(cx)
    }

    fn closeable(&self, cx: &App) -> bool {
        self.read(cx).closeable(cx)
    }

    fn on_activate(&self, window: &mut Window, cx: &mut App) {
        self.update(cx, |this, cx| this.on_activate(window, cx))
    }

    fn on_deactivate(&self, window: &mut Window, cx: &mut App) {
        self.update(cx, |this, cx| this.on_deactivate(window, cx))
    }

    fn try_close(&self, tab_id: &str, window: &mut Window, cx: &mut App) -> Task<bool> {
        let tab_id = tab_id.to_string();
        self.update(cx, |this, cx| this.try_close(&tab_id, window, cx))
    }

    fn width_size(&self, cx: &App) -> Option<Size> {
        self.read(cx).width_size(cx)
    }

    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.read(cx).focus_handle(cx)
    }

    fn view(&self) -> AnyView {
        self.clone().into()
    }

    fn dump(&self, cx: &App) -> serde_json::Value {
        self.read(cx).dump(cx)
    }

    fn sidebar_contributions(&self, cx: &App) -> Vec<SidebarContribution> {
        self.read(cx).sidebar_contributions(cx)
    }

    fn can_split(&self, cx: &App) -> bool {
        self.read(cx).can_split(cx)
    }

    fn subscribe_events(
        &self,
        window: &mut Window,
        cx: &mut Context<TabContainer>,
    ) -> Subscription {
        cx.subscribe_in(
            self,
            window,
            |container, _content, event: &TabContentEvent, _window, cx| {
                container.handle_tab_content_event(event, cx);
            },
        )
    }
}

impl From<&dyn TabContentView> for AnyView {
    fn from(handle: &dyn TabContentView) -> Self {
        handle.view()
    }
}

impl PartialEq for dyn TabContentView {
    fn eq(&self, other: &Self) -> bool {
        self.view() == other.view()
    }
}

// ============================================================================
// TabItem - Represents a single tab with its content
// ============================================================================

pub struct TabItem {
    id: SharedString,
    from: SharedString,
    metadata: HashMap<String, String>,
    content: Arc<dyn TabContentView>,
}

impl TabItem {
    pub fn new<T: TabContent>(
        id: impl Into<String>,
        from: impl Into<String>,
        content: Entity<T>,
    ) -> Self {
        Self {
            id: SharedString::from(id.into()),
            from: SharedString::from(from.into()),
            metadata: HashMap::new(),
            content: Arc::new(content),
        }
    }

    pub fn with_metadata(mut self, metadata: HashMap<String, String>) -> Self {
        self.metadata = metadata;
        self
    }

    pub fn id(&self) -> SharedString {
        self.id.clone()
    }

    pub fn from(&self) -> SharedString {
        self.from.clone()
    }

    pub fn content(&self) -> &Arc<dyn TabContentView> {
        &self.content
    }

    pub fn metadata(&self) -> &HashMap<String, String> {
        &self.metadata
    }
}

// ============================================================================
// TabContentBuilder - Factory trait for rebuilding tabs
// ============================================================================

/// Trait for building TabContent from serialized state
pub trait TabContentBuilder: Send + Sync {
    fn build(
        &self,
        state: &TabItemState,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Arc<dyn TabContentView>>;
}

/// Function-based builder wrapper
pub struct FnTabContentBuilder<F>(pub F);

impl<F> TabContentBuilder for FnTabContentBuilder<F>
where
    F: Fn(&TabItemState, &mut Window, &mut App) -> Option<Arc<dyn TabContentView>> + Send + Sync,
{
    fn build(
        &self,
        state: &TabItemState,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Arc<dyn TabContentView>> {
        self.0(state, window, cx)
    }
}

// ============================================================================
// TabContentRegistry - Registry for rebuilding tabs from state
// ============================================================================

/// Registry for TabContent builders, used to restore tabs from saved state
#[derive(Clone)]
pub struct TabContentRegistry {
    builders: HashMap<SharedString, Arc<dyn TabContentBuilder>>,
}

impl Default for TabContentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl TabContentRegistry {
    pub fn new() -> Self {
        Self {
            builders: HashMap::new(),
        }
    }

    /// Register a builder for a content type
    pub fn register<B: TabContentBuilder + 'static>(
        &mut self,
        content_type: SharedString,
        builder: B,
    ) {
        self.builders.insert(content_type, Arc::new(builder));
    }

    /// Register a builder using a closure
    pub fn register_fn<F>(&mut self, key: SharedString, builder: F)
    where
        F: Fn(&TabItemState, &mut Window, &mut App) -> Option<Arc<dyn TabContentView>>
            + Send
            + Sync
            + 'static,
    {
        self.builders
            .insert(key, Arc::new(FnTabContentBuilder(builder)));
    }

    /// Build a TabContentView from state
    pub fn build(
        &self,
        state: &TabItemState,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Arc<dyn TabContentView>> {
        self.builders.get(&state.key)?.build(state, window, cx)
    }

    /// Check if a builder exists for a content type
    pub fn has_builder(&self, key: &str) -> bool {
        self.builders.contains_key(key)
    }
}

/// Global wrapper for TabContentRegistry
impl gpui::Global for TabContentRegistry {}

// ============================================================================
// TabBarDragState - Window drag state management
// ============================================================================

/// 窗口拖动状态，用于在 Windows 和 Linux 上支持拖动窗口
struct TabBarDragState {
    should_move: bool,
}

impl Render for TabBarDragState {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

// ============================================================================
// DragTab - Visual representation during drag
// ============================================================================

/// Represents a tab being dragged, used for visual feedback
#[derive(Clone)]
pub struct DragTab {
    pub tab_index: usize,
    pub title: SharedString,
    /// 拖拽来源 pane（split 场景下用于跨 pane 移动 tab）
    pub source_pane: Option<Entity<TabContainer>>,
}

impl DragTab {
    pub fn new(tab_index: usize, title: SharedString) -> Self {
        Self {
            tab_index,
            title,
            source_pane: None,
        }
    }

    pub fn with_source_pane(mut self, pane: Entity<TabContainer>) -> Self {
        self.source_pane = Some(pane);
        self
    }
}

impl Render for DragTab {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("drag-tab")
            .cursor_grabbing()
            .py_1()
            .px_3()
            .min_w(px(80.0))
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .border_1()
            .border_color(cx.theme().border)
            .rounded(px(6.0))
            .text_color(cx.theme().tab_foreground)
            .bg(cx.theme().tab_active)
            .opacity(0.85)
            .shadow_md()
            .text_sm()
            .child(self.title.clone())
    }
}

// ============================================================================
// TabListItem - Custom list item for tab dropdown
// ============================================================================

#[derive(IntoElement)]
pub struct TabListItem {
    tab_index: usize,
    title: SharedString,
    icon: Option<Icon>,
    closeable: bool,
    selected: bool,
    container: Entity<TabContainer>,
}

impl TabListItem {
    pub fn new(
        tab_index: usize,
        title: SharedString,
        icon: Option<Icon>,
        closeable: bool,
        selected: bool,
        container: Entity<TabContainer>,
    ) -> Self {
        Self {
            tab_index,
            title,
            icon,
            closeable,
            selected,
            container,
        }
    }
}

impl Selectable for TabListItem {
    fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.selected
    }
}

impl RenderOnce for TabListItem {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let container = self.container.clone();
        let tab_index = self.tab_index;
        let selected = self.selected;
        let drag_border_color = cx.theme().drag_border;
        let drag_title = self.title.clone();

        h_flex()
            .id(SharedString::from(format!("tab-item-{}", tab_index)))
            .w_full()
            .px_2()
            .py_1()
            .rounded(px(4.0))
            .items_center()
            .gap_2()
            .cursor_pointer()
            .when(selected, |el| el.bg(cx.theme().list_active))
            .when(!selected, |el| {
                el.hover(|style| style.bg(cx.theme().list_hover))
            })
            .on_drag(
                DragTab::new(tab_index, drag_title),
                |drag, _, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                    cx.new(|_| drag.clone())
                },
            )
            .drag_over::<DragTab>(move |el, _, _, _cx| {
                el.border_t_2().border_color(drag_border_color)
            })
            .on_drop(
                window.listener_for(&container, move |this, drag: &DragTab, window, cx| {
                    let from_index = drag.tab_index;
                    let to_index = tab_index;
                    if from_index == to_index {
                        return;
                    }
                    this.move_tab(from_index, to_index, cx);
                    this.set_active_index(to_index, window, cx);
                    if let Some(tab_list) = &this.tab_list {
                        let tabs_data: Vec<(usize, SharedString, Option<Icon>, bool)> = this
                            .tabs
                            .iter()
                            .enumerate()
                            .map(|(idx, tab)| {
                                (
                                    idx,
                                    tab.content().title(cx),
                                    tab.content().icon(cx),
                                    tab.content().closeable(cx),
                                )
                            })
                            .collect();
                        tab_list.update(cx, |state, cx| {
                            let delegate = state.delegate_mut();
                            delegate.tabs = tabs_data.clone();
                            delegate.filtered_tabs = tabs_data;
                            cx.notify();
                        });
                    }
                }),
            )
            .when_some(self.icon, |el, icon| {
                el.child(
                    Icon::new(icon)
                        .size_4()
                        .text_color(cx.theme().muted_foreground),
                )
            })
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(self.title),
            )
            .when(self.closeable, |el| {
                let container = container.clone();
                el.child(
                    div()
                        .id(SharedString::from(format!("close-btn-{}", tab_index)))
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(16.0))
                        .h(px(16.0))
                        .rounded(px(2.0))
                        .cursor_pointer()
                        .text_color(cx.theme().muted_foreground)
                        .hover(|style| style.bg(cx.theme().muted).text_color(cx.theme().foreground))
                        .on_mouse_down(MouseButton::Left, move |_event, window, cx| {
                            container.update(cx, |this, cx| {
                                this.close_tab(tab_index, window, cx).detach();
                            });
                        })
                        .child("×"),
                )
            })
    }
}

// ============================================================================
// TabListDelegate - List delegate for tab dropdown
// ============================================================================

pub struct TabListDelegate {
    container: Entity<TabContainer>,
    tabs: Vec<(usize, SharedString, Option<Icon>, bool)>,
    filtered_tabs: Vec<(usize, SharedString, Option<Icon>, bool)>,
    selected_index: Option<IndexPath>,
}

impl ListDelegate for TabListDelegate {
    type Item = TabListItem;

    fn perform_search(
        &mut self,
        query: &str,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Task<()> {
        if query.is_empty() {
            self.filtered_tabs = self.tabs.clone();
        } else {
            let query_lower = query.to_lowercase();
            self.filtered_tabs = self
                .tabs
                .iter()
                .filter(|(_, title, _, _)| title.to_lowercase().contains(&query_lower))
                .cloned()
                .collect();
        }
        cx.notify();
        Task::ready(())
    }

    fn items_count(&self, _section: usize, _cx: &App) -> usize {
        self.filtered_tabs.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let (tab_index, title, icon, closeable) = self.filtered_tabs.get(ix.row)?.clone();
        let active_index = self.container.read(cx).active_index();
        let is_active = tab_index == active_index;

        Some(TabListItem::new(
            tab_index,
            title,
            icon,
            closeable,
            is_active,
            self.container.clone(),
        ))
    }

    fn set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) {
        self.selected_index = ix;
    }

    fn confirm(
        &mut self,
        _secondary: bool,
        window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) {
        if let Some(ix) = self.selected_index {
            if let Some((tab_index, _, _, _)) = self.filtered_tabs.get(ix.row) {
                let tab_index = *tab_index;
                self.container.update(cx, |this, cx| {
                    this.list_popover_open = false;
                    this.set_active_index(tab_index, window, cx);
                });
            }
        }
    }

    fn cancel(&mut self, _window: &mut Window, cx: &mut Context<ListState<Self>>) {
        self.container.update(cx, |this, cx| {
            this.list_popover_open = false;
            cx.notify();
        });
    }
}

// ============================================================================
// TabContainer - Main container component
// ============================================================================

pub struct TabContainer {
    focus_handle: FocusHandle,
    tabs: Vec<TabItem>,
    active_index: usize,
    size: Size,
    show_menu: bool,
    tab_bar_bg_color: Option<gpui::Hsla>,
    tab_bar_border_color: Option<gpui::Hsla>,
    active_tab_bg_color: Option<gpui::Hsla>,
    inactive_tab_hover_color: Option<gpui::Hsla>,
    inactive_tab_bg_color: Option<gpui::Hsla>,
    tab_text_color: Option<gpui::Hsla>,
    tab_close_button_color: Option<gpui::Hsla>,
    left_padding: Option<gpui::Pixels>,
    top_padding: Option<gpui::Pixels>,
    tab_bar_scroll_handle: ScrollHandle,
    list_popover_open: bool,
    tab_list: Option<Entity<ListState<TabListDelegate>>>,
    closing_tabs: HashSet<SharedString>,
    tab_content_subscriptions: Vec<Subscription>,
    show_window_controls: bool,
    /// 窗口置顶切换回调，由上层注入；为 None 时不渲染置顶按钮
    on_toggle_always_on_top: Option<Arc<dyn Fn(&mut Window, &mut App) + Send + Sync>>,
    /// 当前窗口置顶状态读取器，由上层注入
    is_always_on_top: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    /// Pinned tab that stays fixed before the scrollable tab list
    pinned_tab: Option<TabItem>,
    /// Whether the pinned tab is currently active (showing its content)
    pinned_tab_active: bool,
    split_enabled: bool,
    will_split_placement: Option<Placement>,
    sidebar_overrides: HashMap<SidebarPanelId, SidebarPanelOverride>,
    sidebar_size_overrides: HashMap<SidebarPanelId, SidebarPanelSizeOverride>,
    sidebar_resizing: Option<SidebarResizeTarget>,
    sidebar_bounds: Bounds<Pixels>,
}

impl EventEmitter<TabContainerEvent> for TabContainer {}

impl TabContainer {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let _ = window;
        Self {
            focus_handle: cx.focus_handle(),
            tabs: Vec::new(),
            active_index: 0,
            size: Size::Large,
            show_menu: false,
            tab_bar_bg_color: None,
            tab_bar_border_color: None,
            active_tab_bg_color: None,
            inactive_tab_hover_color: None,
            inactive_tab_bg_color: None,
            tab_text_color: None,
            tab_close_button_color: None,
            left_padding: None,
            top_padding: None,
            tab_bar_scroll_handle: ScrollHandle::new(),
            list_popover_open: false,
            tab_list: None,
            closing_tabs: HashSet::new(),
            tab_content_subscriptions: Vec::new(),
            show_window_controls: false,
            on_toggle_always_on_top: None,
            is_always_on_top: None,
            pinned_tab: None,
            pinned_tab_active: false,
            split_enabled: false,
            will_split_placement: None,
            sidebar_overrides: HashMap::new(),
            sidebar_size_overrides: HashMap::new(),
            sidebar_resizing: None,
            sidebar_bounds: Bounds::default(),
        }
    }

    pub fn with_inactive_tab_bg_color(mut self, color: impl Into<Option<gpui::Hsla>>) -> Self {
        self.inactive_tab_bg_color = color.into();
        self
    }

    pub fn with_tab_bar_colors(
        mut self,
        bg_color: impl Into<Option<gpui::Hsla>>,
        border_color: impl Into<Option<gpui::Hsla>>,
    ) -> Self {
        self.tab_bar_bg_color = bg_color.into();
        self.tab_bar_border_color = border_color.into();
        self
    }

    pub fn with_tab_item_colors(
        mut self,
        active_color: impl Into<Option<gpui::Hsla>>,
        hover_color: impl Into<Option<gpui::Hsla>>,
    ) -> Self {
        self.active_tab_bg_color = active_color.into();
        self.inactive_tab_hover_color = hover_color.into();
        self
    }

    pub fn with_tab_content_colors(
        mut self,
        text_color: impl Into<Option<gpui::Hsla>>,
        close_button_color: impl Into<Option<gpui::Hsla>>,
    ) -> Self {
        self.tab_text_color = text_color.into();
        self.tab_close_button_color = close_button_color.into();
        self
    }

    pub fn with_left_padding(mut self, padding: gpui::Pixels) -> Self {
        self.left_padding = Some(padding);
        self
    }

    pub fn with_top_padding(mut self, padding: gpui::Pixels) -> Self {
        self.top_padding = Some(padding);
        self
    }

    pub fn with_window_controls(mut self, show: bool) -> Self {
        self.show_window_controls = show;
        self
    }

    pub fn with_split_enabled(mut self, enabled: bool) -> Self {
        self.split_enabled = enabled;
        self
    }

    /// 注入窗口置顶切换逻辑：`on_toggle` 在用户点击置顶按钮时调用，
    /// `is_active` 在每次渲染时被调用以决定按钮的视觉状态。
    pub fn with_always_on_top_control(
        mut self,
        on_toggle: Arc<dyn Fn(&mut Window, &mut App) + Send + Sync>,
        is_active: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> Self {
        self.on_toggle_always_on_top = Some(on_toggle);
        self.is_always_on_top = Some(is_active);
        self
    }

    /// Set a pinned tab that stays fixed before the scrollable tab list.
    /// The pinned tab is always visible and cannot be scrolled away.
    pub fn set_pinned_tab(&mut self, tab: TabItem, cx: &mut Context<Self>) {
        self.pinned_tab = Some(tab);
        self.pinned_tab_active = self.tabs.is_empty();
        cx.notify();
    }

    /// Returns whether the pinned tab is currently active.
    pub fn is_pinned_tab_active(&self) -> bool {
        self.pinned_tab_active
    }

    /// Returns whether a pinned tab exists.
    pub fn has_pinned_tab(&self) -> bool {
        self.pinned_tab.is_some()
    }

    /// Activate the pinned tab (deactivate regular tabs visually).
    pub fn activate_pinned_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pinned_tab.is_some() {
            self.pinned_tab_active = true;
            if let Some(pinned) = &self.pinned_tab {
                pinned.content().focus_handle(cx).focus(window, cx);
            }
            cx.emit(TabContainerEvent::LayoutChanged);
            cx.notify();
        }
    }

    pub fn set_tab_bar_bg_color(
        &mut self,
        color: impl Into<Option<gpui::Hsla>>,
        cx: &mut Context<Self>,
    ) {
        self.tab_bar_bg_color = color.into();
        cx.notify();
    }

    pub fn set_tab_bar_border_color(
        &mut self,
        color: impl Into<Option<gpui::Hsla>>,
        cx: &mut Context<Self>,
    ) {
        self.tab_bar_border_color = color.into();
        cx.notify();
    }

    pub fn set_active_tab_bg_color(
        &mut self,
        color: impl Into<Option<gpui::Hsla>>,
        cx: &mut Context<Self>,
    ) {
        self.active_tab_bg_color = color.into();
        cx.notify();
    }

    pub fn set_inactive_tab_hover_color(
        &mut self,
        color: impl Into<Option<gpui::Hsla>>,
        cx: &mut Context<Self>,
    ) {
        self.inactive_tab_hover_color = color.into();
        cx.notify();
    }

    /// Add a new tab
    pub fn add_tab(&mut self, tab: TabItem, cx: &mut Context<Self>) {
        self.tabs.push(tab);
        cx.emit(TabContainerEvent::LayoutChanged);
        cx.notify();
    }

    fn subscribe_tab_content(
        &mut self,
        tab: &TabItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tab_content_subscriptions
            .push(tab.content().subscribe_events(window, cx));
    }

    fn handle_tab_content_event(&mut self, event: &TabContentEvent, cx: &mut Context<Self>) {
        match event {
            TabContentEvent::StateChanged => {
                cx.emit(TabContainerEvent::LayoutChanged);
                cx.notify();
            }
        }
    }

    /// Add a new tab and activate it
    pub fn add_and_activate_tab(&mut self, tab: TabItem, cx: &mut Context<Self>) {
        let id = tab.id().to_string();
        self.tabs.push(tab);
        self.active_index = self.tabs.len() - 1;
        self.pinned_tab_active = false;
        self.tab_bar_scroll_handle
            .scroll_to_item(self.tabs.len() - 1);
        cx.emit(TabContainerEvent::TabActivated {
            index: self.active_index,
            id,
        });
        cx.emit(TabContainerEvent::LayoutChanged);
        cx.notify();
    }

    /// Activate existing tab by ID, or create and activate if not exists (lazy loading)
    pub fn activate_or_add_tab_lazy<F>(
        &mut self,
        tab_id: impl Into<String>,
        create_fn: F,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) where
        F: FnOnce(&mut Window, &mut Context<Self>) -> TabItem,
    {
        let tab_id = tab_id.into();

        if let Some(index) = self.tabs.iter().position(|t| t.id() == tab_id) {
            // 激活现有 tab，复用 set_active_index 逻辑
            self.set_active_index(index, window, cx);
        } else {
            // 创建新 tab 并激活
            let tab = create_fn(window, cx);
            self.add_and_activate_tab_with_focus(tab, window, cx);
        }
    }

    /// Add a new tab, activate it, and focus its content
    pub fn add_and_activate_tab_with_focus(
        &mut self,
        tab: TabItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = tab.id().to_string();
        let focus_handle = tab.content.focus_handle(cx);
        self.subscribe_tab_content(&tab, window, cx);
        self.tabs.push(tab);
        self.active_index = self.tabs.len() - 1;
        self.pinned_tab_active = false;
        self.tab_bar_scroll_handle
            .scroll_to_item(self.tabs.len() - 1);

        // 激活新 tab 的 content
        if let Some(new_tab) = self.tabs.get(self.active_index) {
            new_tab.content().on_activate(window, cx);
        }

        // 让 content 获取焦点
        focus_handle.focus(window, cx);

        cx.emit(TabContainerEvent::TabActivated {
            index: self.active_index,
            id,
        });
        cx.emit(TabContainerEvent::LayoutChanged);
        cx.notify();
    }

    /// Close a tab by index
    pub fn close_tab(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        if index >= self.tabs.len() || !self.tabs[index].content().closeable(cx) {
            return Task::ready(false);
        }

        let tab_id = self.tabs[index].id();

        if self.closing_tabs.contains(&tab_id) {
            return Task::ready(false);
        }

        self.closing_tabs.insert(tab_id.clone());

        let tab_id_string = tab_id.to_string();
        let content = self.tabs[index].content().clone();
        let entity = cx.entity();

        let close_task = content.try_close(&tab_id_string, window, cx);

        cx.spawn(async move |_handle, cx| {
            let can_close = close_task.await;
            if can_close {
                let _ = entity.update(cx, |this, cx| {
                    this.do_remove_tab_by_id(&tab_id_string, cx);
                });
            } else {
                let _ = entity.update(cx, |this, _cx| {
                    this.closing_tabs.remove(&tab_id);
                });
            }
            can_close
        })
    }

    fn do_remove_tab_by_id(&mut self, tab_id: &str, cx: &mut Context<Self>) {
        if let Some(index) = self.tabs.iter().position(|t| t.id() == tab_id) {
            let removed_tab_id = self.tabs[index].id();
            self.tabs.remove(index);
            self.closing_tabs.remove(&removed_tab_id);

            if self.tabs.is_empty() {
                // All regular tabs closed, activate pinned tab if present
                self.active_index = 0;
                if self.pinned_tab.is_some() {
                    self.pinned_tab_active = true;
                }
            } else if index < self.active_index {
                self.active_index -= 1;
            } else if index == self.active_index {
                if self.active_index >= self.tabs.len() {
                    self.active_index = self.tabs.len() - 1;
                }
            }

            cx.emit(TabContainerEvent::TabClosed {
                id: tab_id.to_string(),
            });
            cx.emit(TabContainerEvent::LayoutChanged);
            cx.notify();
        }
    }

    /// Close all tabs except the one at the given index
    pub fn close_other_tabs(
        &mut self,
        keep_index: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        if keep_index >= self.tabs.len() {
            return Task::ready(true);
        }

        let keep_id = self.tabs[keep_index].id().to_string();
        let tab_ids: Vec<String> = self
            .tabs
            .iter()
            .filter(|t| t.id() != keep_id && t.content().closeable(cx))
            .map(|t| t.id().to_string())
            .collect();

        if tab_ids.is_empty() {
            return Task::ready(true);
        }

        let entity = cx.entity();
        let window_id = cx.active_window();

        cx.spawn(async move |_handle, cx| {
            for tab_id in tab_ids {
                let should_close =
                    cx.update_window(window_id.expect("No active window"), |_, window, cx| {
                        entity.update(cx, |this, cx| {
                            if let Some(index) = this.tabs.iter().position(|t| t.id() == tab_id) {
                                this.set_active_index(index, window, cx);
                                let content = this.tabs[index].content().clone();
                                Some(content.try_close(&tab_id, window, cx))
                            } else {
                                None
                            }
                        })
                    });

                match should_close {
                    Ok(Some(task)) => {
                        let can_close = task.await;
                        if !can_close {
                            return false;
                        }
                        let _ = entity.update(cx, |this, cx| {
                            this.do_remove_tab_by_id(&tab_id, cx);
                        });
                    }
                    Ok(None) => continue,
                    Err(_) => return false,
                }
            }
            true
        })
    }

    /// Close all tabs
    pub fn close_all_tabs(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> Task<bool> {
        let tab_ids: Vec<String> = self
            .tabs
            .iter()
            .filter(|t| t.content().closeable(cx))
            .map(|t| t.id().to_string())
            .collect();

        if tab_ids.is_empty() {
            return Task::ready(true);
        }

        let entity = cx.entity();
        let window_id = cx.active_window();

        cx.spawn(async move |_handle, cx| {
            for tab_id in tab_ids {
                let should_close =
                    cx.update_window(window_id.expect("No active window"), |_, window, cx| {
                        entity.update(cx, |this, cx| {
                            if let Some(index) = this.tabs.iter().position(|t| t.id() == tab_id) {
                                this.set_active_index(index, window, cx);
                                let content = this.tabs[index].content().clone();
                                Some(content.try_close(&tab_id, window, cx))
                            } else {
                                None
                            }
                        })
                    });

                match should_close {
                    Ok(Some(task)) => {
                        let can_close = task.await;
                        if !can_close {
                            return false;
                        }
                        let _ = entity.update(cx, |this, cx| {
                            this.do_remove_tab_by_id(&tab_id, cx);
                        });
                    }
                    Ok(None) => continue,
                    Err(_) => return false,
                }
            }
            true
        })
    }

    /// Close all tabs to the left of the given index
    pub fn close_tabs_to_left(
        &mut self,
        index: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        if index == 0 || index >= self.tabs.len() {
            return Task::ready(true);
        }

        let tab_ids: Vec<String> = self
            .tabs
            .iter()
            .take(index)
            .filter(|t| t.content().closeable(cx))
            .map(|t| t.id().to_string())
            .collect();

        if tab_ids.is_empty() {
            return Task::ready(true);
        }

        let entity = cx.entity();
        let window_id = cx.active_window();

        cx.spawn(async move |_handle, cx| {
            for tab_id in tab_ids {
                let should_close =
                    cx.update_window(window_id.expect("No active window"), |_, window, cx| {
                        entity.update(cx, |this, cx| {
                            if let Some(idx) = this.tabs.iter().position(|t| t.id() == tab_id) {
                                this.set_active_index(idx, window, cx);
                                let content = this.tabs[idx].content().clone();
                                Some(content.try_close(&tab_id, window, cx))
                            } else {
                                None
                            }
                        })
                    });

                match should_close {
                    Ok(Some(task)) => {
                        let can_close = task.await;
                        if !can_close {
                            return false;
                        }
                        let _ = entity.update(cx, |this, cx| {
                            this.do_remove_tab_by_id(&tab_id, cx);
                        });
                    }
                    Ok(None) => continue,
                    Err(_) => return false,
                }
            }
            true
        })
    }

    /// Close all tabs to the right of the given index
    pub fn close_tabs_to_right(
        &mut self,
        index: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        if index >= self.tabs.len() - 1 {
            return Task::ready(true);
        }

        let tab_ids: Vec<String> = self
            .tabs
            .iter()
            .skip(index + 1)
            .filter(|t| t.content().closeable(cx))
            .map(|t| t.id().to_string())
            .collect();

        if tab_ids.is_empty() {
            return Task::ready(true);
        }

        let entity = cx.entity();
        let window_id = cx.active_window();

        cx.spawn(async move |_handle, cx| {
            for tab_id in tab_ids {
                let should_close =
                    cx.update_window(window_id.expect("No active window"), |_, window, cx| {
                        entity.update(cx, |this, cx| {
                            if let Some(idx) = this.tabs.iter().position(|t| t.id() == tab_id) {
                                this.set_active_index(idx, window, cx);
                                let content = this.tabs[idx].content().clone();
                                Some(content.try_close(&tab_id, window, cx))
                            } else {
                                None
                            }
                        })
                    });

                match should_close {
                    Ok(Some(task)) => {
                        let can_close = task.await;
                        if !can_close {
                            return false;
                        }
                        let _ = entity.update(cx, |this, cx| {
                            this.do_remove_tab_by_id(&tab_id, cx);
                        });
                    }
                    Ok(None) => continue,
                    Err(_) => return false,
                }
            }
            true
        })
    }

    /// Close a tab by ID
    pub fn close_tab_by_id(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        if let Some(index) = self.tabs.iter().position(|t| t.id() == id) {
            self.close_tab(index, window, cx)
        } else {
            Task::ready(false)
        }
    }

    /// Close all tabs from a specific source
    pub fn close_tabs_by_tab_from(
        &mut self,
        tab_from: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        let tab_ids: Vec<String> = self
            .tabs
            .iter()
            .filter(|t| t.from() == tab_from && t.content().closeable(cx))
            .map(|t| t.id().to_string())
            .collect();

        if tab_ids.is_empty() {
            return Task::ready(true);
        }

        let entity = cx.entity();
        let window_id = cx.active_window();

        cx.spawn(async move |_handle, cx| {
            for tab_id in tab_ids {
                let should_close =
                    cx.update_window(window_id.expect("No active window"), |_, window, cx| {
                        entity.update(cx, |this, cx| {
                            if let Some(index) = this.tabs.iter().position(|t| t.id() == tab_id) {
                                this.set_active_index(index, window, cx);
                                let content = this.tabs[index].content().clone();
                                Some(content.try_close(&tab_id, window, cx))
                            } else {
                                None
                            }
                        })
                    });

                match should_close {
                    Ok(Some(task)) => {
                        let can_close = task.await;
                        if !can_close {
                            return false;
                        }
                        let _ = entity.update(cx, |this, cx| {
                            this.do_remove_tab_by_id(&tab_id, cx);
                        });
                    }
                    Ok(None) => continue,
                    Err(_) => return false,
                }
            }
            true
        })
    }

    /// Force close a tab by ID, skipping try_close
    pub fn force_close_tab_by_id(&mut self, id: &str, cx: &mut Context<Self>) {
        self.do_remove_tab_by_id(id, cx);
    }

    /// Set the active tab by index
    pub fn set_active_index(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index < self.tabs.len() && (index != self.active_index || self.pinned_tab_active) {
            if self.pinned_tab_active {
                // Deactivate pinned tab
                if let Some(pinned) = &self.pinned_tab {
                    pinned.content().on_deactivate(window, cx);
                }
                self.pinned_tab_active = false;
            } else if let Some(old_tab) = self.tabs.get(self.active_index) {
                old_tab.content().on_deactivate(window, cx);
            }

            self.tab_bar_scroll_handle.scroll_to_item(index);
            self.active_index = index;

            let tab_id = if let Some(new_tab) = self.tabs.get(self.active_index) {
                new_tab.content().on_activate(window, cx);
                new_tab.content().focus_handle(cx).focus(window, cx);
                new_tab.id().to_string()
            } else {
                String::new()
            };

            cx.emit(TabContainerEvent::TabActivated { index, id: tab_id });
            cx.emit(TabContainerEvent::LayoutChanged);
            cx.notify();
        }
    }

    /// Set the active tab by ID
    pub fn set_active_by_id(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.tabs.iter().position(|t| t.id() == id) {
            self.set_active_index(index, window, cx);
        }
    }

    /// Get the active tab
    pub fn active_tab(&self) -> Option<&TabItem> {
        self.tabs.get(self.active_index)
    }

    pub fn active_content_can_split(&self, cx: &App) -> bool {
        active_content_can_split_for_layout(
            self.pinned_tab_active,
            self.pinned_tab
                .as_ref()
                .map(|tab| tab.content().can_split(cx)),
            self.active_tab().map(|tab| tab.content().can_split(cx)),
        )
    }

    pub fn set_size(&mut self, size: Size, cx: &mut Context<Self>) {
        self.size = size;
        cx.notify();
    }

    pub fn set_show_menu(&mut self, show: bool, cx: &mut Context<Self>) {
        self.show_menu = show;
        cx.notify();
    }

    pub fn tabs(&self) -> &[TabItem] {
        &self.tabs
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn active_index(&self) -> usize {
        self.active_index
    }

    pub fn dump(&self, cx: &App) -> TabContainerState {
        let tabs = self
            .tabs
            .iter()
            .map(|tab| TabItemState {
                id: tab.id(),
                from: tab.from(),
                key: SharedString::from(tab.content().content_key(cx)),
                metadata: tab.metadata().clone(),
                data: tab.content().dump(cx),
            })
            .collect();

        TabContainerState {
            version: Some(1),
            tabs,
            active_index: self.active_index,
            config: self.dump_config(),
        }
    }

    fn dump_config(&self) -> TabContainerConfig {
        TabContainerConfig {
            size: Some(self.size_to_string()),
            left_padding: self.left_padding.map(|p| f32::from(p)),
            top_padding: self.top_padding.map(|p| f32::from(p)),
        }
    }

    fn size_to_string(&self) -> String {
        match self.size {
            Size::XSmall => "xsmall".to_string(),
            Size::Small => "small".to_string(),
            Size::Medium => "medium".to_string(),
            Size::Large => "large".to_string(),
            Size::Size(pixels) => format!("{}px", f32::from(pixels)),
        }
    }

    fn parse_size(s: &str) -> Size {
        match s {
            "xsmall" => Size::XSmall,
            "small" => Size::Small,
            "medium" => Size::Medium,
            "large" => Size::Large,
            s if s.ends_with("px") => s
                .trim_end_matches("px")
                .parse::<f32>()
                .map(|v| Size::Size(px(v)))
                .unwrap_or(Size::Large),
            _ => Size::Large,
        }
    }

    pub fn load(
        &mut self,
        state: TabContainerState,
        registry: &TabContentRegistry,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.tabs.clear();

        for tab_state in &state.tabs {
            if let Some(content) = registry.build(tab_state, window, cx) {
                self.tabs.push(TabItem {
                    id: tab_state.id.clone(),
                    from: tab_state.from.clone(),
                    metadata: tab_state.metadata.clone(),
                    content,
                });
            }
        }

        self.active_index = if self.tabs.is_empty() {
            0 // Empty list: active_index is 0 by convention (active_tab() will return None)
        } else {
            state.active_index.min(self.tabs.len() - 1)
        };

        self.load_config(&state.config);
    }

    fn load_config(&mut self, config: &TabContainerConfig) {
        if let Some(size) = &config.size {
            self.size = Self::parse_size(size);
        }
        if let Some(left_padding) = config.left_padding {
            let default_padding = self.left_padding.map(f32::from).unwrap_or(left_padding);
            self.left_padding = Some(px(left_padding.max(default_padding)));
        }
        if let Some(top_padding) = config.top_padding {
            let default_padding = self.top_padding.map(f32::from).unwrap_or(top_padding);
            self.top_padding = Some(px(top_padding.max(default_padding)));
        }
    }

    pub fn move_tab(&mut self, from_index: usize, to_index: usize, cx: &mut Context<Self>) {
        if from_index >= self.tabs.len() || to_index >= self.tabs.len() || from_index == to_index {
            return;
        }

        let tab = self.tabs.remove(from_index);
        self.tabs.insert(to_index, tab);

        if self.active_index == from_index {
            self.active_index = to_index;
        } else {
            match (
                from_index.cmp(&self.active_index),
                to_index.cmp(&self.active_index),
            ) {
                (Ordering::Less, Ordering::Greater | Ordering::Equal) => {
                    self.active_index -= 1;
                }
                (Ordering::Greater, Ordering::Less | Ordering::Equal) => {
                    self.active_index += 1;
                }
                _ => {}
            }
        }

        cx.emit(TabContainerEvent::LayoutChanged);
        cx.notify();
    }

    pub fn take_tab(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<TabItem> {
        if index >= self.tabs.len() {
            return None;
        }

        let was_active = !self.pinned_tab_active && index == self.active_index;
        let tab = self.tabs.remove(index);

        if self.tabs.is_empty() {
            self.active_index = 0;
            self.pinned_tab_active = self.pinned_tab.is_some();
            if let Some(pinned) = &self.pinned_tab {
                pinned.content().on_activate(window, cx);
            }
        } else {
            if index < self.active_index {
                self.active_index -= 1;
            } else if self.active_index >= self.tabs.len() {
                self.active_index = self.tabs.len() - 1;
            }
            if was_active {
                self.tabs[self.active_index]
                    .content()
                    .on_activate(window, cx);
            }
        }

        cx.emit(TabContainerEvent::LayoutChanged);
        cx.notify();
        Some(tab)
    }

    pub fn insert_tab_at_end_and_activate(
        &mut self,
        tab: TabItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_and_activate_tab_with_focus(tab, window, cx);
    }

    fn get_tab_width(&self, tab: &TabItem, cx: &App) -> gpui::Pixels {
        let size = tab.content().width_size(cx).unwrap_or(self.size);
        self.size_to_pixels(size)
    }

    fn size_to_pixels(&self, size: Size) -> gpui::Pixels {
        match size {
            Size::Size(pixels) => pixels,
            Size::XSmall => px(60.0),
            Size::Small => px(100.0),
            Size::Medium => px(140.0),
            Size::Large => px(180.0),
        }
    }

    fn active_sidebar_contributions(&self, cx: &App) -> Vec<SidebarContribution> {
        if self.pinned_tab_active {
            return self
                .pinned_tab
                .as_ref()
                .map(|tab| tab.content().sidebar_contributions(cx))
                .unwrap_or_default();
        }

        self.active_tab()
            .map(|tab| tab.content().sidebar_contributions(cx))
            .unwrap_or_default()
    }

    fn resolve_sidebar_panel_state(
        &self,
        id: &SidebarPanelId,
        default_placement: SidebarPlacement,
        policy: SidebarPanelPolicy,
    ) -> ResolvedSidebarPanelState {
        let default_placement = normalize_sidebar_placement(default_placement, policy);
        let Some(override_state) = self.sidebar_overrides.get(id).copied() else {
            return ResolvedSidebarPanelState {
                visible: sidebar_panel_initial_visibility(policy),
                placement: default_placement,
            };
        };

        let placement =
            if policy.movable && policy.allowed_placements.contains(override_state.placement) {
                override_state.placement
            } else {
                default_placement
            };
        let visible = if policy.hideable {
            override_state.visible
        } else {
            true
        };

        ResolvedSidebarPanelState { visible, placement }
    }

    fn valid_sidebar_override_placement(
        &self,
        id: &SidebarPanelId,
        default_placement: SidebarPlacement,
        policy: SidebarPanelPolicy,
    ) -> SidebarPlacement {
        self.sidebar_overrides
            .get(id)
            .map(|override_state| override_state.placement)
            .filter(|placement| policy.movable && policy.allowed_placements.contains(*placement))
            .unwrap_or_else(|| normalize_sidebar_placement(default_placement, policy))
    }

    fn sidebar_target_blocked(
        &self,
        id: &SidebarPanelId,
        placement: SidebarPlacement,
        cx: &App,
    ) -> bool {
        self.active_sidebar_contributions(cx)
            .into_iter()
            .filter(|contribution| contribution.id != *id)
            .any(|contribution| {
                let state = self.resolve_sidebar_panel_state(
                    &contribution.id,
                    contribution.default_placement,
                    contribution.policy,
                );
                sidebar_panel_blocks_exclusive_target(
                    state.visible,
                    state.placement,
                    contribution.policy.hideable,
                    contribution.chrome,
                    placement,
                )
            })
    }

    fn hide_sidebar_peers_at_placement(
        &mut self,
        id: &SidebarPanelId,
        placement: SidebarPlacement,
        cx: &App,
    ) {
        let peers = self
            .active_sidebar_contributions(cx)
            .into_iter()
            .filter(|contribution| contribution.id != *id)
            .filter_map(|contribution| {
                let state = self.resolve_sidebar_panel_state(
                    &contribution.id,
                    contribution.default_placement,
                    contribution.policy,
                );
                sidebar_panel_should_hide_for_exclusive_target(
                    state.visible,
                    state.placement,
                    contribution.policy.hideable,
                    contribution.chrome,
                    placement,
                )
                .then_some((contribution.id, state.placement))
            })
            .collect::<Vec<_>>();

        for (id, placement) in peers {
            self.sidebar_overrides.insert(
                id,
                SidebarPanelOverride {
                    visible: false,
                    placement,
                },
            );
        }
    }

    fn move_sidebar_panel(
        &mut self,
        id: SidebarPanelId,
        placement: SidebarPlacement,
        policy: SidebarPanelPolicy,
        cx: &App,
    ) {
        if !policy.movable || !policy.allowed_placements.contains(placement) {
            return;
        }
        if self.sidebar_target_blocked(&id, placement, cx) {
            return;
        }
        self.hide_sidebar_peers_at_placement(&id, placement, cx);
        let visible = self
            .sidebar_overrides
            .get(&id)
            .map(|override_state| override_state.visible || !policy.hideable)
            .unwrap_or(true);
        self.sidebar_overrides
            .insert(id, SidebarPanelOverride { visible, placement });
    }

    fn hide_sidebar_panel(
        &mut self,
        id: SidebarPanelId,
        default_placement: SidebarPlacement,
        policy: SidebarPanelPolicy,
    ) {
        if !policy.hideable {
            return;
        }
        let placement = self.valid_sidebar_override_placement(&id, default_placement, policy);
        self.sidebar_overrides.insert(
            id,
            SidebarPanelOverride {
                visible: false,
                placement,
            },
        );
    }

    fn show_sidebar_panel(
        &mut self,
        id: SidebarPanelId,
        default_placement: SidebarPlacement,
        policy: SidebarPanelPolicy,
        cx: &App,
    ) {
        let placement = self.valid_sidebar_override_placement(&id, default_placement, policy);
        if self.sidebar_target_blocked(&id, placement, cx) {
            return;
        }
        self.hide_sidebar_peers_at_placement(&id, placement, cx);
        self.sidebar_overrides.insert(
            id,
            SidebarPanelOverride {
                visible: true,
                placement,
            },
        );
    }

    fn resolved_sidebar_panels(&self, cx: &App) -> Vec<ResolvedSidebarContribution> {
        self.active_sidebar_contributions(cx)
            .into_iter()
            .map(|contribution| {
                let state = self.resolve_sidebar_panel_state(
                    &contribution.id,
                    contribution.default_placement,
                    contribution.policy,
                );
                ResolvedSidebarContribution {
                    contribution,
                    placement: state.placement,
                    visible: state.visible,
                }
            })
            .collect()
    }

    fn sidebar_panels_for(
        panels: &[ResolvedSidebarContribution],
        placement: SidebarPlacement,
    ) -> Vec<ResolvedSidebarContribution> {
        let mut exclusive_slot_taken = false;
        panels
            .iter()
            .filter_map(|panel| {
                if !panel.visible || panel.placement != placement {
                    return None;
                }
                if !sidebar_panel_uses_exclusive_slot(panel.contribution.chrome) {
                    return Some(panel.clone());
                }
                if exclusive_slot_taken {
                    return None;
                }
                exclusive_slot_taken = true;
                Some(panel.clone())
            })
            .collect()
    }

    fn hidden_sidebar_panels(
        panels: &[ResolvedSidebarContribution],
    ) -> Vec<ResolvedSidebarContribution> {
        panels
            .iter()
            .filter(|panel| !panel.visible && panel.contribution.policy.hideable)
            .cloned()
            .collect()
    }

    fn sidebar_panel_side_width(&self, contribution: &SidebarContribution) -> Pixels {
        if !sidebar_panel_allows_size_override(contribution.size.side_width) {
            return contribution.size.side_width.unwrap_or(TOOLBAR_WIDTH);
        }

        self.sidebar_size_overrides
            .get(&contribution.id)
            .and_then(|size| size.side_width)
            .or(contribution.size.side_width)
            .unwrap_or(SIDEBAR_SIDE_DEFAULT_WIDTH)
    }

    fn sidebar_panel_bottom_height(&self, contribution: &SidebarContribution) -> Pixels {
        if !sidebar_panel_allows_size_override(contribution.size.bottom_height) {
            return contribution.size.bottom_height.unwrap_or(TOOLBAR_WIDTH);
        }

        self.sidebar_size_overrides
            .get(&contribution.id)
            .and_then(|size| size.bottom_height)
            .or(contribution.size.bottom_height)
            .unwrap_or(SIDEBAR_BOTTOM_DEFAULT_HEIGHT)
    }

    fn sidebar_side_width(&self, panels: &[ResolvedSidebarContribution]) -> Pixels {
        panels
            .iter()
            .map(|panel| self.sidebar_panel_side_width(&panel.contribution))
            .fold(px(0.0), |total, width| total + width)
    }

    fn sidebar_bottom_height(&self, panels: &[ResolvedSidebarContribution]) -> Pixels {
        panels
            .iter()
            .map(|panel| self.sidebar_panel_bottom_height(&panel.contribution))
            .max_by(|left, right| f32::from(*left).total_cmp(&f32::from(*right)))
            .unwrap_or(SIDEBAR_BOTTOM_DEFAULT_HEIGHT)
    }

    fn render_sidebar_dock(
        &self,
        placement: SidebarPlacement,
        panels: Vec<ResolvedSidebarContribution>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if panels.is_empty() {
            return div().size_full().into_any_element();
        }

        h_flex()
            .id(SharedString::from(format!(
                "tab-sidebar-dock-{placement:?}"
            )))
            .size_full()
            .overflow_hidden()
            .children(
                panels
                    .into_iter()
                    .map(|panel| self.render_sidebar_panel_slot(panel, placement, cx)),
            )
            .into_any_element()
    }

    fn render_sidebar_panel_slot(
        &self,
        panel: ResolvedSidebarContribution,
        placement: SidebarPlacement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let contribution = panel.contribution;
        let side_width = self.sidebar_panel_side_width(&contribution);
        let bottom_height = self.sidebar_panel_bottom_height(&contribution);
        let can_resize = match placement {
            SidebarPlacement::Left | SidebarPlacement::Right => {
                sidebar_panel_allows_resize(contribution.chrome, Some(side_width), None)
            }
            SidebarPlacement::Bottom => {
                sidebar_panel_allows_resize(contribution.chrome, None, Some(bottom_height))
            }
        };
        div()
            .relative()
            .h_full()
            .overflow_hidden()
            .flex_shrink_0()
            .map(|this| match placement {
                SidebarPlacement::Left | SidebarPlacement::Right => this.w(side_width),
                SidebarPlacement::Bottom => this.flex_1().min_w(SIDEBAR_PANEL_MIN_SIZE),
            })
            .child(self.render_sidebar_panel_frame(contribution.clone(), cx))
            .when(can_resize, |this| {
                this.child(self.render_sidebar_resize_handle(contribution.id, placement, cx))
            })
            .into_any_element()
    }

    fn render_sidebar_resize_handle(
        &self,
        id: SidebarPanelId,
        placement: SidebarPlacement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let container = cx.entity();
        let handle_id = SharedString::from(format!(
            "tab-sidebar-resize-{placement:?}-{}-{}",
            id.owner, id.local_id
        ));
        let neg_offset = -SIDEBAR_HANDLE_PADDING;
        let drag_border = cx.theme().drag_border;
        let border = cx.theme().border;

        div()
            .id(handle_id)
            .occlude()
            .absolute()
            .flex_shrink_0()
            .group("tab-sidebar-resize-handle")
            .map(|this| match placement {
                SidebarPlacement::Left => this
                    .cursor_col_resize()
                    .top_0()
                    .right(px(1.0))
                    .h_full()
                    .w(SIDEBAR_HANDLE_SIZE)
                    .pl(SIDEBAR_HANDLE_PADDING),
                SidebarPlacement::Right => this
                    .cursor_col_resize()
                    .top_0()
                    .left(px(1.0))
                    .h_full()
                    .w(SIDEBAR_HANDLE_SIZE)
                    .pr(SIDEBAR_HANDLE_PADDING),
                SidebarPlacement::Bottom => this
                    .cursor_row_resize()
                    .top(neg_offset)
                    .left_0()
                    .w_full()
                    .h(SIDEBAR_HANDLE_SIZE)
                    .py(SIDEBAR_HANDLE_PADDING),
            })
            .on_drag(SidebarResizeDrag, move |_, _, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
                container.update(cx, |container, cx| {
                    container.sidebar_resizing = Some(SidebarResizeTarget {
                        id: id.clone(),
                        placement,
                    });
                    cx.notify();
                });
                cx.new(|_| SidebarResizePanel)
            })
            .child(
                div()
                    .bg(border)
                    .group_hover("tab-sidebar-resize-handle", move |this| {
                        this.bg(drag_border)
                    })
                    .map(|this| match placement {
                        SidebarPlacement::Left | SidebarPlacement::Right => {
                            this.h_full().w(SIDEBAR_HANDLE_SIZE)
                        }
                        SidebarPlacement::Bottom => this.w_full().h(SIDEBAR_HANDLE_SIZE),
                    }),
            )
            .into_any_element()
    }

    fn render_sidebar_panel_frame(
        &self,
        contribution: SidebarContribution,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if contribution.chrome == SidebarPanelChrome::None {
            return div()
                .id(SharedString::from(format!(
                    "tab-sidebar-panel-{}-{}",
                    contribution.id.owner, contribution.id.local_id
                )))
                .size_full()
                .overflow_hidden()
                .child(contribution.view)
                .into_any_element();
        }

        let background = contribution
            .style
            .background
            .unwrap_or(cx.theme().background);
        let border = contribution.style.border.unwrap_or(cx.theme().border);
        v_flex()
            .id(SharedString::from(format!(
                "tab-sidebar-panel-{}-{}",
                contribution.id.owner, contribution.id.local_id
            )))
            .size_full()
            .overflow_hidden()
            .bg(background)
            .border_1()
            .border_color(border)
            .when(sidebar_panel_renders_header(contribution.chrome), |this| {
                this.child(self.render_sidebar_panel_header(contribution.clone(), cx))
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(contribution.view),
            )
            .into_any_element()
    }

    fn render_sidebar_panel_header(
        &self,
        contribution: SidebarContribution,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let border = contribution.style.border.unwrap_or(cx.theme().border);
        let header_background = contribution.style.header_background.unwrap_or_else(|| {
            contribution
                .style
                .background
                .unwrap_or(cx.theme().background)
        });
        let text_color = contribution.style.text.unwrap_or(cx.theme().foreground);
        h_flex()
            .id(SharedString::from(format!(
                "tab-sidebar-header-{}-{}",
                contribution.id.owner, contribution.id.local_id
            )))
            .h(px(34.0))
            .px_2()
            .gap_2()
            .items_center()
            .bg(header_background)
            .border_b_1()
            .border_color(border)
            .child(
                Icon::new(contribution.icon.clone())
                    .with_size(Size::Small)
                    .text_color(text_color),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_sm()
                    .text_color(text_color)
                    .child(contribution.title.clone()),
            )
            .children(self.render_sidebar_panel_controls(contribution, cx))
            .into_any_element()
    }

    fn render_sidebar_panel_controls(
        &self,
        contribution: SidebarContribution,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut controls = Vec::new();
        if contribution.policy.movable {
            controls.push(self.render_sidebar_move_button(
                contribution.clone(),
                SidebarPlacement::Left,
                "Move left",
                cx,
            ));
            controls.push(self.render_sidebar_move_button(
                contribution.clone(),
                SidebarPlacement::Right,
                "Move right",
                cx,
            ));
            controls.push(self.render_sidebar_move_button(
                contribution.clone(),
                SidebarPlacement::Bottom,
                "Move bottom",
                cx,
            ));
        }
        if contribution.policy.hideable {
            controls.push(self.render_sidebar_hide_button(contribution, cx));
        }
        controls
    }

    fn render_sidebar_move_button(
        &self,
        contribution: SidebarContribution,
        placement: SidebarPlacement,
        tooltip: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let enabled = contribution.policy.allowed_placements.contains(placement);
        let icon = match placement {
            SidebarPlacement::Left => IconName::PanelLeft,
            SidebarPlacement::Right => IconName::PanelRight,
            SidebarPlacement::Bottom => IconName::PanelBottom,
        };
        let container = cx.entity();
        Button::new(SharedString::from(format!(
            "tab-sidebar-move-{placement:?}-{}-{}",
            contribution.id.owner, contribution.id.local_id
        )))
        .icon(icon)
        .ghost()
        .compact()
        .tooltip(tooltip)
        .disabled(!enabled)
        .on_click(move |_, window, cx| {
            if let Some(move_to) = contribution.actions.move_to.as_ref() {
                move_to(placement, window, cx);
            } else {
                container.update(cx, |container, cx| {
                    container.move_sidebar_panel(
                        contribution.id.clone(),
                        placement,
                        contribution.policy,
                        cx,
                    );
                    cx.notify();
                });
            }
        })
        .into_any_element()
    }

    fn render_sidebar_hide_button(
        &self,
        contribution: SidebarContribution,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let container = cx.entity();
        Button::new(SharedString::from(format!(
            "tab-sidebar-hide-{}-{}",
            contribution.id.owner, contribution.id.local_id
        )))
        .icon(IconName::EyeOff)
        .ghost()
        .compact()
        .tooltip("Hide panel")
        .on_click(move |_, window, cx| {
            if let Some(close) = contribution.actions.close.as_ref() {
                close(window, cx);
            } else {
                container.update(cx, |container, cx| {
                    container.hide_sidebar_panel(
                        contribution.id.clone(),
                        contribution.default_placement,
                        contribution.policy,
                    );
                    cx.notify();
                });
            }
        })
        .into_any_element()
    }

    fn render_hidden_sidebar_launcher(
        &self,
        panels: Vec<ResolvedSidebarContribution>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if panels.is_empty() {
            return div().into_any_element();
        }
        let container = cx.entity();
        let background = panels
            .first()
            .and_then(|panel| panel.contribution.style.background)
            .unwrap_or(cx.theme().background);
        let border = panels
            .first()
            .and_then(|panel| panel.contribution.style.border)
            .unwrap_or(cx.theme().border);
        h_flex()
            .id("tab-sidebar-hidden-panels")
            .absolute()
            .top_1()
            .right_1()
            .gap_1()
            .p_1()
            .rounded(px(6.0))
            .border_1()
            .border_color(border)
            .bg(background)
            .children(panels.into_iter().map(|panel| {
                let contribution = panel.contribution;
                let id = contribution.id.clone();
                let text_color = contribution.style.text.unwrap_or(cx.theme().foreground);
                Button::new(SharedString::from(format!(
                    "tab-sidebar-show-{}-{}",
                    id.owner, id.local_id
                )))
                .icon(Icon::new(contribution.icon.clone()).text_color(text_color))
                .ghost()
                .compact()
                .tooltip("Show panel")
                .on_click({
                    let container = container.clone();
                    move |_, _, cx| {
                        container.update(cx, |container, cx| {
                            container.show_sidebar_panel(
                                id.clone(),
                                contribution.default_placement,
                                contribution.policy,
                                cx,
                            );
                            cx.notify();
                        });
                    }
                })
            }))
            .into_any_element()
    }

    fn set_sidebar_side_width(&mut self, id: SidebarPanelId, width: Pixels) {
        self.sidebar_size_overrides
            .entry(id)
            .or_default()
            .side_width = Some(width);
    }

    fn set_sidebar_bottom_height(&mut self, id: SidebarPanelId, height: Pixels) {
        self.sidebar_size_overrides
            .entry(id)
            .or_default()
            .bottom_height = Some(height);
    }

    fn resize_sidebar_panel(
        &mut self,
        mouse_position: Point<Pixels>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.sidebar_resizing.clone() else {
            return;
        };

        if !self.sidebar_resize_target_active(&target, cx) {
            self.sidebar_resizing = None;
            cx.notify();
            return;
        }

        match target.placement {
            SidebarPlacement::Left | SidebarPlacement::Right => {
                self.resize_side_sidebar_panel(target, mouse_position, cx);
            }
            SidebarPlacement::Bottom => {
                self.resize_bottom_sidebar_panel(target, mouse_position);
            }
        }
        cx.notify();
    }

    fn sidebar_resize_target_active(&self, target: &SidebarResizeTarget, cx: &App) -> bool {
        self.resolved_sidebar_panels(cx)
            .into_iter()
            .find(|panel| panel.visible && panel.contribution.id == target.id)
            .is_some_and(|panel| {
                if panel.placement != target.placement {
                    return false;
                }

                match target.placement {
                    SidebarPlacement::Left | SidebarPlacement::Right => {
                        sidebar_panel_allows_resize(
                            panel.contribution.chrome,
                            Some(self.sidebar_panel_side_width(&panel.contribution)),
                            None,
                        )
                    }
                    SidebarPlacement::Bottom => sidebar_panel_allows_resize(
                        panel.contribution.chrome,
                        None,
                        Some(self.sidebar_panel_bottom_height(&panel.contribution)),
                    ),
                }
            })
    }

    fn resize_side_sidebar_panel(
        &mut self,
        target: SidebarResizeTarget,
        mouse_position: Point<Pixels>,
        cx: &App,
    ) {
        let panels = self.resolved_sidebar_panels(cx);
        let same_side = Self::sidebar_panels_for(&panels, target.placement);
        let Some(target_ix) = same_side
            .iter()
            .position(|panel| panel.contribution.id == target.id)
        else {
            return;
        };

        let widths = same_side
            .iter()
            .map(|panel| self.sidebar_panel_side_width(&panel.contribution))
            .collect::<Vec<_>>();
        let before = widths
            .iter()
            .take(target_ix)
            .fold(px(0.0), |total, width| total + *width);
        let after = widths
            .iter()
            .skip(target_ix + 1)
            .fold(px(0.0), |total, width| total + *width);
        let after_min = same_side
            .iter()
            .skip(target_ix + 1)
            .fold(px(0.0), |total, _| total + SIDEBAR_PANEL_MIN_SIZE);
        let opposite_width = match target.placement {
            SidebarPlacement::Left => {
                let right = Self::sidebar_panels_for(&panels, SidebarPlacement::Right);
                self.sidebar_side_width(&right)
            }
            SidebarPlacement::Right => {
                let left = Self::sidebar_panels_for(&panels, SidebarPlacement::Left);
                self.sidebar_side_width(&left)
            }
            SidebarPlacement::Bottom => px(0.0),
        };
        let max_dock_width =
            (self.sidebar_bounds.size.width - SIDEBAR_CENTER_MIN_SIZE - opposite_width)
                .max(SIDEBAR_PANEL_MIN_SIZE);
        let max_width = (max_dock_width - before - after_min).max(SIDEBAR_PANEL_MIN_SIZE);
        let raw_width = match target.placement {
            SidebarPlacement::Left => mouse_position.x - self.sidebar_bounds.left() - before,
            SidebarPlacement::Right => self.sidebar_bounds.right() - after - mouse_position.x,
            SidebarPlacement::Bottom => unreachable!(),
        };
        let width = raw_width.clamp(SIDEBAR_PANEL_MIN_SIZE, max_width);
        self.set_sidebar_side_width(target.id, width);
    }

    fn resize_bottom_sidebar_panel(
        &mut self,
        target: SidebarResizeTarget,
        mouse_position: Point<Pixels>,
    ) {
        let max_height =
            (self.sidebar_bounds.size.height - SIDEBAR_CENTER_MIN_SIZE).max(SIDEBAR_PANEL_MIN_SIZE);
        let height = (self.sidebar_bounds.bottom() - mouse_position.y)
            .clamp(SIDEBAR_PANEL_MIN_SIZE, max_height);
        self.set_sidebar_bottom_height(target.id, height);
    }

    fn finish_sidebar_resize(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_resizing = None;
        cx.notify();
    }

    fn render_content_with_sidebars(
        &self,
        content: AnyElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let panels = self.resolved_sidebar_panels(cx);
        let left = Self::sidebar_panels_for(&panels, SidebarPlacement::Left);
        let right = Self::sidebar_panels_for(&panels, SidebarPlacement::Right);
        let bottom = Self::sidebar_panels_for(&panels, SidebarPlacement::Bottom);
        let hidden = Self::hidden_sidebar_panels(&panels);

        if left.is_empty() && right.is_empty() && bottom.is_empty() && hidden.is_empty() {
            return content;
        }

        let center_content = div()
            .relative()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(content)
            .child(self.render_hidden_sidebar_launcher(hidden, cx));
        let center = if bottom.is_empty() {
            center_content.into_any_element()
        } else {
            v_flex()
                .id("tab-sidebar-center")
                .size_full()
                .min_w_0()
                .min_h_0()
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .overflow_hidden()
                        .child(center_content),
                )
                .child(
                    div()
                        .relative()
                        .w_full()
                        .h(self.sidebar_bottom_height(&bottom))
                        .flex_shrink_0()
                        .overflow_hidden()
                        .child(self.render_sidebar_dock(SidebarPlacement::Bottom, bottom, cx)),
                )
                .into_any_element()
        };

        let mut root = h_flex()
            .id("tab-sidebar-root")
            .relative()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden();
        if !left.is_empty() {
            root = root.child(
                div()
                    .relative()
                    .h_full()
                    .w(self.sidebar_side_width(&left))
                    .flex_shrink_0()
                    .overflow_hidden()
                    .child(self.render_sidebar_dock(SidebarPlacement::Left, left, cx)),
            );
        }
        root = root.child(
            div()
                .flex_1()
                .h_full()
                .min_w_0()
                .min_h_0()
                .overflow_hidden()
                .child(center),
        );
        if !right.is_empty() {
            root = root.child(
                div()
                    .relative()
                    .h_full()
                    .w(self.sidebar_side_width(&right))
                    .flex_shrink_0()
                    .overflow_hidden()
                    .child(self.render_sidebar_dock(SidebarPlacement::Right, right, cx)),
            );
        }

        root.child(SidebarResizeEventHandler {
            container: cx.entity(),
        })
        .into_any_element()
    }

    pub fn render_tab_content(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active_tab = if self.pinned_tab_active {
            self.pinned_tab.as_ref()
        } else {
            self.active_tab()
        };
        let split_enabled = self.split_enabled
            && active_tab
                .map(|tab| tab.content().can_split(cx))
                .unwrap_or(false);
        let sidebar_panels = self.resolved_sidebar_panels(cx);
        let has_sidebar_layout = sidebar_panels
            .iter()
            .any(|panel| panel.visible || (!panel.visible && panel.contribution.policy.hideable));

        div()
            .id("tab-content")
            .flex_1()
            .w_full()
            .overflow_hidden()
            .when(!has_sidebar_layout, |el| {
                let active_view = active_tab.map(|tab| tab.content().view());
                el.when_some(active_view, |el, view| el.child(view))
                    .when(split_enabled, |el| {
                        el.relative()
                            .on_drag_move(cx.listener(Self::on_tab_content_drag_move))
                            .child(
                                div()
                                    .invisible()
                                    .absolute()
                                    .bg(cx.theme().drop_target)
                                    .map(|this| match self.will_split_placement {
                                        Some(Placement::Right) => {
                                            this.right_0().top_0().bottom_0().w(relative(0.5))
                                        }
                                        Some(Placement::Bottom) => {
                                            this.bottom_0().left_0().right_0().h(relative(0.5))
                                        }
                                        _ => this.top_0().left_0().size_full(),
                                    })
                                    .group_drag_over::<DragTab>("", |this| this.visible())
                                    .on_drop(cx.listener(|this, drag: &DragTab, window, cx| {
                                        this.drop_tab_on_content(drag, window, cx);
                                    })),
                            )
                    })
            })
            .when(has_sidebar_layout, |el| {
                let active_view = active_tab.map(|tab| tab.content().view());
                let content = div()
                    .size_full()
                    .overflow_hidden()
                    .when_some(active_view, |el, view| el.child(view))
                    .when(split_enabled, |el| {
                        el.relative()
                            .on_drag_move(cx.listener(Self::on_tab_content_drag_move))
                            .child(
                                div()
                                    .invisible()
                                    .absolute()
                                    .bg(cx.theme().drop_target)
                                    .map(|this| match self.will_split_placement {
                                        Some(Placement::Right) => {
                                            this.right_0().top_0().bottom_0().w(relative(0.5))
                                        }
                                        Some(Placement::Bottom) => {
                                            this.bottom_0().left_0().right_0().h(relative(0.5))
                                        }
                                        _ => this.top_0().left_0().size_full(),
                                    })
                                    .group_drag_over::<DragTab>("", |this| this.visible())
                                    .on_drop(cx.listener(|this, drag: &DragTab, window, cx| {
                                        this.drop_tab_on_content(drag, window, cx);
                                    })),
                            )
                    })
                    .into_any_element();
                el.child(self.render_content_with_sidebars(content, cx))
            })
    }

    fn on_tab_content_drag_move(
        &mut self,
        drag: &DragMoveEvent<DragTab>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let bounds = drag.bounds;
        let position = drag.event.position;

        self.will_split_placement = if position.x > bounds.left() + bounds.size.width * 0.75 {
            Some(Placement::Right)
        } else if position.y > bounds.top() + bounds.size.height * 0.75 {
            Some(Placement::Bottom)
        } else {
            None
        };
        cx.notify();
    }

    fn drop_tab_on_content(&mut self, drag: &DragTab, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = drag.source_pane.clone() else {
            return;
        };
        let Some(placement) = self.will_split_placement.take() else {
            self.move_dragged_tab_into_pane(drag, window, cx);
            return;
        };
        if !matches!(placement, Placement::Right | Placement::Bottom) {
            return;
        }
        let tab_can_split = source
            .read(cx)
            .tabs()
            .get(drag.tab_index)
            .map(|tab| tab.content().can_split(cx))
            .unwrap_or(false);
        let source_tab_count = source.read(cx).tabs().len();
        if !split_command_enabled(self.split_enabled, tab_can_split, source_tab_count) {
            return;
        }
        cx.emit(TabContainerEvent::SplitRequested {
            placement,
            source,
            tab_index: drag.tab_index,
        });
    }

    fn move_dragged_tab_into_pane(
        &mut self,
        drag: &DragTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(source) = drag.source_pane.clone() else {
            return;
        };
        if source == cx.entity() {
            self.set_active_index(drag.tab_index, window, cx);
            return;
        }
        let moved = source.update(cx, |source, cx| source.take_tab(drag.tab_index, window, cx));
        if let Some(tab) = moved {
            self.insert_tab_at_end_and_activate(tab, window, cx);
        }
    }

    pub fn render_tab_bar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity();

        let theme = cx.theme();
        let bg_color = self.tab_bar_bg_color.unwrap_or(theme.tab);
        let border_color = self.tab_bar_border_color.unwrap_or(theme.border);
        let active_tab_color = self.active_tab_bg_color.unwrap_or(theme.tab_active);
        let hover_tab_color = self
            .inactive_tab_hover_color
            .unwrap_or(theme.tab.opacity(0.8));
        let inactive_tab_color = self.inactive_tab_bg_color.unwrap_or(theme.tab.opacity(0.5));
        let text_color = self.tab_text_color.unwrap_or(theme.tab_foreground);
        let close_btn_color = self
            .tab_close_button_color
            .unwrap_or(theme.muted_foreground);
        let drag_border_color = theme.drag_border;
        let active_index = self.active_index;
        let left_padding = self.left_padding.unwrap_or(px(8.0));

        let tab_list = self.tab_list.clone();

        // 窗口拖动状态管理（仅在 Windows/Linux 上需要，且启用窗口控件时）
        let is_linux = cfg!(target_os = "linux");
        let is_macos = cfg!(target_os = "macos");
        let is_client_decorated = matches!(window.window_decorations(), Decorations::Client { .. });
        let show_window_controls = self.show_window_controls;
        let enable_titlebar_interactions = show_window_controls || is_macos;
        let allow_tab_drag = !is_macos;

        // 使用状态管理窗口拖动
        let drag_state = window.use_state(cx, |_, _| TabBarDragState { should_move: false });

        h_flex()
            .id("tab-bar")
            .w_full()
            .h(px(40.0))
            .bg(bg_color)
            .items_center()
            .border_b_1()
            .border_color(border_color)
            // 标题栏交互支持：macOS 始终启用双击/拖动，其他平台跟随窗口控件开关
            .when(enable_titlebar_interactions, |this| {
                this.when(is_linux, |this| {
                    this.on_double_click(|_, window, _| window.zoom_window())
                })
                .when(is_macos, |this| {
                    this.on_double_click(|_, window, _| window.titlebar_double_click())
                })
                .on_mouse_down_out(window.listener_for(&drag_state, |state, _, _, _| {
                    state.should_move = false;
                }))
                .on_mouse_down(
                    MouseButton::Left,
                    window.listener_for(&drag_state, |state, _, _, _| {
                        state.should_move = true;
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    window.listener_for(&drag_state, |state, _, _, _| {
                        state.should_move = false;
                    }),
                )
                .on_mouse_move(window.listener_for(
                    &drag_state,
                    |state, _, window, _| {
                        if state.should_move {
                            state.should_move = false;
                            window.start_window_move();
                        }
                    },
                ))
            })
            .when(is_macos, |this| {
                this.child(
                    div()
                        .flex_shrink_0()
                        .h_full()
                        .w(left_padding)
                        .when_some(self.top_padding, |div, padding| div.pt(padding)),
                )
            })
            // Pinned tab (fixed, not scrollable)
            .when_some(self.pinned_tab.as_ref(), |this, pinned| {
                let pinned_title = pinned.content().title(cx);
                let pinned_icon = pinned.content().icon(cx);
                let is_pinned_active = self.pinned_tab_active;
                let view_for_pinned = view.clone();
                let top_padding = self.top_padding;

                this.child(
                    div()
                        .id("pinned-tab")
                        .flex()
                        .flex_shrink_0()
                        .overflow_hidden()
                        .items_center()
                        .gap_2()
                        .h(px(32.0))
                        .px_3()
                        .when(!is_macos, |el| el.ml(left_padding))
                        .when_some(top_padding, |el, padding| el.mt(padding))
                        .rounded(px(6.0))
                        .when(is_pinned_active, |el| el.bg(active_tab_color))
                        .when(!is_pinned_active, |el| {
                            el.hover(move |style| style.bg(hover_tab_color))
                                .bg(inactive_tab_color)
                        })
                        .cursor_pointer()
                        .on_click(move |_, window, cx| {
                            view_for_pinned.update(cx, |this, cx| {
                                this.activate_pinned_tab(window, cx);
                            });
                        })
                        .when_some(pinned_icon, |el, icon| {
                            el.child(div().flex_shrink_0().flex().items_center().child(icon))
                        })
                        .child(
                            div()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_sm()
                                .text_color(text_color)
                                .text_ellipsis()
                                .child(pinned_title.to_string()),
                        ),
                )
                // Separator between pinned tab and scrollable tabs
                .child(
                    div()
                        .flex_shrink_0()
                        .mx_1()
                        .when_some(top_padding, |el, padding| el.mt(padding))
                        .w(px(1.0))
                        .h(px(16.0))
                        .bg(border_color),
                )
            })
            .child(
                h_flex()
                    .id("tabs")
                    .flex_1()
                    .h_full()
                    .items_center()
                    // 仅在启用窗口控件时设置拖动区域（用于 Windows 原生拖动）
                    .when(show_window_controls, |this| {
                        this.window_control_area(WindowControlArea::Drag)
                            .on_mouse_down_out(window.listener_for(
                                &drag_state,
                                |state, _, _, _| {
                                    state.should_move = false;
                                },
                            ))
                            .on_mouse_down(
                                MouseButton::Left,
                                window.listener_for(&drag_state, |state, _, _, _| {
                                    state.should_move = true;
                                }),
                            )
                            .on_mouse_up(
                                MouseButton::Left,
                                window.listener_for(&drag_state, |state, _, _, _| {
                                    state.should_move = false;
                                }),
                            )
                            .on_mouse_move(window.listener_for(
                                &drag_state,
                                |state, _, window, _| {
                                    if state.should_move {
                                        state.should_move = false;
                                        window.start_window_move();
                                    }
                                },
                            ))
                    })
                    .overflow_x_scroll()
                    .when(!is_macos && self.pinned_tab.is_none(), |this| {
                        this.pl(left_padding)
                    })
                    .when_some(self.top_padding, |div, padding| div.pt(padding))
                    .pr_2()
                    .gap_1()
                    .track_scroll(&self.tab_bar_scroll_handle)
                    // Linux 客户端装饰模式下，右键显示窗口菜单
                    .when(
                        is_linux && is_client_decorated && show_window_controls,
                        |this| {
                            this.child(
                                div()
                                    .top_0()
                                    .left_0()
                                    .absolute()
                                    .size_full()
                                    .h_full()
                                    .on_mouse_down(MouseButton::Right, move |ev, window, _| {
                                        window.show_window_menu(ev.position)
                                    }),
                            )
                        },
                    )
                    .children(self.tabs.iter().enumerate().map(|(idx, tab)| {
                        let title = tab.content().title(cx);
                        let icon = tab.content().icon(cx);
                        let closeable = tab.content().closeable(cx);
                        let is_active = idx == active_index;
                        let view_clone = view.clone();
                        let title_clone = title.clone();
                        let tab_width = self.get_tab_width(tab, cx);

                        div()
                            .id(idx)
                            .flex()
                            .flex_shrink_0()
                            .overflow_hidden()
                            .items_center()
                            .gap_2()
                            .h(px(32.0))
                            .text_ellipsis()
                            .w(tab_width)
                            .px_3()
                            .rounded(px(6.0))
                            .when(is_active, |el| el.bg(active_tab_color))
                            .when(!is_active, |el| {
                                el.hover(move |style| style.bg(hover_tab_color))
                                    .bg(inactive_tab_color)
                            })
                            .when(allow_tab_drag, |el| {
                                el.cursor_grab()
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        move |_evt, window: &mut Window, cx| {
                                            window.prevent_default();
                                            cx.stop_propagation();
                                        },
                                    )
                                    .on_mouse_move(move |_evt, window: &mut Window, cx| {
                                        window.prevent_default();
                                        cx.stop_propagation();
                                    })
                                    .on_drag(
                                        DragTab::new(idx, title.clone())
                                            .with_source_pane(view.clone()),
                                        |drag, _, window, cx| {
                                            window.prevent_default();
                                            cx.stop_propagation();
                                            cx.new(|_| drag.clone())
                                        },
                                    )
                                    .drag_over::<DragTab>(move |el, _, _, _cx| {
                                        el.border_l_2().border_color(drag_border_color)
                                    })
                                    .on_drop(cx.listener(
                                        move |this, drag: &DragTab, window, cx| {
                                            let from_idx = drag.tab_index;
                                            let to_idx = idx;
                                            let source = drag
                                                .source_pane
                                                .clone()
                                                .unwrap_or_else(|| cx.entity());

                                            if source != cx.entity() {
                                                let moved = source.update(cx, |source, cx| {
                                                    source.take_tab(from_idx, window, cx)
                                                });
                                                if let Some(tab) = moved {
                                                    this.tabs.insert(to_idx, tab);
                                                    this.set_active_index(to_idx, window, cx);
                                                    cx.emit(TabContainerEvent::LayoutChanged);
                                                    cx.notify();
                                                }
                                            } else if from_idx != to_idx {
                                                this.move_tab(from_idx, to_idx, cx);
                                                this.set_active_index(to_idx, window, cx);
                                            } else {
                                                this.set_active_index(to_idx, window, cx);
                                            }
                                        },
                                    ))
                            })
                            .on_click(cx.listener(move |this, _event, window, cx| {
                                window.prevent_default();
                                this.set_active_index(idx, window, cx);
                            }))
                            .when_some(icon, |el, icon| {
                                el.child(div().flex_shrink_0().flex().items_center().child(icon))
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_sm()
                                    .text_color(text_color)
                                    .text_ellipsis()
                                    .child(title_clone.to_string()),
                            )
                            .when(closeable, |el| {
                                let view_clone = view_clone.clone();
                                el.child(
                                    div()
                                        .flex_shrink_0()
                                        .w(px(16.0))
                                        .h(px(16.0))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(px(2.0))
                                        .cursor_pointer()
                                        .text_color(close_btn_color)
                                        .hover(|style| {
                                            style.bg(gpui::rgb(0x5a5a5a)).text_color(text_color)
                                        })
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            move |_event, window, cx| {
                                                cx.stop_propagation();
                                                view_clone.update(cx, |this, cx| {
                                                    this.close_tab(idx, window, cx).detach();
                                                });
                                            },
                                        )
                                        .child("×"),
                                )
                            })
                            .context_menu(move |menu, window, cx| {
                                let view_for_menu = view_clone.clone();
                                let tab_count = view_for_menu.read(cx).tabs.len();
                                let has_tabs_left = idx > 0;
                                let has_tabs_right = idx < tab_count - 1;
                                let can_split = view_for_menu
                                    .read(cx)
                                    .tabs
                                    .get(idx)
                                    .map(|tab| tab.content().can_split(cx))
                                    .unwrap_or(false);
                                let split_enabled = split_command_enabled(
                                    view_for_menu.read(cx).split_enabled,
                                    can_split,
                                    tab_count,
                                );
                                let closeable = view_for_menu
                                    .read(cx)
                                    .tabs
                                    .get(idx)
                                    .map(|tab| tab.content().closeable(cx))
                                    .unwrap_or(false);

                                menu.item(
                                    PopupMenuItem::new("Split Right")
                                        .disabled(!split_enabled)
                                        .on_click(window.listener_for(
                                            &view_for_menu,
                                            move |_this, _, _window, cx| {
                                                cx.emit(TabContainerEvent::SplitRequested {
                                                    placement: Placement::Right,
                                                    source: cx.entity(),
                                                    tab_index: idx,
                                                });
                                            },
                                        )),
                                )
                                .item(
                                    PopupMenuItem::new("Split Down")
                                        .disabled(!split_enabled)
                                        .on_click(window.listener_for(
                                            &view_for_menu,
                                            move |_this, _, _window, cx| {
                                                cx.emit(TabContainerEvent::SplitRequested {
                                                    placement: Placement::Bottom,
                                                    source: cx.entity(),
                                                    tab_index: idx,
                                                });
                                            },
                                        )),
                                )
                                .item(PopupMenuItem::new("Close").disabled(!closeable).on_click(
                                    window.listener_for(
                                        &view_for_menu,
                                        move |this, _, window, cx| {
                                            this.close_tab(idx, window, cx).detach();
                                        },
                                    ),
                                ))
                                .item(PopupMenuItem::new("Close All").on_click(
                                    window.listener_for(
                                        &view_for_menu,
                                        move |this, _, window, cx| {
                                            this.close_all_tabs(window, cx).detach();
                                        },
                                    ),
                                ))
                                .item(
                                    PopupMenuItem::new("Close Others")
                                        .disabled(tab_count <= 1)
                                        .on_click(window.listener_for(
                                            &view_for_menu,
                                            move |this, _, window, cx| {
                                                this.close_other_tabs(idx, window, cx).detach();
                                            },
                                        )),
                                )
                                .item(
                                    PopupMenuItem::new("Close Tabs To The Left")
                                        .disabled(!has_tabs_left)
                                        .on_click(window.listener_for(
                                            &view_for_menu,
                                            move |this, _, window, cx| {
                                                this.close_tabs_to_left(idx, window, cx).detach();
                                            },
                                        )),
                                )
                                .item(
                                    PopupMenuItem::new("Close Tabs To The Right")
                                        .disabled(!has_tabs_right)
                                        .on_click(window.listener_for(
                                            &view_for_menu,
                                            move |this, _, window, cx| {
                                                this.close_tabs_to_right(idx, window, cx).detach();
                                            },
                                        )),
                                )
                            })
                    })),
            )
            .child(
                Popover::new("tab-list-popover")
                    .anchor(Anchor::TopRight)
                    .p_0()
                    .open(self.list_popover_open)
                    .on_open_change(cx.listener(move |this, open, window, cx| {
                        this.list_popover_open = *open;
                        if *open {
                            let tabs_data: Vec<(usize, SharedString, Option<Icon>, bool)> = this
                                .tabs
                                .iter()
                                .enumerate()
                                .map(|(idx, tab)| {
                                    (
                                        idx,
                                        tab.content().title(cx),
                                        tab.content().icon(cx),
                                        tab.content().closeable(cx),
                                    )
                                })
                                .collect();
                            let container = cx.entity();

                            if let Some(tab_list) = &this.tab_list {
                                tab_list.update(cx, |state, _| {
                                    let delegate = state.delegate_mut();
                                    delegate.tabs = tabs_data.clone();
                                    delegate.filtered_tabs = tabs_data;
                                });
                            } else {
                                this.tab_list = Some(cx.new(|cx| {
                                    ListState::new(
                                        TabListDelegate {
                                            container,
                                            tabs: tabs_data.clone(),
                                            filtered_tabs: tabs_data,
                                            selected_index: None,
                                        },
                                        window,
                                        cx,
                                    )
                                    .searchable(true)
                                }));
                            }
                        }
                        cx.notify();
                    }))
                    .when_some(tab_list.as_ref(), |popover, list| {
                        popover.track_focus(&list.focus_handle(cx))
                    })
                    .trigger(
                        Button::new("tab-dropdown-btn")
                            .icon(IconName::ChevronDown)
                            .ghost()
                            .compact(),
                    )
                    .when_some(tab_list, |popover, list| {
                        popover.child(
                            List::new(&list)
                                .w(px(280.0))
                                .max_h(px(300.0))
                                .border_1()
                                .border_color(cx.theme().border)
                                .rounded(cx.theme().radius),
                        )
                    }),
            )
            .when(
                cfg!(not(target_os = "macos")) && self.show_window_controls,
                |el| el.child(self.render_window_controls(window)),
            )
    }

    fn render_window_controls(&self, window: &mut Window) -> impl IntoElement {
        let is_linux = cfg!(target_os = "linux");
        let is_windows = cfg!(target_os = "windows");
        let is_maximized = window.is_maximized();

        h_flex()
            .id("window-controls")
            .items_center()
            .flex_shrink_0()
            .h_full()
            .when_some(self.on_toggle_always_on_top.clone(), |el, on_toggle| {
                let is_active = self
                    .is_always_on_top
                    .as_ref()
                    .map(|probe| probe())
                    .unwrap_or(false);
                el.child(self.render_always_on_top_button(on_toggle, is_active))
            })
            .child(self.render_control_button(
                "minimize",
                IconName::WindowMinimize,
                WindowControlArea::Min,
                is_linux,
                is_windows,
                false,
            ))
            .child(self.render_control_button(
                if is_maximized { "restore" } else { "maximize" },
                if is_maximized {
                    IconName::WindowRestore
                } else {
                    IconName::WindowMaximize
                },
                WindowControlArea::Max,
                is_linux,
                is_windows,
                false,
            ))
            .child(self.render_control_button(
                "close",
                IconName::WindowClose,
                WindowControlArea::Close,
                is_linux,
                is_windows,
                true,
            ))
    }

    fn render_control_button(
        &self,
        id: &'static str,
        icon: IconName,
        control_area: WindowControlArea,
        is_linux: bool,
        is_windows: bool,
        is_close: bool,
    ) -> impl IntoElement {
        div()
            .id(id)
            .flex()
            .w(px(34.0))
            .h_full()
            .flex_shrink_0()
            .justify_center()
            .content_center()
            .items_center()
            .text_color(gpui::white())
            .hover(move |style| {
                if is_close {
                    style.bg(gpui::rgb(0xe81123)).text_color(gpui::white())
                } else {
                    style.bg(gpui::rgb(0x3a3a3a)).text_color(gpui::white())
                }
            })
            .active(move |style| {
                if is_close {
                    style.bg(gpui::rgb(0xc50f1f)).text_color(gpui::white())
                } else {
                    style.bg(gpui::rgb(0x2a2a2a)).text_color(gpui::white())
                }
            })
            .when(is_windows, move |this| {
                // Windows 依赖系统原生标题栏控件行为：
                // 仅声明 control area，避免手动 on_click 干扰最大化/还原切换。
                this.window_control_area(control_area)
            })
            .when(is_linux, move |this| {
                this.on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                })
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    match control_area {
                        WindowControlArea::Min => window.minimize_window(),
                        WindowControlArea::Max => window.zoom_window(),
                        WindowControlArea::Close => window.remove_window(),
                        _ => {}
                    }
                })
            })
            .child(Icon::new(icon).with_size(Size::Small))
    }

    /// 渲染窗口置顶按钮，位于最小化按钮左侧。
    /// 该按钮不声明系统窗口控制区，点击时由上层注入的回调完成切换。
    fn render_always_on_top_button(
        &self,
        on_toggle: Arc<dyn Fn(&mut Window, &mut App) + Send + Sync>,
        is_active: bool,
    ) -> impl IntoElement {
        // 置顶激活时用琥珀色高亮，提示当前窗口已置顶
        let icon_color = if is_active {
            gpui::rgb(0xfbbf24)
        } else {
            gpui::rgb(0xffffff)
        };

        div()
            .id("always-on-top")
            .flex()
            .w(px(34.0))
            .h_full()
            .flex_shrink_0()
            .justify_center()
            .content_center()
            .items_center()
            .text_color(icon_color)
            .hover(move |style| style.bg(gpui::rgb(0x3a3a3a)).text_color(icon_color))
            .active(move |style| style.bg(gpui::rgb(0x2a2a2a)).text_color(icon_color))
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                on_toggle(window, cx);
            })
            .child(Icon::new(IconName::Pin).with_size(Size::Small))
    }
}

fn normalize_sidebar_placement(
    requested: SidebarPlacement,
    policy: SidebarPanelPolicy,
) -> SidebarPlacement {
    if policy.allowed_placements.contains(requested) {
        return requested;
    }
    if policy.allowed_placements.right {
        SidebarPlacement::Right
    } else if policy.allowed_placements.left {
        SidebarPlacement::Left
    } else {
        SidebarPlacement::Bottom
    }
}

#[derive(Clone)]
struct SidebarResizeDrag;

struct SidebarResizePanel;

impl Render for SidebarResizePanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

struct SidebarResizeEventHandler {
    container: Entity<TabContainer>,
}

impl IntoElement for SidebarResizeEventHandler {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for SidebarResizeEventHandler {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (window.request_layout(Style::default(), None, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.container.update(cx, |container, _| {
            container.sidebar_bounds = bounds;
        });
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.on_mouse_event({
            let container = self.container.clone();
            let resizing = container.read(cx).sidebar_resizing.clone();
            move |event: &MouseMoveEvent, phase, window, cx| {
                if resizing.is_none() || !phase.bubble() {
                    return;
                }
                container.update(cx, |container, cx| {
                    container.resize_sidebar_panel(event.position, window, cx);
                });
            }
        });

        window.on_mouse_event({
            let container = self.container.clone();
            move |_: &MouseUpEvent, phase, window, cx| {
                if phase.bubble() {
                    container.update(cx, |container, cx| {
                        container.finish_sidebar_resize(window, cx);
                    });
                }
            }
        });
    }
}

impl Focusable for TabContainer {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TabContainer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focus_handle = self.focus_handle(cx);

        div()
            .id("tab-container")
            .track_focus(&focus_handle)
            .relative()
            .size_full()
            .child(
                v_flex()
                    .size_full()
                    .child(self.render_tab_bar(window, cx))
                    .child(self.render_tab_content(window, cx)),
            )
    }
}
