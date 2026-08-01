use std::collections::{HashMap, HashSet};

use gpui::{
    App, AppContext, Context, Entity, IntoElement, ParentElement, SharedString, Styled,
    Subscription, Window, div, px,
};
use gpui_component::{
    Sizable,
    table::{Column, DataTable, TableDelegate, TableState},
};

use crate::{ComponentError, NodePath, VNode, component::stable_component_id};

// ── JSON deserialization types ──────────────────────────────────────────

/// Column definition in the JSON table schema.
#[derive(serde::Deserialize, Debug)]
struct JsonColumn {
    key: String,
    label: String,
    #[serde(default)]
    align: Option<String>,
    #[serde(default)]
    width: Option<f32>,
}

/// The full table data schema:
///
/// ```json
/// {
///   "columns": [
///     {"key": "name", "label": "Name"},
///     {"key": "age", "label": "Age", "align": "right"}
///   ],
///   "rows": [
///     {"name": "Alice", "age": "30"},
///     {"name": "Bob",   "age": "25"}
///   ]
/// }
/// ```
#[derive(serde::Deserialize, Debug)]
pub(crate) struct JsonTable {
    columns: Vec<JsonColumn>,
    #[serde(default)]
    rows: Vec<HashMap<String, String>>,
}

/// Parse a JSON table descriptor string.
pub(crate) fn parse_table(json: &str) -> Result<JsonTable, ComponentError> {
    serde_json::from_str(json)
        .map_err(|err| ComponentError::new(format!("failed to parse table data JSON: {err}")))
}

// ── Table delegate ──────────────────────────────────────────────────────

/// A [`TableDelegate`] backed by parsed JSON data.
///
/// Each row is a `HashMap<String, String>` keyed by column key. The delegate
/// clones the cell string into a `SharedString` on demand — only visible rows
/// are ever rendered thanks to the native virtual scrolling backing store.
pub(crate) struct JsonTableDelegate {
    columns: Vec<Column>,
    column_keys: Vec<String>,
    rows: Vec<HashMap<String, String>>,
}

impl JsonTableDelegate {
    fn new(table: JsonTable) -> Self {
        let columns = table
            .columns
            .iter()
            .map(|c| {
                let mut col = Column::new(c.key.as_str(), c.label.as_str());
                match c.align.as_deref().map(str::to_ascii_lowercase).as_deref() {
                    Some("center") => col = col.text_center(),
                    Some("right") => col = col.text_right(),
                    _ => {}
                }
                if let Some(w) = c.width {
                    if w > 0.0 {
                        col = col.width(px(w));
                    }
                }
                col
            })
            .collect();
        let column_keys = table.columns.iter().map(|c| c.key.clone()).collect();
        Self {
            columns,
            column_keys,
            rows: table.rows,
        }
    }
}

impl TableDelegate for JsonTableDelegate {
    fn columns_count(&self, _cx: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _cx: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _cx: &App) -> Column {
        self.columns[col_ix].clone()
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let key = &self.column_keys[col_ix];
        let text: SharedString = self
            .rows
            .get(row_ix)
            .and_then(|row| row.get(key))
            .cloned()
            .unwrap_or_default()
            .into();
        div().w_full().truncate().child(text)
    }
}

// ── Table cache ─────────────────────────────────────────────────────────

/// Information needed to resolve or refresh a cached [`TableState`].
pub(crate) struct TableRequest {
    pub id: String,
    pub data: JsonTable,
    pub stripe: bool,
    pub bordered: bool,
    pub size: Option<gpui_component::Size>,
}

/// Environment passed to the cache for each render.
pub(crate) struct TableEnvironment<'a> {
    pub window: &'a mut Window,
    pub cx: &'a mut App,
}

struct TableCacheEntry {
    state: Entity<TableState<JsonTableDelegate>>,
    _subscription: Option<Subscription>,
}

/// Cache of live `Entity<TableState<JsonTableDelegate>>` instances, keyed by
/// stable component ID — mirrors [`crate::tree_cache::TreeCache`].
#[derive(Default)]
pub(crate) struct TableCache {
    entries: HashMap<String, TableCacheEntry>,
}

impl TableCache {
    /// Resolve (create or reuse) a `TableState` and return the native
    /// [`DataTable`] element ready to embed in the GPUI tree.
    pub(crate) fn render_table(
        &mut self,
        request: TableRequest,
        environment: TableEnvironment<'_>,
    ) -> DataTable<JsonTableDelegate> {
        let id = request.id.clone();
        let delegate = JsonTableDelegate::new(request.data);

        let entry = self.entries.entry(id.clone()).or_insert_with(|| {
            let state = environment
                .cx
                .new(|cx| TableState::new(delegate, environment.window, cx).row_selectable(true));
            TableCacheEntry {
                state,
                _subscription: None,
            }
        });

        let mut dt = DataTable::new(&entry.state).bordered(request.bordered);
        if request.stripe {
            dt = dt.stripe(true);
        }
        if let Some(size) = request.size {
            dt = dt.with_size(size);
        }
        dt
    }

    /// Remove cache entries for table nodes that are no longer live.
    pub(crate) fn retain_live(&mut self, root: &VNode) {
        let live = live_table_ids(root);
        self.entries.retain(|id, _| live.contains(id));
    }
}

// ── Live identity collection ─────────────────────────────────────────────

fn live_table_ids(root: &VNode) -> HashSet<String> {
    let mut ids = HashSet::new();
    collect_table_ids(root, &NodePath::root(), &mut ids);
    ids
}

fn collect_table_ids(node: &VNode, path: &NodePath, ids: &mut HashSet<String>) {
    match node {
        VNode::Element(element) => {
            if element.tag.eq_ignore_ascii_case("table") && element.attr("data-items").is_some() {
                ids.insert(stable_component_id(element, path));
            }
            for (index, child) in element.children.iter().enumerate() {
                collect_table_ids(child, &path.child(index), ids);
            }
        }
        VNode::Fragment(children) => {
            for (index, child) in children.iter().enumerate() {
                collect_table_ids(child, &path.child(index), ids);
            }
        }
        VNode::Text(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_table_with_columns_and_rows() {
        let json = r#"{
            "columns": [
                {"key": "name", "label": "Name"},
                {"key": "age", "label": "Age", "align": "right"}
            ],
            "rows": [
                {"name": "Alice", "age": "30"},
                {"name": "Bob", "age": "25"}
            ]
        }"#;
        let table = parse_table(json).expect("valid table JSON");
        assert_eq!(table.columns.len(), 2);
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.columns[0].key, "name");
        assert_eq!(table.rows[0].get("name").unwrap(), "Alice");
    }

    #[test]
    fn parse_table_empty_rows() {
        let json = r#"{"columns":[{"key":"x","label":"X"}]}"#;
        let table = parse_table(json).expect("valid table JSON");
        assert_eq!(table.columns.len(), 1);
        assert!(table.rows.is_empty());
    }

    #[test]
    fn parse_table_invalid_json_returns_error() {
        let json = r#"not valid json"#;
        let err = parse_table(json).expect_err("should fail");
        assert!(err.to_string().contains("failed to parse table data JSON"));
    }

    #[test]
    fn delegate_columns_and_rows_counts() {
        let table = parse_table(
            r#"{
            "columns": [{"key":"a","label":"A"},{"key":"b","label":"B"}],
            "rows": [{"a":"1","b":"2"},{"a":"3","b":"4"},{"a":"5","b":"6"}]
        }"#,
        )
        .expect("valid");
        let delegate = JsonTableDelegate::new(table);
        assert_eq!(delegate.columns.len(), 2);
        assert_eq!(delegate.rows.len(), 3);
    }

    #[test]
    fn retain_live_tracks_data_items_tables() {
        let html = r#"<table id="t1" data-items="src"></table>"#;
        let parsed = crate::parse_html(html).expect("valid html");
        let ids = live_table_ids(&parsed);
        assert!(ids.contains("table:t1"));

        let no_data = r#"<table id="t2"><tbody><tr><td>static</td></tr></tbody></table>"#;
        let parsed2 = crate::parse_html(no_data).expect("valid html");
        assert!(live_table_ids(&parsed2).is_empty());
    }
}
