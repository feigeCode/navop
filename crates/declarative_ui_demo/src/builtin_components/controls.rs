use gpui::{IntoElement, Keystroke};
use gpui_component::{
    kbd::Kbd,
    slider::{Slider, SliderScale},
};

use crate::{
    ComponentError, ComponentProps, ComponentRegistry, ComponentRenderer, ComponentResult,
    ComponentSchema, RegistryError, RenderContext,
    slider_cache::{SliderCallbacks, SliderConfig, SliderRequest},
};

use super::{action_event, bool_attribute, bool_attribute_or};

const DEFAULT_SLIDER_MIN: f32 = 0.0;
const DEFAULT_SLIDER_MAX: f32 = 100.0;
const DEFAULT_SLIDER_STEP: f32 = 1.0;

pub(super) fn register(registry: &mut ComponentRegistry) -> Result<(), RegistryError> {
    registry.register_with_schema("kbd", kbd_schema(), KbdComponent)?;
    registry.register_with_schema("slider", slider_schema(), SliderComponent)?;
    Ok(())
}

fn kbd_schema() -> ComponentSchema {
    ComponentSchema::new()
        .required_attribute("stroke")
        .attribute("appearance")
        .attribute("outline")
}

fn slider_schema() -> ComponentSchema {
    ComponentSchema::new()
        .attribute("bind")
        .attribute("value")
        .attribute("min")
        .attribute("max")
        .attribute("step")
        .attribute("scale")
        .attribute("orientation")
        .attribute("disabled")
        .attribute("action")
        .data_attributes()
}

struct KbdComponent;

impl ComponentRenderer for KbdComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        ensure_no_children(&props)?;
        let source = props.element.attr("stroke").unwrap_or_default().trim();
        if source.is_empty() {
            return Err(ComponentError::new(
                "attribute `stroke` on <kbd> must not be empty",
            ));
        }
        let stroke = Keystroke::parse(source).map_err(|_| {
            ComponentError::new(format!(
                "attribute `stroke` on <kbd> must be a valid GPUI keystroke, got `{source}`"
            ))
        })?;
        let mut kbd =
            Kbd::new(stroke).appearance(bool_attribute_or(&props.element, "appearance", true)?);
        if bool_attribute(&props.element, "outline")? {
            kbd = kbd.outline();
        }
        Ok(context.style(kbd, &props).into_any_element())
    }
}

struct SliderComponent;

impl ComponentRenderer for SliderComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        ensure_no_children(&props)?;
        let declaration = SliderDeclaration::parse(&props)?;
        let callbacks = SliderCallbacks::new(
            props.element.attr("bind").map(str::to_owned),
            props
                .element
                .attr("action")
                .map(|action| action_event(action, &props)),
        );
        let request = SliderRequest::new(props.stable_id(), declaration.config, callbacks);
        let state = context.slider_state(request);
        let mut slider = Slider::new(&state).disabled(bool_attribute(&props.element, "disabled")?);
        slider = match declaration.orientation {
            SliderOrientation::Horizontal => slider.horizontal(),
            SliderOrientation::Vertical => slider.vertical(),
        };
        Ok(context.style(slider, &props).into_any_element())
    }
}

struct SliderDeclaration {
    config: SliderConfig,
    orientation: SliderOrientation,
}

impl SliderDeclaration {
    fn parse(props: &ComponentProps) -> Result<Self, ComponentError> {
        let min = parse_f32_attribute(props, "min")?.unwrap_or(DEFAULT_SLIDER_MIN);
        let max = parse_f32_attribute(props, "max")?.unwrap_or(DEFAULT_SLIDER_MAX);
        let step = parse_f32_attribute(props, "step")?.unwrap_or(DEFAULT_SLIDER_STEP);
        let scale = parse_scale(props)?;
        let value = parse_f32_attribute(props, "value")?.unwrap_or(min);
        let config = SliderConfig {
            min,
            max,
            step,
            value,
            scale,
        };
        validate_slider_range(config)?;

        Ok(Self {
            config,
            orientation: parse_orientation(props)?,
        })
    }
}

#[derive(Clone, Copy)]
enum SliderOrientation {
    Horizontal,
    Vertical,
}

fn parse_f32_attribute(props: &ComponentProps, name: &str) -> Result<Option<f32>, ComponentError> {
    let Some(source) = props.element.attr(name) else {
        return Ok(None);
    };
    let value = source.parse::<f32>().map_err(|_| {
        ComponentError::new(format!(
            "attribute `{name}` on <slider> must be a finite number, got `{source}`"
        ))
    })?;
    if !value.is_finite() {
        return Err(ComponentError::new(format!(
            "attribute `{name}` on <slider> must be a finite number, got `{source}`"
        )));
    }
    Ok(Some(value))
}

fn parse_scale(props: &ComponentProps) -> Result<SliderScale, ComponentError> {
    match props
        .element
        .attr("scale")
        .unwrap_or("linear")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "linear" => Ok(SliderScale::Linear),
        "logarithmic" | "log" => Ok(SliderScale::Logarithmic),
        value => Err(ComponentError::new(format!(
            "attribute `scale` on <slider> must be linear or logarithmic, got `{value}`"
        ))),
    }
}

fn parse_orientation(props: &ComponentProps) -> Result<SliderOrientation, ComponentError> {
    match props
        .element
        .attr("orientation")
        .unwrap_or("horizontal")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "horizontal" => Ok(SliderOrientation::Horizontal),
        "vertical" => Ok(SliderOrientation::Vertical),
        value => Err(ComponentError::new(format!(
            "attribute `orientation` on <slider> must be horizontal or vertical, got `{value}`"
        ))),
    }
}

fn validate_slider_range(config: SliderConfig) -> Result<(), ComponentError> {
    if config.min >= config.max {
        return Err(ComponentError::new(format!(
            "attribute `min` on <slider> must be less than `max`, got `{}` and `{}`",
            config.min, config.max
        )));
    }
    if config.step <= 0.0 {
        return Err(ComponentError::new(format!(
            "attribute `step` on <slider> must be greater than zero, got `{}`",
            config.step
        )));
    }
    if config.scale == SliderScale::Logarithmic && config.min <= 0.0 {
        return Err(ComponentError::new(format!(
            "attribute `min` on a logarithmic <slider> must be greater than zero, got `{}`",
            config.min
        )));
    }
    if !(config.min..=config.max).contains(&config.value) {
        return Err(ComponentError::new(format!(
            "attribute `value` on <slider> must be within [{}, {}], got `{}`",
            config.min, config.max, config.value
        )));
    }
    Ok(())
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
