use db::compare::{
    CompareSchemaSide, DataCompareTableFailure, SchemaCompareTableFailure, SyncPlan,
    data_compare_table_failure_warning, schema_compare_table_failure_warning,
};
use gpui::{
    App, AppContext, ClipboardItem, ColorExt, Context, Entity, InteractiveElement, IntoElement,
    ParentElement, Styled, Window, div, prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme, IconName, IndexPath, Sizable, StyledExt, WindowExt,
    button::{Button, ButtonVariants},
    h_flex,
    list::{List, ListDelegate, ListItem, ListState},
    notification::Notification,
    v_flex,
};
use rust_i18n::t;

const COMPARE_ISSUE_ROW_HEIGHT: f32 = 64.0;
const COMPARE_ISSUE_LIST_MAX_HEIGHT: f32 = 200.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CompareIssue {
    pub badge: Option<String>,
    pub title: String,
    pub detail: String,
}

pub(super) type CompareIssueListState = Entity<ListState<CompareIssueListDelegate>>;

pub(super) fn compare_issue_list_state<T: 'static>(
    panel_id: &'static str,
    window: &mut Window,
    cx: &mut Context<T>,
) -> CompareIssueListState {
    cx.new(|cx| {
        ListState::new(CompareIssueListDelegate::new(panel_id), window, cx).selectable(false)
    })
}

pub(super) fn refresh_compare_issue_list<T: 'static>(
    list_state: &CompareIssueListState,
    issues: Vec<CompareIssue>,
    cx: &mut Context<T>,
) {
    list_state.update(cx, |list, cx| {
        list.delegate_mut().set_issues(issues);
        cx.notify();
    });
}

pub(super) fn clear_compare_issue_list<T: 'static>(
    list_state: &CompareIssueListState,
    cx: &mut Context<T>,
) {
    list_state.update(cx, |list, cx| {
        list.delegate_mut().clear();
        cx.notify();
    });
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
    issue_count: usize,
    list_state: CompareIssueListState,
    expanded: Entity<bool>,
    cx: &App,
) -> impl IntoElement {
    let is_expanded = *expanded.read(cx);
    let list_height =
        (issue_count as f32 * COMPARE_ISSUE_ROW_HEIGHT).min(COMPARE_ISSUE_LIST_MAX_HEIGHT);

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
                div()
                    .id(format!("{panel_id}-details"))
                    .h(px(list_height))
                    .min_h_0()
                    .overflow_hidden()
                    .rounded_sm()
                    .child(List::new(&list_state).size_full()),
            )
        })
}

pub(super) struct CompareIssueListDelegate {
    panel_id: &'static str,
    issues: Vec<CompareIssue>,
    selected_index: Option<IndexPath>,
}

impl CompareIssueListDelegate {
    fn new(panel_id: &'static str) -> Self {
        Self {
            panel_id,
            issues: Vec::new(),
            selected_index: None,
        }
    }

    fn set_issues(&mut self, issues: Vec<CompareIssue>) {
        self.issues = issues;
        self.selected_index = None;
    }

    fn clear(&mut self) {
        self.issues.clear();
        self.selected_index = None;
    }
}

impl ListDelegate for CompareIssueListDelegate {
    type Item = ListItem;

    fn items_count(&self, _section: usize, _cx: &App) -> usize {
        self.issues.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let issue = self.issues.get(ix.row)?.clone();
        let copy_text = compare_issue_copy_text(&issue);

        Some(
            ListItem::new((self.panel_id, ix.row))
                .h(px(COMPARE_ISSUE_ROW_HEIGHT))
                .cursor_pointer()
                .border_b_1()
                .border_color(cx.theme().border.opacity(0.55))
                .bg(cx.theme().background.opacity(0.72))
                .child(
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .gap_1()
                        .child(
                            h_flex()
                                .min_w_0()
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
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_sm()
                                        .font_semibold()
                                        .child(issue.title),
                                ),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(issue.detail),
                        ),
                )
                .on_click(move |_, window, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()));
                    window.push_notification(
                        Notification::success(t!("Compare.compare_issue_copied").to_string())
                            .autohide(true),
                        cx,
                    );
                }),
        )
    }

    fn set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) {
        self.selected_index = ix;
    }
}

pub(super) fn compare_issue_copy_text(issue: &CompareIssue) -> String {
    let title = issue.badge.as_ref().map_or_else(
        || issue.title.clone(),
        |badge| format!("{badge} {}", issue.title),
    );
    format!("{title}\n{}", issue.detail)
}

fn schema_side_label(side: CompareSchemaSide) -> String {
    match side {
        CompareSchemaSide::Source => t!("Compare.source").to_string(),
        CompareSchemaSide::Target => t!("Compare.target").to_string(),
    }
}
