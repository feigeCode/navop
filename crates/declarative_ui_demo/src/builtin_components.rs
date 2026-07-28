mod basic;
mod controls;
mod display;
mod feedback;
mod forms;
mod layout;
mod lists;
mod navigation;
mod scroll;
mod tables;

use gpui_component::Size;

use crate::{
    ActionEvent, ComponentError, ComponentProps, ComponentRegistry, RegistryError, VElement,
};

pub(crate) fn register_default_components(
    registry: &mut ComponentRegistry,
) -> Result<(), RegistryError> {
    basic::register(registry)?;
    forms::register(registry)?;
    tables::register(registry)?;
    lists::register(registry)?;
    layout::register(registry)?;
    feedback::register(registry)?;
    display::register(registry)?;
    navigation::register(registry)?;
    controls::register(registry)?;
    scroll::register(registry)?;
    Ok(())
}

pub(super) fn action_event(action: &str, props: &ComponentProps) -> ActionEvent {
    let payload = props
        .element
        .attrs
        .iter()
        .filter_map(|(name, value)| {
            name.strip_prefix("data-")
                .map(|name| (name.to_owned(), value.clone()))
        })
        .collect();
    ActionEvent::new(action, props.stable_id(), props.path.clone()).with_payload(payload)
}

pub(super) fn bool_attribute(element: &VElement, name: &str) -> Result<bool, ComponentError> {
    bool_attribute_or(element, name, false)
}

pub(super) fn bool_attribute_or(
    element: &VElement,
    name: &str,
    default: bool,
) -> Result<bool, ComponentError> {
    let Some(value) = element.attr(name) else {
        return Ok(default);
    };
    parse_bool(value, true).ok_or_else(|| {
        ComponentError::new(format!(
            "attribute `{name}` on <{}> must be a boolean, got `{value}`",
            element.tag
        ))
    })
}

pub(super) fn checked_attribute(props: &ComponentProps) -> Result<bool, ComponentError> {
    bound_bool_attribute_or(props, "checked", false)
}

pub(super) fn bound_bool_attribute_or(
    props: &ComponentProps,
    name: &str,
    default: bool,
) -> Result<bool, ComponentError> {
    let Some(value) = props.element.attr(name) else {
        return Ok(default);
    };
    let empty_is_true = props.element.attr("bind").is_none();
    parse_bool(value, empty_is_true).ok_or_else(|| {
        ComponentError::new(format!(
            "attribute `{name}` on <{}> must be a boolean, got `{value}`",
            props.element.tag
        ))
    })
}

pub(super) fn parse_size_attribute(element: &VElement) -> Result<Option<Size>, ComponentError> {
    let Some(value) = element.attr("size") else {
        return Ok(None);
    };
    let size = match value.trim().to_ascii_lowercase().as_str() {
        "xs" | "xsmall" => Size::XSmall,
        "sm" | "small" => Size::Small,
        "md" | "medium" => Size::Medium,
        "lg" | "large" => Size::Large,
        _ => {
            return Err(ComponentError::new(format!(
                "attribute `size` on <{}> must be one of xs, sm, md, or lg, got `{value}`",
                element.tag
            )));
        }
    };
    Ok(Some(size))
}

pub(super) fn parse_usize_attribute(
    element: &VElement,
    name: &str,
) -> Result<Option<usize>, ComponentError> {
    let Some(value) = element.attr(name) else {
        return Ok(None);
    };
    value.parse::<usize>().map(Some).map_err(|_| {
        ComponentError::new(format!(
            "attribute `{name}` on <{}> must be a non-negative integer, got `{value}`",
            element.tag
        ))
    })
}

pub(super) fn parse_positive_usize_attribute(
    element: &VElement,
    name: &str,
) -> Result<Option<usize>, ComponentError> {
    let value = parse_usize_attribute(element, name)?;
    if value == Some(0) {
        return Err(ComponentError::new(format!(
            "attribute `{name}` on <{}> must be greater than zero",
            element.tag
        )));
    }
    Ok(value)
}

pub(super) fn parse_non_negative_f32(
    element: &VElement,
    name: &str,
    value: &str,
) -> Result<f32, ComponentError> {
    let parsed = value.parse::<f32>().map_err(|_| {
        ComponentError::new(format!(
            "attribute `{name}` on <{}> must be a finite number, got `{value}`",
            element.tag
        ))
    })?;
    if !parsed.is_finite() || parsed < 0.0 {
        return Err(ComponentError::new(format!(
            "attribute `{name}` on <{}> must be a finite non-negative number, got `{value}`",
            element.tag
        )));
    }
    Ok(parsed)
}

fn parse_bool(value: &str, empty_is_true: bool) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" => Some(empty_is_true),
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}
