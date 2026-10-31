use gpui::App;
use gpui_component::setting::{SettingField, SettingGroup, SettingItem};
use one_core::settings::AppSettings;
use rust_i18n::t;

/// 连接显示设置分组：首页是否显示「最近连接」分组。
pub fn connection_display_setting_group() -> SettingGroup {
    let default_settings = AppSettings::default();
    SettingGroup::new()
        .title(t!("Settings.General.ConnectionDisplay.group_title"))
        .item(
            SettingItem::new(
                t!("Settings.General.ConnectionDisplay.show_recent"),
                SettingField::switch(
                    |cx: &App| AppSettings::global(cx).home_show_recent_connections,
                    |val: bool, cx: &mut App| {
                        AppSettings::update_and_save(cx, |settings| {
                            settings.home_show_recent_connections = val;
                        });
                    },
                )
                .default_value(default_settings.home_show_recent_connections),
            )
            .description(t!("Settings.General.ConnectionDisplay.show_recent_desc").to_string()),
        )
}
