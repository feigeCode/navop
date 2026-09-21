//! 表格查找面板
//!
//! 面板只负责「查询词 + 命中计数 + 上下导航」的输入交互，
//! 真正的行匹配由 delegate（知道数据的那一侧）在查询变化时完成。

use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, Subscription, Window, div,
    px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, Size,
    button::Button,
    h_flex,
    input::{Input, InputEvent, InputState},
};
use one_assets::IconName;
use rust_i18n::t;

use super::{FindOutcome, normalize_find_query};

/// 面板对外抛出的查询动作。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchPanelEvent {
    /// 查询词变化（已规范化），`None` 表示清空查询
    QueryChanged(Option<String>),
    /// 跳到下一个命中
    NextMatch,
    /// 跳到上一个命中
    PreviousMatch,
    /// 关闭查找
    Dismissed,
}

/// 查找面板状态与视图。
pub struct SearchPanel {
    input: Entity<InputState>,
    focus_handle: FocusHandle,
    outcome: FindOutcome,
    current_match: usize,
    _input_subscription: Option<Subscription>,
}

impl SearchPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("EditTable.find_placeholder").to_string())
                .clean_on_escape()
        });
        let subscription = cx.subscribe_in(&input, window, Self::on_input_event);
        Self {
            input,
            focus_handle: cx.focus_handle(),
            outcome: FindOutcome::default(),
            current_match: 0,
            _input_subscription: Some(subscription),
        }
    }

    fn on_input_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(event, InputEvent::Change) {
            return;
        }
        self.emit_query(input.read(cx).text().to_string(), cx);
    }

    /// 规范化并上报查询词；空查询会同时清空命中统计。
    fn emit_query(&mut self, raw: String, cx: &mut Context<Self>) {
        let query = normalize_find_query(&raw);
        if query.is_empty() {
            self.outcome = FindOutcome::default();
            self.current_match = 0;
        }
        cx.emit(SearchPanelEvent::QueryChanged(
            (!query.is_empty()).then_some(query),
        ));
        cx.notify();
    }

    /// 焦点落到查询输入框。
    pub fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.input.clone();
        input.update(cx, |state, cx| {
            state.focus(window, cx);
        });
        cx.notify();
    }

    /// 查询词（已规范化）
    pub fn query(&self, cx: &App) -> String {
        normalize_find_query(&self.input.read(cx).text())
    }

    /// 更新命中统计。
    pub fn set_outcome(&mut self, outcome: FindOutcome, cx: &mut Context<Self>) {
        self.outcome = outcome;
        self.current_match = 0;
        cx.notify();
    }

    /// 更新当前命中序号（从 1 开始）。
    pub fn set_current_match(&mut self, current: usize, cx: &mut Context<Self>) {
        self.current_match = current;
        cx.notify();
    }

    /// 清空查询词（关闭查找时调用）。
    pub fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.outcome = FindOutcome::default();
        self.current_match = 0;
        self.input.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
        cx.notify();
    }

    /// 查询词是否为空
    pub fn is_query_empty(&self, cx: &App) -> bool {
        self.query(cx).is_empty()
    }

    fn handle_next(&mut self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(SearchPanelEvent::NextMatch);
    }

    fn handle_previous(&mut self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(SearchPanelEvent::PreviousMatch);
    }

    fn handle_close(&mut self, _: &gpui::ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.clear(window, cx);
        cx.emit(SearchPanelEvent::Dismissed);
    }

    fn counter_label(&self, cx: &App) -> SharedString {
        if self.is_query_empty(cx) {
            return SharedString::default();
        }
        if self.outcome.is_empty() {
            return t!("EditTable.find_no_match").to_string().into();
        }
        format!(
            "{}/{}",
            self.current_match.max(1).min(self.outcome.total),
            self.outcome.total
        )
        .into()
    }
}

impl EventEmitter<SearchPanelEvent> for SearchPanel {}

impl Focusable for SearchPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SearchPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let counter = self.counter_label(cx);
        let has_match = !self.outcome.is_empty();

        h_flex()
            .id("edit-table-find")
            .gap_1()
            .items_center()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .shadow_md()
            .child(
                Icon::new(IconName::Search)
                    .xsmall()
                    .text_color(cx.theme().muted_foreground),
            )
            .child(
                div()
                    .w(px(200.))
                    .child(Input::new(&self.input).small().cleanable(true)),
            )
            .when(!counter.is_empty(), |this| {
                this.child(
                    div()
                        .min_w(px(48.))
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(counter),
                )
            })
            .child(
                Button::new("find-previous")
                    .ghost()
                    .small()
                    .icon(IconName::ChevronUp)
                    .disabled(!has_match)
                    .tooltip(t!("EditTable.find_previous").to_string())
                    .on_click(cx.listener(Self::handle_previous)),
            )
            .child(
                Button::new("find-next")
                    .ghost()
                    .small()
                    .icon(IconName::ChevronDown)
                    .disabled(!has_match)
                    .tooltip(t!("EditTable.find_next").to_string())
                    .on_click(cx.listener(Self::handle_next)),
            )
            .child(
                Button::new("find-close")
                    .ghost()
                    .small()
                    .icon(IconName::Close)
                    .tooltip(t!("EditTable.find_close").to_string())
                    .on_click(cx.listener(Self::handle_close)),
            )
    }
}
