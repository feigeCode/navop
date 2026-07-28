use std::sync::{Arc, Mutex};

use gpui::{App, Axis, Entity, IntoElement, ParentElement, Styled, div, px};
use gpui_component::{
    Sizable,
    accordion::Accordion,
    collapsible::Collapsible,
    resizable::{ResizablePanel, ResizablePanelGroup, resizable_panel},
};

use crate::{
    ActionEvent, ComponentError, ComponentProps, ComponentRegistry, ComponentRenderer,
    ComponentResult, ComponentSchema, RegistryError, RenderContext, Runtime, VNode,
};

use super::{
    action_event, bool_attribute, bool_attribute_or, bound_bool_attribute_or,
    parse_non_negative_f32, parse_size_attribute,
};

const DEFAULT_PANEL_MIN_SIZE_PX: f32 = 100.0;

pub(super) fn register(registry: &mut ComponentRegistry) -> Result<(), RegistryError> {
    registry.register_with_schema("accordion", accordion_schema(), AccordionComponent)?;
    registry.register_with_schema(
        "accordion-item",
        ComponentSchema::new().required_attribute("title"),
        StructuralLayoutComponent,
    )?;
    registry.register_with_schema(
        "collapsible",
        ComponentSchema::new().attribute("open").attribute("bind"),
        CollapsibleComponent,
    )?;
    registry.register_with_schema(
        "collapsible-content",
        ComponentSchema::new(),
        StructuralLayoutComponent,
    )?;
    registry.register_with_schema(
        "resizable",
        ComponentSchema::new()
            .attribute("orientation")
            .attribute("size"),
        ResizableComponent,
    )?;
    registry.register_with_schema(
        "resizable-panel",
        ComponentSchema::new()
            .attribute("size")
            .attribute("min-size")
            .attribute("max-size")
            .attribute("visible"),
        StructuralLayoutComponent,
    )?;
    Ok(())
}

fn accordion_schema() -> ComponentSchema {
    ComponentSchema::new()
        .attribute("bind")
        .attribute("open-indices")
        .attribute("multiple")
        .attribute("bordered")
        .attribute("disabled")
        .attribute("size")
        .attribute("action")
        .data_attributes()
}

struct AccordionComponent;

impl ComponentRenderer for AccordionComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        let item_props = accordion_item_props(&props)?;
        let multiple = bool_attribute(&props.element, "multiple")?;
        let bordered = bool_attribute_or(&props.element, "bordered", true)?;
        let disabled = bool_attribute(&props.element, "disabled")?;
        let size = parse_size_attribute(&props.element)?;
        let open_indices = parse_open_indices(&props, item_props.len(), multiple)?;

        let mut accordion = Accordion::new(props.stable_id())
            .multiple(multiple)
            .bordered(bordered)
            .disabled(disabled);
        if let Some(size) = size {
            accordion = accordion.with_size(size);
        }

        for (index, item_props) in item_props.into_iter().enumerate() {
            let title = item_props
                .element
                .attr("title")
                .filter(|title| !title.trim().is_empty())
                .ok_or_else(|| {
                    ComponentError::new("<accordion-item> requires non-empty `title` attribute")
                })?
                .to_owned();
            let children = context.render_children(&item_props);
            let body = context.style(div().children(children), &item_props);
            let open = open_indices.binary_search(&index).is_ok();
            accordion = accordion.item(move |item| item.title(title).open(open).child(body));
        }

        let binding = props.element.attr("bind").map(str::to_owned);
        let event = props
            .element
            .attr("action")
            .map(|action| action_event(action, &props));
        if binding.is_some() || event.is_some() {
            let runtime = context.runtime_entity();
            let last_open_indices = Arc::new(Mutex::new(open_indices));
            accordion = accordion.on_toggle_click(move |indices, _window, cx| {
                let indices = canonical_open_indices(indices);
                {
                    let mut last = last_open_indices
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    if *last == indices {
                        return;
                    }
                    last.clone_from(&indices);
                }

                let encoded = encode_open_indices(&indices);
                update_accordion_runtime(
                    &runtime,
                    binding.as_deref(),
                    event.as_ref(),
                    &encoded,
                    cx,
                );
            });
        }

        let wrapper = div().w_full().child(accordion);
        Ok(context.style(wrapper, &props).into_any_element())
    }
}

fn accordion_item_props(props: &ComponentProps) -> Result<Vec<ComponentProps>, ComponentError> {
    if props.element.children.is_empty() {
        return Err(ComponentError::new(
            "<accordion> requires at least one direct <accordion-item> child",
        ));
    }

    props
        .element
        .children
        .iter()
        .enumerate()
        .map(|(index, child)| {
            structural_element_props(props, index, child, "accordion-item").map_err(|_| {
                let found = child
                    .element()
                    .map(|element| format!("<{}>", element.tag))
                    .unwrap_or_else(|| "a non-element node".to_owned());
                ComponentError::new(format!(
                    "<accordion> only accepts direct <accordion-item> children, found {found}"
                ))
            })
        })
        .collect()
}

fn parse_open_indices(
    props: &ComponentProps,
    item_count: usize,
    multiple: bool,
) -> Result<Vec<usize>, ComponentError> {
    let Some(value) = props.element.attr("open-indices") else {
        return Ok(Vec::new());
    };
    if props.element.attr("bind").is_some() && value.trim().is_empty() {
        return Ok(Vec::new());
    }

    let parsed = serde_json::from_str::<Vec<usize>>(value).map_err(|_| {
        ComponentError::new(format!(
            "attribute `open-indices` on <accordion> must be a JSON array of non-negative integers, got `{value}`"
        ))
    })?;
    let indices = canonical_open_indices(&parsed);
    if let Some(index) = indices.iter().copied().find(|index| *index >= item_count) {
        return Err(ComponentError::new(format!(
            "index {index} in `open-indices` on <accordion> is out of range for {item_count} items"
        )));
    }
    if !multiple && indices.len() > 1 {
        return Err(ComponentError::new(format!(
            "attribute `open-indices` on <accordion> contains {} items while `multiple=false`",
            indices.len()
        )));
    }
    Ok(indices)
}

fn canonical_open_indices(indices: &[usize]) -> Vec<usize> {
    let mut indices = indices.to_vec();
    indices.sort_unstable();
    indices.dedup();
    indices
}

fn encode_open_indices(indices: &[usize]) -> String {
    let body = indices
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(",");
    format!("[{body}]")
}

fn update_accordion_runtime(
    runtime: &Entity<Runtime>,
    binding: Option<&str>,
    event: Option<&ActionEvent>,
    encoded: &str,
    cx: &mut App,
) {
    if let Some(binding) = binding {
        let _ = runtime.update(cx, |runtime, cx| {
            runtime.set(binding.to_owned(), encoded.to_owned(), cx);
        });
    }
    if let Some(event) = event {
        let _ = runtime.update(cx, |runtime, cx| runtime.dispatch(event.clone(), cx));
    }
}

struct CollapsibleComponent;

impl ComponentRenderer for CollapsibleComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        let content_count = props
            .element
            .children
            .iter()
            .filter(|child| child_has_tag(child, "collapsible-content"))
            .count();
        if content_count != 1 {
            return Err(ComponentError::new(format!(
                "<collapsible> requires exactly one direct <collapsible-content> child, found {content_count}"
            )));
        }

        let open = bound_bool_attribute_or(&props, "open", false)?;
        let mut collapsible = Collapsible::new().open(open);
        for (index, child) in props.element.children.iter().enumerate() {
            if child_has_tag(child, "collapsible-content") {
                let content_props =
                    structural_element_props(&props, index, child, "collapsible-content")?;
                let content = div().children(context.render_children(&content_props));
                collapsible = collapsible.content(context.style(content, &content_props));
            } else {
                collapsible = collapsible.child(context.render_child(&props, index));
            }
        }

        Ok(context.style(collapsible, &props).into_any_element())
    }
}

struct ResizableComponent;

impl ComponentRenderer for ResizableComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        let axis = parse_orientation(&props)?;
        let group_size = parse_positive_f32_attribute(&props, "size")?;
        let panels = build_resizable_panels(&props, context)?;

        let mut group = ResizablePanelGroup::new(props.stable_id())
            .axis(axis)
            .children(panels);
        if let Some(size) = group_size {
            group = group.size(px(size));
        }

        let wrapper = match (axis, group_size) {
            (Axis::Horizontal, Some(size)) => div().w_full().h(px(size)),
            (Axis::Vertical, Some(size)) => div().h_full().w(px(size)),
            (_, None) => div().size_full(),
        }
        .child(group);

        Ok(context.style(wrapper, &props).into_any_element())
    }
}

fn build_resizable_panels(
    props: &ComponentProps,
    context: &mut RenderContext<'_>,
) -> Result<Vec<ResizablePanel>, ComponentError> {
    if props.element.children.len() < 2 {
        return Err(ComponentError::new(format!(
            "<resizable> requires at least two direct <resizable-panel> children, found {}",
            props.element.children.len()
        )));
    }

    props
        .element
        .children
        .iter()
        .enumerate()
        .map(|(index, child)| {
            let panel_props = structural_element_props(props, index, child, "resizable-panel")
                .map_err(|_| {
                    let found = child
                        .element()
                        .map(|element| format!("<{}>", element.tag))
                        .unwrap_or_else(|| "a non-element node".to_owned());
                    ComponentError::new(format!(
                        "<resizable> only accepts direct <resizable-panel> children, found {found}"
                    ))
                })?;
            build_resizable_panel(panel_props, context)
        })
        .collect()
}

fn build_resizable_panel(
    props: ComponentProps,
    context: &mut RenderContext<'_>,
) -> Result<ResizablePanel, ComponentError> {
    let min_size = parse_optional_non_negative_f32_attribute(&props, "min-size")?
        .unwrap_or(DEFAULT_PANEL_MIN_SIZE_PX);
    let max_size =
        parse_optional_non_negative_f32_attribute(&props, "max-size")?.unwrap_or(f32::MAX);
    if max_size <= min_size {
        return Err(ComponentError::new(format!(
            "attribute `max-size` on <resizable-panel> must be greater than `min-size` ({min_size}), got `{max_size}`"
        )));
    }

    let initial_size = parse_optional_non_negative_f32_attribute(&props, "size")?;
    if let Some(initial_size) = initial_size
        && !(min_size..=max_size).contains(&initial_size)
    {
        return Err(ComponentError::new(format!(
            "attribute `size` on <resizable-panel> must be within the configured range [{min_size}, {max_size}], got `{initial_size}`"
        )));
    }

    let visible = bool_attribute_or(&props.element, "visible", true)?;
    let mut panel = resizable_panel()
        .visible(visible)
        .size_range(px(min_size)..px(max_size))
        .children(context.render_children(&props));
    if let Some(initial_size) = initial_size {
        panel = panel.size(px(initial_size));
    }
    Ok(context.style(panel, &props))
}

fn parse_orientation(props: &ComponentProps) -> Result<Axis, ComponentError> {
    let Some(value) = props.element.attr("orientation") else {
        return Ok(Axis::Horizontal);
    };
    match value.trim().to_ascii_lowercase().as_str() {
        "horizontal" => Ok(Axis::Horizontal),
        "vertical" => Ok(Axis::Vertical),
        _ => Err(ComponentError::new(format!(
            "attribute `orientation` on <resizable> must be horizontal or vertical, got `{value}`"
        ))),
    }
}

fn parse_positive_f32_attribute(
    props: &ComponentProps,
    name: &str,
) -> Result<Option<f32>, ComponentError> {
    let Some(value) = props.element.attr(name) else {
        return Ok(None);
    };
    let parsed = value.parse::<f32>().map_err(|_| {
        ComponentError::new(format!(
            "attribute `{name}` on <{}> must be a finite positive number, got `{value}`",
            props.element.tag
        ))
    })?;
    if !parsed.is_finite() || parsed <= 0.0 {
        return Err(ComponentError::new(format!(
            "attribute `{name}` on <{}> must be a finite positive number, got `{value}`",
            props.element.tag
        )));
    }
    Ok(Some(parsed))
}

fn parse_optional_non_negative_f32_attribute(
    props: &ComponentProps,
    name: &str,
) -> Result<Option<f32>, ComponentError> {
    props
        .element
        .attr(name)
        .map(|value| parse_non_negative_f32(&props.element, name, value))
        .transpose()
}

struct StructuralLayoutComponent;

impl ComponentRenderer for StructuralLayoutComponent {
    fn render(&self, props: ComponentProps, _context: &mut RenderContext<'_>) -> ComponentResult {
        let parent = match props.element.tag.as_str() {
            "accordion-item" => "<accordion>",
            "collapsible-content" => "<collapsible>",
            "resizable-panel" => "<resizable>",
            _ => unreachable!("only registered structural layout tags use this renderer"),
        };
        Err(ComponentError::new(format!(
            "<{}> must be rendered inside a structurally valid {parent}",
            props.element.tag
        )))
    }
}

fn child_has_tag(child: &VNode, tag: &str) -> bool {
    child.element().is_some_and(|element| element.tag == tag)
}

fn structural_element_props(
    parent: &ComponentProps,
    index: usize,
    child: &VNode,
    expected_tag: &str,
) -> Result<ComponentProps, ComponentError> {
    let Some(element) = child.element() else {
        return Err(ComponentError::new(format!(
            "<{}> requires a direct <{expected_tag}> element child",
            parent.element.tag
        )));
    };
    if element.tag != expected_tag {
        return Err(ComponentError::new(format!(
            "<{}> requires a direct <{expected_tag}> child, found <{}>",
            parent.element.tag, element.tag
        )));
    }
    Ok(ComponentProps::new(
        element.clone(),
        parent.path.child(index),
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use gpui::{AppContext, TestAppContext};

    use crate::{ActionEvent, ComponentProps, NodePath, Runtime, StateStore, VElement};

    use super::{encode_open_indices, parse_open_indices, update_accordion_runtime};

    #[test]
    fn accordion_open_indices_are_canonicalized() {
        let props = accordion_props(Some("[2,0,2]"), false);

        assert_eq!(vec![0, 2], parse_open_indices(&props, 3, true).unwrap());
        assert_eq!("[0,2]", encode_open_indices(&[0, 2]));
    }

    #[test]
    fn accordion_missing_binding_value_means_no_open_items() {
        let props = accordion_props(Some(""), true);

        assert!(parse_open_indices(&props, 1, false).unwrap().is_empty());
    }

    #[test]
    fn accordion_explicit_empty_state_is_invalid() {
        let props = accordion_props(Some(""), false);

        let error = parse_open_indices(&props, 1, false).unwrap_err();
        assert!(error.to_string().contains("JSON array"));
    }

    #[test]
    fn accordion_open_indices_honor_range_and_multiple_constraints() {
        let out_of_range = accordion_props(Some("[2]"), false);
        assert!(
            parse_open_indices(&out_of_range, 2, true)
                .unwrap_err()
                .to_string()
                .contains("out of range")
        );

        let multiple = accordion_props(Some("[0,1]"), false);
        assert!(
            parse_open_indices(&multiple, 2, false)
                .unwrap_err()
                .to_string()
                .contains("multiple=false")
        );
    }

    #[gpui::test]
    fn accordion_binding_is_committed_before_action(cx: &mut TestAppContext) {
        let runtime = cx.update(|cx| {
            let mut state = StateStore::default();
            state.set("open_sections", "[]");
            cx.new(|_| {
                let mut runtime = Runtime::new(state);
                runtime
                    .on("observe-accordion", |context| {
                        let value = context.get("open_sections").unwrap_or_default().to_owned();
                        context.set("observed_open_sections", value);
                        Ok(())
                    })
                    .expect("unique test action");
                runtime
            })
        });
        let event = ActionEvent::new("observe-accordion", "accordion:test", NodePath::root());

        cx.update(|cx| {
            update_accordion_runtime(&runtime, Some("open_sections"), Some(&event), "[0,2]", cx);
        });

        runtime.read_with(cx, |runtime, _| {
            assert_eq!(Some("[0,2]"), runtime.get("open_sections"));
            assert_eq!(Some("[0,2]"), runtime.get("observed_open_sections"));
        });
    }

    fn accordion_props(open_indices: Option<&str>, bound: bool) -> ComponentProps {
        let mut attrs = BTreeMap::new();
        if let Some(open_indices) = open_indices {
            attrs.insert("open-indices".to_owned(), open_indices.to_owned());
        }
        if bound {
            attrs.insert("bind".to_owned(), "open_sections".to_owned());
        }
        ComponentProps::new(
            VElement {
                tag: "accordion".to_owned(),
                attrs,
                classes: Vec::new(),
                children: Vec::new(),
            },
            NodePath::root(),
        )
    }
}
