use gpui::{IntoElement, ParentElement, div};
use gpui_component::{
    Sizable, alert::Alert, badge::Badge, progress::Progress, separator::Separator, spinner::Spinner,
};

use crate::{
    ComponentError, ComponentProps, ComponentRegistry, ComponentRenderer, ComponentResult,
    ComponentSchema, RegistryError, RenderContext, VElement,
};

use super::{bool_attribute, bool_attribute_or, parse_size_attribute, parse_usize_attribute};

pub(super) fn register(registry: &mut ComponentRegistry) -> Result<(), RegistryError> {
    registry.register_with_schema("alert", alert_schema(), AlertComponent)?;
    registry.register_with_schema("badge", badge_schema(), BadgeComponent)?;
    registry.register_with_schema("progress", progress_schema(), ProgressComponent)?;
    registry.register_with_schema(
        "spinner",
        ComponentSchema::new().attribute("size"),
        SpinnerComponent,
    )?;
    registry.register_with_schema("separator", rule_schema(), RuleComponent)?;
    // gpui-component has a Divider implementation internally, but its public
    // crate API does not export that module. Keep <divider> as a documented
    // semantic alias backed by the public Separator component.
    registry.register_with_schema("divider", rule_schema(), RuleComponent)?;
    Ok(())
}

fn alert_schema() -> ComponentSchema {
    ComponentSchema::new()
        .attribute("bind")
        .attribute("variant")
        .attribute("title")
        .attribute("banner")
        .attribute("visible")
        .attribute("size")
}

fn badge_schema() -> ComponentSchema {
    ComponentSchema::new()
        .attribute("bind")
        .attribute("count")
        .attribute("max")
        .attribute("dot")
        .attribute("size")
}

fn progress_schema() -> ComponentSchema {
    ComponentSchema::new()
        .attribute("bind")
        .attribute("value")
        .attribute("loading")
        .attribute("size")
}

fn rule_schema() -> ComponentSchema {
    ComponentSchema::new()
        .attribute("orientation")
        .attribute("dashed")
        .attribute("label")
}

struct AlertComponent;

impl ComponentRenderer for AlertComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        let id = props.stable_id();
        let message = props.element.text_content();
        let variant = props
            .element
            .attr("variant")
            .unwrap_or("default")
            .trim()
            .to_ascii_lowercase();
        let mut alert = match variant.as_str() {
            "default" => Alert::new(id, message),
            "info" => Alert::info(id, message),
            "success" => Alert::success(id, message),
            "warning" => Alert::warning(id, message),
            "error" | "danger" => Alert::error(id, message),
            _ => {
                return Err(ComponentError::new(format!(
                    "attribute `variant` on <alert> must be default, info, success, warning, \
                     error, or danger, got `{variant}`"
                )));
            }
        };
        if let Some(title) = props.element.attr("title") {
            alert = alert.title(title.to_owned());
        }
        if bool_attribute(&props.element, "banner")? {
            alert = alert.banner();
        }
        alert = alert.visible(bool_attribute_or(&props.element, "visible", true)?);
        if let Some(size) = parse_size_attribute(&props.element)? {
            alert = alert.with_size(size);
        }
        Ok(context.style(alert, &props).into_any_element())
    }
}

struct BadgeComponent;

impl ComponentRenderer for BadgeComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        let mut badge = Badge::new().children(context.render_children(&props));
        if let Some(count) = parse_usize_attribute(&props.element, "count")? {
            badge = badge.count(count);
        }
        if let Some(max) = parse_usize_attribute(&props.element, "max")? {
            badge = badge.max(max);
        }
        if bool_attribute(&props.element, "dot")? {
            badge = badge.dot();
        }
        if let Some(size) = parse_size_attribute(&props.element)? {
            badge = badge.with_size(size);
        }

        // Badge does not currently expose Styled, so classes refine a stable
        // wrapper while its native relative/overlay layout remains intact.
        Ok(context.style(div().child(badge), &props).into_any_element())
    }
}

struct ProgressComponent;

impl ComponentRenderer for ProgressComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        let mut progress =
            Progress::new(props.stable_id()).loading(bool_attribute(&props.element, "loading")?);
        if let Some(value) = props.element.attr("value") {
            progress = progress.value(parse_finite_f32(&props.element, "value", value)?);
        }
        if let Some(size) = parse_size_attribute(&props.element)? {
            progress = progress.with_size(size);
        }
        Ok(context.style(progress, &props).into_any_element())
    }
}

struct SpinnerComponent;

impl ComponentRenderer for SpinnerComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        let mut spinner = Spinner::new().animation_id(props.stable_id());
        if let Some(size) = parse_size_attribute(&props.element)? {
            spinner = spinner.with_size(size);
        }

        // Spinner has no Styled implementation; keep the declared classes on
        // a wrapper and give every instance a stable animation identifier.
        Ok(context
            .style(div().child(spinner), &props)
            .into_any_element())
    }
}

struct RuleComponent;

impl ComponentRenderer for RuleComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        let mut rule = match orientation(&props)? {
            Orientation::Horizontal => Separator::horizontal(),
            Orientation::Vertical => Separator::vertical(),
        };
        if bool_attribute(&props.element, "dashed")? {
            rule = rule.dashed();
        }
        if let Some(label) = props.element.attr("label") {
            rule = rule.label(label.to_owned());
        }
        Ok(context.style(rule, &props).into_any_element())
    }
}

#[derive(Clone, Copy)]
enum Orientation {
    Horizontal,
    Vertical,
}

fn orientation(props: &ComponentProps) -> Result<Orientation, ComponentError> {
    match props
        .element
        .attr("orientation")
        .unwrap_or("horizontal")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "horizontal" => Ok(Orientation::Horizontal),
        "vertical" => Ok(Orientation::Vertical),
        value => Err(ComponentError::new(format!(
            "attribute `orientation` on <{}> must be horizontal or vertical, got `{value}`",
            props.element.tag
        ))),
    }
}

fn parse_finite_f32(element: &VElement, name: &str, value: &str) -> Result<f32, ComponentError> {
    let parsed = value.parse::<f32>().map_err(|_| {
        ComponentError::new(format!(
            "attribute `{name}` on <{}> must be a finite number, got `{value}`",
            element.tag
        ))
    })?;
    if !parsed.is_finite() {
        return Err(ComponentError::new(format!(
            "attribute `{name}` on <{}> must be a finite number, got `{value}`",
            element.tag
        )));
    }
    Ok(parsed)
}
