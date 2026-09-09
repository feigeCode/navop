use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, ColorExt, Context, IntoElement, ParentElement, Render, Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, Size,
    button::{Button, ButtonVariants},
    clipboard::Clipboard,
    h_flex,
    popover::Popover,
    scroll::ScrollableElement,
    v_flex,
};
use one_ui::IconButton;
use rust_i18n::t;

use super::execution_history::{ExecutionRecord, ExecutionStatus};
use super::execution_history_panel::{ExecutionHistoryFilter, ExecutionHistoryPanel};

impl ExecutionHistoryPanel {
    fn render_record(
        index: usize,
        record: &ExecutionRecord,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (status_color, status_icon) = match record.status {
            ExecutionStatus::Success => (cx.theme().success, IconName::CircleCheck),
            ExecutionStatus::Error => (cx.theme().danger, IconName::TriangleAlert),
        };
        let sql_preview = record
            .sql
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_string();
        let popover_record = record.clone();

        Popover::new(("database-execution-history-record", index))
            .trigger(
                Button::new(("database-execution-history-trigger", index))
                    .ghost()
                    .w_full()
                    .h(px(88.))
                    .p_2()
                    .child(
                        v_flex()
                            .w_full()
                            .min_w_0()
                            .gap_1()
                            .child(
                                h_flex()
                                    .items_start()
                                    .gap_2()
                                    .child(
                                        Icon::new(status_icon)
                                            .with_size(Size::XSmall)
                                            .text_color(status_color),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_sm()
                                            .text_color(status_color)
                                            .text_ellipsis()
                                            .child(record.summary.clone()),
                                    ),
                            )
                            .when(!sql_preview.is_empty(), |this| {
                                this.child(
                                    div()
                                        .pl_5()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .text_ellipsis()
                                        .child(sql_preview),
                                )
                            })
                            .child(Self::render_metadata(record, cx)),
                    ),
            )
            .content(move |_state, _window, cx| Self::render_details(index, &popover_record, cx))
            .max_w(px(680.))
    }

    fn render_metadata(record: &ExecutionRecord, cx: &App) -> AnyElement {
        let scope = record
            .context
            .database
            .iter()
            .chain(record.context.schema.iter())
            .cloned()
            .collect::<Vec<_>>()
            .join(".");

        h_flex()
            .pl_5()
            .gap_3()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .when(!scope.is_empty(), |this| this.child(scope))
            .when_some(record.returned_rows, |this, count| {
                this.child(t!("DatabaseSidebar.returned_rows", count = count).to_string())
            })
            .when(
                record.returned_rows.is_none()
                    && (record.status == ExecutionStatus::Success || record.affected_rows > 0),
                |this| {
                    this.child(
                        t!(
                            "DatabaseSidebar.affected_rows",
                            count = record.affected_rows
                        )
                        .to_string(),
                    )
                },
            )
            .child(
                t!(
                    "DatabaseSidebar.execution_time",
                    duration = record.elapsed_ms
                )
                .to_string(),
            )
            .into_any_element()
    }

    fn render_details(index: usize, record: &ExecutionRecord, cx: &App) -> AnyElement {
        let details = record.details.join("\n");
        let sql = record.sql.clone();
        let (status_color, status_icon) = match record.status {
            ExecutionStatus::Success => (cx.theme().success, IconName::CircleCheck),
            ExecutionStatus::Error => (cx.theme().danger, IconName::TriangleAlert),
        };

        v_flex()
            .w(px(620.))
            .h(px(460.))
            .gap_2()
            .p_3()
            .overflow_y_scrollbar()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Icon::new(status_icon)
                            .with_size(Size::Small)
                            .text_color(status_color),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .text_color(status_color)
                            .child(record.summary.clone()),
                    ),
            )
            .child(Self::render_metadata(record, cx))
            .when(!details.is_empty(), |this| {
                this.child(Self::render_code_block(
                    t!("DatabaseSidebar.server_result").to_string(),
                    details.clone(),
                    Clipboard::new(("database-execution-history-copy-result", index))
                        .value(details),
                    cx,
                ))
            })
            .when(!sql.is_empty(), |this| {
                this.child(Self::render_code_block(
                    t!("DatabaseSidebar.executed_sql").to_string(),
                    sql.clone(),
                    Clipboard::new(("database-execution-history-copy-sql", index)).value(sql),
                    cx,
                ))
            })
            .into_any_element()
    }

    fn render_code_block(
        label: String,
        content: String,
        action: Clipboard,
        cx: &App,
    ) -> impl IntoElement {
        v_flex()
            .gap_1()
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(label),
                    )
                    .child(action),
            )
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .max_h(px(220.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().muted.opacity(0.14))
                    .p_2()
                    .overflow_scrollbar()
                    .child(
                        div()
                            .min_w_full()
                            .flex_shrink_0()
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_xs()
                            .child(content),
                    ),
            )
    }

    fn render_filter_button(
        &self,
        filter: ExecutionHistoryFilter,
        label: String,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        Button::new(("database-execution-history-filter", filter as usize))
            .ghost()
            .with_size(Size::XSmall)
            .when(self.filter == filter, |this| this.primary())
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| this.set_filter(filter, cx)))
    }
}

impl Render for ExecutionHistoryPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.history.records().len();
        let success_count = self
            .history
            .records()
            .iter()
            .filter(|record| record.status == ExecutionStatus::Success)
            .count();
        let error_count = count - success_count;
        let records = self
            .history
            .records()
            .iter()
            .enumerate()
            .filter(|(_, record)| match self.filter {
                ExecutionHistoryFilter::All => true,
                ExecutionHistoryFilter::Success => record.status == ExecutionStatus::Success,
                ExecutionHistoryFilter::Error => record.status == ExecutionStatus::Error,
            })
            .rev()
            .map(|(index, record)| Self::render_record(index, record, cx).into_any_element())
            .collect::<Vec<AnyElement>>();
        let visible_count = records.len();

        v_flex()
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .child(
                h_flex()
                    .h(px(40.))
                    .flex_shrink_0()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(t!("DatabaseSidebar.execution_history").to_string()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                t!("DatabaseSidebar.execution_history_count", count = count)
                                    .to_string(),
                            ),
                    )
                    .child(div().flex_1())
                    .when(count > 0, |this| {
                        this.child(
                            IconButton::new("clear-database-execution-history", IconName::Delete)
                                .tooltip(t!("DatabaseSidebar.clear_execution_history").to_string())
                                .on_click(cx.listener(|this, _, _, cx| this.clear(cx))),
                        )
                    }),
            )
            .child(
                h_flex()
                    .h(px(36.))
                    .flex_shrink_0()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(self.render_filter_button(
                        ExecutionHistoryFilter::All,
                        t!("DatabaseSidebar.filter_all", count = count).to_string(),
                        cx,
                    ))
                    .child(self.render_filter_button(
                        ExecutionHistoryFilter::Success,
                        t!("DatabaseSidebar.filter_success", count = success_count).to_string(),
                        cx,
                    ))
                    .child(self.render_filter_button(
                        ExecutionHistoryFilter::Error,
                        t!("DatabaseSidebar.filter_failed", count = error_count).to_string(),
                        cx,
                    )),
            )
            .child(
                div().flex_1().h_full().min_h_0().overflow_hidden().child(
                    v_flex()
                        .size_full()
                        .gap_2()
                        .p_2()
                        .overflow_y_scrollbar()
                        .when(visible_count == 0, |this| {
                            this.child(
                                div()
                                    .p_3()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(t!("DatabaseSidebar.no_execution_history").to_string()),
                            )
                        })
                        .children(records),
                ),
            )
    }
}
