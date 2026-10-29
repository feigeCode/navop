//! 独立产品的基础设置页。
//!
//! 页面由通用分区（外观 / MCP / 关于）与各领域通过
//! [`domain_api::SettingsSectionProvider`] 提供的分区组成，
//! 统一交给 `gpui_component` 的 [`Settings`] 容器渲染。

use gpui::{
    App, AppContext as _, Context, FocusHandle, Focusable, IntoElement, ParentElement, Render,
    SharedString, Styled, Window, px,
};
use gpui_component::{
    ActiveTheme as _,
    setting::{SettingField, SettingGroup, SettingItem, SettingPage, Settings},
    v_flex,
};
use one_core::popup_window::{PopupWindowOptions, open_reusable_popup_window};
use one_core::settings::{AppSettings, McpPermissionMode, effective_locale_for_setting};
use one_core::themes;

const DIALOG_WIDTH: f32 = 960.0;
const DIALOG_HEIGHT: f32 = 680.0;
const SIDEBAR_WIDTH: f32 = 180.0;
const SETTINGS_REUSE_KEY: &str = "app-kit-settings";

/// 打开设置页。
pub fn open_settings(window: &mut Window, cx: &mut App) {
    open_reusable_popup_window(
        PopupWindowOptions::new("设置").size(DIALOG_WIDTH, DIALOG_HEIGHT),
        SETTINGS_REUSE_KEY,
        |window, cx| cx.new(|cx| SettingsDialog::new(window, cx)),
        Some(window),
        cx,
    );
}

/// 设置弹窗：通用页在前，领域页按 order 追加。
pub struct SettingsDialog {
    focus_handle: FocusHandle,
    pages: Vec<SettingPage>,
}

impl SettingsDialog {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        let mut pages = vec![general_page(cx)];
        pages.extend(crate::registry().settings_pages(cx));
        Self {
            focus_handle,
            pages,
        }
    }
}

impl Focusable for SettingsDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SettingsDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().size_full().bg(cx.theme().background).child(
            Settings::new("app-kit-settings")
                .pages(self.pages.clone())
                .sidebar_width(px(SIDEBAR_WIDTH)),
        )
    }
}

fn general_page(cx: &App) -> SettingPage {
    SettingPage::new("通用").groups([appearance_group(), mcp_group(cx), about_group()])
}

fn appearance_group() -> SettingGroup {
    SettingGroup::new().title("外观").items([
        SettingItem::new(
            "主题模式",
            SettingField::dropdown(theme_options(), current_theme_mode, set_theme_mode),
        )
        .description("立即应用到当前界面"),
        SettingItem::new(
            "界面语言",
            SettingField::dropdown(locale_options(), current_locale, set_locale),
        )
        .description("部分领域文案仍为中文"),
    ])
}

fn theme_options() -> Vec<(SharedString, SharedString)> {
    [("system", "跟随系统"), ("light", "浅色"), ("dark", "深色")]
        .into_iter()
        .map(|(value, label)| {
            (
                SharedString::new_static(value),
                SharedString::new_static(label),
            )
        })
        .collect()
}

fn locale_options() -> Vec<(SharedString, SharedString)> {
    [
        ("system", "跟随系统"),
        ("zh-CN", "简体中文"),
        ("zh-HK", "繁體中文"),
        ("en", "English"),
    ]
    .into_iter()
    .map(|(value, label)| {
        (
            SharedString::new_static(value),
            SharedString::new_static(label),
        )
    })
    .collect()
}

fn current_theme_mode(cx: &App) -> SharedString {
    AppSettings::global(cx).theme_mode.clone().into()
}

fn current_locale(cx: &App) -> SharedString {
    AppSettings::global(cx).locale.clone().into()
}

fn set_theme_mode(value: SharedString, cx: &mut App) {
    if AppSettings::global(cx).theme_mode == value.as_ref() {
        return;
    }
    let mode = value.to_string();
    AppSettings::update_and_save(cx, |settings| {
        settings.auto_switch_theme = mode == "system";
        settings.theme_mode = mode;
    });
    themes::apply_appearance(&AppSettings::current(cx), cx);
}

fn set_locale(value: SharedString, cx: &mut App) {
    let locale = value.to_string();
    if AppSettings::global(cx).locale == locale {
        return;
    }
    AppSettings::update_and_save(cx, |settings| settings.locale = locale);
    let settings = AppSettings::current(cx);
    gpui_component::set_locale(effective_locale_for_setting(&settings.locale));
    themes::apply_appearance(&settings, cx);
}

fn mcp_group(cx: &App) -> SettingGroup {
    let mode = AppSettings::global(cx).mcp.permission_mode.clone();
    SettingGroup::new().title("Public MCP").items([
        SettingItem::new(
            "允许其它客户端调用",
            SettingField::switch(mcp_host::settings_enabled, set_mcp_enabled),
        )
        .description("开启后，外部 AI 客户端可通过 Public MCP 使用本产品的工具集"),
        SettingItem::new(
            "权限模式",
            SettingField::dropdown(
                permission_options(),
                move |_cx| SharedString::from(mode.as_str()),
                set_permission_mode,
            ),
        )
        .description("决定工具调用的审批策略，修改后立即重启 MCP 服务"),
    ])
}

fn permission_options() -> Vec<(SharedString, SharedString)> {
    [("deny", "拒绝"), ("ask", "每次询问"), ("allow", "直接允许")]
        .into_iter()
        .map(|(value, label)| {
            (
                SharedString::new_static(value),
                SharedString::new_static(label),
            )
        })
        .collect()
}

fn set_mcp_enabled(enabled: bool, cx: &mut App) {
    let _ = mcp_host::set_enabled(cx, enabled);
}

fn set_permission_mode(value: SharedString, cx: &mut App) {
    let mode = McpPermissionMode::from_str(value.as_ref());
    if AppSettings::global(cx).mcp.permission_mode == mode {
        return;
    }
    AppSettings::update_and_save(cx, |settings| settings.mcp.permission_mode = mode);
    if !mcp_host::is_running(cx) {
        return;
    }
    mcp_host::stop(cx);
    if let Some(identity) = crate::identity() {
        let _ = mcp_host::start(cx, identity);
    }
}

fn about_group() -> SettingGroup {
    SettingGroup::new()
        .title("关于")
        .item(SettingItem::render(|_options, _window, _cx| {
            let name = crate::identity()
                .map(|identity| identity.display_name)
                .unwrap_or("Navop");
            let data_dir = one_core::app_paths::initialized_paths()
                .and_then(|paths| paths.data_dir())
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "未初始化".to_string());
            v_flex()
                .gap_1()
                .text_sm()
                .child(format!("产品：{name}"))
                .child(format!("数据目录：{data_dir}"))
                .child("独立产品与全量产品数据隔离，可分别安装、升级。")
        }))
}
