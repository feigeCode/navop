use gpui::IntoElement;

use crate::{
    ComponentError, ComponentProps, ComponentRegistry, ComponentRenderer, ComponentResult,
    ComponentSchema, RegistryError, RenderContext, VNode, tree_cache::build_tree_item,
};

use super::{action_event, parse_positive_usize_attribute};

/// Maximum number of flat rows in a `<data-list>`.
const MAX_DATA_ROWS: usize = 100_000;

pub(super) fn register(registry: &mut ComponentRegistry) -> Result<(), RegistryError> {
    registry.register_with_schema("tree", tree_schema(), TreeComponent)?;
    registry.register_with_schema("tree-node", tree_node_schema(), TreeNodeComponent)?;
    registry.register_with_schema("data-list", data_list_schema(), DataListComponent)?;
    Ok(())
}

fn tree_schema() -> ComponentSchema {
    ComponentSchema::new()
        .required_attribute("id")
        .attribute("bind")
        .attribute("selected-id")
        .attribute("data-items")
        .attribute("action")
        .data_attributes()
}

fn tree_node_schema() -> ComponentSchema {
    ComponentSchema::new()
        .attribute("label")
        .attribute("expanded")
        .attribute("disabled")
        .attribute("action")
        .data_attributes()
}

fn data_list_schema() -> ComponentSchema {
    ComponentSchema::new()
        .required_attribute("id")
        .attribute("data-count")
        .attribute("data-items")
        .attribute("bind")
        .attribute("selected-id")
        .attribute("action")
        .attribute("data-label")
        .data_attributes()
}

// ── <tree> ───────────────────────────────────────────────────────────────

struct TreeComponent;

impl ComponentRenderer for TreeComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        let stable_id = props.stable_id();
        let items = if let Some(state_key) = props.element.attr("data-items") {
            let json = context.get_state(state_key).ok_or_else(|| {
                ComponentError::new(format!(
                    "`data-items=\"{state_key}\"` references an undefined state key"
                ))
            })?;
            crate::tree_cache::parse_tree_items(&json)?
        } else {
            collect_tree_nodes(&props)?
        };
        let selected_id = resolve_selected_id(&props, "selected-id");

        let request = crate::tree_cache::TreeRequest {
            id: stable_id,
            items,
            selected_id,
            binding: props.element.attr("bind").map(str::to_owned),
            action: props
                .element
                .attr("action")
                .map(|action| action_event(action, &props)),
        };

        let tree = context.render_tree(request);
        Ok(tree.into_any_element())
    }
}

/// Collect `<tree-node>` children of a `<tree>` into native [`TreeItem`]s.
fn collect_tree_nodes(
    props: &ComponentProps,
) -> Result<Vec<gpui_component::tree::TreeItem>, ComponentError> {
    let mut items = Vec::new();
    for child in &props.element.children {
        match child {
            VNode::Element(element) if element.tag.eq_ignore_ascii_case("tree-node") => {
                items.push(build_tree_item(element)?);
            }
            VNode::Text(text) if text.trim().is_empty() => {}
            _ => {
                return Err(ComponentError::new(format!(
                    "<tree> only accepts <tree-node> children, found <{}>",
                    child.element().map(|e| e.tag.as_str()).unwrap_or("text")
                )));
            }
        }
    }
    Ok(items)
}

// ── <tree-node> ──────────────────────────────────────────────────────────

/// `<tree-node>` rendered standalone is a structural error — it must be
/// consumed by its parent `<tree>`.
struct TreeNodeComponent;

impl ComponentRenderer for TreeNodeComponent {
    fn render(&self, _props: ComponentProps, _context: &mut RenderContext<'_>) -> ComponentResult {
        Err(ComponentError::new(
            "<tree-node> must be rendered inside <tree>".to_string(),
        ))
    }
}

// ── <data-list> ──────────────────────────────────────────────────────────

struct DataListComponent;

impl ComponentRenderer for DataListComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        let stable_id = props.stable_id();
        let items = if let Some(state_key) = props.element.attr("data-items") {
            let json = context.get_state(state_key).ok_or_else(|| {
                ComponentError::new(format!(
                    "`data-items=\"{state_key}\"` references an undefined state key"
                ))
            })?;
            crate::tree_cache::parse_data_rows(&json)?
        } else {
            let count =
                parse_positive_usize_attribute(&props.element, "data-count")?.ok_or_else(|| {
                    ComponentError::new(
                        "<data-list> requires either `data-items` or `data-count` attribute",
                    )
                })?;
            if count > MAX_DATA_ROWS {
                return Err(ComponentError::new(format!(
                    "attribute `data-count` on <data-list> must not exceed {MAX_DATA_ROWS}, got \
                     `{count}`"
                )));
            }
            let label_template = props
                .element
                .attr("data-label")
                .map(str::to_owned)
                .unwrap_or_else(|| "Item {n}".to_owned());
            (0..count)
                .map(|i| {
                    let n = i + 1;
                    let label = label_template.replace("{n}", &n.to_string());
                    gpui_component::tree::TreeItem::new(format!("row-{n}"), label)
                })
                .collect()
        };

        let selected_id = resolve_selected_id(&props, "selected-id");

        let request = crate::tree_cache::TreeRequest {
            id: stable_id,
            items,
            selected_id,
            binding: props.element.attr("bind").map(str::to_owned),
            action: props
                .element
                .attr("action")
                .map(|action| action_event(action, &props)),
        };

        let tree = context.render_tree(request);
        Ok(tree.into_any_element())
    }
}

// ── Shared helpers ───────────────────────────────────────────────────────

/// Resolve the currently selected item ID from either a bound state key
/// or a static `selected-id` attribute.
fn resolve_selected_id(props: &ComponentProps, attr_name: &str) -> Option<String> {
    // If there's a `bind`, the binding resolver already wrote the resolved
    // value into the attribute named `selected-id`.
    props.element.attr(attr_name).map(str::to_owned)
}
