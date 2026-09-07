use gpui::{App, SharedString};
use gpui_component::setting::{NumberFieldOptions, SettingField, SettingGroup, SettingItem};
use one_core::settings::{AppSettings, DatabaseOpenMode, LargeTextCellEditorOpenMode};
use rust_i18n::t;

/// 数据库设置分组：打开模式、大文本编辑器打开方式、自动保存、查询上限与行高。
pub fn database_setting_group() -> SettingGroup {
    let default_settings = AppSettings::default();
    SettingGroup::new()
        .title(t!("Settings.General.Database.group_title"))
        .items(vec![
            SettingItem::new(
                t!("Settings.General.Database.open_mode"),
                SettingField::dropdown(
                    vec![
                        (
                            "single".into(),
                            t!("Settings.General.Database.open_mode_single").into(),
                        ),
                        (
                            "workspace".into(),
                            t!("Settings.General.Database.open_mode_workspace").into(),
                        ),
                    ],
                    |cx: &App| {
                        SharedString::from(
                            AppSettings::global(cx).database_open_mode.as_str(),
                        )
                    },
                    |val: SharedString, cx: &mut App| {
                        AppSettings::update_and_save(cx, |settings| {
                            settings.database_open_mode = DatabaseOpenMode::from_str(&val);
                        });
                    },
                )
                .default_value(SharedString::from(
                    default_settings.database_open_mode.as_str(),
                )),
            )
            .description(t!("Settings.General.Database.open_mode_desc").to_string()),
            SettingItem::new(
                t!("Settings.General.Database.large_text_editor_open_mode"),
                SettingField::dropdown(
                    vec![
                        (
                            "sidebar_preview".into(),
                            t!(
                                "Settings.General.Database.large_text_editor_open_mode_sidebar"
                            )
                            .into(),
                        ),
                        (
                            "dialog".into(),
                            t!(
                                "Settings.General.Database.large_text_editor_open_mode_dialog"
                            )
                            .into(),
                        ),
                    ],
                    |cx: &App| {
                        SharedString::from(
                            AppSettings::global(cx)
                                .large_text_cell_editor_open_mode
                                .as_str(),
                        )
                    },
                    |val: SharedString, cx: &mut App| {
                        let mode = LargeTextCellEditorOpenMode::from_str(val.as_ref());
                        AppSettings::update_and_save(cx, |settings| {
                            settings.large_text_cell_editor_open_mode = mode;
                        });
                    },
                )
                .default_value(SharedString::from(
                    default_settings
                        .large_text_cell_editor_open_mode
                        .as_str(),
                )),
            )
            .description(
                t!("Settings.General.Database.large_text_editor_open_mode_desc").to_string(),
            ),
            SettingItem::new(
                t!("Settings.General.Database.auto_save"),
                SettingField::switch(
                    |cx: &App| AppSettings::global(cx).enable_sql_auto_save,
                    |val: bool, cx: &mut App| {
                        let interval = AppSettings::global(cx).sql_auto_save_interval;
                        AppSettings::update_and_save(cx, |settings| {
                            settings.enable_sql_auto_save = val;
                        });
                        AppSettings::update_auto_save_config(val, interval, cx);
                    },
                )
                .default_value(default_settings.enable_sql_auto_save),
            )
            .description(t!("Settings.General.Database.auto_save_desc").to_string()),
            SettingItem::new(
                t!("Settings.General.Database.auto_save_interval"),
                SettingField::number_input(
                    NumberFieldOptions {
                        min: 1.0,
                        max: 60.0,
                        step: 1.0,
                    },
                    |cx: &App| AppSettings::global(cx).sql_auto_save_interval,
                    |val: f64, cx: &mut App| {
                        let enabled = AppSettings::global(cx).enable_sql_auto_save;
                        AppSettings::update_and_save(cx, |settings| {
                            settings.sql_auto_save_interval = val;
                        });
                        AppSettings::update_auto_save_config(enabled, val, cx);
                    },
                )
                .default_value(default_settings.sql_auto_save_interval),
            )
            .description(t!("Settings.General.Database.auto_save_interval_desc").to_string()),
            SettingItem::new(
                t!("Settings.General.Database.sql_query_max_rows"),
                SettingField::number_input(
                    NumberFieldOptions {
                        min: 0.0,
                        max: 1_000_000.0,
                        step: 100.0,
                    },
                    |cx: &App| AppSettings::global(cx).sql_query_max_rows as f64,
                    |val: f64, cx: &mut App| {
                        AppSettings::update_and_save(cx, |settings| {
                            settings.sql_query_max_rows = val as u32;
                        });
                    },
                )
                .default_value(default_settings.sql_query_max_rows as f64),
            )
            .description(t!("Settings.General.Database.sql_query_max_rows_desc").to_string()),
            SettingItem::new(
                t!("Settings.General.Database.table_row_height"),
                SettingField::number_input(
                    NumberFieldOptions {
                        min: 24.0,
                        max: 100.0,
                        step: 2.0,
                    },
                    |cx: &App| AppSettings::global(cx).table_row_height as f64,
                    |val: f64, cx: &mut App| {
                        let height = val as u32;
                        AppSettings::update_and_save(cx, |settings| {
                            settings.table_row_height = height;
                        });
                        one_ui::set_table_row_height(height, cx);
                    },
                )
                .default_value(default_settings.table_row_height as f64),
            )
            .description(t!("Settings.General.Database.table_row_height_desc").to_string()),
        ])
}
