//! Minimal preferences used by the embedded Markdown editor.

use anyhow::Context as _;
use gpui::{App, Global};

use super::VelotypeConfigDirs;

const DEFAULT_THEME_ID: &str = "velotype";
const DEFAULT_LANGUAGE_ID: &str = "en-US";

/// Where pasted clipboard images should be stored before inserting Markdown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImagePasteBehavior {
    None,
    CopyToDocumentFolder,
    CopyToAssetsFolder,
    CopyToNamedAssetsFolder,
}

impl ImagePasteBehavior {
    fn from_str(value: &str) -> Self {
        match value {
            "copy_to_document_folder" => Self::CopyToDocumentFolder,
            "copy_to_assets_folder" => Self::CopyToAssetsFolder,
            "copy_to_named_assets_folder" => Self::CopyToNamedAssetsFolder,
            _ => Self::None,
        }
    }
}

/// Preferences consumed by the embedded editor, language catalog, and theme catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AppPreferences {
    pub(crate) default_language_id: String,
    pub(crate) default_theme_id: String,
    pub(crate) show_table_headers: bool,
    pub(crate) image_paste_behavior: ImagePasteBehavior,
}

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            default_language_id: DEFAULT_LANGUAGE_ID.into(),
            default_theme_id: DEFAULT_THEME_ID.into(),
            show_table_headers: true,
            image_paste_behavior: ImagePasteBehavior::None,
        }
    }
}

/// Runtime settings needed on the editor render path.
pub struct EditorSettings {
    show_table_headers: bool,
}

impl Global for EditorSettings {}

impl EditorSettings {
    /// Whether table top rows are styled as headers. Defaults to `true` when
    /// the global has not been installed.
    pub fn show_table_headers(cx: &App) -> bool {
        cx.try_global::<Self>()
            .map(|settings| settings.show_table_headers)
            .unwrap_or(true)
    }

    /// Updates the runtime value and persists only this TOML key, preserving
    /// unrelated host/application preferences in the same file.
    pub fn set_show_table_headers(cx: &mut App, show_table_headers: bool) {
        cx.set_global(Self { show_table_headers });
        if let Err(err) = persist_show_table_headers(show_table_headers) {
            eprintln!("failed to save table header preference: {err}");
        }
    }
}

pub(crate) fn read_app_preferences() -> anyhow::Result<AppPreferences> {
    read_app_preferences_with_dirs(&VelotypeConfigDirs::from_system()?)
}

fn read_app_preferences_with_dirs(dirs: &VelotypeConfigDirs) -> anyhow::Result<AppPreferences> {
    let path = dirs.app_config_file();
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(AppPreferences::default());
        }
        Err(err) => {
            return Err(err).with_context(|| format!("failed to read '{}'", path.display()));
        }
    };
    let Ok(value) = toml::from_str::<toml::Value>(&text) else {
        return Ok(AppPreferences::default());
    };

    Ok(app_preferences_from_toml_value(&value))
}

fn app_preferences_from_toml_value(value: &toml::Value) -> AppPreferences {
    let default_language_id = value
        .get("language")
        .and_then(|language| language.get("default_language_id"))
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .unwrap_or(DEFAULT_LANGUAGE_ID)
        .to_string();
    let default_theme_id = value
        .get("theme")
        .and_then(|theme| theme.get("default_theme_id"))
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .unwrap_or(DEFAULT_THEME_ID)
        .to_string();
    let show_table_headers = value
        .get("editor")
        .and_then(|editor| editor.get("show_table_headers"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(true);
    let image_paste_behavior = value
        .get("editor")
        .and_then(|editor| editor.get("image_paste_behavior"))
        .and_then(toml::Value::as_str)
        .map(ImagePasteBehavior::from_str)
        .unwrap_or(ImagePasteBehavior::None);

    AppPreferences {
        default_language_id,
        default_theme_id,
        show_table_headers,
        image_paste_behavior,
    }
}

fn persist_show_table_headers(show_table_headers: bool) -> anyhow::Result<()> {
    let dirs = VelotypeConfigDirs::from_system()?;
    let path = dirs.app_config_file();
    let mut value = match std::fs::read_to_string(&path) {
        Ok(text) => toml::from_str::<toml::Value>(&text).unwrap_or_else(|_| empty_preferences()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => empty_preferences(),
        Err(err) => {
            return Err(err).with_context(|| format!("failed to read '{}'", path.display()));
        }
    };

    let root = value
        .as_table_mut()
        .expect("empty preferences and parsed TOML are tables");
    let editor = root
        .entry("editor")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    if !editor.is_table() {
        *editor = toml::Value::Table(toml::map::Map::new());
    }
    editor
        .as_table_mut()
        .expect("editor preference was normalized to a table")
        .insert(
            "show_table_headers".into(),
            toml::Value::Boolean(show_table_headers),
        );

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create '{}'", parent.display()))?;
    }
    let text = toml::to_string_pretty(&value)?;
    std::fs::write(&path, text).with_context(|| format!("failed to write '{}'", path.display()))
}

fn empty_preferences() -> toml::Value {
    toml::Value::Table(toml::map::Map::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_preferences_keep_embedded_defaults() {
        let value: toml::Value = toml::from_str(
            r#"
            [theme]
            default_theme_id = "velotype-light"

            [editor]
            show_table_headers = false
            image_paste_behavior = "copy_to_assets_folder"
            "#,
        )
        .unwrap();

        let preferences = app_preferences_from_toml_value(&value);
        assert_eq!(preferences.default_language_id, DEFAULT_LANGUAGE_ID);
        assert_eq!(preferences.default_theme_id, "velotype-light");
        assert!(!preferences.show_table_headers);
        assert_eq!(
            preferences.image_paste_behavior,
            ImagePasteBehavior::CopyToAssetsFolder
        );
    }
}
