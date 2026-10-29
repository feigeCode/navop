//! 独立产品的设置分区：终端设置页。
//!
//! 独立产品没有主产品的设置系统，因此这里自建一页终端设置，直接读写
//! `one_core::settings::AppSettings`。终端视图通过 `observe_global_in::<AppSettings>`
//! 监听全局设置，保存后立即生效，不需要额外的应用钩子。

use domain_api::SettingsSectionProvider;

mod local_terminal;
mod local_terminal_profiles;
use gpui::{App, SharedString};
use gpui_component::setting::{
    NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage,
};
use one_core::settings::AppSettings;
use rust_i18n::t;
use terminal_view::{MAX_FONT_SIZE, MIN_FONT_SIZE, available_monospace_fonts};

/// 终端设置页在设置列表中的排序（排在数据领域之后）。
const TERMINAL_SETTINGS_ORDER: i32 = 20;

/// 终端设置分区的提供方。
pub(crate) struct ShellSettingsProvider;

impl SettingsSectionProvider for ShellSettingsProvider {
    fn pages(&self, cx: &mut App) -> Vec<(i32, SettingPage)> {
        vec![(TERMINAL_SETTINGS_ORDER, terminal_settings_page(cx))]
    }
}

/// 终端设置页：终端外观/行为 + 本地终端默认 shell 与自定义终端。
fn terminal_settings_page(cx: &mut App) -> SettingPage {
    let profile = AppSettings::global(cx).local_terminal_profile.clone();
    SettingPage::new(t!("Settings.terminal.title"))
        .default_open(true)
        .resettable(true)
        .groups([
            terminal_setting_group(),
            local_terminal::local_terminal_setting_group(&profile),
        ])
}

/// 终端设置分组：字体、字号、回滚行数与剪贴板/补全开关。
fn terminal_setting_group() -> SettingGroup {
    let defaults = AppSettings::default();
    SettingGroup::new()
        .title(t!("Settings.terminal.group_title"))
        .items(vec![
            font_family_item(&defaults),
            font_size_item(&defaults),
            scrollback_lines_item(&defaults),
            auto_copy_item(&defaults),
            autocomplete_item(&defaults),
        ])
}

/// 终端等宽字体下拉；选项来自终端视图导出的可用字体列表。
fn font_family_item(defaults: &AppSettings) -> SettingItem {
    let options = available_monospace_fonts()
        .into_iter()
        .map(|font| (SharedString::from(font), SharedString::from(font)))
        .collect();
    SettingItem::new(
        t!("Settings.terminal.font_family"),
        SettingField::scrollable_dropdown(
            options,
            |cx: &App| SharedString::from(AppSettings::global(cx).terminal_font_family.clone()),
            |value: SharedString, cx: &mut App| {
                AppSettings::update_and_save(cx, |settings| {
                    settings.terminal_font_family = value.to_string();
                });
            },
        )
        .default_value(SharedString::from(defaults.terminal_font_family.clone())),
    )
    .description(t!("Settings.terminal.font_family_description").to_string())
}

/// 终端字号；范围与终端视图的缩放边界保持一致。
fn font_size_item(defaults: &AppSettings) -> SettingItem {
    let options = NumberFieldOptions {
        min: MIN_FONT_SIZE as f64,
        max: MAX_FONT_SIZE as f64,
        ..Default::default()
    };
    SettingItem::new(
        t!("Settings.terminal.font_size"),
        SettingField::number_input(
            options,
            |cx: &App| AppSettings::global(cx).terminal_font_size,
            |value: f64, cx: &mut App| {
                AppSettings::update_and_save(cx, |settings| {
                    settings.terminal_font_size = value;
                });
            },
        )
        .default_value(defaults.terminal_font_size),
    )
    .description(t!("Settings.terminal.font_size_description").to_string())
}

/// 终端回滚行数；保存时由 `AppSettings` 归一到合法区间。
fn scrollback_lines_item(defaults: &AppSettings) -> SettingItem {
    let options = NumberFieldOptions {
        min: AppSettings::MIN_TERMINAL_SCROLLBACK_LINES as f64,
        max: AppSettings::MAX_TERMINAL_SCROLLBACK_LINES as f64,
        ..Default::default()
    };
    SettingItem::new(
        t!("Settings.terminal.scrollback_lines"),
        SettingField::number_input(
            options,
            |cx: &App| AppSettings::global(cx).terminal_scrollback_lines as f64,
            |value: f64, cx: &mut App| {
                AppSettings::update_and_save(cx, |settings| {
                    settings.terminal_scrollback_lines = value as usize;
                });
            },
        )
        .default_value(defaults.terminal_scrollback_lines as f64),
    )
    .description(t!("Settings.terminal.scrollback_lines_description").to_string())
}

/// 选中文本后自动复制。
fn auto_copy_item(defaults: &AppSettings) -> SettingItem {
    SettingItem::new(
        t!("Settings.terminal.auto_copy"),
        SettingField::switch(
            |cx: &App| AppSettings::global(cx).terminal_auto_copy,
            |value: bool, cx: &mut App| {
                AppSettings::update_and_save(cx, |settings| {
                    settings.terminal_auto_copy = value;
                });
            },
        )
        .default_value(defaults.terminal_auto_copy),
    )
    .description(t!("Settings.terminal.auto_copy_description").to_string())
}

/// 终端命令自动补全。
fn autocomplete_item(defaults: &AppSettings) -> SettingItem {
    SettingItem::new(
        t!("Settings.terminal.autocomplete"),
        SettingField::switch(
            |cx: &App| AppSettings::global(cx).terminal_enable_autocomplete,
            |value: bool, cx: &mut App| {
                AppSettings::update_and_save(cx, |settings| {
                    settings.terminal_enable_autocomplete = value;
                });
            },
        )
        .default_value(defaults.terminal_enable_autocomplete),
    )
    .description(t!("Settings.terminal.autocomplete_description").to_string())
}
