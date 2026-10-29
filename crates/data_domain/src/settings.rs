//! 数据领域的设置分区：数据库设置页。
//!
//! 独立产品没有 `main` 的设置系统，本模块自建一页数据库设置，直接读写
//! `one_core::settings::AppSettings`；文案硬编码中文（`data_domain` 不依赖 rust-i18n）。

use domain_api::SettingsSectionProvider;

mod sql_format;
use gpui::{App, SharedString};
use gpui_component::setting::{
    NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage,
};
use one_core::settings::{
    AppSettings, DatabaseOpenMode, LargeTextCellEditorOpenMode, TableViewMode,
};
use one_ui::{set_table_column_comment_in_header, set_table_row_height};

/// 数据设置页在设置列表中的排序（排在通用页之后）。
const DATABASE_SETTINGS_ORDER: i32 = 10;

/// 数据领域的设置分区提供方。
pub struct DataSettingsProvider;

impl SettingsSectionProvider for DataSettingsProvider {
    fn pages(&self, _cx: &mut App) -> Vec<(i32, SettingPage)> {
        vec![
            (DATABASE_SETTINGS_ORDER, database_settings_page()),
            (DATABASE_SETTINGS_ORDER + 1, sql_format_page()),
        ]
    }
}

/// SQL 格式化设置页：关键字大小写、缩进风格与预览。
fn sql_format_page() -> SettingPage {
    SettingPage::new("SQL 格式化")
        .resettable(true)
        .groups([sql_format::sql_format_setting_group()])
}

/// 数据库设置页。
fn database_settings_page() -> SettingPage {
    SettingPage::new("数据库")
        .resettable(true)
        .groups([database_setting_group()])
}

/// 数据库设置分组：打开方式、大文本编辑器、自动保存、查询上限与表格展示。
fn database_setting_group() -> SettingGroup {
    let defaults = AppSettings::default();
    SettingGroup::new().title("数据库连接与表格").items(vec![
        database_open_mode_item(&defaults),
        large_text_editor_item(&defaults),
        auto_save_switch_item(&defaults),
        auto_save_interval_item(&defaults),
        sql_query_max_rows_item(&defaults),
        table_row_height_item(&defaults),
        table_column_comment_item(&defaults),
        table_view_mode_item(&defaults),
    ])
}

/// 连接打开方式：单库 / 工作区分组。
fn database_open_mode_item(defaults: &AppSettings) -> SettingItem {
    SettingItem::new(
        "连接打开方式",
        SettingField::dropdown(
            vec![
                ("single".into(), "单库模式".into()),
                ("workspace".into(), "工作区模式".into()),
            ],
            |cx: &App| SharedString::from(AppSettings::global(cx).database_open_mode.as_str()),
            |val: SharedString, cx: &mut App| {
                AppSettings::update_and_save(cx, |settings| {
                    settings.database_open_mode = DatabaseOpenMode::from_str(&val);
                });
            },
        )
        .default_value(SharedString::from(defaults.database_open_mode.as_str())),
    )
    .description("单库模式每个数据库一个标签页；工作区模式把同一工作区的数据库放进同一标签页。")
}

/// 大文本单元格的打开方式：侧栏预览 / 弹窗。
fn large_text_editor_item(defaults: &AppSettings) -> SettingItem {
    SettingItem::new(
        "大文本单元格打开方式",
        SettingField::dropdown(
            vec![
                ("sidebar_preview".into(), "侧栏预览".into()),
                ("dialog".into(), "弹窗".into()),
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
            defaults.large_text_cell_editor_open_mode.as_str(),
        )),
    )
    .description("双击大文本单元格时的展示方式。")
}

/// SQL 自动保存开关。
fn auto_save_switch_item(defaults: &AppSettings) -> SettingItem {
    SettingItem::new(
        "自动保存 SQL",
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
        .default_value(defaults.enable_sql_auto_save),
    )
    .description("编辑 SQL 时按固定间隔自动保存草稿。")
}

/// 自动保存间隔（秒）。
fn auto_save_interval_item(defaults: &AppSettings) -> SettingItem {
    SettingItem::new(
        "自动保存间隔（秒）",
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
        .default_value(defaults.sql_auto_save_interval),
    )
    .description("开启自动保存后生效，范围 1–60 秒。")
}

/// 单次查询的最大返回行数。
fn sql_query_max_rows_item(defaults: &AppSettings) -> SettingItem {
    SettingItem::new(
        "查询最大返回行数",
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
        .default_value(defaults.sql_query_max_rows as f64),
    )
    .description("超过上限的查询结果会被截断。")
}

/// 数据表格行高。
fn table_row_height_item(defaults: &AppSettings) -> SettingItem {
    SettingItem::new(
        "表格行高",
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
                set_table_row_height(height, cx);
            },
        )
        .default_value(defaults.table_row_height as f64),
    )
    .description("结果表格每行的像素高度，范围 24–100。")
}

/// 表头是否展示列注释。
fn table_column_comment_item(defaults: &AppSettings) -> SettingItem {
    SettingItem::new(
        "表头显示列注释",
        SettingField::switch(
            |cx: &App| AppSettings::global(cx).table_show_column_comment,
            |val: bool, cx: &mut App| {
                AppSettings::update_and_save(cx, |settings| {
                    settings.table_show_column_comment = val;
                });
                set_table_column_comment_in_header(val, cx);
            },
        )
        .default_value(defaults.table_show_column_comment),
    )
    .description("在结果表格的列标题旁显示数据库里的列注释。")
}

/// 表格显示方式：网格 / 纵向。
fn table_view_mode_item(defaults: &AppSettings) -> SettingItem {
    SettingItem::new(
        "表格显示方式",
        SettingField::dropdown(
            vec![
                ("grid".into(), "网格".into()),
                ("vertical".into(), "纵向".into()),
            ],
            |cx: &App| SharedString::from(AppSettings::global(cx).table_view_mode.as_str()),
            |val: SharedString, cx: &mut App| {
                let mode = TableViewMode::from_str(val.as_ref());
                AppSettings::update_and_save(cx, |settings| {
                    settings.table_view_mode = mode;
                });
            },
        )
        .default_value(SharedString::from(defaults.table_view_mode.as_str())),
    )
    .description("网格按行列平铺；纵向按「列名：值」逐行展开，适合宽表。")
}
