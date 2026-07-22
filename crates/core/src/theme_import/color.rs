use gpui_component::ThemeMode;
use serde_json::{Map, Value};

pub(super) fn mode_from_value(value: Option<&Value>, colors: &Map<String, Value>) -> ThemeMode {
    match value
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("light") => ThemeMode::Light,
        Some("dark") => ThemeMode::Dark,
        _ => mode_from_hex(
            first_color(
                colors,
                &["editor.background", "background", "primary.background"],
            )
            .as_deref(),
        ),
    }
}

pub(super) fn mode_from_hex(value: Option<&str>) -> ThemeMode {
    let Some(value) = value else {
        return ThemeMode::Dark;
    };
    let value = value.trim_start_matches('#');
    if value.len() < 6 {
        return ThemeMode::Dark;
    }
    let red = u8::from_str_radix(&value[0..2], 16).unwrap_or(0) as f32;
    let green = u8::from_str_radix(&value[2..4], 16).unwrap_or(0) as f32;
    let blue = u8::from_str_radix(&value[4..6], 16).unwrap_or(0) as f32;
    if (red * 0.299 + green * 0.587 + blue * 0.114) > 150.0 {
        ThemeMode::Light
    } else {
        ThemeMode::Dark
    }
}

pub(super) fn first_color(colors: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| colors.get(*key).and_then(color))
}

pub(super) fn object_color(object: &Map<String, Value>, key: &str) -> Option<String> {
    object.get(key).and_then(color)
}

pub(super) fn color(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|value| value.starts_with('#'))
        .map(str::to_string)
}

pub(super) fn set_color(target: &mut Option<gpui::SharedString>, value: Option<String>) {
    *target = value.map(Into::into);
}
