use gpui::{App, Axis, IntoElement, ParentElement, Window};
use gpui_component::{
    Disableable, Sizable,
    breadcrumb::{Breadcrumb, BreadcrumbItem},
    pagination::Pagination,
    rating::Rating,
    stepper::{Stepper, StepperItem},
    tab::{Tab, TabBar, TabVariant},
};

use crate::{
    ComponentError, ComponentProps, ComponentRegistry, ComponentRenderer, ComponentResult,
    ComponentSchema, RegistryError, RenderContext, VNode,
};

use super::{
    action_event, bool_attribute, parse_positive_usize_attribute, parse_size_attribute,
    parse_usize_attribute,
};

const MAX_VISIBLE_PAGES: usize = 100;
const MAX_RATING_STARS: usize = 100;

pub(super) fn register(registry: &mut ComponentRegistry) -> Result<(), RegistryError> {
    registry.register_with_schema("breadcrumb", ComponentSchema::new(), BreadcrumbComponent)?;
    registry.register_with_schema(
        "breadcrumb-item",
        breadcrumb_item_schema(),
        StructuralNavigationComponent,
    )?;
    registry.register_with_schema("pagination", pagination_schema(), PaginationComponent)?;
    registry.register_with_schema("rating", rating_schema(), RatingComponent)?;
    registry.register_with_schema("tabs", tabs_schema(), TabsComponent)?;
    registry.register_with_schema("tab", tab_schema(), StructuralNavigationComponent)?;
    registry.register_with_schema("stepper", stepper_schema(), StepperComponent)?;
    registry.register_with_schema(
        "stepper-item",
        stepper_item_schema(),
        StructuralNavigationComponent,
    )?;
    Ok(())
}

fn breadcrumb_item_schema() -> ComponentSchema {
    ComponentSchema::new()
        .attribute("label")
        .attribute("disabled")
        .attribute("action")
        .data_attributes()
}

fn pagination_schema() -> ComponentSchema {
    ComponentSchema::new()
        .attribute("bind")
        .attribute("current-page")
        .attribute("total-pages")
        .attribute("visible-pages")
        .attribute("compact")
        .attribute("disabled")
        .attribute("size")
        .attribute("action")
        .data_attributes()
}

fn rating_schema() -> ComponentSchema {
    ComponentSchema::new()
        .attribute("bind")
        .attribute("value")
        .attribute("max")
        .attribute("disabled")
        .attribute("size")
        .attribute("action")
        .data_attributes()
}

fn tabs_schema() -> ComponentSchema {
    ComponentSchema::new()
        .attribute("bind")
        .attribute("selected-index")
        .attribute("variant")
        .attribute("menu")
        .attribute("size")
        .attribute("action")
        .data_attributes()
}

fn tab_schema() -> ComponentSchema {
    ComponentSchema::new()
        .attribute("label")
        .attribute("disabled")
}

fn stepper_schema() -> ComponentSchema {
    ComponentSchema::new()
        .attribute("bind")
        .attribute("selected-index")
        .attribute("layout")
        .attribute("text-center")
        .attribute("disabled")
        .attribute("size")
        .attribute("action")
        .data_attributes()
}

fn stepper_item_schema() -> ComponentSchema {
    ComponentSchema::new().attribute("disabled")
}

struct BreadcrumbComponent;

impl ComponentRenderer for BreadcrumbComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        let items = props
            .element
            .children
            .iter()
            .enumerate()
            .map(|(index, child)| {
                let child_props = structural_child_props(&props, index, child, "breadcrumb-item")?;
                build_breadcrumb_item(child_props, context)
            })
            .collect::<Result<Vec<_>, ComponentError>>()?;
        let breadcrumb = Breadcrumb::new().children(items);
        Ok(context.style(breadcrumb, &props).into_any_element())
    }
}

fn build_breadcrumb_item(
    props: ComponentProps,
    context: &mut RenderContext<'_>,
) -> Result<BreadcrumbItem, ComponentError> {
    let label = label_attribute_or_text(&props)?;
    let mut item = BreadcrumbItem::new(label).disabled(bool_attribute(&props.element, "disabled")?);
    if let Some(action) = props.element.attr("action") {
        let event = action_event(action, &props);
        let dispatcher = context.action_dispatcher();
        item = item.on_click(move |_event, _window, cx| {
            dispatcher(event.clone(), cx);
        });
    }
    Ok(context.style(item, &props))
}

struct PaginationComponent;

impl ComponentRenderer for PaginationComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        ensure_no_children(&props)?;
        let current_page =
            parse_positive_usize_attribute(&props.element, "current-page")?.unwrap_or(1);
        let total_pages =
            parse_positive_usize_attribute(&props.element, "total-pages")?.unwrap_or(1);
        let visible_pages = parse_positive_usize_attribute(&props.element, "visible-pages")?;
        if visible_pages.is_some_and(|value| value > MAX_VISIBLE_PAGES) {
            return Err(ComponentError::new(format!(
                "attribute `visible-pages` on <pagination> must not exceed \
                 {MAX_VISIBLE_PAGES}"
            )));
        }

        // total_pages intentionally follows current_page: the native builder
        // then clamps an out-of-range current page to the available range.
        let mut pagination = Pagination::new(props.stable_id())
            .current_page(current_page)
            .total_pages(total_pages)
            .disabled(bool_attribute(&props.element, "disabled")?)
            .on_click(selection_handler(&props, context));
        if let Some(visible_pages) = visible_pages {
            pagination = pagination.visible_pages(visible_pages);
        }
        if bool_attribute(&props.element, "compact")? {
            pagination = pagination.compact();
        }
        if let Some(size) = parse_size_attribute(&props.element)? {
            pagination = pagination.with_size(size);
        }
        Ok(context.style(pagination, &props).into_any_element())
    }
}

struct RatingComponent;

impl ComponentRenderer for RatingComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        ensure_no_children(&props)?;
        let value = parse_usize_attribute(&props.element, "value")?.unwrap_or(0);
        let max = parse_positive_usize_attribute(&props.element, "max")?.unwrap_or(5);
        if max > MAX_RATING_STARS {
            return Err(ComponentError::new(format!(
                "attribute `max` on <rating> must not exceed {MAX_RATING_STARS}, got `{max}`"
            )));
        }

        // max follows value so the native component clamps value to 0..=max.
        let mut rating = Rating::new(props.stable_id())
            .value(value)
            .max(max)
            .disabled(bool_attribute(&props.element, "disabled")?)
            .on_click(selection_handler(&props, context));
        if let Some(size) = parse_size_attribute(&props.element)? {
            rating = rating.with_size(size);
        }
        Ok(context.style(rating, &props).into_any_element())
    }
}

struct TabsComponent;

impl ComponentRenderer for TabsComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        let tabs = props
            .element
            .children
            .iter()
            .enumerate()
            .map(|(index, child)| {
                let child_props = structural_child_props(&props, index, child, "tab")?;
                build_tab(child_props, context)
            })
            .collect::<Result<Vec<_>, ComponentError>>()?;
        if tabs.is_empty() {
            return Err(ComponentError::new(
                "<tabs> requires at least one direct <tab> child",
            ));
        }

        let selected_index = parse_usize_attribute(&props.element, "selected-index")?.unwrap_or(0);
        validate_selected_index(&props, selected_index, tabs.len())?;

        let variant = match props
            .element
            .attr("variant")
            .unwrap_or("tab")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "tab" => TabVariant::Tab,
            "outline" => TabVariant::Outline,
            "pill" => TabVariant::Pill,
            "segmented" => TabVariant::Segmented,
            "underline" => TabVariant::Underline,
            value => {
                return Err(ComponentError::new(format!(
                    "attribute `variant` on <tabs> must be tab, outline, pill, segmented, or \
                     underline, got `{value}`"
                )));
            }
        };

        let mut tab_bar = TabBar::new(props.stable_id())
            .with_variant(variant)
            .menu(bool_attribute(&props.element, "menu")?)
            .selected_index(selected_index)
            .children(tabs)
            .on_click(selection_handler(&props, context));
        if let Some(size) = parse_size_attribute(&props.element)? {
            tab_bar = tab_bar.with_size(size);
        }
        Ok(context.style(tab_bar, &props).into_any_element())
    }
}

fn build_tab(
    props: ComponentProps,
    context: &mut RenderContext<'_>,
) -> Result<Tab, ComponentError> {
    let tab = Tab::new()
        .label(label_attribute_or_text(&props)?)
        .disabled(bool_attribute(&props.element, "disabled")?);
    Ok(context.style(tab, &props))
}

struct StepperComponent;

impl ComponentRenderer for StepperComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        let items = props
            .element
            .children
            .iter()
            .enumerate()
            .map(|(index, child)| {
                let child_props = structural_child_props(&props, index, child, "stepper-item")?;
                build_stepper_item(child_props, context)
            })
            .collect::<Result<Vec<_>, ComponentError>>()?;
        if items.is_empty() {
            return Err(ComponentError::new(
                "<stepper> requires at least one direct <stepper-item> child",
            ));
        }

        let selected_index = parse_usize_attribute(&props.element, "selected-index")?.unwrap_or(0);
        validate_selected_index(&props, selected_index, items.len())?;
        let layout = match props.element.attr("layout").unwrap_or("horizontal") {
            value if value.eq_ignore_ascii_case("horizontal") => Axis::Horizontal,
            value if value.eq_ignore_ascii_case("vertical") => Axis::Vertical,
            value => {
                return Err(ComponentError::new(format!(
                    "attribute `layout` on <stepper> must be horizontal or vertical, got \
                     `{value}`"
                )));
            }
        };

        let mut stepper = Stepper::new(props.stable_id())
            .selected_index(selected_index)
            .layout(layout)
            .text_center(bool_attribute(&props.element, "text-center")?)
            .disabled(bool_attribute(&props.element, "disabled")?)
            .items(items)
            .on_click(selection_handler(&props, context));
        if let Some(size) = parse_size_attribute(&props.element)? {
            stepper = stepper.with_size(size);
        }
        Ok(context.style(stepper, &props).into_any_element())
    }
}

fn build_stepper_item(
    props: ComponentProps,
    context: &mut RenderContext<'_>,
) -> Result<StepperItem, ComponentError> {
    let children = context.render_children(&props);
    let item = StepperItem::new()
        .disabled(bool_attribute(&props.element, "disabled")?)
        .children(children);
    Ok(context.style(item, &props))
}

struct StructuralNavigationComponent;

impl ComponentRenderer for StructuralNavigationComponent {
    fn render(&self, props: ComponentProps, _context: &mut RenderContext<'_>) -> ComponentResult {
        Err(ComponentError::new(format!(
            "<{}> must be rendered inside its structurally valid parent",
            props.element.tag
        )))
    }
}

fn selection_handler(
    props: &ComponentProps,
    context: &RenderContext<'_>,
) -> impl Fn(&usize, &mut Window, &mut App) + 'static {
    let binding = props.element.attr("bind").map(str::to_owned);
    let state_dispatcher = context.state_dispatcher();
    let event = props
        .element
        .attr("action")
        .map(|action| action_event(action, props));
    let action_dispatcher = context.action_dispatcher();
    move |value, _window, cx| {
        // Keep this ordering contractual: an Action handler observing the
        // binding must see the newly selected value.
        if let Some(binding) = &binding {
            state_dispatcher(binding.clone(), value.to_string(), cx);
        }
        if let Some(event) = &event {
            action_dispatcher(event.clone(), cx);
        }
    }
}

fn label_attribute_or_text(props: &ComponentProps) -> Result<String, ComponentError> {
    if let Some(label) = props.element.attr("label") {
        if !props.element.children.is_empty() {
            return Err(ComponentError::new(format!(
                "<{}> must use either `label` or direct text, not both",
                props.element.tag
            )));
        }
        if label.trim().is_empty() {
            return Err(ComponentError::new(format!(
                "attribute `label` on <{}> must not be empty",
                props.element.tag
            )));
        }
        return Ok(label.to_owned());
    }

    let mut parts = Vec::new();
    for child in &props.element.children {
        match child {
            VNode::Text(text) => parts.push(text.as_str()),
            VNode::Element(_) | VNode::Fragment(_) => {
                return Err(ComponentError::new(format!(
                    "<{}> only accepts direct text when `label` is not declared",
                    props.element.tag
                )));
            }
        }
    }
    let label = parts.join(" ");
    if label.is_empty() {
        return Err(ComponentError::new(format!(
            "<{}> requires `label` or direct text",
            props.element.tag
        )));
    }
    Ok(label)
}

fn validate_selected_index(
    props: &ComponentProps,
    selected_index: usize,
    item_count: usize,
) -> Result<(), ComponentError> {
    if selected_index < item_count {
        return Ok(());
    }
    Err(ComponentError::new(format!(
        "attribute `selected-index` on <{}> is out of range: got `{selected_index}`, but the \
         component has {item_count} items",
        props.element.tag
    )))
}

fn ensure_no_children(props: &ComponentProps) -> Result<(), ComponentError> {
    if props.element.children.is_empty() {
        return Ok(());
    }
    Err(ComponentError::new(format!(
        "<{}> does not accept children",
        props.element.tag
    )))
}

fn structural_child_props(
    parent: &ComponentProps,
    index: usize,
    child: &VNode,
    expected_tag: &str,
) -> Result<ComponentProps, ComponentError> {
    let Some(element) = child.element() else {
        return Err(ComponentError::new(format!(
            "<{}> only accepts direct <{expected_tag}> children",
            parent.element.tag
        )));
    };
    if !element.tag.eq_ignore_ascii_case(expected_tag) {
        return Err(ComponentError::new(format!(
            "<{}> only accepts direct <{expected_tag}> children, found <{}>",
            parent.element.tag, element.tag
        )));
    }
    Ok(ComponentProps::new(
        element.clone(),
        parent.path.child(index),
    ))
}
