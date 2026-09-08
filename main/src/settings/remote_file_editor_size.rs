//! 远程文件编辑器的最大可打开文件大小（MiB）设置项。

use gpui::App;
use gpui_component::setting::{NumberFieldOptions, SettingField, SettingItem};
use one_core::settings::{AppSettings, RemoteFileEditorUserSettings};
use rust_i18n::t;

/// 内置编辑器可编辑文件大小的最小上限（MiB）。
const MIN_FILE_SIZE_MIB: u32 = RemoteFileEditorUserSettings::MIN_FILE_SIZE_MIB;
/// 内置编辑器可编辑文件大小的最大上限（MiB）。
const MAX_FILE_SIZE_MIB: u32 = RemoteFileEditorUserSettings::MAX_FILE_SIZE_MIB;

fn normalize_max_file_size_mib(value: f64) -> u32 {
    (value.round() as u32).clamp(MIN_FILE_SIZE_MIB, MAX_FILE_SIZE_MIB)
}

pub fn max_file_size_item(default_settings: &RemoteFileEditorUserSettings) -> SettingItem {
    SettingItem::new(
        t!("Settings.General.RemoteFileEditor.max_file_size_mib"),
        SettingField::number_input(
            NumberFieldOptions {
                min: MIN_FILE_SIZE_MIB as f64,
                max: MAX_FILE_SIZE_MIB as f64,
                step: 1.0,
            },
            |cx: &App| AppSettings::global(cx).remote_file_editor.max_file_size_mib as f64,
            |value: f64, cx: &mut App| {
                let size = normalize_max_file_size_mib(value);
                AppSettings::update_and_save(cx, |settings| {
                    settings.remote_file_editor.max_file_size_mib = size;
                });
            },
        )
        .default_value(default_settings.max_file_size_mib as f64),
    )
    .description(t!("Settings.General.RemoteFileEditor.max_file_size_mib_desc").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_file_size_values_are_clamped_to_the_named_bounds() {
        assert_eq!(MIN_FILE_SIZE_MIB, normalize_max_file_size_mib(0.0));
        assert_eq!(MIN_FILE_SIZE_MIB, normalize_max_file_size_mib(-4.0));
        assert_eq!(10, normalize_max_file_size_mib(10.0));
        assert_eq!(MAX_FILE_SIZE_MIB, normalize_max_file_size_mib(2000.0));
        assert_eq!(MAX_FILE_SIZE_MIB, normalize_max_file_size_mib(1024.0));
    }
}
