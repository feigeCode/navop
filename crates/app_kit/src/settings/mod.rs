//! 独立产品的设置页。
//!
//! 页面由通用分区（外观 / 快捷键 / Public MCP / 关于）与各领域通过
//! [`domain_api::SettingsSectionProvider`] 提供的分区组成，
//! 统一交给 `gpui_component` 的 [`Settings`] 容器渲染。

mod shortcuts;

use gpui::{
    App, AppContext as _, Context, FocusHandle, Focusable, IntoElement, ParentElement, Render,
    SharedString, Styled, Window, px,
};
use gpui_component::{
    ActiveTheme as _, ThemeMode, ThemeRegistry,
    button::Button,
    setting::{NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings},
    v_flex,
};
use one_core::popup_window::{PopupWindowOptions, open_reusable_popup_window};
use one_core::settings::{AppSettings, McpPermissionMode, effective_locale_for_setting};
use one_core::themes;

const DIALOG_WIDTH: f32 = 960.0;
const DIALOG_HEIGHT: f32 = 680.0;
const SIDEBAR_WIDTH: f32 = 180.0;
const SETTINGS_REUSE_KEY: &str = "app-kit-settings";
const FONT_SIZE_RANGE: (f64, f64) = (8.0, 32.0);
const UI_SCALE_RANGE: (f64, f64) = (80.0, 200.0);
const APPROVAL_TIMEOUT_RANGE: (f64, f64) = (5_000.0, 600_000.0);

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
        let mut pages = vec![general_page(cx), shortcuts::shortcuts_page(cx)];
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
    let mut groups = vec![appearance_group(cx), mcp_group(cx), security_group()];
    if let Some(group) = migration_group() {
        groups.push(group);
    }
    groups.push(about_group());
    SettingPage::new("通用").groups(groups)
}

/// 迁移：从全量产品导入连接。
///
/// 只对独立产品展示：全量产品自己就是来源，没有可导入的对象。默认按全量产品自己
/// 的目录规则自动找库，便携版才需要手动选文件。导入走同一份迁移包规划逻辑
/// （判重、重命名、工作区匹配），导入后通过连接事件刷新列表。
fn migration_group() -> Option<SettingGroup> {
    let identity = crate::identity()?;
    if identity.app_id == domain_api::AppIdentity::FULL.app_id {
        return None;
    }
    Some(SettingGroup::new().title("迁移").items([button_item(
        "import-from-full-product",
        "从全量产品导入连接",
        "默认自动找到全量产品的 one-hub.db；便携版可手动选择，也可以改用迁移包文件",
        |window, cx| {
            crate::import::open_import_dialog(window, cx);
        },
    )]))
}

/// 安全与凭据：主密钥修改/重置与凭据库入口。
///
/// 独立产品没有账户与云同步，但主密钥、凭据库都是纯本地能力，必须能用；
/// 云端相关动作由 `VaultHostHooks` 注入，未注入时自然降级为 no-op。
fn security_group() -> SettingGroup {
    SettingGroup::new().title("安全与凭据").items([
        button_item(
            "open-credential-vault",
            "打开凭据库",
            "管理已保存的凭据；锁定状态下只能看摘要",
            |window, cx| {
                crate::vault::open_credential_vault(window, cx);
            },
        ),
        button_item(
            "change-master-key",
            "修改主密钥",
            "需要用旧主密钥解锁一次",
            |window, cx| {
                crate::vault::dialogs::show_change_master_key_dialog(window, cx, None);
            },
        ),
        button_item(
            "reset-master-key",
            "重置主密钥",
            "会清空本地已加密数据，需输入 RESET 确认",
            |window, cx| {
                crate::vault::dialogs::show_reset_master_key_dialog(window, cx, None);
            },
        ),
    ])
}

/// 一行「说明 + 按钮」的设置项。
fn button_item(
    id: &'static str,
    label: &'static str,
    description: &'static str,
    on_click: fn(&mut Window, &mut App),
) -> SettingItem {
    SettingItem::render(move |_options, _window, _cx| {
        Button::new(id).label(label).on_click(move |_, window, cx| {
            on_click(window, cx);
        })
    })
    .description(description)
}

fn appearance_group(cx: &App) -> SettingGroup {
    SettingGroup::new().title("外观").items([
        SettingItem::new(
            "主题模式",
            SettingField::dropdown(theme_options(), current_theme_mode, set_theme_mode),
        )
        .description("立即应用到当前界面"),
        SettingItem::new(
            "浅色主题",
            SettingField::scrollable_dropdown(
                theme_names(cx, ThemeMode::Light),
                |cx| SharedString::from(AppSettings::global(cx).light_theme.clone()),
                set_light_theme,
            ),
        )
        .description("跟随系统或浅色模式时使用"),
        SettingItem::new(
            "深色主题",
            SettingField::scrollable_dropdown(
                theme_names(cx, ThemeMode::Dark),
                |cx| SharedString::from(AppSettings::global(cx).dark_theme.clone()),
                set_dark_theme,
            ),
        )
        .description("跟随系统或深色模式时使用"),
        SettingItem::new(
            "界面语言",
            SettingField::dropdown(locale_options(), current_locale, set_locale),
        )
        .description("部分领域文案仍为中文"),
        SettingItem::new(
            "界面字体大小",
            SettingField::number_input(
                NumberFieldOptions {
                    min: FONT_SIZE_RANGE.0,
                    max: FONT_SIZE_RANGE.1,
                    step: 1.0,
                },
                |cx| AppSettings::global(cx).font_size,
                set_font_size,
            ),
        )
        .description("重新打开窗口后完全生效"),
        SettingItem::new(
            "界面缩放（%）",
            SettingField::number_input(
                NumberFieldOptions {
                    min: UI_SCALE_RANGE.0,
                    max: UI_SCALE_RANGE.1,
                    step: 5.0,
                },
                |cx| f64::from(AppSettings::global(cx).ui_scale_percent),
                set_ui_scale,
            ),
        )
        .description("整套界面的缩放比例"),
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

/// 主题名列表：按当前模式过滤，附带默认主题置顶。
fn theme_names(cx: &App, mode: ThemeMode) -> Vec<(SharedString, SharedString)> {
    ThemeRegistry::global(cx)
        .sorted_themes()
        .into_iter()
        .filter(|theme| theme.mode == mode)
        .map(|theme| (theme.name.clone(), theme.name.clone()))
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

fn set_light_theme(value: SharedString, cx: &mut App) {
    let name = value.to_string();
    if AppSettings::global(cx).light_theme == name {
        return;
    }
    AppSettings::update_and_save(cx, |settings| settings.light_theme = name);
    themes::apply_appearance(&AppSettings::current(cx), cx);
}

fn set_dark_theme(value: SharedString, cx: &mut App) {
    let name = value.to_string();
    if AppSettings::global(cx).dark_theme == name {
        return;
    }
    AppSettings::update_and_save(cx, |settings| settings.dark_theme = name);
    themes::apply_appearance(&AppSettings::current(cx), cx);
}

fn set_font_size(value: f64, cx: &mut App) {
    if AppSettings::global(cx).font_size == value {
        return;
    }
    AppSettings::update_and_save(cx, |settings| settings.font_size = value);
    themes::apply_appearance(&AppSettings::current(cx), cx);
}

fn set_ui_scale(value: f64, cx: &mut App) {
    let percent = value.clamp(UI_SCALE_RANGE.0, UI_SCALE_RANGE.1) as u32;
    if AppSettings::global(cx).ui_scale_percent == percent {
        return;
    }
    AppSettings::update_and_save(cx, |settings| settings.ui_scale_percent = percent);
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
        SettingItem::new(
            "审批超时（毫秒）",
            SettingField::number_input(
                NumberFieldOptions {
                    min: APPROVAL_TIMEOUT_RANGE.0,
                    max: APPROVAL_TIMEOUT_RANGE.1,
                    step: 1_000.0,
                },
                |cx| AppSettings::global(cx).mcp.approval_timeout_ms as f64,
                set_approval_timeout,
            ),
        )
        .description("超过该时间未处理的审批请求会自动拒绝"),
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

fn set_approval_timeout(value: f64, cx: &mut App) {
    let timeout = value.clamp(APPROVAL_TIMEOUT_RANGE.0, APPROVAL_TIMEOUT_RANGE.1) as u64;
    if AppSettings::global(cx).mcp.approval_timeout_ms == timeout {
        return;
    }
    AppSettings::update_and_save(cx, |settings| settings.mcp.approval_timeout_ms = timeout);
}

fn about_group() -> SettingGroup {
    SettingGroup::new()
        .title("关于")
        .item(SettingItem::render(|_options, _window, _cx| {
            let name = crate::identity()
                .map(|identity| identity.display_name)
                .unwrap_or("Navop");
            let app_id = crate::identity()
                .map(|identity| identity.app_id)
                .unwrap_or("navop");
            let data_dir = one_core::app_paths::initialized_paths()
                .and_then(|paths| paths.data_dir())
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "未初始化".to_string());
            v_flex()
                .gap_1()
                .text_sm()
                .child(format!("产品：{name}（{app_id}）"))
                .child(format!("数据目录：{data_dir}"))
                .child("独立产品与全量产品数据隔离，可分别安装、升级。")
        }))
}
