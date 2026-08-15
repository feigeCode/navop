use gpui::prelude::FluentBuilder;
use gpui::{
    FontWeight, InteractiveElement, IntoElement, ParentElement, Render, Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    scroll::ScrollableElement,
    v_flex,
};
use rust_i18n::t;
use terminal::recording::SessionLogEntry;

use super::SessionLogsPage;

impl Render for SessionLogsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let entries = self.filtered_entries(cx);
        let filtered = entries.len();
        let total = self.catalog.entries.len();
        let has_query = !self.search_input.read(cx).value().trim().is_empty();
        let content = self.render_content(entries, has_query, cx);

        v_flex()
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .child(self.render_toolbar(cx))
            .child(
                v_flex()
                    .w_full()
                    .min_h_0()
                    .flex_1()
                    .overflow_hidden()
                    .child(self.render_header(total, filtered, cx))
                    .child(content),
            )
    }
}

impl SessionLogsPage {
    fn render_toolbar(&self, cx: &gpui::Context<Self>) -> impl IntoElement {
        h_flex()
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .flex_wrap()
            .justify_between()
            .items_center()
            .gap_3()
            .px_4()
            .py_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                Button::new("session-logs-refresh")
                    .icon(IconName::Refresh)
                    .label(if self.loading {
                        t!("SessionLogs.refreshing").to_string()
                    } else {
                        t!("SessionLogs.refresh").to_string()
                    })
                    .small()
                    .ghost()
                    .disabled(self.loading || self.favorite_saving)
                    .on_click(cx.listener(|page, _, _, cx| page.refresh(cx))),
            )
            .child(
                div().min_w(px(240.0)).max_w(px(480.0)).flex_1().child(
                    Input::new(&self.search_input)
                        .prefix(Icon::new(IconName::Search).text_color(cx.theme().muted_foreground))
                        .cleanable(true)
                        .small()
                        .w_full(),
                ),
            )
    }

    fn render_header(
        &self,
        total: usize,
        filtered: usize,
        cx: &gpui::Context<Self>,
    ) -> impl IntoElement {
        let summary = if total == filtered {
            format!("{total}")
        } else {
            format!("{filtered} / {total}")
        };
        v_flex()
            .w_full()
            .flex_shrink_0()
            .gap_1()
            .px_4()
            .pt_4()
            .pb_3()
            .child(self.render_header_summary(summary, cx))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("SessionLogs.source_help").to_string()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("SessionLogs.output_only_help").to_string()),
            )
    }

    fn render_header_summary(&self, summary: String, cx: &gpui::Context<Self>) -> impl IntoElement {
        h_flex()
            .w_full()
            .items_center()
            .gap_2()
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::BOLD)
                    .child(t!("SessionLogs.title").to_string()),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(summary),
            )
            .when(!self.catalog.skipped.is_empty(), |this| {
                this.child(div().text_xs().text_color(cx.theme().warning).child(
                    t!("SessionLogs.skipped", count = self.catalog.skipped.len()).to_string(),
                ))
            })
    }

    fn render_content(
        &self,
        entries: Vec<SessionLogEntry>,
        has_query: bool,
        cx: &gpui::Context<Self>,
    ) -> gpui::AnyElement {
        if let Some(error) = self.load_error.clone() {
            return error_state(error, cx).into_any_element();
        }
        if entries.is_empty() {
            return empty_state(has_query, self.loading, cx).into_any_element();
        }
        div()
            .id("session-logs-list")
            .w_full()
            .min_h_0()
            .flex_1()
            .overflow_y_scrollbar()
            .px_4()
            .pb_4()
            .child(
                v_flex().gap_2().children(
                    entries
                        .into_iter()
                        .map(|entry| self.render_entry(entry, cx)),
                ),
            )
            .into_any_element()
    }
}

fn empty_state(has_query: bool, loading: bool, cx: &gpui::App) -> impl IntoElement {
    let title = if loading {
        t!("SessionLogs.refreshing").to_string()
    } else if has_query {
        t!("SessionLogs.empty_search").to_string()
    } else {
        t!("SessionLogs.empty").to_string()
    };
    v_flex()
        .w_full()
        .min_h_0()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_3()
        .p_6()
        .child(Icon::new(if has_query {
            IconName::Search
        } else {
            IconName::Terminal
        }))
        .child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .text_color(cx.theme().muted_foreground)
                .child(title),
        )
}

fn error_state(error: String, cx: &gpui::Context<SessionLogsPage>) -> impl IntoElement {
    v_flex()
        .w_full()
        .min_h_0()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_3()
        .p_6()
        .child(Icon::new(IconName::Refresh).text_color(cx.theme().danger))
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().danger)
                .child(t!("SessionLogs.load_failed", error = error).to_string()),
        )
        .child(
            Button::new("session-logs-retry")
                .icon(IconName::Refresh)
                .label(t!("SessionLogs.refresh").to_string())
                .on_click(cx.listener(|page, _, _, cx| page.refresh(cx))),
        )
}
