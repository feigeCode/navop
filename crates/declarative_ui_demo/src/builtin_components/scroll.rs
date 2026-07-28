use gpui::{
    Div, InteractiveElement, IntoElement, ParentElement, ScrollHandle, Stateful,
    StatefulInteractiveElement, Styled, div, px,
};
use gpui_component::scroll::{Scrollbar, ScrollbarAxis, ScrollbarShow};

use crate::{
    ComponentError, ComponentProps, ComponentRegistry, ComponentRenderer, ComponentResult,
    ComponentSchema, RegistryError, RenderContext, VElement,
};

const DEFAULT_AXIS: ScrollbarAxis = ScrollbarAxis::Vertical;
const DEFAULT_SCROLLBAR_SHOW: ScrollbarShow = ScrollbarShow::Scrolling;

pub(super) fn register(registry: &mut ComponentRegistry) -> Result<(), RegistryError> {
    registry.register_with_schema("scroll", scroll_schema(), ScrollComponent)
}

fn scroll_schema() -> ComponentSchema {
    ComponentSchema::new()
        .required_attribute("id")
        .attribute("axis")
        .attribute("scrollbar-show")
        .attribute("width")
        .attribute("height")
}

struct ScrollComponent;

impl ComponentRenderer for ScrollComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        let axis = parse_axis(&props.element)?;
        let show = parse_scrollbar_show(&props.element)?;
        let width = parse_dimension(&props.element, "width")?;
        let height = parse_dimension(&props.element, "height")?;
        let stable_id = props.stable_id();
        let scroll_handle = context.scroll_handle(&props);
        let content = div()
            .children(context.render_children(&props))
            .size_auto()
            .flex_1();
        let viewport = scroll_viewport(&stable_id, content, &scroll_handle, axis);
        let scrollbar = scrollbar_layer(&stable_id, &scroll_handle, axis, show);
        let wrapper = div()
            .id(stable_id)
            .size_full()
            .relative()
            .child(viewport)
            .child(scrollbar);
        let mut wrapper = context.style(wrapper, &props);
        if let Some(width) = width {
            wrapper = wrapper.w(px(width));
        }
        if let Some(height) = height {
            wrapper = wrapper.h(px(height));
        }
        Ok(wrapper.into_any_element())
    }
}

fn scroll_viewport(
    stable_id: &str,
    content: Div,
    scroll_handle: &ScrollHandle,
    axis: ScrollbarAxis,
) -> Stateful<Div> {
    let viewport = div()
        .id(format!("{stable_id}:viewport"))
        .flex()
        .size_full()
        .track_scroll(scroll_handle);
    let viewport = match axis {
        ScrollbarAxis::Vertical => viewport.flex_col().overflow_y_scroll(),
        ScrollbarAxis::Horizontal => viewport.flex_row().overflow_x_scroll(),
        ScrollbarAxis::Both => viewport.overflow_scroll(),
    };
    viewport.child(content)
}

fn scrollbar_layer(
    stable_id: &str,
    scroll_handle: &ScrollHandle,
    axis: ScrollbarAxis,
    show: ScrollbarShow,
) -> Div {
    div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom_0()
        .child(
            Scrollbar::new(scroll_handle)
                .id(format!("{stable_id}:scrollbar"))
                .axis(axis)
                .scrollbar_show(show),
        )
}

fn parse_axis(element: &VElement) -> Result<ScrollbarAxis, ComponentError> {
    let Some(value) = element.attr("axis") else {
        return Ok(DEFAULT_AXIS);
    };
    match value.trim().to_ascii_lowercase().as_str() {
        "vertical" => Ok(ScrollbarAxis::Vertical),
        "horizontal" => Ok(ScrollbarAxis::Horizontal),
        "both" => Ok(ScrollbarAxis::Both),
        _ => Err(ComponentError::new(format!(
            "attribute `axis` on <scroll> must be vertical, horizontal, or both, got `{value}`"
        ))),
    }
}

fn parse_scrollbar_show(element: &VElement) -> Result<ScrollbarShow, ComponentError> {
    let Some(value) = element.attr("scrollbar-show") else {
        return Ok(DEFAULT_SCROLLBAR_SHOW);
    };
    match value.trim().to_ascii_lowercase().as_str() {
        "scrolling" => Ok(ScrollbarShow::Scrolling),
        "hover" => Ok(ScrollbarShow::Hover),
        "always" => Ok(ScrollbarShow::Always),
        _ => Err(ComponentError::new(format!(
            "attribute `scrollbar-show` on <scroll> must be scrolling, hover, or always, got \
             `{value}`"
        ))),
    }
}

fn parse_dimension(element: &VElement, name: &str) -> Result<Option<f32>, ComponentError> {
    let Some(value) = element.attr(name) else {
        return Ok(None);
    };
    let dimension = value
        .parse::<f32>()
        .map_err(|_| dimension_error(name, value))?;
    if !dimension.is_finite() || dimension <= 0.0 {
        return Err(dimension_error(name, value));
    }
    Ok(Some(dimension))
}

fn dimension_error(name: &str, value: &str) -> ComponentError {
    ComponentError::new(format!(
        "attribute `{name}` on <scroll> must be a finite positive pixel value, got `{value}`"
    ))
}
