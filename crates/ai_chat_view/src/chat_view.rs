use crate::message_view::render_messages;
use crate::session_sidebar::{self, SessionSummary};
use crate::{ChatMessageUI, ChatViewState};
use gpui::{
    App, AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder,
};
use gpui_component::{ActiveTheme, IconName, h_flex, v_flex};
use one_ui::{IconButton, IconButtonRole, PanelHeader, PanelHeaderVariant};
use rust_i18n::t;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn chat_task_sidebar_title() -> String {
    t!("AgentUi.history_tasks").to_string()
}

pub struct ChatView {
    state: ChatViewState,
    sidebar_collapsed: bool,
    sidebar_hidden: bool,
    scroll_handle: ScrollHandle,
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

const SAMPLE_JSON: &str =
    r#"{"name":"onetcli","feature":"chat cards","registered":["json"],"ok":true}"#;

impl ChatView {
    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self::new(window, cx))
    }

    pub fn view_with_state(
        state: ChatViewState,
        _window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        cx.new(|_| Self::from_state(state))
    }

    fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self::from_state(Self::sample_state())
    }

    pub(crate) fn from_state(state: ChatViewState) -> Self {
        Self {
            state,
            sidebar_collapsed: false,
            sidebar_hidden: false,
            scroll_handle: ScrollHandle::new(),
        }
    }

    pub fn state(&self) -> &ChatViewState {
        &self.state
    }

    pub fn replace_messages(&mut self, messages: Vec<ChatMessageUI>, cx: &mut Context<Self>) {
        self.state.replace_messages(messages);
        cx.notify();
    }

    pub fn push_message(&mut self, message: ChatMessageUI, cx: &mut Context<Self>) {
        self.state.replace_messages(
            self.state
                .messages()
                .iter()
                .cloned()
                .chain(std::iter::once(message))
                .collect(),
        );
        self.scroll_handle.scroll_to_bottom();
        cx.notify();
    }

    pub fn push_assistant(&mut self, content: impl Into<String>, cx: &mut Context<Self>) {
        self.state.push_assistant(content);
        self.scroll_handle.scroll_to_bottom();
        cx.notify();
    }

    pub fn push_card(
        &mut self,
        kind: impl Into<String>,
        content: impl Into<String>,
        cx: &mut Context<Self>,
    ) {
        self.state.push_card(kind, content);
        self.scroll_handle.scroll_to_bottom();
        cx.notify();
    }

    pub fn clear_messages(&mut self, cx: &mut Context<Self>) {
        self.state.clear_messages();
        cx.notify();
    }

    fn sample_state() -> ChatViewState {
        let now = now_secs();
        let messages = vec![
            ChatMessageUI::system("通用聊天视图(机制演示):侧栏可收起,卡片可扩展。"),
            ChatMessageUI::user("给我看看 JSON 卡片"),
            ChatMessageUI::card("json", SAMPLE_JSON),
            ChatMessageUI::assistant("上面是由 `JsonCard` 自定义渲染的卡片。"),
        ];
        let sessions = vec![
            SessionSummary::new("s1", "当前任务", now),
            SessionSummary::new("s2", "昨天的排查任务", now - 90_000),
            SessionSummary::new("s3", "上周的笔记任务", now - 700_000),
        ];
        ChatViewState::with_messages(messages).with_sessions(sessions)
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        cx.notify();
    }

    fn set_sidebar_hidden(&mut self, hidden: bool, cx: &mut Context<Self>) {
        self.sidebar_hidden = hidden;
        self.sidebar_collapsed = false;
        cx.notify();
    }

    fn new_session(&mut self, cx: &mut Context<Self>) {
        let mut sessions = self.state.sessions().to_vec();
        let id = format!("s{}", sessions.len() + 1);
        sessions.insert(0, SessionSummary::new(id, "新任务", now_secs()));
        self.state.replace_sessions(sessions);
        self.state.clear_messages();
        cx.notify();
    }

    fn render_sidebar(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let border = cx.theme().border;
        let muted = cx.theme().muted;

        if self.sidebar_hidden {
            return div().into_any_element();
        }

        if self.sidebar_collapsed {
            return v_flex()
                .w(cx.theme().geometry.layout.compact_rail)
                .h_full()
                .flex_shrink_0()
                .border_r_1()
                .border_color(border)
                .bg(muted)
                .items_center()
                .py_2()
                .gap_2()
                .child(
                    IconButton::new("ai-chat-expand-sidebar", IconName::PanelLeftOpen)
                        .role(IconButtonRole::Compact)
                        .tooltip(t!("AgentUi.open_sidebar").to_string())
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
                )
                .child(
                    IconButton::new("ai-chat-new-session-collapsed", IconName::Plus)
                        .role(IconButtonRole::Compact)
                        .tooltip(t!("AgentUi.new_task").to_string())
                        .on_click(cx.listener(|this, _, _, cx| this.new_session(cx))),
                )
                .into_any_element();
        }

        let mut rows: Vec<gpui::AnyElement> = Vec::new();
        for session in self.state.sessions() {
            let id = session.id.clone();
            let is_current = self.state.current_session_id() == Some(session.id.as_str());
            let row = session_sidebar::session_row(session, is_current, cx)
                .id(SharedString::from(format!(
                    "ai-chat-session-{}",
                    session.id
                )))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.state.select_session(&id) {
                        cx.notify();
                    }
                }));
            rows.push(row.into_any_element());
        }

        v_flex()
            .w(cx.theme().geometry.layout.context_sidebar_default)
            .h_full()
            .min_h_0()
            .flex_shrink_0()
            .border_r_1()
            .border_color(border)
            .bg(muted)
            .child(
                PanelHeader::new("ai-chat-sidebar-header")
                    .variant(PanelHeaderVariant::Sidebar)
                    .background(muted)
                    .border_color(border)
                    .leading(
                        IconButton::new("ai-chat-collapse-sidebar", IconName::PanelLeftClose)
                            .role(IconButtonRole::Compact)
                            .tooltip(t!("AgentUi.close_sidebar").to_string())
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
                    )
                    .title_text(chat_task_sidebar_title())
                    .trailing(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(
                                IconButton::new("ai-chat-new-session", IconName::Plus)
                                    .role(IconButtonRole::Compact)
                                    .tooltip(t!("AgentUi.new_task").to_string())
                                    .on_click(cx.listener(|this, _, _, cx| this.new_session(cx))),
                            )
                            .child(
                                IconButton::new("ai-chat-close-sidebar", IconName::Close)
                                    .role(IconButtonRole::Compact)
                                    .tooltip(t!("AgentUi.close_sidebar").to_string())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_sidebar_hidden(true, cx)
                                    })),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .id("ai-chat-session-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_2()
                    .gap_1()
                    .children(rows),
            )
            .into_any_element()
    }
}

impl Render for ChatView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sidebar = self.render_sidebar(cx);
        let messages = render_messages(self.state.messages(), &self.scroll_handle, window, cx);

        h_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(sidebar)
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .relative()
                    .child(v_flex().size_full().child(messages))
                    .when(self.sidebar_hidden, |this| {
                        this.child(
                            div().absolute().top_2().left_2().child(
                                IconButton::new("ai-chat-open-sidebar", IconName::PanelLeftOpen)
                                    .role(IconButtonRole::Compact)
                                    .tooltip(t!("AgentUi.open_sidebar").to_string())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_sidebar_hidden(false, cx)
                                    })),
                            ),
                        )
                    }),
            )
    }
}
