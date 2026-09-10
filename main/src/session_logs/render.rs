use gpui::prelude::FluentBuilder;
use gpui::{
    FontWeight, IntoElement, ListSizingBehavior, ParentElement, Render, Styled, Window, div, px,
    uniform_list,
};
use gpui_component::{ActiveTheme, Disableable, Icon, Sizable, button::{Button, ButtonVariants as _}, h_flex, input::Input, v_flex};
use one_assets::IconName;
use rust_i18n::t;
use std::ops::Range;
use terminal::recording::SessionLogEntry;

use super::{SESSION_LOG_LOAD_MORE_THRESHOLD, SessionLogsPage};

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
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("session-logs-select-all")
                            .label(t!("SessionLogs.select_all").to_string())
                            .small()
                            .ghost()
                            .disabled(self.loading || self.deleting)
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.select_all_filtered(cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("session-logs-clear-selection")
                            .label(t!("SessionLogs.clear_selection").to_string())
                            .small()
                            .ghost()
                            .disabled(self.selected_ids.is_empty() || self.deleting)
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.clear_selection();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("session-logs-delete-selected")
                            .icon(IconName::Delete)
                            .label(if self.selected_ids.is_empty() {
                                t!("SessionLogs.delete_selected").to_string()
                            } else {
                                t!(
                                    "SessionLogs.delete_selected_count",
                                    count = self.selected_ids.len()
                                )
                                .to_string()
                            })
                            .small()
                            .danger()
                            .disabled(self.selected_ids.is_empty() || self.loading || self.deleting)
                            .on_click(cx.listener(|page, _, window, cx| {
                                page.request_delete_selected(window, cx)
                            })),
                    ),
            )
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
                    .disabled(self.loading || self.favorite_saving || self.deleting)
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
        &mut self,
        entries: Vec<SessionLogEntry>,
        has_query: bool,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::AnyElement {
        if let Some(error) = self.load_error.clone() {
            return error_state(error, self.favorite_saving || self.deleting, cx)
                .into_any_element();
        }
        if entries.is_empty() {
            return empty_state(has_query, self.loading, cx).into_any_element();
        }
        let total_count = entries.len();
        let item_count = total_count.min(self.visible_count);
        let entries = entries.into_iter().take(item_count).collect::<Vec<_>>();
        uniform_list("session-logs-list", item_count, {
            cx.processor(
                move |this: &mut SessionLogsPage, range: Range<usize>, _window, cx| {
                    if range.end >= item_count.saturating_sub(SESSION_LOG_LOAD_MORE_THRESHOLD) {
                        this.load_more(total_count, cx);
                    }
                    range
                        .filter_map(|index| {
                            let entry = entries.get(index).cloned()?;
                            Some(
                                div()
                                    .w_full()
                                    .px_4()
                                    .pb_2()
                                    .when(index == 0, |this| this.pt_1())
                                    .child(this.render_entry(entry, cx))
                                    .into_any_element(),
                            )
                        })
                        .collect()
                },
            )
        })
        .size_full()
        .track_scroll(&self.scroll_handle)
        .with_sizing_behavior(ListSizingBehavior::Auto)
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

fn error_state(
    error: String,
    disabled: bool,
    cx: &gpui::Context<SessionLogsPage>,
) -> impl IntoElement {
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
                .disabled(disabled)
                .on_click(cx.listener(|page, _, _, cx| page.refresh(cx))),
        )
}
