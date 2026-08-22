use db::compare::{
    CompareSchemaSide, DataCompareTableFailure, SchemaCompareTableFailure, SyncPlan,
    data_compare_table_failure_warning, schema_compare_table_failure_warning,
};
use gpui::{
    App, ColorExt, Entity, InteractiveElement, IntoElement, ParentElement, Styled, div,
    prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme, IconName, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    scroll::ScrollableElement,
    v_flex,
};
use rust_i18n::t;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CompareIssue {
    pub badge: Option<String>,
    pub title: String,
    pub detail: String,
}

pub(super) fn data_compare_failure_issues(
    failures: &[DataCompareTableFailure],
) -> Vec<CompareIssue> {
    failures
        .iter()
        .map(|failure| CompareIssue {
            badge: None,
            title: failure.table.clone(),
            detail: failure.error.clone(),
        })
        .collect()
}

pub(super) fn schema_compare_failure_issues(
    failures: &[SchemaCompareTableFailure],
) -> Vec<CompareIssue> {
    failures
        .iter()
        .map(|failure| CompareIssue {
            badge: Some(schema_side_label(failure.side)),
            title: failure.table.clone(),
            detail: failure.error.clone(),
        })
        .collect()
}

pub(super) fn hide_data_compare_failure_warnings(
    plan: &mut SyncPlan,
    failures: &[DataCompareTableFailure],
) {
    hide_compare_failure_warnings(
        plan,
        failures.iter().map(data_compare_table_failure_warning),
    );
}

pub(super) fn hide_schema_compare_failure_warnings(
    plan: &mut SyncPlan,
    failures: &[SchemaCompareTableFailure],
) {
    hide_compare_failure_warnings(
        plan,
        failures.iter().map(schema_compare_table_failure_warning),
    );
}

fn hide_compare_failure_warnings(
    plan: &mut SyncPlan,
    failure_warnings: impl IntoIterator<Item = String>,
) {
    let failure_warnings = failure_warnings
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    plan.warnings
        .retain(|warning| !failure_warnings.contains(warning));
}

pub(super) fn failure_details_panel(
    panel_id: &'static str,
    toggle_id: &'static str,
    summary: String,
    issues: Vec<CompareIssue>,
    expanded: Entity<bool>,
    cx: &App,
) -> impl IntoElement {
    let is_expanded = *expanded.read(cx);
    let issue_count = issues.len();

    v_flex()
        .id(panel_id)
        .flex_none()
        .gap_2()
        .p_2()
        .border_1()
        .border_color(cx.theme().warning.opacity(0.45))
        .rounded_md()
        .bg(cx.theme().warning.opacity(0.08))
        .child(
            h_flex()
                .items_center()
                .gap_2()
                .child(
                    Button::new(toggle_id)
                        .icon(if is_expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .small()
                        .ghost()
                        .on_click({
                            let expanded = expanded.clone();
                            move |_, _, cx| {
                                expanded.update(cx, |expanded, cx| {
                                    *expanded = !*expanded;
                                    cx.notify();
                                });
                            }
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .font_semibold()
                        .text_color(cx.theme().warning)
                        .child(t!("Compare.compare_issues", count = issue_count).to_string()),
                ),
        )
        .child(
            div()
                .px_1()
                .text_xs()
                .text_color(cx.theme().warning)
                .child(summary),
        )
        .when(is_expanded, |this| {
            this.child(
                v_flex()
                    .id(format!("{panel_id}-details"))
                    .max_h(px(200.0))
                    .gap_1()
                    .overflow_y_scrollbar()
                    .children(issues.into_iter().enumerate().map(|(index, issue)| {
                        v_flex()
                            .id((panel_id, index))
                            .gap_1()
                            .p_2()
                            .rounded_sm()
                            .bg(cx.theme().background.opacity(0.72))
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_2()
                                    .when_some(issue.badge, |this, badge| {
                                        this.child(
                                            div()
                                                .flex_none()
                                                .px_1()
                                                .rounded_sm()
                                                .bg(cx.theme().warning.opacity(0.15))
                                                .text_xs()
                                                .text_color(cx.theme().warning)
                                                .child(badge),
                                        )
                                    })
                                    .child(
                                        div()
                                            .min_w_0()
                                            .text_sm()
                                            .font_semibold()
                                            .child(issue.title),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(issue.detail),
                            )
                    })),
            )
        })
}

fn schema_side_label(side: CompareSchemaSide) -> String {
    match side {
        CompareSchemaSide::Source => t!("Compare.source").to_string(),
        CompareSchemaSide::Target => t!("Compare.target").to_string(),
    }
}
