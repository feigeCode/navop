use std::collections::HashSet;

use db::compare::{SyncPlan, SyncStatement};
use gpui::{Entity, IntoElement, ParentElement, Styled, div, px};
use gpui_component::{checkbox::Checkbox, h_flex, scroll::ScrollableElement, v_flex};

pub(super) fn default_selected_statement_ids(plan: &SyncPlan) -> HashSet<String> {
    plan.statements
        .iter()
        .filter(|statement| statement.selected_by_default)
        .map(|statement| statement.id.clone())
        .collect()
}

pub(super) fn selected_sync_sql_text_for_ids(
    plan: &SyncPlan,
    selected_ids: &HashSet<String>,
) -> String {
    plan.statements
        .iter()
        .filter(|statement| selected_ids.contains(&statement.id))
        .map(|statement| statement.sql.as_str())
        .collect::<Vec<_>>()
        .join("\n")
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
        format!("Selected sync SQL: {selected} statement(s)")
    } else {
        format!("Selected sync SQL: {selected} statement(s), skipped {skipped} statement(s)")
    }
}

pub(super) fn sync_statement_picker(
    plan: SyncPlan,
    selected_ids: Entity<HashSet<String>>,
    selected_snapshot: HashSet<String>,
) -> impl IntoElement {
    v_flex()
        .gap_1()
        .max_h(px(180.0))
        .overflow_y_scrollbar()
        .children(plan.statements.into_iter().map(|statement| {
            statement_row(statement, selected_snapshot.clone(), selected_ids.clone())
        }))
}

fn statement_row(
    statement: SyncStatement,
    selected_snapshot: HashSet<String>,
    selected_ids: Entity<HashSet<String>>,
) -> impl IntoElement {
    let checked = selected_snapshot.contains(&statement.id);
    let id = statement.id.clone();
    h_flex()
        .gap_2()
        .items_start()
        .child(
            Checkbox::new(format!("sync-statement-{id}"))
                .checked(checked)
                .on_click(move |checked, _, cx| {
                    selected_ids.update(cx, |ids, cx| {
                        if *checked {
                            ids.insert(id.clone());
                        } else {
                            ids.remove(&id);
                        }
                        cx.notify();
                    });
                }),
        )
        .child(
            v_flex()
                .gap_1()
                .child(div().text_sm().child(statement_label(&statement)))
                .child(div().text_xs().child(statement.sql)),
        )
}

fn statement_label(statement: &SyncStatement) -> String {
    let mut label = format!("{:?}", statement.kind);
    if let Some(object_name) = statement.object_name.as_ref() {
        label.push_str(" - ");
        label.push_str(object_name);
    }
    if statement.destructive {
        label.push_str(" | destructive");
    }
    label
}
