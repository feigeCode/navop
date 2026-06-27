//! Agent 作为通用 TabContent 的预设封装。
//!
//! 该类型模仿项目里 Database/Terminal 等 tab content 的接入方式:
//! 它只负责 tab 容器协议(title/icon/focus/render),真实 Agent 交互仍由
//! [`AgentChatView`](crate::AgentChatView) 完成。

use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, Styled, Window, div,
};
use gpui_component::{Icon, IconName, Sizable, Size};
use one_core::tab_container::{TabContent, TabContentEvent, TabItem};

use crate::agent_view::{AgentChatView, AgentChatViewConfig};

pub const AGENT_TAB_CONTENT_KEY: &str = "Agent";

/// 可直接放进 `TabContainer` 的 Agent 预设 tab。
pub struct AgentTabContent {
    view: Entity<AgentChatView>,
    focus_handle: FocusHandle,
    title: SharedString,
}

impl AgentTabContent {
    pub fn new(config: AgentChatViewConfig, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let view = cx.new(|cx| AgentChatView::new(config, window, cx));
        Self {
            view,
            focus_handle: cx.focus_handle(),
            title: SharedString::from("Agent"),
        }
    }

    pub fn view(config: AgentChatViewConfig, window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self::new(config, window, cx))
    }

    pub fn inner(&self) -> Entity<AgentChatView> {
        self.view.clone()
    }

    pub fn tab_item(
        id: impl Into<String>,
        from: impl Into<String>,
        config: AgentChatViewConfig,
        window: &mut Window,
        cx: &mut App,
    ) -> TabItem {
        TabItem::new(id, from, Self::view(config, window, cx))
    }
}

impl EventEmitter<TabContentEvent> for AgentTabContent {}

impl Focusable for AgentTabContent {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for AgentTabContent {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .child(self.view.clone())
    }
}

impl TabContent for AgentTabContent {
    fn content_key(&self) -> &'static str {
        AGENT_TAB_CONTENT_KEY
    }

    fn title(&self, _cx: &App) -> SharedString {
        self.title.clone()
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        Some(IconName::AI.color().with_size(Size::Medium))
    }

    fn closeable(&self, _cx: &App) -> bool {
        false
    }

    fn width_size(&self, _cx: &App) -> Option<Size> {
        Some(Size::Small)
    }

    fn dump(&self, _cx: &App) -> serde_json::Value {
        serde_json::json!({
            "version": 1,
        })
    }
}
