use crate::settings::AppSettings;
use crate::storage::get_config_dir;
use crate::theme_sources::BUNDLED_THEMES;
use gpui::{Action, App, SharedString, hsla};
use gpui_component::{
    ActiveTheme, Colorize, Theme, ThemeConfig, ThemeMode, ThemeRegistry, scroll::ScrollbarShow,
    try_parse_color,
};
use serde::{Deserialize, Serialize};
use std::rc::Rc;

const STATE_FILE: &str = "target/state.json";
const ACCENT_LIGHTNESS_THRESHOLD: f32 = 0.62;
const ACCENT_HOVER_AMOUNT: f32 = 0.1;
const ACCENT_ACTIVE_AMOUNT: f32 = 0.1;
const CUSTOM_SELECTION_ALPHA: f32 = 0.25;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct State {
    theme: SharedString,
    scrollbar_show: Option<ScrollbarShow>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            theme: "Default Light".into(),
            scrollbar_show: Some(ScrollbarShow::Hover),
        }
    }
}

pub fn load_bundled(cx: &mut App) {
    let registry = ThemeRegistry::global_mut(cx);
    for source in BUNDLED_THEMES {
        if let Err(error) = registry.load_themes_from_str(source) {
            tracing::warn!(%error, "Failed to load bundled theme");
        }
    }
}

pub fn load_imported(cx: &mut App) {
    let Ok(directory) = imported_themes_dir() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        if let Err(error) = ThemeRegistry::global_mut(cx).load_themes_from_str(&source) {
            tracing::warn!(%error, "Failed to load imported theme");
        }
    }
}

pub fn import_theme_files(paths: &[std::path::PathBuf], cx: &mut App) -> Result<usize, String> {
    let directory = imported_themes_dir().map_err(|error| error.to_string())?;
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let mut imported = 0;
    for path in paths {
        let source = std::fs::read_to_string(path)
            .map_err(|error| format!("{}: {}", path.display(), error))?;
        validate_theme_source(&source).map_err(|error| format!("{}: {}", path.display(), error))?;
        ThemeRegistry::global_mut(cx)
            .load_themes_from_str(&source)
            .map_err(|error| format!("{}: {}", path.display(), error))?;
        let file_stem = path
            .file_stem()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("无效主题文件名: {}", path.display()))?;
        std::fs::write(directory.join(format!("{file_stem}.json")), source)
            .map_err(|error| error.to_string())?;
        imported += 1;
    }
    apply_appearance(&AppSettings::current(cx), cx);
    Ok(imported)
}

fn imported_themes_dir() -> anyhow::Result<std::path::PathBuf> {
    Ok(get_config_dir()?.join("themes"))
}

fn validate_theme_source(source: &str) -> Result<(), String> {
    let set = serde_json::from_str::<gpui_component::ThemeSet>(source)
        .map_err(|error| error.to_string())?;
    if set.themes.is_empty() {
        return Err("主题文件中没有 themes 配置".to_string());
    }
    Ok(())
}

pub fn init(cx: &mut App) {
    let json = std::fs::read_to_string(STATE_FILE).unwrap_or_default();
    let state = serde_json::from_str::<State>(&json).unwrap_or_default();
    if let Some(scrollbar_show) = state.scrollbar_show {
        Theme::global_mut(cx).scrollbar_show = scrollbar_show;
    }
    cx.observe_global::<Theme>(|cx| {
        let state = State {
            theme: cx.theme().theme_name().clone(),
            scrollbar_show: Some(cx.theme().scrollbar_show),
        };
        if let Ok(json) = serde_json::to_string_pretty(&state) {
            let _ = std::fs::write(STATE_FILE, json);
        }
    })
    .detach();

    cx.on_action(|switch: &SwitchTheme, cx| {
        let Some(config) = ThemeRegistry::global(cx).themes().get(&switch.0).cloned() else {
            return;
        };
        AppSettings::update_and_save(cx, |settings| {
            if config.mode.is_dark() {
                settings.dark_theme = config.name.to_string();
            } else {
                settings.light_theme = config.name.to_string();
            }
        });
        apply_appearance(&AppSettings::current(cx), cx);
    });
    cx.on_action(|switch: &SwitchThemeMode, cx| {
        let mode = switch.0;
        AppSettings::update_and_save(cx, |settings| {
            settings.theme_mode = if mode.is_dark() {
                "dark".to_string()
            } else {
                "light".to_string()
            };
            settings.auto_switch_theme = false;
        });
        apply_appearance(&AppSettings::current(cx), cx);
    });
}

pub fn apply_appearance(settings: &AppSettings, cx: &mut App) {
    let system_mode = ThemeMode::from(cx.window_appearance());
    let mode = settings.effective_theme_mode(system_mode);
    let (light_theme, dark_theme) = resolve_theme_pair(settings, cx);
    {
        let theme = Theme::global_mut(cx);
        theme.light_theme = light_theme;
        theme.dark_theme = dark_theme;
    }
    Theme::change(mode, None, cx);
    apply_custom_accent(settings, cx);
    cx.refresh_windows();
}

fn resolve_theme_pair(settings: &AppSettings, cx: &App) -> (Rc<ThemeConfig>, Rc<ThemeConfig>) {
    let registry = ThemeRegistry::global(cx);
    let light = find_theme(&settings.light_theme, ThemeMode::Light, registry)
        .unwrap_or_else(|| registry.default_light_theme().clone());
    let dark = find_theme(&settings.dark_theme, ThemeMode::Dark, registry)
        .unwrap_or_else(|| registry.default_dark_theme().clone());
    (light, dark)
}

fn find_theme(name: &str, mode: ThemeMode, registry: &ThemeRegistry) -> Option<Rc<ThemeConfig>> {
    registry
        .themes()
        .get(name)
        .filter(|theme| theme.mode == mode)
        .cloned()
}

pub fn apply_custom_accent(settings: &AppSettings, cx: &mut App) {
    if !settings.custom_accent_enabled {
        return;
    }
    let Ok(accent) = try_parse_color(&settings.custom_accent_color) else {
        return;
    };
    let foreground = if accent.l > ACCENT_LIGHTNESS_THRESHOLD {
        hsla(0., 0., 0.08, 1.0)
    } else {
        hsla(0., 0., 1.0, 1.0)
    };
    let theme = Theme::global_mut(cx);
    theme.primary = accent;
    theme.primary_hover = accent.lighten(ACCENT_HOVER_AMOUNT);
    theme.primary_active = accent.darken(ACCENT_ACTIVE_AMOUNT);
    theme.primary_foreground = foreground;
    theme.button_primary = accent;
    theme.button_primary_hover = theme.primary_hover;
    theme.button_primary_active = theme.primary_active;
    theme.button_primary_foreground = foreground;
    theme.accent = accent;
    theme.accent_foreground = foreground;
    theme.ring = accent;
    theme.list_active_border = accent;
    theme.table_active_border = accent;
    theme.drag_border = accent;
    theme.sidebar_primary = accent;
    theme.sidebar_primary_foreground = foreground;
    theme.selection = accent.alpha(CUSTOM_SELECTION_ALPHA);
}

#[derive(Action, Clone, PartialEq)]
#[action(namespace = themes, no_json)]
pub struct SwitchTheme(pub SharedString);

#[derive(Action, Clone, PartialEq)]
#[action(namespace = themes, no_json)]
pub struct SwitchThemeMode(pub ThemeMode);

#[cfg(test)]
mod tests {
    use gpui_component::{Colorize, ThemeMode, ThemeSet};

    use super::{BUNDLED_THEMES, validate_theme_source};

    #[test]
    fn bundled_themes_include_light_and_dark_palettes() {
        let themes = BUNDLED_THEMES
            .iter()
            .flat_map(|source| serde_json::from_str::<ThemeSet>(source).unwrap().themes)
            .collect::<Vec<_>>();

        assert!(themes.iter().any(|theme| theme.name == "Ayu Light"));
        assert!(themes.iter().any(|theme| theme.name == "Tokyo Night"));
        assert!(themes.iter().any(|theme| theme.name == "Matrix"));
        assert!(themes.iter().any(|theme| theme.mode == ThemeMode::Light));
        assert!(themes.iter().any(|theme| theme.mode == ThemeMode::Dark));
    }

    #[test]
    fn imported_theme_sources_must_contain_theme_entries() {
        assert!(validate_theme_source(r#"{"themes":[]}"#).is_err());
        assert!(validate_theme_source(BUNDLED_THEMES[0]).is_ok());
    }

    #[gpui::test]
    fn custom_accent_updates_application_semantic_colors(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.set_global(gpui_component::Theme::default());
            let settings = crate::settings::AppSettings {
                custom_accent_enabled: true,
                custom_accent_color: "#ff0000".to_string(),
                ..Default::default()
            };

            super::apply_custom_accent(&settings, cx);

            assert_eq!(
                "#FF0000",
                gpui_component::Theme::global(cx).primary.to_hex()
            );
            assert_eq!(
                gpui_component::Theme::global(cx).primary,
                gpui_component::Theme::global(cx).ring
            );
            assert_eq!(
                gpui_component::Theme::global(cx).primary,
                gpui_component::Theme::global(cx).sidebar_primary
            );
        });
    }

    #[gpui::test]
    fn unknown_saved_theme_names_fall_back_to_defaults(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            super::load_bundled(cx);
            let settings = crate::settings::AppSettings {
                light_theme: "Missing Light".to_string(),
                dark_theme: "Missing Dark".to_string(),
                ..Default::default()
            };

            super::apply_appearance(&settings, cx);

            assert_eq!(
                gpui_component::Theme::global(cx).light_theme.name,
                "Default Light"
            );
            assert_eq!(
                gpui_component::Theme::global(cx).dark_theme.name,
                "Default Dark"
            );
            assert!(
                gpui_component::ThemeRegistry::global(cx)
                    .themes()
                    .contains_key("Tokyo Night")
            );
        });
    }
}
