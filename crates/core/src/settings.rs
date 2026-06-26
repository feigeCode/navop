use crate::cloud_sync::{GlobalCloudUser, UserInfo};
use crate::storage::get_config_dir;
use crate::utils::auto_save_config::AutoSaveConfig;
use gpui::http_client::Url;
use gpui::{App, Global};
use gpui_component::{Theme, ThemeMode};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tracing::{error, info};

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

impl Global for GlobalCurrentUser {}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LargeTextCellEditorOpenMode {
    #[default]
    SidebarPreview,
    Dialog,
}

impl LargeTextCellEditorOpenMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            LargeTextCellEditorOpenMode::SidebarPreview => "sidebar_preview",
            LargeTextCellEditorOpenMode::Dialog => "dialog",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "dialog" => LargeTextCellEditorOpenMode::Dialog,
            _ => LargeTextCellEditorOpenMode::SidebarPreview,
        }
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpServerMode {
    #[default]
    Temporary,
    Persistent,
}

impl McpServerMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            McpServerMode::Temporary => "temporary",
            McpServerMode::Persistent => "persistent",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "persistent" => McpServerMode::Persistent,
            _ => McpServerMode::Temporary,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpPermissionMode {
    #[default]
    Deny,
    Ask,
    Allow,
}

impl McpPermissionMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            McpPermissionMode::Deny => "deny",
            McpPermissionMode::Ask => "ask",
            McpPermissionMode::Allow => "allow",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "allow" => McpPermissionMode::Allow,
            "ask" => McpPermissionMode::Ask,
            _ => McpPermissionMode::Deny,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpToolsetSettings {
    #[serde(default = "default_true")]
    pub terminal: bool,
    #[serde(default = "default_true")]
    pub connections: bool,
    #[serde(default)]
    pub sftp: bool,
    #[serde(default)]
    pub database: bool,
    #[serde(default)]
    pub redis: bool,
    #[serde(default)]
    pub internal_functions: bool,
}

impl Default for McpToolsetSettings {
    fn default() -> Self {
        Self {
            terminal: true,
            connections: true,
            sftp: false,
            database: false,
            redis: false,
            internal_functions: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpSettings {
    #[serde(default)]
    pub server_enabled: bool,
    #[serde(default)]
    pub server_mode: McpServerMode,
    #[serde(default)]
    pub permission_mode: McpPermissionMode,
    #[serde(default)]
    pub toolsets: McpToolsetSettings,
}

impl Default for McpSettings {
    fn default() -> Self {
        Self {
            server_enabled: false,
            server_mode: McpServerMode::Temporary,
            permission_mode: McpPermissionMode::Deny,
            toolsets: McpToolsetSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomFont {
    pub path: String,
    #[serde(default)]
    pub families: Vec<String>,
    #[serde(default)]
    pub monospace_families: Vec<String>,
}

fn deserialize_custom_fonts<'de, D>(deserializer: D) -> Result<Vec<CustomFont>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum CustomFontEntry {
        Path(String),
        Font(CustomFont),
    }

    let entries = Vec::<CustomFontEntry>::deserialize(deserializer)?;
    Ok(entries
        .into_iter()
        .map(|entry| match entry {
            CustomFontEntry::Path(path) => CustomFont {
                path,
                families: Vec::new(),
                monospace_families: Vec::new(),
            },
            CustomFontEntry::Font(font) => font,
        })
        .collect())
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
    #[serde(default = "default_monospace_font_family")]
    pub sql_editor_font_family: String,
    #[serde(default = "default_monospace_font_family")]
    pub table_preview_font_family: String,
    #[serde(default = "default_monospace_font_family")]
    pub terminal_font_family: String,
    #[serde(
        default,
        alias = "custom_font_paths",
        deserialize_with = "deserialize_custom_fonts"
    )]
    pub custom_fonts: Vec<CustomFont>,
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
    #[serde(default)]
    pub log_file_path: String,
    #[serde(default = "default_true")]
    pub auto_update: bool,
    #[serde(default)]
    pub global_proxy: GlobalProxySettings,
    #[serde(default)]
    pub mcp: McpSettings,
    #[serde(default)]
    pub database_open_mode: DatabaseOpenMode,
    #[serde(default)]
    pub large_text_cell_editor_open_mode: LargeTextCellEditorOpenMode,
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
    /// 表格行高（像素），默认44
    #[serde(default = "default_table_row_height")]
    pub table_row_height: u32,
    /// SQL 查询默认最大返回行数，0 表示不限制
    #[serde(default = "default_sql_query_max_rows")]
    pub sql_query_max_rows: u32,
    #[serde(default)]
    pub custom_keybindings: HashMap<String, Vec<String>>,
}

pub(crate) const DEFAULT_SYSTEM_HOTKEY_MACOS: &str = "cmd-alt-m";
pub(crate) const DEFAULT_SYSTEM_HOTKEY_OTHER: &str = "ctrl-alt-m";
pub const DEFAULT_SQL_QUERY_MAX_ROWS: u32 = 1000;

fn default_font_family() -> String {
    "Arial".to_string()
}

fn default_font_size() -> f64 {
    14.0
}

fn default_monospace_font_family() -> String {
    if cfg!(target_os = "macos") {
        "Menlo"
    } else if cfg!(target_os = "windows") {
        "Consolas"
    } else {
        "DejaVu Sans Mono"
    }
    .to_string()
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

fn default_table_row_height() -> u32 {
    44
}

fn default_sql_query_max_rows() -> u32 {
    DEFAULT_SQL_QUERY_MAX_ROWS
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            locale: "zh-CN".to_string(),
            theme_mode: "light".to_string(),
            auto_switch_theme: false,
            font_family: default_font_family(),
            font_size: default_font_size(),
            sql_editor_font_family: default_monospace_font_family(),
            table_preview_font_family: default_monospace_font_family(),
            terminal_font_family: default_monospace_font_family(),
            custom_fonts: Vec::new(),
            terminal_font_size: default_terminal_font_size(),
            terminal_auto_copy: default_true(),
            terminal_enable_autocomplete: default_true(),
            terminal_middle_click_paste: default_true(),
            terminal_sync_path_with_terminal: false,
            terminal_theme: default_terminal_theme(),
            terminal_cursor_blink: false,
            terminal_confirm_multiline_paste: default_true(),
            terminal_confirm_high_risk_command: default_true(),
            log_file_path: String::new(),
            auto_update: true,
            global_proxy: GlobalProxySettings::default(),
            mcp: McpSettings::default(),
            database_open_mode: DatabaseOpenMode::default(),
            large_text_cell_editor_open_mode: LargeTextCellEditorOpenMode::default(),
            enable_sql_auto_save: true,
            sql_auto_save_interval: default_auto_save_interval(),
            system_hotkey_macos: default_system_hotkey_macos(),
            system_hotkey_other: default_system_hotkey_other(),
            table_row_height: default_table_row_height(),
            sql_query_max_rows: default_sql_query_max_rows(),
            custom_keybindings: HashMap::new(),
        }
    }
}

impl Global for AppSettings {}

impl AppSettings {
    pub fn current(cx: &App) -> Self {
        cx.try_global::<AppSettings>().cloned().unwrap_or_default()
    }

    pub fn global(cx: &App) -> &AppSettings {
        cx.global::<AppSettings>()
    }

    pub fn global_mut(cx: &mut App) -> &mut AppSettings {
        cx.global_mut::<AppSettings>()
    }

    pub fn update(cx: &mut App, update: impl FnOnce(&mut AppSettings)) {
        let mut settings = Self::current(cx);
        update(&mut settings);
        cx.set_global(settings);
    }

    pub fn update_and_save(cx: &mut App, update: impl FnOnce(&mut AppSettings)) {
        let mut settings = Self::current(cx);
        update(&mut settings);
        settings.save();
        cx.set_global(settings);
    }

    pub fn current_system_hotkey(&self) -> &str {
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

#[cfg(test)]
mod tests {
    use super::{
        AppSettings, CustomFont, LargeTextCellEditorOpenMode, McpPermissionMode, McpServerMode,
    };

    #[test]
    fn large_text_editor_open_mode_defaults_to_sidebar_preview() {
        assert_eq!(
            LargeTextCellEditorOpenMode::from_str("unknown"),
            LargeTextCellEditorOpenMode::SidebarPreview
        );
    }

    #[test]
    fn large_text_editor_open_mode_parses_dialog() {
        assert_eq!(
            LargeTextCellEditorOpenMode::from_str("dialog"),
            LargeTextCellEditorOpenMode::Dialog
        );
    }

    #[test]
    fn app_settings_default_keeps_mcp_server_disabled() {
        let settings = AppSettings::default();

        assert!(!settings.mcp.server_enabled);
        assert_eq!(settings.mcp.server_mode, McpServerMode::Temporary);
        assert_eq!(settings.mcp.permission_mode, McpPermissionMode::Deny);
        assert!(settings.mcp.toolsets.terminal);
        assert!(settings.mcp.toolsets.connections);
        assert!(!settings.mcp.toolsets.database);
        assert!(!settings.mcp.toolsets.redis);
        assert!(!settings.mcp.toolsets.sftp);
    }

    #[test]
    fn app_settings_default_sets_sql_query_max_rows() {
        let settings = AppSettings::default();

        assert_eq!(1000, settings.sql_query_max_rows);
    }

    #[test]
    fn app_settings_deserializes_sql_query_max_rows_from_legacy_json() {
        let settings: AppSettings = serde_json::from_value(serde_json::json!({
            "locale": "en",
            "theme_mode": "dark"
        }))
        .expect("旧版 settings.json 应能读取");

        assert_eq!(1000, settings.sql_query_max_rows);
    }

    #[test]
    fn app_settings_deserializes_mcp_defaults_from_legacy_json() {
        let settings: AppSettings = serde_json::from_value(serde_json::json!({
            "locale": "en",
            "theme_mode": "dark"
        }))
        .expect("旧版 settings.json 应能读取");

        assert_eq!("en", settings.locale);
        assert!(!settings.mcp.server_enabled);
        assert_eq!(settings.mcp.permission_mode, McpPermissionMode::Deny);
        assert!(settings.mcp.toolsets.terminal);
        assert!(settings.mcp.toolsets.connections);
    }

    #[test]
    fn app_settings_default_system_hotkey_other_avoids_input_method_shortcut() {
        let settings = AppSettings::default();

        assert_eq!("ctrl-alt-m", settings.system_hotkey_other);
    }

    #[test]
    fn app_settings_deserializes_font_defaults_from_legacy_json() {
        let settings: AppSettings = serde_json::from_value(serde_json::json!({
            "locale": "en",
            "theme_mode": "dark"
        }))
        .expect("旧版 settings.json 应能读取");

        assert!(!settings.sql_editor_font_family.is_empty());
        assert_eq!(
            settings.sql_editor_font_family,
            settings.table_preview_font_family
        );
        assert_eq!(
            settings.sql_editor_font_family,
            settings.terminal_font_family
        );
        assert!(settings.custom_fonts.is_empty());
    }

    #[test]
    fn app_settings_round_trip_preserves_custom_fonts() {
        let mut settings = AppSettings::default();
        settings.custom_fonts = vec![CustomFont {
            path: "/tmp/NotoSansCJK-Regular.ttc".to_string(),
            families: vec!["Noto Sans Mono CJK SC".to_string()],
            monospace_families: vec!["Noto Sans Mono CJK SC".to_string()],
        }];

        let json = serde_json::to_string(&settings).expect("应序列化 AppSettings");
        let restored: AppSettings = serde_json::from_str(&json).expect("应反序列化 AppSettings");

        assert_eq!(
            vec![CustomFont {
                path: "/tmp/NotoSansCJK-Regular.ttc".to_string(),
                families: vec!["Noto Sans Mono CJK SC".to_string()],
                monospace_families: vec!["Noto Sans Mono CJK SC".to_string()],
            }],
            restored.custom_fonts
        );
    }

    #[test]
    fn app_settings_reads_legacy_custom_font_paths() {
        let settings: AppSettings = serde_json::from_value(serde_json::json!({
            "custom_font_paths": ["/tmp/NotoSansCJK-Regular.ttc"]
        }))
        .expect("旧版自定义字体路径应能读取");

        assert_eq!(
            vec![CustomFont {
                path: "/tmp/NotoSansCJK-Regular.ttc".to_string(),
                families: Vec::new(),
                monospace_families: Vec::new(),
            }],
            settings.custom_fonts
        );
    }

    #[test]
    fn app_settings_round_trip_preserves_mcp_settings() {
        let mut settings = AppSettings::default();
        settings.mcp.server_enabled = true;
        settings.mcp.server_mode = McpServerMode::Persistent;
        settings.mcp.permission_mode = McpPermissionMode::Ask;
        settings.mcp.toolsets.connections = false;
        settings.mcp.toolsets.database = true;
        settings.mcp.toolsets.redis = true;

        let json = serde_json::to_string(&settings).expect("应序列化 AppSettings");
        let loaded: AppSettings = serde_json::from_str(&json).expect("应反序列化 AppSettings");

        assert!(loaded.mcp.server_enabled);
        assert_eq!(loaded.mcp.server_mode, McpServerMode::Persistent);
        assert_eq!(loaded.mcp.permission_mode, McpPermissionMode::Ask);
        assert!(loaded.mcp.toolsets.terminal);
        assert!(!loaded.mcp.toolsets.connections);
        assert!(loaded.mcp.toolsets.database);
        assert!(loaded.mcp.toolsets.redis);
    }
}
