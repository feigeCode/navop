use db::compare::{
    DataCompareBatchResult, DataCompareResult, RowData, SyncPlan, SyncStatementKind,
};
use gpui::{
    App, AppContext, ColorExt, Context, Entity, InteractiveElement, IntoElement, ParentElement,
    Styled, Task, Window, div, prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme, IconName, IndexPath, Sizable, StyledExt,
    checkbox::Checkbox,
    h_flex,
    list::{List, ListDelegate, ListItem, ListState},
    tag::Tag,
    v_flex,
};
use one_ui::ContentState;
use rust_i18n::t;
use serde_json::{Map, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

const DATA_DIFF_VALUE_SUMMARY_LIMIT: usize = 240;
const DATA_DIFF_ROW_HEIGHT: f32 = 64.0;
const DATA_DIFF_SECTION_HEADER_HEIGHT: f32 = 40.0;

type StatementLookupKey = (String, u8, String);
type StatementIndex = HashMap<StatementLookupKey, Vec<String>>;

pub(super) type DataDiffListState = Entity<ListState<DataDiffListDelegate>>;

pub(super) fn data_diff_list_state<T: 'static>(
    selected_ids: Entity<HashSet<String>>,
    window: &mut Window,
    cx: &mut Context<T>,
) -> DataDiffListState {
    cx.new(|cx| {
        ListState::new(DataDiffListDelegate::new(selected_ids), window, cx).selectable(false)
    })
}

pub(super) fn refresh_data_diff_list<T: 'static>(
    list_state: &DataDiffListState,
    result: Arc<DataCompareBatchResult>,
    plan: Option<&SyncPlan>,
    cx: &mut Context<T>,
) {
    list_state.update(cx, |list, cx| {
        list.delegate_mut().set_result(result, plan);
        cx.notify();
    });
}

pub(super) fn clear_data_diff_list<T: 'static>(
    list_state: &DataDiffListState,
    cx: &mut Context<T>,
) {
    list_state.update(cx, |list, cx| {
        list.delegate_mut().clear();
        cx.notify();
    });
}

pub(super) fn data_diff_detail_panel(list_state: DataDiffListState, cx: &App) -> impl IntoElement {
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
        .child(
            div()
                .id("data-compare-diff-scroll")
                .flex_1()
                .h_full()
                .min_h_0()
                .min_w_0()
                .border_1()
                .border_color(cx.theme().border)
                .rounded_md()
                .bg(cx.theme().background)
                .overflow_hidden()
                .child(List::new(&list_state).size_full()),
        )
}

pub(super) struct DataDiffListDelegate {
    result: Option<Arc<DataCompareBatchResult>>,
    table_indices: Vec<usize>,
    statement_index: StatementIndex,
    selected_ids: Entity<HashSet<String>>,
    selected_index: Option<IndexPath>,
}

impl DataDiffListDelegate {
    fn new(selected_ids: Entity<HashSet<String>>) -> Self {
        Self {
            result: None,
            table_indices: Vec::new(),
            statement_index: StatementIndex::new(),
            selected_ids,
            selected_index: None,
        }
    }

    fn set_result(&mut self, result: Arc<DataCompareBatchResult>, plan: Option<&SyncPlan>) {
        self.table_indices = result
            .table_results
            .iter()
            .enumerate()
            .filter_map(|(index, table)| {
                (!table.added.is_empty() || !table.removed.is_empty() || !table.modified.is_empty())
                    .then_some(index)
            })
            .collect();
        self.statement_index = build_statement_index(plan);
        self.result = Some(result);
        self.selected_index = None;
    }

    fn clear(&mut self) {
        self.result = None;
        self.table_indices.clear();
        self.statement_index.clear();
        self.selected_index = None;
    }

    fn table(&self, section: usize) -> Option<&DataCompareResult> {
        let result = self.result.as_ref()?;
        result.table_results.get(*self.table_indices.get(section)?)
    }
}

impl ListDelegate for DataDiffListDelegate {
    type Item = ListItem;

    fn sections_count(&self, _cx: &App) -> usize {
        self.table_indices.len().max(1)
    }

    fn items_count(&self, section: usize, _cx: &App) -> usize {
        self.table(section)
            .map(|table| table.added.len() + table.removed.len() + table.modified.len())
            .unwrap_or_default()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let table = self.table(ix.section)?;
        let table_name = table.target_table.as_str();
        let added_count = table.added.len();
        let removed_count = table.removed.len();

        if ix.row < added_count {
            let row = table.added.get(ix.row)?;
            let key_values = row_key(row, &table.key_columns);
            let canonical_key = canonical_row_key(&key_values);
            return Some(data_row(
                row_ui_id(table_name, &SyncStatementKind::Insert, &canonical_key),
                t!("Compare.added").to_string(),
                Tag::success(),
                key_values.clone(),
                &table.key_columns,
                row_value_summary(row, &table.columns),
                statement_ids_for_row(
                    &self.statement_index,
                    table_name,
                    &SyncStatementKind::Insert,
                    &key_values,
                ),
                self.selected_ids.clone(),
                cx,
            ));
        }

        let removed_row = ix.row.saturating_sub(added_count);
        if removed_row < removed_count {
            let row = table.removed.get(removed_row)?;
            let key_values = row_key(row, &table.key_columns);
            let canonical_key = canonical_row_key(&key_values);
            return Some(data_row(
                row_ui_id(table_name, &SyncStatementKind::Delete, &canonical_key),
                t!("Compare.removed").to_string(),
                Tag::danger(),
                key_values.clone(),
                &table.key_columns,
                row_value_summary(row, &table.columns),
                statement_ids_for_row(
                    &self.statement_index,
                    table_name,
                    &SyncStatementKind::Delete,
                    &key_values,
                ),
                self.selected_ids.clone(),
                cx,
            ));
        }

        let modified_row = ix
            .row
            .saturating_sub(added_count)
            .saturating_sub(removed_count);
        let modified = table.modified.get(modified_row)?;
        let details = table
            .columns
            .iter()
            .filter_map(|column| {
                modified.changes.get(column).map(|(source, target)| {
                    format!("{}: {} → {}", column, cell_text(target), cell_text(source))
                })
            })
            .collect::<Vec<_>>()
            .join(" · ");
        let canonical_key = canonical_row_key(&modified.key_values);
        Some(data_row(
            row_ui_id(table_name, &SyncStatementKind::Update, &canonical_key),
            t!("Compare.modified").to_string(),
            Tag::warning(),
            modified.key_values.clone(),
            &table.key_columns,
            truncate_text(&details, DATA_DIFF_VALUE_SUMMARY_LIMIT),
            statement_ids_for_row(
                &self.statement_index,
                table_name,
                &SyncStatementKind::Update,
                &modified.key_values,
            ),
            self.selected_ids.clone(),
            cx,
        ))
    }

    fn render_section_header(
        &mut self,
        section: usize,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<impl IntoElement> {
        let table = self.table(section)?;
        let total = table.added.len() + table.removed.len() + table.modified.len();
        Some(
            h_flex()
                .h(px(DATA_DIFF_SECTION_HEADER_HEIGHT))
                .px_3()
                .gap_2()
                .border_b_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().muted.opacity(0.18))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .font_semibold()
                        .text_sm()
                        .child(table.target_table.clone()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("Compare.data_diff_row_count", count = total).to_string()),
                ),
        )
    }

    fn render_empty(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        ContentState::empty(t!("Compare.data_diff_no_changes").to_string())
            .icon(IconName::CircleCheck)
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

fn data_row(
    id: String,
    kind_label: String,
    tag: Tag,
    key_values: HashMap<String, Value>,
    key_columns: &[String],
    details: String,
    statement_ids: Vec<String>,
    selected_ids: Entity<HashSet<String>>,
    cx: &App,
) -> ListItem {
    let checked = !statement_ids.is_empty()
        && statement_ids
            .iter()
            .all(|id| selected_ids.read(cx).contains(id));
    let selectable = !statement_ids.is_empty();
    let key_text = ordered_key_text(&key_values, key_columns);
    let statement_ids_for_click = statement_ids.clone();
    let checkbox_id = format!("{id}-checkbox");

    let row = h_flex()
        .w_full()
        .gap_2()
        .items_start()
        .when(selectable, |this| {
            this.child(Checkbox::new(checkbox_id).checked(checked))
        })
        .child(tag.small().child(kind_label))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(div().truncate().text_xs().child(key_text))
                .child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(details),
                ),
        );

    ListItem::new(id)
        .h(px(DATA_DIFF_ROW_HEIGHT))
        .child(row)
        .when(checked, |this| this.bg(cx.theme().list_active.opacity(0.7)))
        .when(selectable, |this| {
            this.on_click(move |_, _, cx| {
                selected_ids.update(cx, |ids, cx| {
                    let all_selected = statement_ids_for_click.iter().all(|id| ids.contains(id));
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
        // Json 列经过 serde 解析后，内嵌字符串里原本的字面量 `\r\n` 已被解码为真实换行。
        // 这里重新转义为可见的 `\r` / `\n`，保证对比面板与查询面板显示一致，不再被折行。
        Value::String(value) => value.replace('\r', "\\r").replace('\n', "\\n"),
        Value::Bool(value) => value.to_string(),
        _ => serde_json::to_string(value).unwrap_or_else(|_| "?".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_statement_index, canonical_row_key, cell_text, ordered_key_text, row_key, row_ui_id,
        row_value_summary, statement_ids_for_row, statement_kind_code,
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
    fn cell_text_escapes_real_newlines_in_string_values() {
        // Json 列解析后，内嵌字符串里的真实 CRLF 应显示为字面量 `\r\n`，与查询面板一致。
        let value = json!("a\r\nb");
        assert_eq!(cell_text(&value), "a\\r\\nb");
    }

    #[test]
    fn cell_text_object_branch_escapes_control_characters_via_serde() {
        // Object 分支走 serde_json::to_string，行为保持不变（内嵌真实换行仍会被转义）。
        let value = json!({"mapboxUrl": "a\r\nb"});
        assert_eq!(cell_text(&value), r#"{"mapboxUrl":"a\r\nb"}"#);
    }

    #[test]
    fn row_value_summary_escapes_newlines_in_json_string_values() {
        let row: RowData = [
            ("id".to_string(), json!(1)),
            ("map".to_string(), json!("a\r\nb")),
        ]
        .into_iter()
        .collect();

        assert_eq!(
            row_value_summary(&row, &["id".to_string(), "map".to_string()]),
            "id=1, map=a\\r\\nb"
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
