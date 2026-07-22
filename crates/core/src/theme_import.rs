use gpui_component::{ThemeConfig, ThemeConfigColors, ThemeMode, ThemeSet};
use serde_json::Value;
use std::{io::Cursor, path::Path};

mod color;
use color::{color, first_color, mode_from_hex, mode_from_value, object_color, set_color};

const DEFAULT_IMPORT_NAME: &str = "Imported Theme";

pub fn normalize_theme_source(path: &Path, source: &str) -> Result<String, String> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    let set = match extension.as_str() {
        "itermcolors" | "plist" => parse_iterm(source, path)?,
        "toml" => parse_alacritty(toml::from_str(source).map_err(|error| error.to_string())?)?,
        "yaml" | "yml" => {
            let value: serde_yaml::Value =
                serde_yaml::from_str(source).map_err(|error| error.to_string())?;
            parse_structured(
                serde_json::to_value(value).map_err(|error| error.to_string())?,
                path,
            )?
        }
        _ => parse_structured(
            serde_json::from_str(source).map_err(|error| error.to_string())?,
            path,
        )?,
    };

    if set.themes.is_empty() {
        return Err("主题文件中没有可用主题".to_string());
    }
    serde_json::to_string_pretty(&set).map_err(|error| error.to_string())
}

fn parse_structured(value: Value, path: &Path) -> Result<ThemeSet, String> {
    if let Ok(set) = serde_json::from_value::<ThemeSet>(value.clone()) {
        if !set.themes.is_empty() {
            return Ok(set);
        }
    }

    if let Some(themes) = value.as_array() {
        let mut set = ThemeSet {
            name: import_name(path).into(),
            ..ThemeSet::default()
        };
        for (index, theme) in themes.iter().enumerate() {
            set.themes.push(parse_vscode_theme(theme, path, index + 1)?);
        }
        return Ok(set);
    }

    let alacritty_colors = value
        .get("colors")
        .and_then(Value::as_object)
        .is_some_and(|colors| colors.get("primary").is_some());
    if value.get("primary").is_some() || alacritty_colors {
        return parse_alacritty(value);
    }

    if value.get("tokenColors").is_some() || value.get("colors").is_some() {
        return Ok(single_theme_set(parse_vscode_theme(&value, path, 1)?));
    }

    Err("无法识别主题格式；支持 GPUI、VS Code、Alacritty YAML/TOML 和 iTerm2".to_string())
}

fn parse_vscode_theme(value: &Value, path: &Path, index: usize) -> Result<ThemeConfig, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "VS Code 主题条目必须是对象".to_string())?;
    let colors = object
        .get("colors")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut config = theme_config(
        object
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| {
                if index == 1 {
                    import_name(path)
                } else {
                    format!("{} {index}", import_name(path))
                }
            }),
        mode_from_value(object.get("type"), &colors),
    );
    let token_colors = object
        .get("tokenColors")
        .and_then(Value::as_array)
        .and_then(|tokens| {
            tokens.iter().find_map(|token| {
                token
                    .get("settings")
                    .filter(|settings| settings.get("foreground").is_some())
            })
        });
    let foreground = first_color(
        &colors,
        &[
            "editor.foreground",
            "foreground",
            "textPreformat.foreground",
        ],
    )
    .or_else(|| token_colors.and_then(|settings| settings.get("foreground").and_then(color)));
    let background = first_color(
        &colors,
        &[
            "editor.background",
            "sideBar.background",
            "panel.background",
        ],
    )
    .or_else(|| token_colors.and_then(|settings| settings.get("background").and_then(color)));

    set_color(&mut config.colors.background, background);
    set_color(&mut config.colors.foreground, foreground);
    set_color(
        &mut config.colors.primary,
        first_color(
            &colors,
            &[
                "button.background",
                "textLink.foreground",
                "focusBorder",
                "editorCursor.foreground",
                "activityBarBadge.background",
            ],
        ),
    );
    set_color(
        &mut config.colors.primary_foreground,
        first_color(
            &colors,
            &["button.foreground", "activityBarBadge.foreground"],
        ),
    );
    set_color(
        &mut config.colors.border,
        first_color(
            &colors,
            &["contrastBorder", "panel.border", "sideBar.border"],
        ),
    );
    set_color(
        &mut config.colors.selection,
        first_color(
            &colors,
            &[
                "editor.selectionBackground",
                "editor.inactiveSelectionBackground",
            ],
        ),
    );
    set_color(
        &mut config.colors.sidebar,
        first_color(&colors, &["sideBar.background", "activityBar.background"]),
    );
    set_color(
        &mut config.colors.sidebar_foreground,
        first_color(&colors, &["sideBar.foreground", "activityBar.foreground"]),
    );
    set_color(
        &mut config.colors.secondary,
        first_color(
            &colors,
            &[
                "panel.background",
                "editorWidget.background",
                "input.background",
            ],
        ),
    );
    set_color(
        &mut config.colors.title_bar,
        first_color(
            &colors,
            &["titleBar.activeBackground", "titleBar.inactiveBackground"],
        ),
    );
    set_color(
        &mut config.colors.ring,
        first_color(&colors, &["focusBorder", "editorCursor.foreground"]),
    );
    Ok(config)
}

fn parse_alacritty(value: Value) -> Result<ThemeSet, String> {
    let colors = value
        .get("colors")
        .unwrap_or(&value)
        .as_object()
        .ok_or_else(|| "Alacritty colors 必须是对象".to_string())?;
    let primary = colors
        .get("primary")
        .and_then(Value::as_object)
        .unwrap_or(colors);
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_IMPORT_NAME);
    let mode = mode_from_value(value.get("type"), primary);
    let mut config = theme_config(name.to_string(), mode);
    set_color(
        &mut config.colors.background,
        object_color(primary, "background"),
    );
    set_color(
        &mut config.colors.foreground,
        object_color(primary, "foreground"),
    );
    set_color(
        &mut config.colors.primary,
        object_color(
            colors
                .get("cursor")
                .and_then(Value::as_object)
                .unwrap_or(primary),
            "cursor",
        )
        .or_else(|| object_color(primary, "foreground")),
    );
    set_color(
        &mut config.colors.selection,
        colors
            .get("selection")
            .and_then(Value::as_object)
            .and_then(|selection| object_color(selection, "background")),
    );
    Ok(single_theme_set(config))
}

fn parse_iterm(source: &str, path: &Path) -> Result<ThemeSet, String> {
    let value = plist::Value::from_reader(Cursor::new(source.as_bytes()))
        .map_err(|error| error.to_string())?;
    let dict = value
        .as_dictionary()
        .ok_or_else(|| "iTerm2 主题必须是 plist 字典".to_string())?;
    let background = plist_color(dict, "Background Color");
    let foreground = plist_color(dict, "Foreground Color");
    let cursor = plist_color(dict, "Cursor Color").or_else(|| foreground.clone());
    let selection = plist_color(dict, "Selection Color");
    let mut config = theme_config(import_name(path), mode_from_hex(background.as_deref()));
    set_color(&mut config.colors.background, background);
    set_color(&mut config.colors.foreground, foreground);
    set_color(&mut config.colors.primary, cursor);
    set_color(&mut config.colors.selection, selection);
    Ok(single_theme_set(config))
}

fn theme_config(name: String, mode: ThemeMode) -> ThemeConfig {
    ThemeConfig {
        name: name.into(),
        mode,
        colors: ThemeConfigColors::default(),
        ..ThemeConfig::default()
    }
}

fn single_theme_set(theme: ThemeConfig) -> ThemeSet {
    ThemeSet {
        name: theme.name.clone(),
        themes: vec![theme],
        ..ThemeSet::default()
    }
}

fn import_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.trim().is_empty())
        .unwrap_or(DEFAULT_IMPORT_NAME)
        .replace(['_', '-'], " ")
}

fn plist_color(dict: &plist::Dictionary, key: &str) -> Option<String> {
    let color = dict.get(key)?.as_dictionary()?;
    let component = |name: &str| {
        color
            .get(name)
            .and_then(|value| {
                value
                    .as_real()
                    .or_else(|| value.as_signed_integer().map(|value| value as f64))
            })
            .map(|value| (value.clamp(0.0, 1.0) * 255.0).round() as u8)
    };
    Some(format!(
        "#{:02X}{:02X}{:02X}",
        component("Red Component")?,
        component("Green Component")?,
        component("Blue Component")?
    ))
}

#[cfg(test)]
#[path = "theme_import/tests.rs"]
mod tests;
