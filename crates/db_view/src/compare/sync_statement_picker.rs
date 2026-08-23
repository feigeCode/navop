use std::collections::HashSet;

use db::compare::{SyncPlan, SyncStatement, SyncStatementKind};
use gpui::{
    App, AppContext, ColorExt, Context, Entity, InteractiveElement, IntoElement, ParentElement,
    Styled, Task, Window, div, prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme, ContentState, IconName, IndexPath, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    h_flex,
    list::{List, ListDelegate, ListItem, ListState},
    scroll::ScrollableElement,
    tag::Tag,
    v_flex,
};
use rust_i18n::t;

pub(super) type SyncStatementListState = Entity<ListState<SyncStatementListDelegate>>;

const SYNC_STATEMENT_ROW_HEIGHT: f32 = 74.0;

pub(super) fn sync_statement_empty_picker(cx: &App) -> impl IntoElement {
    v_flex()
        .flex_1()
        .min_h_0()
        .gap_1()
        .child(
            div()
                .text_sm()
                .font_semibold()
                .child(t!("Compare.sync_statements").to_string()),
        )
        .child(
            div()
                .flex_1()
                .h_full()
                .min_h_0()
                .min_w_0()
                .border_1()
                .border_color(cx.theme().border)
                .rounded_md()
                .overflow_hidden()
                .child(
                    ContentState::empty(t!("Compare.no_sync_statements").to_string())
                        .icon(IconName::File)
                        .detail(t!("Compare.sync_statements_empty_detail").to_string())
                        .compact(),
                ),
        )
}

/// Immutable execution input derived from a sync plan and the selected statement ids.
///
/// The SQL preview editor is intentionally not part of this snapshot. It may contain
/// user edits, while execution safety and destructive confirmation must use the same
/// structured statements that were selected from the generated plan.
#[derive(Clone, Debug)]
pub(super) struct SyncExecutionSnapshot {
    pub plan_id: String,
    pub statements: Vec<SyncStatement>,
    pub sql: String,
}

impl SyncExecutionSnapshot {
    pub(super) fn is_empty(&self) -> bool {
        self.statements.is_empty() || self.sql.trim().is_empty()
    }

    pub(super) fn is_destructive(&self) -> bool {
        self.statements
            .iter()
            .any(|statement| statement.destructive)
    }
}

pub(super) fn sync_statement_list_state<T: 'static>(
    selected_ids: Entity<HashSet<String>>,
    window: &mut Window,
    cx: &mut Context<T>,
) -> SyncStatementListState {
    cx.new(|cx| {
        ListState::new(SyncStatementListDelegate::new(selected_ids), window, cx).selectable(false)
    })
}

pub(super) fn default_selected_statement_ids(plan: &SyncPlan) -> HashSet<String> {
    plan.statements
        .iter()
        .filter(|statement| statement.selected_by_default)
        .map(|statement| statement.id.clone())
        .collect()
}

pub(super) fn refresh_sync_statement_list<T: 'static>(
    list_state: &SyncStatementListState,
    plan: &SyncPlan,
    cx: &mut Context<T>,
) {
    list_state.update(cx, |list, cx| {
        list.delegate_mut().set_statements(plan.statements.clone());
        cx.notify();
    });
}

pub(super) fn clear_sync_statement_list<T: 'static>(
    list_state: &SyncStatementListState,
    cx: &mut Context<T>,
) {
    list_state.update(cx, |list, cx| {
        list.delegate_mut().set_statements(Vec::new());
        cx.notify();
    });
}

pub(super) fn selected_sync_sql_text_for_ids(
    plan: &SyncPlan,
    selected_ids: &HashSet<String>,
) -> String {
    selected_sync_execution_snapshot(plan, selected_ids).sql
}

pub(super) fn selected_sync_execution_snapshot(
    plan: &SyncPlan,
    selected_ids: &HashSet<String>,
) -> SyncExecutionSnapshot {
    let statements = plan
        .statements
        .iter()
        .filter(|statement| selected_ids.contains(&statement.id))
        .cloned()
        .collect::<Vec<_>>();
    let sql = statements
        .iter()
        .map(|statement| statement.sql.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    SyncExecutionSnapshot {
        plan_id: plan.id.clone(),
        statements,
        sql,
    }
}

pub(super) fn selected_sync_sql_summary_for_ids(
    plan: &SyncPlan,
    selected_ids: &HashSet<String>,
) -> String {
    let selected = plan
        .statements
        .iter()
        .filter(|statement| selected_ids.contains(&statement.id))
        .count();
    let skipped = plan.statements.len().saturating_sub(selected);
    if skipped == 0 {
        t!("Compare.sync_statements_selected", selected = selected).to_string()
    } else {
        t!(
            "Compare.sync_statements_selected_skipped",
            selected = selected,
            skipped = skipped
        )
        .to_string()
    }
}

pub(super) fn sync_statement_picker(
    plan: SyncPlan,
    selected_ids: Entity<HashSet<String>>,
    list_state: SyncStatementListState,
    warnings_expanded: Entity<bool>,
    cx: &App,
) -> impl IntoElement {
    let all_ids: Vec<String> = plan.statements.iter().map(|s| s.id.clone()).collect();
    let safe_ids: Vec<String> = plan
        .statements
        .iter()
        .filter(|s| !s.destructive)
        .map(|s| s.id.clone())
        .collect();
    let plan_warnings = plan.warnings.clone();
    let warnings_are_expanded = *warnings_expanded.read(cx);

    v_flex()
        .flex_1()
        .min_h_0()
        .gap_1()
        .child(picker_header(all_ids, safe_ids, selected_ids.clone()))
        .when(!plan_warnings.is_empty(), |this| {
            this.child(sync_plan_warnings_panel(
                plan_warnings,
                warnings_are_expanded,
                warnings_expanded,
                cx,
            ))
        })
        .child(
            v_flex()
                .flex_1()
                .min_h_0()
                .border_1()
                .border_color(cx.theme().border)
                .rounded_md()
                .overflow_hidden()
                .child(List::new(&list_state).size_full()),
        )
}

fn sync_plan_warnings_panel(
    warnings: Vec<String>,
    expanded: bool,
    expanded_state: Entity<bool>,
    cx: &App,
) -> impl IntoElement {
    let warning_count = warnings.len();

    v_flex()
        .id("sync-plan-warnings")
        .flex_none()
        .gap_1()
        .p_1()
        .border_1()
        .border_color(cx.theme().warning.opacity(0.45))
        .rounded_md()
        .bg(cx.theme().warning.opacity(0.08))
        .child(
            h_flex()
                .items_center()
                .gap_1()
                .child(
                    Button::new("toggle-sync-plan-warnings")
                        .icon(if expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .xsmall()
                        .ghost()
                        .on_click(move |_, _, cx| {
                            expanded_state.update(cx, |expanded, cx| {
                                *expanded = !*expanded;
                                cx.notify();
                            });
                        }),
                )
                .child(
                    div()
                        .text_xs()
                        .font_semibold()
                        .text_color(cx.theme().warning)
                        .child(t!("Compare.sync_plan_warnings", count = warning_count).to_string()),
                ),
        )
        .when(expanded, |this| {
            this.child(
                v_flex()
                    .id("sync-plan-warning-details")
                    .max_h(px(112.0))
                    .gap_1()
                    .px_2()
                    .pb_1()
                    .overflow_y_scrollbar()
                    .children(warnings.into_iter().enumerate().map(|(index, warning)| {
                        div()
                            .id(("sync-plan-warning", index))
                            .text_xs()
                            .text_color(cx.theme().warning)
                            .child(warning)
                    })),
            )
        })
}

fn picker_header(
    all_ids: Vec<String>,
    safe_ids: Vec<String>,
    selected_ids: Entity<HashSet<String>>,
) -> impl IntoElement {
    h_flex()
        .justify_between()
        .child(
            div()
                .text_sm()
                .font_semibold()
                .child(t!("Compare.sync_statements").to_string()),
        )
        .child(
            h_flex()
                .gap_1()
                .child(
                    Button::new("sync-select-all")
                        .small()
                        .child(t!("Common.select_all").to_string())
                        .on_click({
                            let all = all_ids;
                            let sel = selected_ids.clone();
                            move |_, _, cx| {
                                sel.update(cx, |ids, cx| {
                                    *ids = all.iter().cloned().collect();
                                    cx.notify();
                                });
                            }
                        }),
                )
                .child(
                    Button::new("sync-select-none")
                        .small()
                        .child(t!("Common.deselect_all").to_string())
                        .on_click({
                            let sel = selected_ids.clone();
                            move |_, _, cx| {
                                sel.update(cx, |ids, cx| {
                                    ids.clear();
                                    cx.notify();
                                });
                            }
                        }),
                )
                .child(
                    Button::new("sync-select-safe")
                        .small()
                        .ghost()
                        .child(t!("Compare.select_safe_only").to_string())
                        .on_click({
                            let safe = safe_ids;
                            let sel = selected_ids;
                            move |_, _, cx| {
                                sel.update(cx, |ids, cx| {
                                    *ids = safe.iter().cloned().collect();
                                    cx.notify();
                                });
                            }
                        }),
                ),
        )
}

pub(super) struct SyncStatementListDelegate {
    statements: Vec<SyncStatement>,
    selected_ids: Entity<HashSet<String>>,
    selected_index: Option<IndexPath>,
}

impl SyncStatementListDelegate {
    fn new(selected_ids: Entity<HashSet<String>>) -> Self {
        Self {
            statements: Vec::new(),
            selected_ids,
            selected_index: None,
        }
    }

    pub(super) fn set_statements(&mut self, statements: Vec<SyncStatement>) {
        self.statements = statements;
        self.selected_index = None;
    }
}

impl ListDelegate for SyncStatementListDelegate {
    type Item = ListItem;

    fn items_count(&self, _section: usize, _cx: &App) -> usize {
        self.statements.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let statement = self.statements.get(ix.row)?;
        let checked = self.selected_ids.read(cx).contains(&statement.id);
        Some(statement_row(
            statement,
            checked,
            self.selected_ids.clone(),
            cx,
        ))
    }

    fn render_empty(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        ContentState::empty(t!("Compare.no_sync_statements").to_string())
            .icon(IconName::File)
            .compact()
    }

    fn perform_search(
        &mut self,
        _query: &str,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) -> Task<()> {
        Task::ready(())
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

fn statement_row(
    statement: &SyncStatement,
    checked: bool,
    selected_ids: Entity<HashSet<String>>,
    cx: &App,
) -> ListItem {
    let id = statement.id.clone();
    let object = statement
        .object_name
        .clone()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| t!("Compare.unnamed_object").to_string());
    let destructive = statement.destructive;
    let sql_preview = sql_preview(&statement.sql);
    let warnings = statement.warnings.join(" · ");

    let row = h_flex()
        .w_full()
        .gap_2()
        .items_start()
        .child(Checkbox::new(format!("sync-statement-{id}")).checked(checked))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(kind_tag(&statement.kind))
                        .child(div().flex_1().min_w_0().truncate().text_sm().child(object))
                        .when(destructive, |this| {
                            this.child(
                                Tag::danger()
                                    .small()
                                    .outline()
                                    .child(t!("Compare.destructive").to_string()),
                            )
                        }),
                )
                .child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(sql_preview),
                )
                .child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(cx.theme().warning)
                        .child(warnings),
                ),
        );

    ListItem::new(format!("sync-statement-row-{id}"))
        .h(px(SYNC_STATEMENT_ROW_HEIGHT))
        .child(row)
        .when(checked, |this| this.bg(cx.theme().list_active))
        .on_click(move |_, _, cx| {
            selected_ids.update(cx, |ids, cx| {
                if ids.contains(&id) {
                    ids.remove(&id);
                } else {
                    ids.insert(id.clone());
                }
                cx.notify();
            });
        })
}

fn kind_tag(kind: &SyncStatementKind) -> impl IntoElement {
    use SyncStatementKind::*;
    let (tag, label) = match kind {
        CreateTable => (Tag::success(), t!("Compare.statement_create_table")),
        DropTable => (Tag::danger(), t!("Compare.statement_drop_table")),
        AlterTable => (Tag::warning(), t!("Compare.statement_alter_table")),
        CreateIndex => (Tag::success(), t!("Compare.statement_create_index")),
        DropIndex => (Tag::danger(), t!("Compare.statement_drop_index")),
        Insert => (Tag::success(), t!("Compare.statement_insert")),
        Update => (Tag::warning(), t!("Compare.statement_update")),
        Delete => (Tag::danger(), t!("Compare.statement_delete")),
        Comment => (Tag::info(), t!("Compare.statement_comment")),
        Unknown => (Tag::secondary(), t!("Compare.statement_unknown")),
    };
    tag.small().child(label.to_string())
}

fn sql_preview(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}
