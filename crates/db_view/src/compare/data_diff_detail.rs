use db::compare::{
    DataCompareBatchResult, DataCompareResult, RowData, SyncPlan, SyncStatementKind,
};
use gpui::{
    App, ColorExt, Entity, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder,
};
use gpui_component::{
    ActiveTheme, Sizable, StyledExt, checkbox::Checkbox, h_flex, scroll::ScrollableElement,
    tag::Tag, v_flex,
};
use rust_i18n::t;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

const DATA_DIFF_PREVIEW_LIMIT: usize = 100;
const DATA_DIFF_VALUE_SUMMARY_LIMIT: usize = 240;

type StatementLookupKey = (String, u8, String);
type StatementIndex = HashMap<StatementLookupKey, Vec<String>>;

pub(super) fn data_diff_detail_panel(
    result: &DataCompareBatchResult,
    plan: Option<&SyncPlan>,
    selected_ids: Entity<HashSet<String>>,
    cx: &App,
) -> impl IntoElement {
    let statement_index = build_statement_index(plan);
    let has_changes = result.table_results.iter().any(|table| {
        !table.added.is_empty() || !table.removed.is_empty() || !table.modified.is_empty()
    });

    v_flex()
        .flex_1()
        .min_h_0()
        .gap_1()
        .child(
            h_flex()
                .justify_between()
                .child(
                    div()
                        .text_sm()
                        .font_semibold()
                        .child(t!("Compare.data_diff_details").to_string()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("Compare.data_diff_direction").to_string()),
                ),
        )
        .when(!has_changes, |this| {
            this.child(
                div()
                    .p_2()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("Compare.data_diff_no_changes").to_string()),
            )
        })
        .when(has_changes, |this| {
            this.child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .gap_2()
                    .overflow_y_scrollbar()
                    .children(
                        result
                            .table_results
                            .iter()
                            .filter(|table| {
                                !table.added.is_empty()
                                    || !table.removed.is_empty()
                                    || !table.modified.is_empty()
                            })
                            .map(|table| {
                                table_diff_panel(table, &statement_index, selected_ids.clone(), cx)
                            }),
                    ),
            )
        })
}

fn table_diff_panel(
    result: &DataCompareResult,
    statement_index: &StatementIndex,
    selected_ids: Entity<HashSet<String>>,
    cx: &App,
) -> impl IntoElement {
    let table_name = result.target_table.clone();
    let added_count = result.added.len();
    let removed_count = result.removed.len();
    let modified_count = result.modified.len();
    let mut rows = Vec::new();

    for (_index, row) in result
        .added
        .iter()
        .take(DATA_DIFF_PREVIEW_LIMIT)
        .enumerate()
    {
        let key_values = row_key(row, &result.key_columns);
        let canonical_key = canonical_row_key(&key_values);
        rows.push(data_row(
            row_ui_id(&table_name, &SyncStatementKind::Insert, &canonical_key),
            t!("Compare.added").to_string(),
            Tag::success(),
            key_values.clone(),
            &result.key_columns,
            vec![row_value_summary(row, &result.columns)],
            statement_ids_for_row(
                statement_index,
                &table_name,
                &SyncStatementKind::Insert,
                &key_values,
            ),
            selected_ids.clone(),
            cx,
        ));
    }

    for (_index, row) in result
        .removed
        .iter()
        .take(DATA_DIFF_PREVIEW_LIMIT)
        .enumerate()
    {
        let key_values = row_key(row, &result.key_columns);
        let canonical_key = canonical_row_key(&key_values);
        rows.push(data_row(
            row_ui_id(&table_name, &SyncStatementKind::Delete, &canonical_key),
            t!("Compare.removed").to_string(),
            Tag::danger(),
            key_values.clone(),
            &result.key_columns,
            vec![row_value_summary(row, &result.columns)],
            statement_ids_for_row(
                statement_index,
                &table_name,
                &SyncStatementKind::Delete,
                &key_values,
            ),
            selected_ids.clone(),
            cx,
        ));
    }

    for (_index, modified) in result
        .modified
        .iter()
        .take(DATA_DIFF_PREVIEW_LIMIT)
        .enumerate()
    {
        let changes = result
            .columns
            .iter()
            .filter_map(|column| {
                modified.changes.get(column).map(|(source, target)| {
                    format!("{}: {} → {}", column, cell_text(target), cell_text(source))
                })
            })
            .collect();
        let canonical_key = canonical_row_key(&modified.key_values);
        rows.push(data_row(
            row_ui_id(&table_name, &SyncStatementKind::Update, &canonical_key),
            t!("Compare.modified").to_string(),
            Tag::warning(),
            modified.key_values.clone(),
            &result.key_columns,
            changes,
            statement_ids_for_row(
                statement_index,
                &table_name,
                &SyncStatementKind::Update,
                &modified.key_values,
            ),
            selected_ids.clone(),
            cx,
        ));
    }

    let total = added_count + removed_count + modified_count;
    v_flex()
        .gap_1()
        .p_2()
        .border_1()
        .border_color(cx.theme().border)
        .rounded_md()
        .child(
            h_flex()
                .gap_2()
                .child(div().font_semibold().text_sm().child(table_name))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("Compare.data_diff_row_count", count = total).to_string()),
                ),
        )
        .children(rows)
        .when(
            has_hidden_data_diff_rows(added_count, removed_count, modified_count),
            |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            t!(
                                "Compare.data_diff_show_limit",
                                count = DATA_DIFF_PREVIEW_LIMIT
                            )
                            .to_string(),
                        ),
                )
            },
        )
}

fn has_hidden_data_diff_rows(added: usize, removed: usize, modified: usize) -> bool {
    added > DATA_DIFF_PREVIEW_LIMIT
        || removed > DATA_DIFF_PREVIEW_LIMIT
        || modified > DATA_DIFF_PREVIEW_LIMIT
}

fn data_row(
    id: String,
    kind_label: String,
    tag: Tag,
    key_values: HashMap<String, Value>,
    key_columns: &[String],
    details: Vec<String>,
    statement_ids: Vec<String>,
    selected_ids: Entity<HashSet<String>>,
    cx: &App,
) -> impl IntoElement {
    let checked = !statement_ids.is_empty()
        && statement_ids
            .iter()
            .all(|id| selected_ids.read(cx).contains(id));
    let selectable = !statement_ids.is_empty();
    let key_text = ordered_key_text(&key_values, key_columns);
    let statement_ids_for_click = statement_ids.clone();
    let checkbox_id = format!("{id}-checkbox");

    h_flex()
        .id(id)
        .w_full()
        .gap_2()
        .items_start()
        .p_1()
        .when(selectable, |this| {
            this.child(Checkbox::new(checkbox_id).checked(checked))
        })
        .child(tag.small().child(kind_label))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(div().text_xs().child(key_text))
                .children(
                    details
                        .into_iter()
                        .filter(|detail| !detail.is_empty())
                        .map(|detail| {
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(detail)
                        }),
                ),
        )
        .when(selectable, |this| {
            this.bg(cx
                .theme()
                .list_active
                .opacity(if checked { 0.7 } else { 0.0 }))
                .on_click(move |_, _, cx| {
                    selected_ids.update(cx, |ids, cx| {
                        let all_selected =
                            statement_ids_for_click.iter().all(|id| ids.contains(id));
                        for id in &statement_ids_for_click {
                            if all_selected {
                                ids.remove(id);
                            } else {
                                ids.insert(id.clone());
                            }
                        }
                        cx.notify();
                    });
                })
        })
}

fn row_key(row: &RowData, key_columns: &[String]) -> HashMap<String, Value> {
    key_columns
        .iter()
        .filter_map(|column| {
            row.get(column)
                .cloned()
                .map(|value| (column.clone(), value))
        })
        .collect()
}

fn ordered_key_text(key_values: &HashMap<String, Value>, key_columns: &[String]) -> String {
    ordered_row_entries(key_values, key_columns)
        .into_iter()
        .map(|(column, value)| format!("{column}={}", cell_text(value)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn row_value_summary(row: &RowData, columns: &[String]) -> String {
    let summary = ordered_row_entries(row, columns)
        .into_iter()
        .map(|(column, value)| format!("{column}={}", cell_text(value)))
        .collect::<Vec<_>>()
        .join(", ");
    truncate_text(&summary, DATA_DIFF_VALUE_SUMMARY_LIMIT)
}

fn ordered_row_entries<'a>(
    row: &'a HashMap<String, Value>,
    preferred_columns: &[String],
) -> Vec<(&'a str, &'a Value)> {
    let mut seen = HashSet::new();
    let mut entries = preferred_columns
        .iter()
        .filter_map(|column| {
            row.get_key_value(column).map(|(column, value)| {
                seen.insert(column.as_str());
                (column.as_str(), value)
            })
        })
        .collect::<Vec<_>>();
    let mut remaining = row
        .iter()
        .filter(|(column, _)| !seen.contains(column.as_str()))
        .map(|(column, value)| (column.as_str(), value))
        .collect::<Vec<_>>();
    remaining.sort_by(|(left, _), (right, _)| left.cmp(right));
    entries.extend(remaining);
    entries
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut truncated = value
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    truncated.push('…');
    truncated
}

fn build_statement_index(plan: Option<&SyncPlan>) -> StatementIndex {
    let mut index = StatementIndex::new();
    let Some(plan) = plan else {
        return index;
    };
    for statement in &plan.statements {
        let kind = statement_kind_code(&statement.kind);
        let (Some(table), Some(row_key)) = (&statement.object_name, &statement.row_key) else {
            continue;
        };
        if kind == 0 {
            continue;
        }
        index
            .entry((table.clone(), kind, canonical_map_key(row_key)))
            .or_default()
            .push(statement.id.clone());
    }
    index
}

fn statement_ids_for_row(
    index: &StatementIndex,
    table: &str,
    kind: &SyncStatementKind,
    row_key: &HashMap<String, Value>,
) -> Vec<String> {
    index
        .get(&(
            table.to_string(),
            statement_kind_code(kind),
            canonical_row_key(row_key),
        ))
        .cloned()
        .unwrap_or_default()
}

fn row_ui_id(table: &str, kind: &SyncStatementKind, canonical_key: &str) -> String {
    let identity = serde_json::to_string(&(table, statement_kind_code(kind), canonical_key))
        .unwrap_or_else(|_| format!("{table}:{}:{canonical_key}", statement_kind_code(kind)));
    format!("data-diff-row-{identity}")
}

fn statement_kind_code(kind: &SyncStatementKind) -> u8 {
    match kind {
        SyncStatementKind::Insert => 1,
        SyncStatementKind::Update => 2,
        SyncStatementKind::Delete => 3,
        _ => 0,
    }
}

fn canonical_map_key(row_key: &Map<String, Value>) -> String {
    let mut entries: Vec<_> = row_key.iter().collect();
    entries.sort_by(|(left, _), (right, _)| left.cmp(right));
    serde_json::to_string(
        &entries
            .into_iter()
            .map(|(key, value)| (key, value))
            .collect::<Vec<_>>(),
    )
    .unwrap_or_default()
}

fn canonical_row_key(row_key: &HashMap<String, Value>) -> String {
    let mut entries: Vec<_> = row_key.iter().collect();
    entries.sort_by(|(left, _), (right, _)| left.cmp(right));
    serde_json::to_string(
        &entries
            .into_iter()
            .map(|(key, value)| (key, value))
            .collect::<Vec<_>>(),
    )
    .unwrap_or_default()
}

fn cell_text(value: &Value) -> String {
    match value {
        Value::Null => "NULL".to_string(),
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        _ => serde_json::to_string(value).unwrap_or_else(|_| "?".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_statement_index, canonical_row_key, has_hidden_data_diff_rows, ordered_key_text,
        row_key, row_ui_id, row_value_summary, statement_ids_for_row, statement_kind_code,
    };
    use db::compare::{RowData, SyncPlan, SyncPlanSummary, SyncStatement, SyncStatementKind};
    use serde_json::{Map, json};

    #[test]
    fn canonical_row_key_is_independent_of_map_insertion_order() {
        let mut first = std::collections::HashMap::new();
        first.insert("tenant_id".to_string(), json!(1));
        first.insert("id".to_string(), json!(2));
        let mut second = std::collections::HashMap::new();
        second.insert("id".to_string(), json!(2));
        second.insert("tenant_id".to_string(), json!(1));

        assert_eq!(canonical_row_key(&first), canonical_row_key(&second));
    }

    #[test]
    fn row_key_only_contains_declared_key_columns() {
        let row: RowData = [
            ("id".to_string(), json!(1)),
            ("name".to_string(), json!("Alice")),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            row_key(&row, &["id".to_string()]),
            std::collections::HashMap::from_iter([("id".to_string(), json!(1))])
        );
    }

    #[test]
    fn displayed_key_values_follow_declared_key_order() {
        let key_values = std::collections::HashMap::from([
            ("id".to_string(), json!(2)),
            ("tenant_id".to_string(), json!(1)),
        ]);

        assert_eq!(
            ordered_key_text(&key_values, &["tenant_id".to_string(), "id".to_string()]),
            "tenant_id=1, id=2"
        );
    }

    #[test]
    fn added_and_removed_row_values_follow_compare_column_order() {
        let row: RowData = [
            ("name".to_string(), json!("Alice")),
            ("id".to_string(), json!(1)),
        ]
        .into_iter()
        .collect();

        assert_eq!(
            row_value_summary(&row, &["id".to_string(), "name".to_string()]),
            "id=1, name=Alice"
        );
    }

    #[test]
    fn statement_kind_codes_do_not_alias_data_operations() {
        assert_ne!(
            statement_kind_code(&SyncStatementKind::Insert),
            statement_kind_code(&SyncStatementKind::Update)
        );
        assert_ne!(
            statement_kind_code(&SyncStatementKind::Update),
            statement_kind_code(&SyncStatementKind::Delete)
        );
    }

    #[test]
    fn row_ui_ids_include_table_and_operation_namespaces() {
        let key = canonical_row_key(&std::collections::HashMap::from([(
            "id".to_string(),
            json!(1),
        )]));

        assert_ne!(
            row_ui_id("users", &SyncStatementKind::Insert, &key),
            row_ui_id("orders", &SyncStatementKind::Insert, &key)
        );
        assert_ne!(
            row_ui_id("users", &SyncStatementKind::Insert, &key),
            row_ui_id("users", &SyncStatementKind::Update, &key)
        );
    }

    #[test]
    fn statement_index_keeps_table_and_operation_matches_separate() {
        let row_key = Map::from_iter([("id".to_string(), json!(1))]);
        let plan = SyncPlan {
            id: "plan".to_string(),
            target_table: "2 tables".to_string(),
            statements: vec![
                statement(
                    "users-insert",
                    "users",
                    SyncStatementKind::Insert,
                    row_key.clone(),
                ),
                statement(
                    "users-update",
                    "users",
                    SyncStatementKind::Update,
                    row_key.clone(),
                ),
                statement(
                    "orders-insert",
                    "orders",
                    SyncStatementKind::Insert,
                    row_key,
                ),
            ],
            summary: SyncPlanSummary {
                insert_count: 2,
                update_count: 1,
                delete_count: 0,
                ddl_count: 0,
                total_count: 3,
            },
            warnings: Vec::new(),
            sql_text: String::new(),
        };
        let index = build_statement_index(Some(&plan));
        let key = std::collections::HashMap::from([("id".to_string(), json!(1))]);

        assert_eq!(
            statement_ids_for_row(&index, "users", &SyncStatementKind::Insert, &key),
            vec!["users-insert".to_string()]
        );
        assert_eq!(
            statement_ids_for_row(&index, "users", &SyncStatementKind::Update, &key),
            vec!["users-update".to_string()]
        );
        assert_eq!(
            statement_ids_for_row(&index, "orders", &SyncStatementKind::Insert, &key),
            vec!["orders-insert".to_string()]
        );
    }

    #[test]
    fn hidden_row_notice_tracks_each_preview_group_independently() {
        assert!(has_hidden_data_diff_rows(101, 0, 0));
        assert!(has_hidden_data_diff_rows(0, 101, 0));
        assert!(has_hidden_data_diff_rows(0, 0, 101));
        assert!(!has_hidden_data_diff_rows(100, 100, 100));
    }

    fn statement(
        id: &str,
        table: &str,
        kind: SyncStatementKind,
        row_key: Map<String, serde_json::Value>,
    ) -> SyncStatement {
        SyncStatement {
            id: id.to_string(),
            sql: String::new(),
            kind,
            object_name: Some(table.to_string()),
            row_key: Some(row_key),
            destructive: false,
            transactional_safe: true,
            selected_by_default: true,
            warnings: Vec::new(),
        }
    }
}
