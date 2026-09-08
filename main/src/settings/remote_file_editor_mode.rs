//! “默认打开方式”设置项：内置编辑器 / 外部编辑器。

use gpui::{App, SharedString};
use gpui_component::setting::{SettingField, SettingItem};
use one_core::settings::{AppSettings, RemoteFileEditorUserSettings, RemoteFileOpenMode};
use rust_i18n::t;

const OPEN_MODE_BUILT_IN: &str = "built_in";
const OPEN_MODE_EXTERNAL: &str = "external";

fn open_mode_as_value(mode: RemoteFileOpenMode) -> &'static str {
    match mode {
        RemoteFileOpenMode::BuiltIn => OPEN_MODE_BUILT_IN,
        RemoteFileOpenMode::External => OPEN_MODE_EXTERNAL,
    }
}

fn open_mode_from_value(value: &str) -> RemoteFileOpenMode {
    match value {
        OPEN_MODE_EXTERNAL => RemoteFileOpenMode::External,
        _ => RemoteFileOpenMode::BuiltIn,
    }
}

pub fn open_mode_item(default_settings: &RemoteFileEditorUserSettings) -> SettingItem {
    SettingItem::new(
        t!("Settings.General.RemoteFileEditor.open_mode"),
        SettingField::dropdown(
            vec![
                (
                    OPEN_MODE_BUILT_IN.into(),
                    t!("Settings.General.RemoteFileEditor.open_mode_built_in").into(),
                ),
                (
                    OPEN_MODE_EXTERNAL.into(),
                    t!("Settings.General.RemoteFileEditor.open_mode_external").into(),
                ),
            ],
            |cx: &App| {
                SharedString::from(open_mode_as_value(
                    AppSettings::global(cx).remote_file_editor.open_mode,
                ))
            },
            |value: SharedString, cx: &mut App| {
                let mode = open_mode_from_value(&value);
                AppSettings::update_and_save(cx, |settings| {
                    settings.remote_file_editor.open_mode = mode;
                });
            },
        )
        .default_value(SharedString::from(open_mode_as_value(
            default_settings.open_mode,
        ))),
    )
    .description(t!("Settings.General.RemoteFileEditor.open_mode_desc").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_mode_values_match_the_persisted_snake_case_values() {
        assert_eq!(
            OPEN_MODE_BUILT_IN,
            open_mode_as_value(RemoteFileOpenMode::BuiltIn)
        );
        assert_eq!(
            OPEN_MODE_EXTERNAL,
            open_mode_as_value(RemoteFileOpenMode::External)
        );
        assert_eq!(
            RemoteFileOpenMode::BuiltIn,
            open_mode_from_value(OPEN_MODE_BUILT_IN)
        );
        assert_eq!(
            RemoteFileOpenMode::External,
            open_mode_from_value(OPEN_MODE_EXTERNAL)
        );
    }

    #[test]
    fn unknown_open_mode_value_falls_back_to_built_in() {
        assert_eq!(RemoteFileOpenMode::BuiltIn, open_mode_from_value(""));
        assert_eq!(RemoteFileOpenMode::BuiltIn, open_mode_from_value("native"));
    }
}
