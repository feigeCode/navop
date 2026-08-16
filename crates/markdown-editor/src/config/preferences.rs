//! Minimal preferences used by the embedded Markdown editor.

use anyhow::Context as _;
use gpui::{App, Global};

use super::VelotypeConfigDirs;

const DEFAULT_THEME_ID: &str = "velotype";

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

/// Preferences consumed by the embedded editor and theme catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AppPreferences {
    pub(crate) default_theme_id: String,
    pub(crate) show_table_headers: bool,
    pub(crate) image_paste_behavior: ImagePasteBehavior,
}

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
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
        default_theme_id,
        show_table_headers,
        image_paste_behavior,
    }
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
        assert_eq!(preferences.default_theme_id, "velotype-light");
        assert!(!preferences.show_table_headers);
        assert_eq!(
            preferences.image_paste_behavior,
            ImagePasteBehavior::CopyToAssetsFolder
        );
    }
}
