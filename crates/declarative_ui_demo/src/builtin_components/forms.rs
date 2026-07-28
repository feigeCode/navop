use gpui::{App, IntoElement, ParentElement, Rems, Window, div, px};
use gpui_component::{
    Disableable, Sizable,
    checkbox::Checkbox,
    form::{Field, Form},
    radio::Radio,
    switch::Switch,
};

use crate::{
    ComponentError, ComponentProps, ComponentRegistry, ComponentRenderer, ComponentResult,
    ComponentSchema, RegistryError, RenderContext, VElement, VNode,
};

use super::{
    action_event, bool_attribute, bool_attribute_or, checked_attribute, parse_non_negative_f32,
    parse_positive_usize_attribute, parse_size_attribute,
};

pub(super) fn register(registry: &mut ComponentRegistry) -> Result<(), RegistryError> {
    registry.register_with_schema("form", form_schema(), FormComponent)?;
    registry.register_with_schema("field", field_schema(), StructuralFormComponent)?;
    registry.register_with_schema(
        "checkbox",
        toggle_schema(),
        ToggleComponent::new(ToggleKind::Checkbox),
    )?;
    registry.register_with_schema(
        "switch",
        toggle_schema(),
        ToggleComponent::new(ToggleKind::Switch),
    )?;
    registry.register_with_schema(
        "radio",
        toggle_schema(),
        ToggleComponent::new(ToggleKind::Radio),
    )?;
    Ok(())
}

fn form_schema() -> ComponentSchema {
    ComponentSchema::new()
        .attribute("layout")
        .attribute("columns")
        .attribute("label-width")
        .attribute("label-text-size")
        .attribute("size")
}

fn field_schema() -> ComponentSchema {
    ComponentSchema::new()
        .attribute("label")
        .attribute("description")
        .attribute("required")
        .attribute("visible")
        .attribute("label-indent")
        .attribute("col-span")
        .attribute("col-start")
        .attribute("col-end")
        .attribute("label-justify")
        .attribute("align")
}

fn toggle_schema() -> ComponentSchema {
    ComponentSchema::new()
        .attribute("bind")
        .attribute("checked")
        .attribute("disabled")
        .attribute("action")
        .attribute("size")
        .attribute("tooltip")
        .data_attributes()
}

struct FormComponent;

impl ComponentRenderer for FormComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        let mut form = match props.element.attr("layout").unwrap_or("vertical") {
            value if value.eq_ignore_ascii_case("vertical") => Form::vertical(),
            value if value.eq_ignore_ascii_case("horizontal") => Form::horizontal(),
            value => {
                return Err(ComponentError::new(format!(
                    "attribute `layout` on <form> must be vertical or horizontal, got `{value}`"
                )));
            }
        };

        if let Some(columns) = parse_positive_usize_attribute(&props.element, "columns")? {
            if columns > u16::MAX.into() {
                return Err(ComponentError::new(format!(
                    "attribute `columns` on <form> is too large: `{columns}`"
                )));
            }
            form = form.columns(columns);
        }
        if let Some(width) = props.element.attr("label-width") {
            let width = parse_non_negative_f32(&props.element, "label-width", width)?;
            form = form.label_width(px(width));
        }
        if let Some(size) = parse_label_text_size(&props.element)? {
            form = form.label_text_size(size);
        }
        if let Some(size) = parse_size_attribute(&props.element)? {
            form = form.with_size(size);
        }

        let fields = props
            .element
            .children
            .iter()
            .enumerate()
            .map(|(index, child)| {
                let child_props = structural_child_props(&props, index, child, "field")?;
                build_field(child_props, context)
            })
            .collect::<Result<Vec<_>, ComponentError>>()?;
        form = form.children(fields);

        // Form currently does not refine its own StyleRefinement during render,
        // so the declarative class contract is applied to a stable wrapper.
        Ok(context.style(div().child(form), &props).into_any_element())
    }
}

fn build_field(
    props: ComponentProps,
    context: &mut RenderContext<'_>,
) -> Result<Field, ComponentError> {
    let mut field = Field::new()
        .required(bool_attribute(&props.element, "required")?)
        .visible(bool_attribute_or(&props.element, "visible", true)?)
        .label_indent(bool_attribute_or(&props.element, "label-indent", true)?);

    if let Some(label) = props.element.attr("label") {
        field = field.label(label.to_owned());
    }
    if let Some(description) = props.element.attr("description") {
        field = field.description(description.to_owned());
    }
    if let Some(justify) = props.element.attr("label-justify") {
        field = match justify.trim().to_ascii_lowercase().as_str() {
            "start" => field.label_justify_start(),
            "center" => field.label_justify_center(),
            "end" => field.label_justify_end(),
            _ => {
                return Err(ComponentError::new(format!(
                    "attribute `label-justify` on <field> must be start, center, or end, got \
                     `{justify}`"
                )));
            }
        };
    }
    if let Some(span) = parse_positive_usize_attribute(&props.element, "col-span")? {
        let span = u16::try_from(span).map_err(|_| {
            ComponentError::new(format!(
                "attribute `col-span` on <field> is too large: `{span}`"
            ))
        })?;
        field = field.col_span(span);
    }
    if let Some(start) = parse_i16_attribute(&props.element, "col-start")? {
        field = field.col_start(start);
    }
    if let Some(end) = parse_i16_attribute(&props.element, "col-end")? {
        field = field.col_end(end);
    }
    if let Some(align) = props.element.attr("align") {
        field = match align.trim().to_ascii_lowercase().as_str() {
            "start" => field.items_start(),
            "center" => field.items_center(),
            "end" => field.items_end(),
            _ => {
                return Err(ComponentError::new(format!(
                    "attribute `align` on <field> must be start, center, or end, got `{align}`"
                )));
            }
        };
    }
    field = field.children(context.render_children(&props));
    Ok(context.style(field, &props))
}

fn parse_label_text_size(element: &VElement) -> Result<Option<Rems>, ComponentError> {
    let Some(value) = element.attr("label-text-size") else {
        return Ok(None);
    };
    let size = value
        .parse::<f32>()
        .map_err(|_| invalid_label_text_size(value))?;
    if !size.is_finite() || size <= 0.0 {
        return Err(invalid_label_text_size(value));
    }
    Ok(Some(Rems(size)))
}

fn invalid_label_text_size(value: &str) -> ComponentError {
    ComponentError::new(format!(
        "attribute `label-text-size` on <form> must be a finite positive rem value, got `{value}`"
    ))
}

fn parse_i16_attribute(element: &VElement, name: &str) -> Result<Option<i16>, ComponentError> {
    let Some(value) = element.attr(name) else {
        return Ok(None);
    };
    value.parse::<i16>().map(Some).map_err(|_| {
        ComponentError::new(format!(
            "attribute `{name}` on <{}> must be an integer from {} to {}, got `{value}`",
            element.tag,
            i16::MIN,
            i16::MAX
        ))
    })
}

struct StructuralFormComponent;

impl ComponentRenderer for StructuralFormComponent {
    fn render(&self, _props: ComponentProps, _context: &mut RenderContext<'_>) -> ComponentResult {
        Err(ComponentError::new(
            "<field> must be a direct child of <form>",
        ))
    }
}

#[derive(Clone, Copy)]
enum ToggleKind {
    Checkbox,
    Switch,
    Radio,
}

struct ToggleComponent {
    kind: ToggleKind,
}

impl ToggleComponent {
    fn new(kind: ToggleKind) -> Self {
        Self { kind }
    }
}

impl ComponentRenderer for ToggleComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        let checked = checked_attribute(&props)?;
        let disabled = bool_attribute(&props.element, "disabled")?;
        let label = props.element.text_content();
        let size = parse_size_attribute(&props.element)?;

        match self.kind {
            ToggleKind::Checkbox => {
                let mut control = Checkbox::new(props.stable_id())
                    .label(label)
                    .checked(checked)
                    .disabled(disabled)
                    .on_click(toggle_handler(&props, context));
                if let Some(size) = size {
                    control = control.with_size(size);
                }
                if let Some(tooltip) = props.element.attr("tooltip") {
                    control = control.tooltip(tooltip.to_owned());
                }
                Ok(context.style(control, &props).into_any_element())
            }
            ToggleKind::Switch => {
                let mut control = Switch::new(props.stable_id())
                    .label(label)
                    .checked(checked)
                    .disabled(disabled)
                    .on_click(toggle_handler(&props, context));
                if let Some(size) = size {
                    control = control.with_size(size);
                }
                if let Some(tooltip) = props.element.attr("tooltip") {
                    control = control.tooltip(tooltip.to_owned());
                }
                Ok(context.style(control, &props).into_any_element())
            }
            ToggleKind::Radio => {
                let mut control = Radio::new(props.stable_id())
                    .label(label)
                    .checked(checked)
                    .disabled(disabled)
                    .on_click(toggle_handler(&props, context));
                if let Some(size) = size {
                    control = control.with_size(size);
                }
                if let Some(tooltip) = props.element.attr("tooltip") {
                    control = control.tooltip(tooltip.to_owned());
                }
                Ok(context.style(control, &props).into_any_element())
            }
        }
    }
}

fn toggle_handler(
    props: &ComponentProps,
    context: &RenderContext<'_>,
) -> impl Fn(&bool, &mut Window, &mut App) + 'static {
    let binding = props.element.attr("bind").map(str::to_owned);
    let state_dispatcher = context.state_dispatcher();
    let event = props
        .element
        .attr("action")
        .map(|action| action_event(action, props));
    let action_dispatcher = context.action_dispatcher();
    move |checked, _window, cx| {
        if let Some(binding) = &binding {
            state_dispatcher(binding.clone(), checked.to_string(), cx);
        }
        if let Some(event) = &event {
            action_dispatcher(event.clone(), cx);
        }
    }
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
