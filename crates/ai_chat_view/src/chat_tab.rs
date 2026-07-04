//! Workbench tab adapter for the reusable chat view.
//!
//! The adapter owns tab-container concerns while `ChatView` keeps rendering
//! messages and the task-history sidebar.

use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, Styled, Window, div,
};
use gpui_component::{Icon, IconName, Sizable, Size};
use one_core::tab_container::{TabContent, TabContentEvent, TabItem};

use crate::{ChatView, ChatViewState};

pub const CHAT_TAB_CONTENT_KEY: &str = "ChatWorkbench";

pub struct ChatTabContent {
    view: Entity<ChatView>,
    focus_handle: FocusHandle,
    title: SharedString,
}

impl ChatTabContent {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new_with_state(ChatViewState::new(), window, cx)
    }

    pub fn new_with_state(
        state: ChatViewState,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let view = cx.new(|_| ChatView::from_state(state));
        Self {
            view,
            focus_handle: cx.focus_handle(),
            title: SharedString::from("AI 工作台"),
        }
    }

    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self::new(window, cx))
    }

    pub fn tab_item(
        id: impl Into<String>,
        from: impl Into<String>,
        window: &mut Window,
        cx: &mut App,
    ) -> TabItem {
        TabItem::new(id, from, Self::view(window, cx))
    }

    pub fn inner(&self) -> Entity<ChatView> {
        self.view.clone()
    }
}

impl EventEmitter<TabContentEvent> for ChatTabContent {}

impl Focusable for ChatTabContent {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ChatTabContent {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .child(self.view.clone())
    }
}

impl TabContent for ChatTabContent {
    fn content_key(&self) -> &'static str {
        CHAT_TAB_CONTENT_KEY
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
