use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use gpui::http_client::{AsyncBody, Method, Request, Url};
use gpui::prelude::FluentBuilder;
use gpui::{
    div, App, AppContext, AsyncApp, ClickEvent, Context, Entity, EventEmitter, FocusHandle,
    Focusable, FontWeight, InteractiveElement, IntoElement, Keystroke, ParentElement,
    PathPromptOptions, Render, SharedString, Styled, WeakEntity, Window,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    clipboard::Clipboard,
    group_box::GroupBoxVariant,
    h_flex,
    input::{Input, InputState},
    kbd::Kbd,
    scroll::ScrollableElement,
    select::{Select, SelectItem, SelectState},
    setting::{NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings},
    switch::Switch,
    v_flex, ActiveTheme, Disableable, Icon, IconName, IndexPath, Sizable, Size, Theme, ThemeMode,
    TitleBar, WindowExt,
};
use one_core::cloud_sync::GlobalCloudUser;
use one_core::cloud_sync::UserInfo;
use one_core::gpui_tokio::Tokio;
use one_core::llm::manager::GlobalProviderState;
use one_core::popup_window::{open_popup_window, PopupWindowOptions};
use one_core::storage::manager::get_config_dir;
use one_core::tab_container::{TabContent, TabContentEvent};
use one_core::utils::auto_save_config::AutoSaveConfig;
use reqwest_client::ReqwestClient;
use rust_i18n::t;
use serde::{Deserialize, Serialize};
use terminal_view::TerminalSettings;
use tracing::{error, info};

use crate::app_init::is_valid_system_hotkey;
use crate::auth::get_auth_service;
use crate::license::{get_license_service, offline_license_public_key};
use crate::settings::llm_providers_view::LlmProvidersView;
use crate::update;

// ============================================================================
// 全局用户状态
// ============================================================================

/// 全局当前用户状态
///
/// 用于在设置面板中显示用户信息和执行登出操作。
#[derive(Clone, Default)]
pub struct GlobalCurrentUser {
    user: Arc<RwLock<Option<UserInfo>>>,
}

impl gpui::Global for GlobalCurrentUser {}

impl GlobalCurrentUser {
    /// 获取当前用户
    pub fn get_user(cx: &App) -> Option<UserInfo> {
        if let Some(state) = cx.try_global::<GlobalCurrentUser>() {
            state.user.read().ok().and_then(|u| u.clone())
        } else {
            None
        }
    }

    /// 设置当前用户
    pub fn set_user(user: Option<UserInfo>, cx: &mut App) {
        if !cx.has_global::<GlobalCurrentUser>() {
            cx.set_global(GlobalCurrentUser::default());
        }
        if let Some(state) = cx.try_global::<GlobalCurrentUser>() {
            if let Ok(mut guard) = state.user.write() {
                *guard = user.clone();
            }
        }
        GlobalCloudUser::set_user(user, cx);
    }
}

// ============================================================================
// 数据库配置
// ============================================================================

/// 数据库打开方式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DatabaseOpenMode {
    /// 单库模式：每个数据库单独打开一个标签页
    #[default]
    Single,
    /// 工作区模式：按工作区分组打开，同一工作区的数据库在同一标签页
    Workspace,
}

impl DatabaseOpenMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            DatabaseOpenMode::Single => "single",
            DatabaseOpenMode::Workspace => "workspace",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "workspace" => DatabaseOpenMode::Workspace,
            _ => DatabaseOpenMode::Single,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyType {
    Http,
    Https,
    #[default]
    Socks5,
}

impl ProxyType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProxyType::Http => "http",
            ProxyType::Https => "https",
            ProxyType::Socks5 => "socks5",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalProxySettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub proxy_type: ProxyType,
    #[serde(default)]
    pub host: String,
    #[serde(default = "default_proxy_port")]
    pub port: u16,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
}

fn default_proxy_port() -> u16 {
    1080
}

impl Default for GlobalProxySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            proxy_type: ProxyType::default(),
            host: String::new(),
            port: default_proxy_port(),
            username: String::new(),
            password: String::new(),
        }
    }
}

impl GlobalProxySettings {
    pub fn validate(&self) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }

        if self.host.trim().is_empty() {
            return Err("代理主机不能为空".to_string());
        }

        if self.port == 0 {
            return Err("代理端口不能为空".to_string());
        }

        if self.username.trim().is_empty() && !self.password.is_empty() {
            return Err("填写代理密码时必须同时填写用户名".to_string());
        }

        Ok(())
    }

    pub fn to_proxy_url(&self) -> Result<Option<Url>, String> {
        if !self.enabled {
            return Ok(None);
        }

        self.validate()?;

        let base = format!(
            "{}://{}:{}",
            self.proxy_type.as_str(),
            self.host.trim(),
            self.port
        );
        let mut url = Url::parse(&base).map_err(|err| format!("代理地址格式不正确: {}", err))?;

        if !self.username.trim().is_empty() {
            url.set_username(self.username.trim())
                .map_err(|_| "代理用户名格式不正确".to_string())?;
        }

        if !self.password.is_empty() {
            url.set_password(Some(&self.password))
                .map_err(|_| "代理密码格式不正确".to_string())?;
        }

        Ok(Some(url))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default)]
    pub locale: String,
    #[serde(default)]
    pub theme_mode: String,
    #[serde(default)]
    pub auto_switch_theme: bool,
    #[serde(default = "default_font_family")]
    pub font_family: String,
    #[serde(default = "default_font_size")]
    pub font_size: f64,
    #[serde(default = "default_terminal_font_size")]
    pub terminal_font_size: f64,
    #[serde(default = "default_true")]
    pub terminal_auto_copy: bool,
    #[serde(default = "default_true")]
    pub terminal_enable_autocomplete: bool,
    #[serde(default = "default_true")]
    pub terminal_middle_click_paste: bool,
    #[serde(default)]
    pub terminal_sync_path_with_terminal: bool,
    #[serde(default = "default_terminal_theme")]
    pub terminal_theme: String,
    #[serde(default)]
    pub terminal_cursor_blink: bool,
    #[serde(default = "default_true")]
    pub terminal_confirm_multiline_paste: bool,
    #[serde(default = "default_true")]
    pub terminal_confirm_high_risk_command: bool,
    #[serde(default = "default_true")]
    pub auto_update: bool,
    #[serde(default)]
    pub global_proxy: GlobalProxySettings,
    #[serde(default)]
    pub database_open_mode: DatabaseOpenMode,
    /// 是否启用SQL查询的自动保存功能
    #[serde(default = "default_true")]
    pub enable_sql_auto_save: bool,
    /// SQL查询自动保存的间隔（秒），默认5秒
    #[serde(default = "default_auto_save_interval")]
    pub sql_auto_save_interval: f64,
    #[serde(default = "default_system_hotkey_macos")]
    pub system_hotkey_macos: String,
    #[serde(default = "default_system_hotkey_other")]
    pub system_hotkey_other: String,
}

pub(crate) const DEFAULT_SYSTEM_HOTKEY_MACOS: &str = "cmd-alt-m";
pub(crate) const DEFAULT_SYSTEM_HOTKEY_OTHER: &str = "ctrl-space";

fn default_font_family() -> String {
    "Arial".to_string()
}

fn default_font_size() -> f64 {
    14.0
}

fn default_terminal_font_size() -> f64 {
    15.0
}

fn default_terminal_theme() -> String {
    "ocean".to_string()
}

fn default_true() -> bool {
    true
}

fn default_auto_save_interval() -> f64 {
    5.0
}

fn default_system_hotkey_macos() -> String {
    DEFAULT_SYSTEM_HOTKEY_MACOS.to_string()
}

fn default_system_hotkey_other() -> String {
    DEFAULT_SYSTEM_HOTKEY_OTHER.to_string()
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            locale: "zh-CN".to_string(),
            theme_mode: "light".to_string(),
            auto_switch_theme: false,
            font_family: default_font_family(),
            font_size: default_font_size(),
            terminal_font_size: default_terminal_font_size(),
            terminal_auto_copy: default_true(),
            terminal_enable_autocomplete: default_true(),
            terminal_middle_click_paste: default_true(),
            terminal_sync_path_with_terminal: false,
            terminal_theme: default_terminal_theme(),
            terminal_cursor_blink: false,
            terminal_confirm_multiline_paste: default_true(),
            terminal_confirm_high_risk_command: default_true(),
            auto_update: true,
            global_proxy: GlobalProxySettings::default(),
            database_open_mode: DatabaseOpenMode::default(),
            enable_sql_auto_save: true,
            sql_auto_save_interval: default_auto_save_interval(),
            system_hotkey_macos: default_system_hotkey_macos(),
            system_hotkey_other: default_system_hotkey_other(),
        }
    }
}

impl gpui::Global for AppSettings {}

impl AppSettings {
    pub fn global(cx: &App) -> &AppSettings {
        cx.global::<AppSettings>()
    }

    pub fn global_mut(cx: &mut App) -> &mut AppSettings {
        cx.global_mut::<AppSettings>()
    }

    pub(crate) fn current_system_hotkey(&self) -> &str {
        #[cfg(target_os = "macos")]
        {
            &self.system_hotkey_macos
        }

        #[cfg(not(target_os = "macos"))]
        {
            &self.system_hotkey_other
        }
    }

    fn config_path() -> Option<PathBuf> {
        get_config_dir().ok().map(|dir| dir.join("settings.json"))
    }

    pub fn load() -> Self {
        let Some(path) = Self::config_path() else {
            return Self::default();
        };

        if !path.exists() {
            return Self::default();
        }

        match std::fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(settings) => {
                    info!("Settings loaded from {:?}", path);
                    settings
                }
                Err(e) => {
                    error!("Failed to parse settings: {}", e);
                    Self::default()
                }
            },
            Err(e) => {
                error!("Failed to read settings file: {}", e);
                Self::default()
            }
        }
    }

    pub fn save(&self) {
        let Some(path) = Self::config_path() else {
            error!("Could not determine config path");
            return;
        };

        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                error!("Failed to create config directory: {}", e);
                return;
            }
        }

        match serde_json::to_string_pretty(self) {
            Ok(content) => {
                if let Err(e) = std::fs::write(&path, content) {
                    error!("Failed to write settings file: {}", e);
                } else {
                    info!("Settings saved to {:?}", path);
                }
            }
            Err(e) => {
                error!("Failed to serialize settings: {}", e);
            }
        }
    }

    pub fn apply(&self, cx: &mut App) {
        gpui_component::set_locale(&self.locale);

        let mode = if self.theme_mode == "dark" {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        };
        Theme::global_mut(cx).mode = mode;
        Theme::change(mode, None, cx);

        // 同步自动保存配置
        self.sync_auto_save_config(cx);
    }

    /// 同步自动保存配置到全局状态
    pub fn sync_auto_save_config(&self, cx: &mut App) {
        Self::update_auto_save_config(self.enable_sql_auto_save, self.sql_auto_save_interval, cx);
    }

    /// 更新自动保存配置（静态方法，避免借用冲突）
    pub fn update_auto_save_config(enabled: bool, interval_seconds: f64, cx: &mut App) {
        if let Some(config) = cx.try_global::<AutoSaveConfig>() {
            config.set_enabled(enabled);
            config.set_interval_seconds(interval_seconds);
        }
    }
}

pub fn init_settings(cx: &mut App) {
    let settings = AppSettings::load();
    terminal_view::init_settings(cx, Some(legacy_terminal_settings(&settings)));
    // 初始化自动保存配置全局状态
    cx.set_global(AutoSaveConfig::new(
        settings.enable_sql_auto_save,
        settings.sql_auto_save_interval,
    ));
    settings.apply(cx);
    cx.set_global(settings);
}

fn legacy_terminal_settings(settings: &AppSettings) -> TerminalSettings {
    TerminalSettings {
        font_size: settings.terminal_font_size as f32,
        auto_copy: settings.terminal_auto_copy,
        enable_autocomplete: settings.terminal_enable_autocomplete,
        middle_click_paste: settings.terminal_middle_click_paste,
        sync_path_with_terminal: settings.terminal_sync_path_with_terminal,
        theme: settings.terminal_theme.clone(),
        cursor_blink: settings.terminal_cursor_blink,
        confirm_multiline_paste: settings.terminal_confirm_multiline_paste,
        confirm_high_risk_command: settings.terminal_confirm_high_risk_command,
    }
}

pub(crate) fn build_app_http_client(
    proxy: &GlobalProxySettings,
) -> Result<Arc<ReqwestClient>, String> {
    let proxy_url = proxy.to_proxy_url()?;
    ReqwestClient::proxy_and_user_agent(proxy_url, "one-hub")
        .map(Arc::new)
        .map_err(|err| format!("HTTP 客户端初始化失败: {}", err))
}

pub struct SettingsPanel {
    focus_handle: FocusHandle,
    llm_providers_view: Entity<LlmProvidersView>,
    size: Size,
    group_variant: GroupBoxVariant,
}

impl SettingsPanel {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let llm_providers_view = cx.new(|cx| LlmProvidersView::new(cx));
        Self {
            focus_handle: cx.focus_handle(),
            llm_providers_view,
            size: Size::default(),
            group_variant: GroupBoxVariant::Outline,
        }
    }

    fn setting_pages(&self, _window: &mut Window, _cx: &App) -> Vec<SettingPage> {
        let llm_view = self.llm_providers_view.clone();
        let default_settings = AppSettings::default();
        let default_system_hotkey = AppSettings::default().current_system_hotkey().to_string();

        vec![
            SettingPage::new(t!("Settings.General.title"))
                .resettable(true)
                .default_open(true)
                .groups(vec![
                    SettingGroup::new()
                        .title(t!("Settings.General.Language.group_title"))
                        .items(vec![SettingItem::new(
                            t!("Settings.General.Language.ui_language"),
                            SettingField::dropdown(
                                vec![
                                    ("zh-CN".into(), t!("Settings.General.Language.zh_cn").into()),
                                    ("zh-HK".into(), t!("Settings.General.Language.zh_hk").into()),
                                    ("en".into(), t!("Settings.General.Language.en").into()),
                                ],
                                |cx: &App| {
                                    SharedString::from(AppSettings::global(cx).locale.clone())
                                },
                                |val: SharedString, cx: &mut App| {
                                    let settings = AppSettings::global_mut(cx);
                                    settings.locale = val.to_string();
                                    gpui_component::set_locale(&settings.locale);
                                    settings.save();
                                },
                            )
                            .default_value(SharedString::from(default_settings.locale)),
                        )
                        .description(
                            t!("Settings.General.Language.ui_language_desc").to_string(),
                        )]),
                    SettingGroup::new()
                        .title(t!("Settings.General.Appearance.group_title"))
                        .items(vec![
                            SettingItem::new(
                                t!("Settings.General.Appearance.dark_mode"),
                                SettingField::switch(
                                    |cx: &App| cx.theme().mode.is_dark(),
                                    |val: bool, cx: &mut App| {
                                        let mode = if val {
                                            ThemeMode::Dark
                                        } else {
                                            ThemeMode::Light
                                        };
                                        Theme::global_mut(cx).mode = mode;
                                        Theme::change(mode, None, cx);

                                        let settings = AppSettings::global_mut(cx);
                                        settings.theme_mode = if val {
                                            "dark".to_string()
                                        } else {
                                            "light".to_string()
                                        };
                                        settings.save();
                                    },
                                )
                                .default_value(false),
                            )
                            .description(
                                t!("Settings.General.Appearance.dark_mode_desc").to_string(),
                            ),
                            SettingItem::new(
                                t!("Settings.General.Appearance.auto_switch_theme"),
                                SettingField::checkbox(
                                    |cx: &App| AppSettings::global(cx).auto_switch_theme,
                                    |val: bool, cx: &mut App| {
                                        let settings = AppSettings::global_mut(cx);
                                        settings.auto_switch_theme = val;
                                        settings.save();
                                    },
                                )
                                .default_value(default_settings.auto_switch_theme),
                            )
                            .description(
                                t!("Settings.General.Appearance.auto_switch_theme_desc")
                                    .to_string(),
                            ),
                        ]),
                    SettingGroup::new()
                        .title(t!("Settings.General.Font.group_title"))
                        .item(
                            SettingItem::new(
                                t!("Settings.General.Font.font_family"),
                                SettingField::dropdown(
                                    vec![
                                        ("Arial".into(), "Arial".into()),
                                        ("Helvetica".into(), "Helvetica".into()),
                                        ("Times New Roman".into(), "Times New Roman".into()),
                                        ("Courier New".into(), "Courier New".into()),
                                    ],
                                    |cx: &App| {
                                        SharedString::from(
                                            AppSettings::global(cx).font_family.clone(),
                                        )
                                    },
                                    |val: SharedString, cx: &mut App| {
                                        let settings = AppSettings::global_mut(cx);
                                        settings.font_family = val.to_string();
                                        settings.save();
                                    },
                                )
                                .default_value(SharedString::from(default_settings.font_family)),
                            )
                            .description(t!("Settings.General.Font.font_family_desc").to_string()),
                        )
                        .item(
                            SettingItem::new(
                                t!("Settings.General.Font.font_size"),
                                SettingField::number_input(
                                    NumberFieldOptions {
                                        min: 8.0,
                                        max: 72.0,
                                        ..Default::default()
                                    },
                                    |cx: &App| AppSettings::global(cx).font_size,
                                    |val: f64, cx: &mut App| {
                                        let settings = AppSettings::global_mut(cx);
                                        settings.font_size = val;
                                        settings.save();
                                    },
                                )
                                .default_value(default_settings.font_size),
                            )
                            .description(t!("Settings.General.Font.font_size_desc").to_string()),
                        ),
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
                                            t!("Settings.General.Database.open_mode_workspace")
                                                .into(),
                                        ),
                                    ],
                                    |cx: &App| {
                                        SharedString::from(
                                            AppSettings::global(cx).database_open_mode.as_str(),
                                        )
                                    },
                                    |val: SharedString, cx: &mut App| {
                                        let settings = AppSettings::global_mut(cx);
                                        settings.database_open_mode =
                                            DatabaseOpenMode::from_str(&val);
                                        settings.save();
                                    },
                                )
                                .default_value(SharedString::from(
                                    default_settings.database_open_mode.as_str(),
                                )),
                            )
                            .description(
                                t!("Settings.General.Database.open_mode_desc").to_string(),
                            ),
                            SettingItem::new(
                                t!("Settings.General.Database.auto_save"),
                                SettingField::switch(
                                    |cx: &App| AppSettings::global(cx).enable_sql_auto_save,
                                    |val: bool, cx: &mut App| {
                                        let settings = AppSettings::global_mut(cx);
                                        settings.enable_sql_auto_save = val;
                                        settings.save();
                                        AppSettings::update_auto_save_config(
                                            val,
                                            cx.global::<AppSettings>().sql_auto_save_interval,
                                            cx,
                                        );
                                    },
                                )
                                .default_value(default_settings.enable_sql_auto_save),
                            )
                            .description(
                                t!("Settings.General.Database.auto_save_desc").to_string(),
                            ),
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
                                        let settings = AppSettings::global_mut(cx);
                                        settings.sql_auto_save_interval = val;
                                        settings.save();
                                        AppSettings::update_auto_save_config(
                                            cx.global::<AppSettings>().enable_sql_auto_save,
                                            val,
                                            cx,
                                        );
                                    },
                                )
                                .default_value(default_settings.sql_auto_save_interval),
                            )
                            .description(
                                t!("Settings.General.Database.auto_save_interval_desc").to_string(),
                            ),
                        ]),
                    SettingGroup::new()
                        .title(t!("Settings.General.Update.group_title"))
                        .items(vec![
                            SettingItem::new(
                                t!("Settings.General.Update.auto_update"),
                                SettingField::switch(
                                    |cx: &App| AppSettings::global(cx).auto_update,
                                    |val: bool, cx: &mut App| {
                                        let settings = AppSettings::global_mut(cx);
                                        settings.auto_update = val;
                                        settings.save();
                                    },
                                )
                                .default_value(default_settings.auto_update),
                            )
                            .description(
                                t!("Settings.General.Update.auto_update_desc").to_string(),
                            ),
                            SettingItem::render(move |_options, _window, cx| {
                                render_manual_update_check_item(cx)
                            }),
                        ]),
                    SettingGroup::new()
                        .title(t!("Settings.General.Proxy.group_title"))
                        .item(SettingItem::render(move |_options, _window, cx| {
                            render_global_proxy_settings_item(cx)
                        })),
                ]),
            // 快捷键页面
            SettingPage::new(t!("Settings.Shortcuts.title")).group(
                SettingGroup::new()
                    .item(
                        SettingItem::new(
                            t!("Settings.Shortcuts.system_hotkey"),
                            SettingField::input(
                                |cx: &App| {
                                    SharedString::from(
                                        AppSettings::global(cx).current_system_hotkey().to_string(),
                                    )
                                },
                                |val: SharedString, cx: &mut App| {
                                    let spec = val.trim().to_string();
                                    if spec.is_empty() {
                                        let settings = AppSettings::global_mut(cx);
                                        #[cfg(target_os = "macos")]
                                        {
                                            settings.system_hotkey_macos =
                                                DEFAULT_SYSTEM_HOTKEY_MACOS.to_string();
                                        }
                                        #[cfg(not(target_os = "macos"))]
                                        {
                                            settings.system_hotkey_other =
                                                DEFAULT_SYSTEM_HOTKEY_OTHER.to_string();
                                        }
                                        settings.save();
                                        return;
                                    }

                                    if !is_valid_system_hotkey(&spec) {
                                        return;
                                    }

                                    let settings = AppSettings::global_mut(cx);
                                    #[cfg(target_os = "macos")]
                                    {
                                        settings.system_hotkey_macos = spec;
                                    }
                                    #[cfg(not(target_os = "macos"))]
                                    {
                                        settings.system_hotkey_other = spec;
                                    }
                                    settings.save();
                                },
                            )
                            .default_value(SharedString::from(default_system_hotkey)),
                        )
                        .description(t!("Settings.Shortcuts.system_hotkey_desc").to_string()),
                    )
                    .item(SettingItem::render(move |_options, _window, cx| {
                        render_shortcuts_section(cx)
                    })),
            ),
            SettingPage::new(t!("LlmProviders.title")).group(SettingGroup::new().item(
                SettingItem::render(move |_options, _window, _cx| {
                    llm_view.clone().into_any_element()
                }),
            )),
            // 账户设置页
            SettingPage::new(t!("Settings.Account.title")).group(SettingGroup::new().item(
                SettingItem::render(move |_options, window, cx| render_account_section(window, cx)),
            )),
            // 关于页面
            SettingPage::new(t!("Settings.About.title")).group(SettingGroup::new().item(
                SettingItem::render(move |_options, _window, cx| render_about_section(cx)),
            )),
        ]
    }
}

impl Focusable for SettingsPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<TabContentEvent> for SettingsPanel {}

impl TabContent for SettingsPanel {
    fn content_key(&self) -> &'static str {
        "Settings"
    }

    fn title(&self, _cx: &App) -> SharedString {
        SharedString::from(t!("Common.settings"))
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        Some(IconName::SettingColor.color())
    }

    fn closeable(&self, _cx: &App) -> bool {
        true
    }

    fn on_activate(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if !cx.has_global::<AppSettings>() {
            init_settings(cx);
        }
    }
}

impl Render for SettingsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !cx.has_global::<AppSettings>() {
            init_settings(cx);
        }

        div().track_focus(&self.focus_handle).size_full().child(
            Settings::new("main-app-settings")
                .with_size(self.size)
                .with_group_variant(self.group_variant)
                .pages(self.setting_pages(window, cx)),
        )
    }
}

fn render_manual_update_check_item(cx: &mut App) -> gpui::AnyElement {
    h_flex()
        .w_full()
        .justify_between()
        .items_center()
        .gap_3()
        .child(
            v_flex()
                .gap_1()
                .flex_1()
                .child(
                    div()
                        .text_sm()
                        .child(t!("Settings.General.Update.check_now").to_string()),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("Settings.General.Update.check_now_desc").to_string()),
                ),
        )
        .child(
            Button::new("settings-check-update")
                .icon(IconName::Refresh)
                .label(t!("Settings.General.Update.check_now"))
                .on_click(|_, window, cx| {
                    update::check_for_updates_manually(window, cx);
                }),
        )
        .into_any_element()
}

fn render_global_proxy_settings_item(cx: &mut App) -> gpui::AnyElement {
    h_flex()
        .w_full()
        .justify_between()
        .items_center()
        .gap_3()
        .child(
            v_flex()
                .gap_1()
                .flex_1()
                .child(
                    div()
                        .text_sm()
                        .child(t!("Settings.General.Proxy.title").to_string()),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("Settings.General.Proxy.description").to_string()),
                ),
        )
        .child(
            Button::new("settings-global-proxy")
                .icon(IconName::Globe)
                .label(t!("Settings.General.Proxy.open").to_string())
                .on_click(|_, _window, cx| {
                    show_global_proxy_settings_window(cx);
                }),
        )
        .into_any_element()
}

#[derive(Clone, PartialEq)]
struct ProxyTypeOption {
    value: ProxyType,
    label: SharedString,
}

impl SelectItem for ProxyTypeOption {
    type Value = ProxyType;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

struct GlobalProxySettingsView {
    focus_handle: FocusHandle,
    enabled: bool,
    proxy_type_select: Entity<SelectState<Vec<ProxyTypeOption>>>,
    host_input: Entity<InputState>,
    port_input: Entity<InputState>,
    username_input: Entity<InputState>,
    password_input: Entity<InputState>,
    testing: bool,
    status_message: Option<(bool, String)>,
}

impl GlobalProxySettingsView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let current = AppSettings::global(cx).global_proxy.clone();
        let proxy_types = vec![
            ProxyTypeOption {
                value: ProxyType::Http,
                label: "HTTP".into(),
            },
            ProxyTypeOption {
                value: ProxyType::Https,
                label: "HTTPS".into(),
            },
            ProxyTypeOption {
                value: ProxyType::Socks5,
                label: "SOCKS5".into(),
            },
        ];
        let selected_index = match current.proxy_type {
            ProxyType::Http => 0,
            ProxyType::Https => 1,
            ProxyType::Socks5 => 2,
        };
        let proxy_type_select = cx.new(|cx| {
            SelectState::new(
                proxy_types,
                Some(IndexPath::new(selected_index)),
                window,
                cx,
            )
        });
        let host_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("127.0.0.1");
            if !current.host.is_empty() {
                state.set_value(current.host.clone(), window, cx);
            }
            state
        });
        let port_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("1080");
            state.set_value(current.port.to_string(), window, cx);
            state
        });
        let username_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("Settings.General.Proxy.username_placeholder"));
            if !current.username.is_empty() {
                state.set_value(current.username.clone(), window, cx);
            }
            state
        });
        let password_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("Settings.General.Proxy.password_placeholder"));
            if !current.password.is_empty() {
                state.set_value(current.password.clone(), window, cx);
            }
            state
        });

        Self {
            focus_handle: cx.focus_handle(),
            enabled: current.enabled,
            proxy_type_select,
            host_input,
            port_input,
            username_input,
            password_input,
            testing: false,
            status_message: None,
        }
    }

    fn build_proxy_settings(&self, cx: &App) -> GlobalProxySettings {
        GlobalProxySettings {
            enabled: self.enabled,
            proxy_type: self
                .proxy_type_select
                .read(cx)
                .selected_value()
                .copied()
                .unwrap_or_default(),
            host: self
                .host_input
                .read(cx)
                .text()
                .to_string()
                .trim()
                .to_string(),
            port: self
                .port_input
                .read(cx)
                .text()
                .to_string()
                .trim()
                .parse::<u16>()
                .unwrap_or(0),
            username: self
                .username_input
                .read(cx)
                .text()
                .to_string()
                .trim()
                .to_string(),
            password: self.password_input.read(cx).text().to_string(),
        }
    }

    fn render_form_row(
        &self,
        label: String,
        child: impl IntoElement,
        disabled: bool,
        cx: &App,
    ) -> gpui::AnyElement {
        h_flex()
            .gap_3()
            .items_center()
            .child(
                div()
                    .w(gpui::px(120.0))
                    .text_sm()
                    .text_color(cx.theme().foreground)
                    .child(label),
            )
            .child(
                div()
                    .flex_1()
                    .child(child)
                    .when(disabled, |this| this.opacity(0.55)),
            )
            .into_any_element()
    }

    fn on_test(&mut self, cx: &mut Context<Self>) {
        if self.testing || !self.enabled {
            return;
        }

        let proxy_settings = self.build_proxy_settings(cx);
        let client = match build_app_http_client(&proxy_settings) {
            Ok(client) => client,
            Err(err) => {
                self.status_message = Some((false, err));
                cx.notify();
                return;
            }
        };

        self.testing = true;
        self.status_message = None;
        cx.notify();

        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let test_task = Tokio::spawn(cx, async move {
                let http_client: Arc<dyn gpui::http_client::HttpClient> = client;
                test_proxy_connectivity(http_client).await
            });

            let result = match test_task.await {
                Ok(result) => result,
                Err(err) => Err(format!("代理测试任务执行失败: {}", err)),
            };

            let _ = this.update(cx, |view, cx| {
                view.testing = false;
                view.status_message = Some(match result {
                    Ok(()) => (true, t!("Settings.General.Proxy.test_success").to_string()),
                    Err(err) => (false, err),
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn on_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.testing {
            return;
        }

        let proxy_settings = self.build_proxy_settings(cx);
        let new_client = match build_app_http_client(&proxy_settings) {
            Ok(client) => client,
            Err(err) => {
                self.status_message = Some((false, err));
                cx.notify();
                return;
            }
        };

        let proxy_settings_for_apply = proxy_settings.clone();
        let new_client_for_apply = new_client.clone();
        cx.defer(move |cx| {
            let settings = AppSettings::global_mut(cx);
            settings.global_proxy = proxy_settings_for_apply;
            settings.save();
            apply_global_http_client(new_client_for_apply, cx);
        });

        window.push_notification(t!("Settings.General.Proxy.save_success").to_string(), cx);
        window.remove_window();
    }

    fn on_cancel(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        if self.testing {
            return;
        }
        window.remove_window();
    }
}

impl Focusable for GlobalProxySettingsView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for GlobalProxySettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let disabled = !self.enabled;

        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(
                TitleBar::new().child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .flex_1()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child(t!("Settings.General.Proxy.dialog_title").to_string()),
                ),
            )
            .child(
                div().flex_1().min_h_0().overflow_y_scrollbar().p_4().child(
                    v_flex()
                        .gap_4()
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("Settings.General.Proxy.dialog_desc").to_string()),
                        )
                        .child(
                            h_flex()
                                .justify_between()
                                .items_center()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(t!("Settings.General.Proxy.enable").to_string()),
                                )
                                .child(
                                    Switch::new("global-proxy-enabled")
                                        .checked(self.enabled)
                                        .on_click(cx.listener(|view, checked, _, cx| {
                                            view.enabled = *checked;
                                            view.status_message = None;
                                            cx.notify();
                                        })),
                                ),
                        )
                        .child(self.render_form_row(
                            t!("Settings.General.Proxy.type").to_string(),
                            Select::new(&self.proxy_type_select).disabled(disabled),
                            disabled,
                            cx,
                        ))
                        .child(self.render_form_row(
                            t!("Settings.General.Proxy.host").to_string(),
                            Input::new(&self.host_input).disabled(disabled),
                            disabled,
                            cx,
                        ))
                        .child(self.render_form_row(
                            t!("Settings.General.Proxy.port").to_string(),
                            Input::new(&self.port_input).disabled(disabled),
                            disabled,
                            cx,
                        ))
                        .child(self.render_form_row(
                            t!("Settings.General.Proxy.username").to_string(),
                            Input::new(&self.username_input).disabled(disabled),
                            disabled,
                            cx,
                        ))
                        .child(
                            self.render_form_row(
                                t!("Settings.General.Proxy.password").to_string(),
                                Input::new(&self.password_input)
                                    .mask_toggle()
                                    .disabled(disabled),
                                disabled,
                                cx,
                            ),
                        )
                        .when_some(self.status_message.clone(), |this, (success, message)| {
                            this.child(
                                div()
                                    .text_sm()
                                    .text_color(if success {
                                        cx.theme().muted_foreground
                                    } else {
                                        cx.theme().danger
                                    })
                                    .child(message),
                            )
                        }),
                ),
            )
            .child(
                h_flex()
                    .flex_shrink_0()
                    .justify_end()
                    .gap_2()
                    .p_4()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        Button::new("proxy-test")
                            .small()
                            .label(if self.testing {
                                t!("Settings.General.Proxy.testing").to_string()
                            } else {
                                t!("Settings.General.Proxy.test").to_string()
                            })
                            .disabled(self.testing || !self.enabled)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.on_test(cx);
                            })),
                    )
                    .child(
                        Button::new("proxy-cancel")
                            .small()
                            .label(t!("Common.cancel").to_string())
                            .disabled(self.testing)
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.on_cancel(window, cx);
                            })),
                    )
                    .child(
                        Button::new("proxy-save")
                            .small()
                            .primary()
                            .label(t!("Common.save").to_string())
                            .disabled(self.testing)
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.on_save(window, cx);
                            })),
                    ),
            )
    }
}

fn show_global_proxy_settings_window(cx: &mut App) {
    open_popup_window(
        PopupWindowOptions::new(t!("Settings.General.Proxy.dialog_title").to_string())
            .size(560.0, 460.0),
        move |window, cx| cx.new(|cx| GlobalProxySettingsView::new(window, cx)),
        cx,
    );
}

fn apply_global_http_client(http_client: Arc<ReqwestClient>, cx: &mut App) {
    let auth_service = get_auth_service(cx);
    let http_for_auth: Arc<dyn gpui::http_client::HttpClient> = http_client.clone();
    auth_service.replace_http_client(http_for_auth);

    if let Some(provider_state) = cx.try_global::<GlobalProviderState>() {
        provider_state.set_cloud_client(auth_service.cloud_client());
        provider_state.manager().clear_cache();
    }

    cx.set_http_client(http_client);
}

async fn test_proxy_connectivity(
    http_client: Arc<dyn gpui::http_client::HttpClient>,
) -> Result<(), String> {
    let request = Request::builder()
        .method(Method::HEAD)
        .uri("https://www.gstatic.com/generate_204")
        .header("User-Agent", "onetcli-updater")
        .body(AsyncBody::empty())
        .map_err(|err| format!("构建代理测试请求失败: {}", err))?;

    let response = http_client
        .send(request)
        .await
        .map_err(|err| format!("代理连接测试失败: {}", err))?;

    if !response.status().is_success() {
        return Err(format!("代理测试返回异常状态码: {}", response.status()));
    }

    Ok(())
}

/// 渲染账户设置区域
fn render_account_section(_window: &mut Window, cx: &App) -> gpui::AnyElement {
    let user = GlobalCurrentUser::get_user(cx);

    if let Some(user) = user {
        // 已登录状态：显示用户信息和登出按钮
        let email: SharedString = user.email.clone().into();
        let display_name: SharedString = user
            .username
            .clone()
            .unwrap_or_else(|| {
                user.email
                    .split('@')
                    .next()
                    .unwrap_or(&user.email)
                    .to_string()
            })
            .into();

        v_flex()
            .gap_4()
            .p_4()
            // 用户信息区域
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(t!("Settings.Account.username").to_string()),
                            )
                            .child(div().text_sm().child(display_name)),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(t!("Settings.Account.email").to_string()),
                            )
                            .child(div().text_sm().child(email)),
                    ),
            )
            // 登出按钮
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("import-license-button")
                            .icon(IconName::File)
                            .label("导入离线 License")
                            .on_click(move |_, window, cx| {
                                let public_key = match offline_license_public_key() {
                                    Ok(key) => key,
                                    Err(msg) => {
                                        window.push_notification(msg, cx);
                                        return;
                                    }
                                };
                                let license_service = get_license_service(cx);
                                let future = cx.prompt_for_paths(PathPromptOptions {
                                    files: true,
                                    directories: false,
                                    multiple: false,
                                    prompt: Some("选择 License 文件".into()),
                                });

                                window
                                    .spawn(cx, async move |cx| {
                                        if let Ok(Ok(Some(paths))) = future.await {
                                            if let Some(path) = paths.into_iter().next() {
                                                let result = license_service
                                                    .import_offline_license_from_path(
                                                        &path,
                                                        &public_key,
                                                        None,
                                                    );
                                                let message = match result {
                                                    Ok(_) => "离线 License 导入成功".to_string(),
                                                    Err(err) => {
                                                        format!("离线 License 导入失败: {}", err)
                                                    }
                                                };
                                                let _ = cx.update(|_view, cx: &mut App| {
                                                    if let Some(window_id) = cx.active_window() {
                                                        let _ = cx.update_window(
                                                            window_id,
                                                            |_, window, cx| {
                                                                window
                                                                    .push_notification(message, cx);
                                                            },
                                                        );
                                                    }
                                                });
                                            }
                                        }
                                    })
                                    .detach();
                            }),
                    )
                    .child(
                        Button::new("logout-button")
                            .icon(IconName::Close)
                            .label(t!("Auth.logout"))
                            .danger()
                            .on_click(move |_, _window, cx| {
                                // 清除 License
                                get_license_service(cx).clear();

                                // 执行登出
                                let auth = get_auth_service(cx);
                                cx.spawn(async move |cx: &mut AsyncApp| {
                                    auth.sign_out().await;
                                    cx.update(|cx| {
                                        GlobalCurrentUser::set_user(None, cx);
                                    });
                                })
                                .detach();
                            }),
                    ),
            )
            .into_any_element()
    } else {
        // 未登录状态：显示提示信息
        v_flex()
            .gap_2()
            .p_4()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("Settings.Account.not_logged_in").to_string()),
            )
            .into_any_element()
    }
}

// ============================================================================
// 快捷键设置页
// ============================================================================

/// 快捷键条目
struct ShortcutEntry {
    /// macOS 快捷键字符串（Keystroke::parse 格式）
    key_macos: &'static str,
    /// Windows/Linux 快捷键字符串（Keystroke::parse 格式）
    key_other: &'static str,
    /// 国际化翻译 key
    label_key: &'static str,
}

/// 快捷键分组
struct ShortcutGroup {
    title_key: &'static str,
    entries: &'static [ShortcutEntry],
}

const WINDOW_SHORTCUTS: &[ShortcutEntry] = &[
    ShortcutEntry {
        key_macos: "cmd-q",
        key_other: "alt-f4",
        label_key: "Settings.Shortcuts.quit_app",
    },
    ShortcutEntry {
        key_macos: DEFAULT_SYSTEM_HOTKEY_MACOS,
        key_other: DEFAULT_SYSTEM_HOTKEY_OTHER,
        label_key: "Settings.Shortcuts.minimize_window",
    },
    ShortcutEntry {
        key_macos: "ctrl-cmd-f",
        key_other: "alt-enter",
        label_key: "Settings.Shortcuts.toggle_fullscreen",
    },
    ShortcutEntry {
        key_macos: "shift-escape",
        key_other: "shift-escape",
        label_key: "Settings.Shortcuts.toggle_zoom",
    },
    ShortcutEntry {
        key_macos: "ctrl-w",
        key_other: "ctrl-w",
        label_key: "Settings.Shortcuts.close_panel",
    },
];

const TAB_SHORTCUTS: &[ShortcutEntry] = &[
    ShortcutEntry {
        key_macos: "cmd-1",
        key_other: "alt-1",
        label_key: "Settings.Shortcuts.switch_tab_n",
    },
    ShortcutEntry {
        key_macos: "shift-cmd-t",
        key_other: "alt-shift-t",
        label_key: "Settings.Shortcuts.duplicate_tab",
    },
    ShortcutEntry {
        key_macos: "cmd-o",
        key_other: "alt-o",
        label_key: "Settings.Shortcuts.quick_open",
    },
    ShortcutEntry {
        key_macos: "cmd-n",
        key_other: "alt-n",
        label_key: "Settings.Shortcuts.new_connection",
    },
];

const TERMINAL_SHORTCUTS: &[ShortcutEntry] = &[
    ShortcutEntry {
        key_macos: "cmd-c",
        key_other: "ctrl-shift-c",
        label_key: "Settings.Shortcuts.terminal_copy",
    },
    ShortcutEntry {
        key_macos: "cmd-v",
        key_other: "ctrl-shift-v",
        label_key: "Settings.Shortcuts.terminal_paste",
    },
    ShortcutEntry {
        key_macos: "cmd-f",
        key_other: "ctrl-shift-f",
        label_key: "Settings.Shortcuts.terminal_search",
    },
    ShortcutEntry {
        key_macos: "cmd-a",
        key_other: "ctrl-shift-a",
        label_key: "Settings.Shortcuts.terminal_select_all",
    },
    ShortcutEntry {
        key_macos: "cmd-+",
        key_other: "ctrl-+",
        label_key: "Settings.Shortcuts.terminal_zoom_in",
    },
    ShortcutEntry {
        key_macos: "cmd--",
        key_other: "ctrl--",
        label_key: "Settings.Shortcuts.terminal_zoom_out",
    },
    ShortcutEntry {
        key_macos: "cmd-0",
        key_other: "ctrl-0",
        label_key: "Settings.Shortcuts.terminal_zoom_reset",
    },
    ShortcutEntry {
        key_macos: "f7",
        key_other: "f7",
        label_key: "Settings.Shortcuts.terminal_toggle_vi",
    },
];

const SHORTCUT_GROUPS: &[ShortcutGroup] = &[
    ShortcutGroup {
        title_key: "Settings.Shortcuts.window",
        entries: WINDOW_SHORTCUTS,
    },
    ShortcutGroup {
        title_key: "Settings.Shortcuts.tabs",
        entries: TAB_SHORTCUTS,
    },
    ShortcutGroup {
        title_key: "Settings.Shortcuts.terminal",
        entries: TERMINAL_SHORTCUTS,
    },
];

fn shortcut_spec_for_entry(entry: &ShortcutEntry, cx: &App) -> String {
    if entry.label_key == "Settings.Shortcuts.minimize_window" {
        return AppSettings::global(cx).current_system_hotkey().to_string();
    }

    if cfg!(target_os = "macos") {
        entry.key_macos.to_string()
    } else {
        entry.key_other.to_string()
    }
}

fn render_shortcut_value(key_str: &str, cx: &App) -> gpui::AnyElement {
    match Keystroke::parse(key_str) {
        Ok(keystroke) => Kbd::new(keystroke).into_any_element(),
        Err(_) => div()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(key_str.to_string())
            .into_any_element(),
    }
}

/// 渲染快捷键说明页面
fn render_shortcuts_section(cx: &App) -> gpui::AnyElement {
    let mut container = v_flex().gap_4().p_4();

    for group in SHORTCUT_GROUPS {
        let mut group_container = v_flex().gap_2();

        // 分组标题
        group_container = group_container.child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(t!(group.title_key).to_string()),
        );

        // 快捷键列表
        let mut list = v_flex().gap_1().pl_2();

        for entry in group.entries {
            let key_str = shortcut_spec_for_entry(entry, cx);

            list = list.child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .py_1()
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!(entry.label_key).to_string()),
                    )
                    .child(render_shortcut_value(&key_str, cx)),
            );
        }

        group_container = group_container.child(list);
        container = container.child(group_container);
    }

    container.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{AppSettings, GlobalProxySettings, ProxyType};

    #[test]
    fn global_proxy_settings_build_proxy_url_without_auth() {
        let settings = GlobalProxySettings {
            enabled: true,
            proxy_type: ProxyType::Socks5,
            host: "127.0.0.1".to_string(),
            port: 7890,
            username: String::new(),
            password: String::new(),
        };

        let proxy_url = settings
            .to_proxy_url()
            .expect("代理 URL 应构建成功")
            .expect("启用代理时应返回 URL");

        assert_eq!(proxy_url.as_str(), "socks5://127.0.0.1:7890");
    }

    #[test]
    fn global_proxy_settings_build_proxy_url_with_auth() {
        let settings = GlobalProxySettings {
            enabled: true,
            proxy_type: ProxyType::Http,
            host: "proxy.example.com".to_string(),
            port: 8080,
            username: "demo-user".to_string(),
            password: "demo-pass".to_string(),
        };

        let proxy_url = settings
            .to_proxy_url()
            .expect("代理 URL 应构建成功")
            .expect("启用代理时应返回 URL");

        assert_eq!(
            proxy_url.as_str(),
            "http://demo-user:demo-pass@proxy.example.com:8080/"
        );
    }

    #[test]
    fn disabled_global_proxy_settings_return_none() {
        let settings = GlobalProxySettings {
            enabled: false,
            ..GlobalProxySettings::default()
        };

        let proxy_url = settings.to_proxy_url().expect("禁用代理时不应返回错误");

        assert!(proxy_url.is_none());
    }

    #[test]
    fn global_proxy_settings_validate_required_fields() {
        let settings = GlobalProxySettings {
            enabled: true,
            proxy_type: ProxyType::Https,
            host: String::new(),
            port: 0,
            username: String::new(),
            password: String::new(),
        };

        let err = settings.validate().expect_err("缺少主机和端口时应校验失败");

        assert!(err.contains("主机"));
    }

    #[test]
    fn legacy_terminal_settings_maps_terminal_fields() {
        let settings = AppSettings::default();
        let legacy = super::legacy_terminal_settings(&settings);

        assert_eq!(legacy.font_size, settings.terminal_font_size as f32);
        assert_eq!(legacy.auto_copy, settings.terminal_auto_copy);
        assert_eq!(
            legacy.enable_autocomplete,
            settings.terminal_enable_autocomplete
        );
        assert_eq!(legacy.theme, settings.terminal_theme);
    }
}

/// GitHub 开源地址
const GITHUB_URL: &str = "https://github.com/feigeCode/onetcli";

/// 渲染关于页面
fn render_about_section(cx: &App) -> gpui::AnyElement {
    let version = env!("CARGO_PKG_VERSION");
    let muted = cx.theme().muted_foreground;

    let disclaimer_items: Vec<String> = (1..=5)
        .map(|i| {
            let key = format!("Settings.About.disclaimer_item_{}", i);
            let text = t!(&key).to_string();
            format!("{}. {}", i, text)
        })
        .collect();

    let data_safety_items: Vec<String> = (1..=3)
        .map(|i| {
            let key = format!("Settings.About.data_safety_item_{}", i);
            let text = t!(&key).to_string();
            format!("• {}", text)
        })
        .collect();

    v_flex()
        .gap_4()
        .p_4()
        // 版本信息
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(div().text_sm().child(format!(
                    "{}: {}",
                    t!("Settings.About.version"),
                    version
                ))),
        )
        // GitHub 开源地址
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .text_sm()
                        .child(format!("{}: ", t!("Settings.About.opensource_label"))),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().link)
                        .child(GITHUB_URL),
                )
                .child(Clipboard::new("about-copy-github-url").value(GITHUB_URL))
                .child(
                    Button::new("about-open-github")
                        .icon(IconName::ExternalLink)
                        .xsmall()
                        .ghost()
                        .on_click(|_: &ClickEvent, _, cx| {
                            cx.open_url(GITHUB_URL);
                        }),
                ),
        )
        // 免责声明
        .child(
            v_flex()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(t!("Settings.About.disclaimer_title").to_string()),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child(t!("Settings.About.disclaimer_status").to_string()),
                )
                .child(
                    v_flex().gap_1().pl_2().children(
                        disclaimer_items
                            .into_iter()
                            .map(|item| div().text_sm().text_color(muted).child(item)),
                    ),
                ),
        )
        // 数据与安全提示
        .child(
            v_flex()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(t!("Settings.About.data_safety_title").to_string()),
                )
                .child(
                    v_flex().gap_1().pl_2().children(
                        data_safety_items
                            .into_iter()
                            .map(|item| div().text_sm().text_color(muted).child(item)),
                    ),
                ),
        )
        .into_any_element()
}
