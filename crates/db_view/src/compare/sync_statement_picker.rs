use std::collections::HashSet;

use db::compare::{SyncPlan, SyncStatement, SyncStatementKind};
use gpui::{Entity, IntoElement, ParentElement, Styled, div, prelude::FluentBuilder};
use gpui_component::{
    Sizable,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    h_flex,
    scroll::ScrollableElement,
    tag::Tag,
    v_flex,
};

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
        format!("已选 {selected} 条同步语句")
    } else {
        format!("已选 {selected} 条同步语句,跳过 {skipped} 条")
    }
}

pub(super) fn sync_statement_picker(
    plan: SyncPlan,
    selected_ids: Entity<HashSet<String>>,
    selected_snapshot: HashSet<String>,
) -> impl IntoElement {
    let all_ids: Vec<String> = plan.statements.iter().map(|s| s.id.clone()).collect();
    let safe_ids: Vec<String> = plan
        .statements
        .iter()
        .filter(|s| !s.destructive)
        .map(|s| s.id.clone())
        .collect();

    v_flex()
        .flex_1()
        .min_h_0()
        .gap_2()
        .child(picker_toolbar(all_ids, safe_ids, selected_ids.clone()))
        .child(
            v_flex()
                .flex_1()
                .min_h_0()
                .gap_1()
                .overflow_y_scrollbar()
                .children(plan.statements.into_iter().map(|statement| {
                    statement_row(statement, selected_snapshot.clone(), selected_ids.clone())
                })),
        )
}

fn picker_toolbar(
    all_ids: Vec<String>,
    safe_ids: Vec<String>,
    selected_ids: Entity<HashSet<String>>,
) -> impl IntoElement {
    h_flex()
        .gap_2()
        .child(
            Button::new("sync-select-all")
                .small()
                .ghost()
                .child("全选")
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
                .ghost()
                .child("全不选")
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
                .child("仅安全")
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
        )
}

fn statement_row(
    statement: SyncStatement,
    selected_snapshot: HashSet<String>,
    selected_ids: Entity<HashSet<String>>,
) -> impl IntoElement {
    let checked = selected_snapshot.contains(&statement.id);
    let id = statement.id.clone();
    let object = statement.object_name.clone().unwrap_or_default();
    let destructive = statement.destructive;

    h_flex()
        .gap_2()
        .items_center()
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
        .child(kind_tag(&statement.kind))
        .child(div().text_sm().child(object))
        .when(destructive, |this| {
            this.child(Tag::danger().small().outline().child("破坏性"))
        })
}

fn kind_tag(kind: &SyncStatementKind) -> impl IntoElement {
    use SyncStatementKind::*;
    let (tag, label) = match kind {
        CreateTable => (Tag::success(), "建表"),
        DropTable => (Tag::danger(), "删表"),
        AlterTable => (Tag::warning(), "改表"),
        CreateIndex => (Tag::success(), "建索引"),
        DropIndex => (Tag::danger(), "删索引"),
        Insert => (Tag::success(), "插入"),
        Update => (Tag::warning(), "更新"),
        Delete => (Tag::danger(), "删除"),
        Comment => (Tag::info(), "注释"),
        Unknown => (Tag::secondary(), "其他"),
    };
    tag.small().child(label)
}
