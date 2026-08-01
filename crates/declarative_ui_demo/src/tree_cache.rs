use std::collections::{HashMap, HashSet};

use gpui::{App, AppContext, Entity, ParentElement as _, Styled as _, Subscription, px};
use gpui_component::{
    list::ListItem,
    tree::{Tree, TreeItem, TreeState},
};

use crate::{
    ActionEvent, ComponentError, NodePath, VElement, VNode,
    component::stable_component_id,
    render_context::{ActionDispatcher, StateDispatcher},
};

// ── VNode → TreeItem conversion ──────────────────────────────────────────

/// Convert a `<tree-node>` or `<data-row>` VElement into a native [`TreeItem`].
pub(crate) fn build_tree_item(element: &VElement) -> Result<TreeItem, ComponentError> {
    let label = node_label(element)?;
    let id = element
        .attr("id")
        .or_else(|| element.attr("key"))
        .map(str::to_owned)
        .unwrap_or_else(|| label.clone());
    let mut item = TreeItem::new(id, label);
    if parse_bool(element.attr("expanded")) == Some(true) {
        item = item.expanded(true);
    }
    if parse_bool(element.attr("disabled")) == Some(true) {
        item = item.disabled(true);
    }
    for child in &element.children {
        if let VNode::Element(child_el) = child {
            if child_el.tag.eq_ignore_ascii_case("tree-node") {
                item = item.child(build_tree_item(child_el)?);
            }
        }
    }
    Ok(item)
}

/// Extract the display label from a tree-node or data-row element.
fn node_label(element: &VElement) -> Result<String, ComponentError> {
    if let Some(label) = element.attr("label") {
        return Ok(label.to_owned());
    }
    // Collect direct text children as the label.
    let mut parts = Vec::new();
    for child in &element.children {
        match child {
            VNode::Text(text) => parts.push(text.as_str()),
            VNode::Element(el) if el.tag.eq_ignore_ascii_case("tree-node") => {}
            _ => {
                return Err(ComponentError::new(format!(
                    "<{}> requires a `label` attribute or direct text",
                    element.tag
                )));
            }
        }
    }
    Ok(parts.join(" "))
}

fn parse_bool(value: Option<&str>) -> Option<bool> {
    let value = value?.trim();
    match value.to_ascii_lowercase().as_str() {
        "" | "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

// ── JSON array → TreeItem conversion ────────────────────────────────────

/// A deserializable representation of a tree node from JSON state.
///
/// Each item requires `id` and `label`; `expanded`, `disabled`, and `children`
/// are optional. This allows templates to bind a Runtime state key containing
/// a JSON array instead of declaring every `<tree-node>` in HTML.
#[derive(serde::Deserialize)]
struct JsonTreeItem {
    id: String,
    label: String,
    #[serde(default)]
    expanded: bool,
    #[serde(default)]
    disabled: bool,
    #[serde(default)]
    children: Vec<JsonTreeItem>,
}

impl JsonTreeItem {
    fn into_tree_item(self) -> TreeItem {
        let mut item = TreeItem::new(self.id, self.label);
        if self.expanded {
            item = item.expanded(true);
        }
        if self.disabled {
            item = item.disabled(true);
        }
        for child in self.children {
            item = item.child(child.into_tree_item());
        }
        item
    }
}

/// Parse a JSON array string into a vector of native [`TreeItem`]s.
///
/// Accepts a top-level JSON array where each element has `id`, `label`, and
/// optionally `expanded`, `disabled`, and nested `children`.
pub(crate) fn parse_tree_items(json: &str) -> Result<Vec<TreeItem>, ComponentError> {
    let items: Vec<JsonTreeItem> = serde_json::from_str(json)
        .map_err(|err| ComponentError::new(format!("failed to parse tree data JSON: {err}")))?;
    Ok(items
        .into_iter()
        .map(JsonTreeItem::into_tree_item)
        .collect())
}

/// A flat data row for `<data-list>` or `<list>` from JSON state.
#[derive(serde::Deserialize)]
struct JsonDataRow {
    id: String,
    label: String,
    #[serde(default)]
    disabled: bool,
}

/// Parse a JSON array of flat rows `{id, label}` into [`TreeItem`]s.
pub(crate) fn parse_data_rows(json: &str) -> Result<Vec<TreeItem>, ComponentError> {
    let rows: Vec<JsonDataRow> = serde_json::from_str(json)
        .map_err(|err| ComponentError::new(format!("failed to parse data rows JSON: {err}")))?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let mut item = TreeItem::new(row.id, row.label);
            if row.disabled {
                item = item.disabled(true);
            }
            item
        })
        .collect())
}

// ── Tree cache ───────────────────────────────────────────────────────────

/// Information needed to resolve or refresh a cached [`TreeState`].
pub(crate) struct TreeRequest {
    pub id: String,
    pub items: Vec<TreeItem>,
    pub selected_id: Option<String>,
    pub binding: Option<String>,
    pub action: Option<ActionEvent>,
}

pub(crate) struct TreeEnvironment<'a> {
    pub state_dispatcher: StateDispatcher,
    pub action_dispatcher: ActionDispatcher,
    pub cx: &'a mut App,
}

struct TreeCacheEntry {
    state: Entity<TreeState>,
    binding: Option<String>,
    action: Option<ActionEvent>,
    item_count: usize,
    _subscription: Option<Subscription>,
}

/// Cache of live `Entity<TreeState>` instances, keyed by stable component ID.
///
/// Mirrors the lifecycle of [`crate::input_cache::InputCache`] and
/// [`crate::slider_cache::SliderCache`]: entries survive across reconciles as
/// long as the corresponding `<tree>` or `<data-list>` node remains in the
/// resolved VNode tree.
#[derive(Default)]
pub(crate) struct TreeCache {
    entries: HashMap<String, TreeCacheEntry>,
}

impl TreeCache {
    /// Resolve (create or reuse) a `TreeState` for the given request, and
    /// return the native [`Tree`] element ready to embed in the GPUI tree.
    pub(crate) fn render_tree(
        &mut self,
        request: TreeRequest,
        environment: TreeEnvironment<'_>,
    ) -> Tree {
        let id = request.id.clone();

        // Reuse or create the entity.
        let entry = self.entries.entry(id.clone()).or_insert_with(|| {
            let state = environment.cx.new(|cx| {
                let mut ts = TreeState::new(cx);
                ts.set_items(request.items.clone(), cx);
                ts
            });
            TreeCacheEntry {
                state,
                binding: None,
                action: None,
                item_count: request.items.len(),
                _subscription: None,
            }
        });

        // Sync items when the count changed (data added/removed at runtime).
        let needs_update = entry.item_count != request.items.len();
        if needs_update {
            entry.state.update(environment.cx, |ts, cx| {
                ts.set_items(request.items.clone(), cx);
            });
            entry.item_count = request.items.len();
        }

        // Sync selection from binding.
        if let Some(selected_id) = &request.selected_id {
            let current = entry
                .state
                .read(environment.cx)
                .selected_item()
                .map(|item| item.id.to_string());
            if current.as_deref() != Some(selected_id.as_str()) {
                let target = request
                    .items
                    .iter()
                    .find(|item| item.id.as_ref() == selected_id.as_str());
                entry.state.update(environment.cx, |ts, cx| {
                    ts.set_selected_item(target, cx);
                });
            }
        }

        // Update callbacks (binding/action may change across reconciles).
        entry.binding = request.binding;
        entry.action = request.action;

        // Build the Tree element with a render closure that handles
        // selection write-back via on_click.
        let state = entry.state.clone();
        let binding = entry.binding.clone();
        let action = entry.action.clone();
        let state_dispatcher = environment.state_dispatcher.clone();
        let action_dispatcher = environment.action_dispatcher.clone();

        Tree::new(&state, move |ix, tree_entry, _selected, _window, _cx| {
            let depth = tree_entry.depth();
            let item = tree_entry.item();
            let label = item.label.clone();

            let mut list_item = ListItem::new(ix).pl(px(16.) * depth as f32).child(label);

            if tree_entry.is_disabled() {
                list_item = list_item.disabled(true);
            }

            // Attach click handler for selection write-back.
            let binding = binding.clone();
            let action = action.clone();
            let state_dispatcher = state_dispatcher.clone();
            let action_dispatcher = action_dispatcher.clone();
            let item_id = item.id.clone();

            list_item = list_item.on_click(move |_event, _window, cx| {
                if let Some(binding) = &binding {
                    state_dispatcher(binding.clone(), item_id.to_string(), cx);
                }
                if let Some(action) = &action {
                    action_dispatcher(action.clone(), cx);
                }
            });

            list_item
        })
    }

    /// Remove cache entries for tree/data-list nodes that are no longer live.
    pub(crate) fn retain_live(&mut self, root: &VNode) {
        let live = live_tree_ids(root);
        self.entries.retain(|id, _| live.contains(id));
    }
}

// ── Live identity collection ─────────────────────────────────────────────

fn live_tree_ids(root: &VNode) -> HashSet<String> {
    let mut ids = HashSet::new();
    collect_tree_ids(root, &NodePath::root(), &mut ids);
    ids
}

fn collect_tree_ids(node: &VNode, path: &NodePath, ids: &mut HashSet<String>) {
    match node {
        VNode::Element(element) => {
            if element.tag.eq_ignore_ascii_case("tree")
                || element.tag.eq_ignore_ascii_case("data-list")
            {
                ids.insert(stable_component_id(element, path));
            }
            for (index, child) in element.children.iter().enumerate() {
                collect_tree_ids(child, &path.child(index), ids);
            }
        }
        VNode::Fragment(children) => {
            for (index, child) in children.iter().enumerate() {
                collect_tree_ids(child, &path.child(index), ids);
            }
        }
        VNode::Text(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_tree_item_from_label_attribute() {
        let element = crate::parse_html(r#"<tree-node label="Hello"></tree-node>"#)
            .expect("valid html")
            .element()
            .expect("element")
            .to_owned();
        let item = build_tree_item(&element).expect("tree item");
        assert_eq!(item.label.as_ref(), "Hello");
        assert!(!item.is_folder());
    }

    #[test]
    fn build_tree_item_with_children() {
        let root = crate::parse_html(
            r#"
            <tree-node label="root">
                <tree-node label="child-a"></tree-node>
                <tree-node label="child-b"></tree-node>
            </tree-node>
            "#,
        )
        .expect("valid html");
        let element = root.element().expect("element");
        let item = build_tree_item(element).expect("tree item");
        assert!(item.is_folder());
        assert_eq!(item.children.len(), 2);
    }

    #[test]
    fn retain_live_removes_obsolete_trees() {
        let old =
            crate::parse_html(r#"<tree id="my-tree"><tree-node label="a"></tree-node></tree>"#)
                .expect("old");
        let next = crate::parse_html("<div></div>").expect("next");
        assert!(live_tree_ids(&old).contains("tree:my-tree"));
        assert!(live_tree_ids(&next).is_empty());
    }
}
