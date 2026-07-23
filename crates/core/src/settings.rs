use crate::cloud_sync::{GlobalCloudUser, UserInfo};
use crate::storage::get_config_dir;
use crate::utils::auto_save_config::AutoSaveConfig;
use gpui::http_client::Url;
use gpui::{App, Font, FontFallbacks, Global, font, px};
use gpui_component::{Theme, ThemeMode};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tracing::{error, info};

mod locale;
mod remote_file_editor;

pub use locale::{
    LOCALE_EN, LOCALE_SYSTEM, LOCALE_ZH_CN, LOCALE_ZH_HK, effective_locale_for_setting,
};
pub use remote_file_editor::{
    RemoteFileEditorOverride, RemoteFileEditorUserSettings, RemoteFileOpenMode,
};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartupDefaultPage {
    #[default]
    Home,
    AiWorkbench,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HomeConnectionLayout {
    #[default]
    Card,
    List,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HomePageStyle {
    Legacy,
    #[default]
    Modern,
}

impl HomePageStyle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::Modern => "modern",
        }
    }

    pub fn from_value(value: &str) -> Self {
        match value {
            "legacy" => Self::Legacy,
            _ => Self::Modern,
        }
    }

    pub fn uses_persistent_sidebar(self) -> bool {
        self == Self::Modern
    }
}

impl HomeConnectionLayout {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Card => "card",
            Self::List => "list",
        }
    }

    pub fn from_value(value: &str) -> Self {
        match value {
            "list" => Self::List,
            _ => Self::Card,
        }
    }
}

impl StartupDefaultPage {
    pub fn as_str(&self) -> &'static str {
        match self {
            StartupDefaultPage::Home => "home",
            StartupDefaultPage::AiWorkbench => "ai_workbench",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "home" => StartupDefaultPage::Home,
            _ => StartupDefaultPage::AiWorkbench,
        }
    }
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

    pub fn profile_id(&self) -> &'static str {
        match self {
            McpPermissionMode::Deny => "safe",
            McpPermissionMode::Ask => "confirm",
            McpPermissionMode::Allow => "auto",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolExposureToolsetSettings {
    #[serde(default = "default_true")]
    pub terminal: bool,
    #[serde(default = "default_true")]
    pub terminal_ssh_exec: bool,
    #[serde(default = "default_true")]
    pub terminal_exec: bool,
    #[serde(default = "default_true")]
    pub connections: bool,
    #[serde(default)]
    pub sftp: bool,
    #[serde(default)]
    pub database: bool,
    #[serde(default)]
    pub redis: bool,
    #[serde(default)]
    pub mongo: bool,
    #[serde(default)]
    pub internal_functions: bool,
}

impl ToolExposureToolsetSettings {
    pub fn public_mcp_default() -> Self {
        Self {
            terminal: true,
            terminal_ssh_exec: true,
            terminal_exec: true,
            connections: true,
            sftp: false,
            database: false,
            redis: false,
            mongo: false,
            internal_functions: false,
        }
    }

    pub fn agent_default() -> Self {
        Self {
            terminal: true,
            terminal_ssh_exec: true,
            terminal_exec: true,
            connections: true,
            sftp: true,
            database: true,
            redis: true,
            mongo: true,
            internal_functions: true,
        }
    }
}

impl Default for ToolExposureToolsetSettings {
    fn default() -> Self {
        Self::public_mcp_default()
    }
}

pub type McpToolsetSettings = ToolExposureToolsetSettings;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolExposureSettings {
    #[serde(default = "ToolExposureToolsetSettings::public_mcp_default")]
    pub mcp: ToolExposureToolsetSettings,
    #[serde(default = "ToolExposureToolsetSettings::agent_default")]
    pub agent: ToolExposureToolsetSettings,
}

impl Default for ToolExposureSettings {
    fn default() -> Self {
        Self {
            mcp: ToolExposureToolsetSettings::public_mcp_default(),
            agent: ToolExposureToolsetSettings::agent_default(),
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
    #[serde(default, rename = "toolsets", skip_serializing)]
    pub legacy_toolsets: Option<ToolExposureToolsetSettings>,
}

impl Default for McpSettings {
    fn default() -> Self {
        Self {
            server_enabled: false,
            server_mode: McpServerMode::Temporary,
            permission_mode: McpPermissionMode::Deny,
            legacy_toolsets: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiChatSettings {}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonalSyncBackendKind {
    #[default]
    Folder,
    Git,
}

impl PersonalSyncBackendKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Folder => "folder",
            Self::Git => "git",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "git" => Self::Git,
            _ => Self::Folder,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncProvider {
    #[default]
    OnetCloud,
    Personal,
}

impl SyncProvider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OnetCloud => "onet_cloud",
            Self::Personal => "personal",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "personal" => Self::Personal,
            _ => Self::OnetCloud,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalGitSyncSettings {
    #[serde(default = "default_true")]
    pub auto_push: bool,
}

impl Default for PersonalGitSyncSettings {
    fn default() -> Self {
        Self {
            auto_push: default_true(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalSyncSettings {
    #[serde(default)]
    pub backend: PersonalSyncBackendKind,
    #[serde(default)]
    pub path: String,
    #[serde(default = "default_true")]
    pub auto_sync: bool,
    #[serde(default)]
    pub git: PersonalGitSyncSettings,
}

impl Default for PersonalSyncSettings {
    fn default() -> Self {
        Self {
            backend: PersonalSyncBackendKind::Folder,
            path: String::new(),
            auto_sync: default_true(),
            git: PersonalGitSyncSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalTerminalProfileKind {
    #[default]
    System,
    Zsh,
    Bash,
    Fish,
    Nushell,
    PowerShell,
    Cmd,
    Wsl,
    GitBash,
    Custom,
}

impl LocalTerminalProfileKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Zsh => "zsh",
            Self::Bash => "bash",
            Self::Fish => "fish",
            Self::Nushell => "nushell",
            Self::PowerShell => "powershell",
            Self::Cmd => "cmd",
            Self::Wsl => "wsl",
            Self::GitBash => "git_bash",
            Self::Custom => "custom",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "powershell" => Self::PowerShell,
            "zsh" => Self::Zsh,
            "bash" => Self::Bash,
            "fish" => Self::Fish,
            "nushell" => Self::Nushell,
            "cmd" => Self::Cmd,
            "wsl" => Self::Wsl,
            "git_bash" => Self::GitBash,
            "custom" => Self::Custom,
            _ => Self::System,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LocalTerminalCustomProfile {
    pub id: String,
    pub name: String,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LocalTerminalProfileSettings {
    #[serde(default)]
    pub kind: LocalTerminalProfileKind,
    #[serde(default)]
    pub custom_program: String,
    #[serde(default)]
    pub custom_arguments: String,
    #[serde(default)]
    pub custom_profiles: Vec<LocalTerminalCustomProfile>,
    #[serde(default)]
    pub default_custom_profile_id: Option<String>,
}

impl LocalTerminalProfileSettings {
    pub fn effective_custom_profiles(&self) -> Vec<LocalTerminalCustomProfile> {
        let profiles: Vec<_> = self
            .custom_profiles
            .iter()
            .filter(|profile| !profile.name.trim().is_empty() && !profile.command.trim().is_empty())
            .cloned()
            .collect();
        if !profiles.is_empty() {
            return profiles;
        }
        let program = self.custom_program.trim();
        if program.is_empty() {
            return Vec::new();
        }
        let command = format_legacy_custom_command(program, self.custom_arguments.trim());
        vec![LocalTerminalCustomProfile {
            id: "legacy-custom".to_string(),
            name: program.to_string(),
            command,
        }]
    }

    pub fn selected_custom_profile(&self) -> Option<LocalTerminalCustomProfile> {
        let profiles = self.effective_custom_profiles();
        self.default_custom_profile_id
            .as_deref()
            .and_then(|id| profiles.iter().find(|profile| profile.id == id).cloned())
            .or_else(|| profiles.into_iter().next())
    }
}

fn format_legacy_custom_command(program: &str, arguments: &str) -> String {
    if arguments.is_empty() {
        program.to_string()
    } else {
        format!("{program} {arguments}")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default)]
    pub main_window_size: Option<MainWindowSize>,
    #[serde(default = "default_locale")]
    pub locale: String,
    #[serde(default = "default_theme_mode")]
    pub theme_mode: String,
    #[serde(default)]
    pub auto_switch_theme: bool,
    #[serde(default = "default_light_theme")]
    pub light_theme: String,
    #[serde(default = "default_dark_theme")]
    pub dark_theme: String,
    #[serde(default = "default_window_opacity")]
    pub window_opacity: f32,
    #[serde(default)]
    pub custom_accent_enabled: bool,
    #[serde(default = "default_custom_accent_color")]
    pub custom_accent_color: String,
    #[serde(default = "default_font_family")]
    pub font_family: String,
    #[serde(default = "default_font_size")]
    pub font_size: f64,
    #[serde(default = "default_monospace_font_family")]
    pub sql_editor_font_family: String,
    #[serde(default = "default_sql_editor_font_size")]
    pub sql_editor_font_size: f64,
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
    pub terminal_right_click_paste: bool,
    #[serde(default = "default_true")]
    pub terminal_paste_image_upload: bool,
    #[serde(default = "default_true")]
    pub terminal_sync_path_with_terminal: bool,
    #[serde(default)]
    pub terminal_cursor_blink: bool,
    #[serde(default = "default_true")]
    pub terminal_confirm_multiline_paste: bool,
    #[serde(default = "default_true")]
    pub terminal_confirm_high_risk_command: bool,
    #[serde(default)]
    pub local_terminal_profile: LocalTerminalProfileSettings,
    #[serde(default)]
    pub log_file_path: String,
    #[serde(default = "default_true")]
    pub auto_update: bool,
    #[serde(default)]
    pub sync_provider: SyncProvider,
    #[serde(default)]
    pub global_proxy: GlobalProxySettings,
    #[serde(default)]
    pub mcp: McpSettings,
    #[serde(default)]
    pub tool_exposure: ToolExposureSettings,
    #[serde(default)]
    pub ai_chat: AiChatSettings,
    #[serde(default)]
    pub personal_sync: PersonalSyncSettings,
    #[serde(default)]
    pub remote_file_editor: RemoteFileEditorUserSettings,
    #[serde(default)]
    pub database_open_mode: DatabaseOpenMode,
    #[serde(default)]
    pub large_text_cell_editor_open_mode: LargeTextCellEditorOpenMode,
    #[serde(default)]
    pub startup_default_page: StartupDefaultPage,
    /// 是否要求每次启动时输入主密钥后才能访问已保存的连接
    #[serde(default)]
    pub require_master_key_on_startup: bool,
    #[serde(default)]
    pub home_connection_layout: HomeConnectionLayout,
    #[serde(default)]
    pub home_page_style: HomePageStyle,
    #[serde(default = "default_true")]
    pub connection_sidebar_expanded: bool,
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

fn default_locale() -> String {
    LOCALE_SYSTEM.to_string()
}

fn default_theme_mode() -> String {
    "light".to_string()
}

fn default_light_theme() -> String {
    "Default Light".to_string()
}

fn default_dark_theme() -> String {
    "Default Dark".to_string()
}

fn default_window_opacity() -> f32 {
    1.0
}

fn default_custom_accent_color() -> String {
    "#3b82f6".to_string()
}

fn default_font_size() -> f64 {
    14.0
}

fn default_sql_editor_font_size() -> f64 {
    14.0
}

fn default_monospace_font_family() -> String {
    default_grid_monospace_font_family().to_string()
}

pub fn default_grid_monospace_font_family() -> &'static str {
    if cfg!(target_os = "macos") {
        "Menlo"
    } else if cfg!(target_os = "windows") {
        "Consolas"
    } else {
        "DejaVu Sans Mono"
    }
}

fn grid_monospace_resolution_candidates() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &[
            "Menlo",
            "Monaco",
            "SF Mono",
            "Courier New",
            "Fira Code",
            "JetBrains Mono",
            "Source Code Pro",
            "Cascadia Code",
            "Hack",
            "IBM Plex Mono",
        ]
    } else if cfg!(target_os = "windows") {
        &[
            "Consolas",
            "Cascadia Mono",
            "Cascadia Code",
            "Courier New",
            "Lucida Console",
            "Fira Code",
            "JetBrains Mono",
            "Source Code Pro",
            "Hack",
            "IBM Plex Mono",
        ]
    } else {
        &[
            "DejaVu Sans Mono",
            "Ubuntu Mono",
            "Liberation Mono",
            "Courier New",
            "Fira Code",
            "JetBrains Mono",
            "Source Code Pro",
            "Cascadia Code",
            "Hack",
            "IBM Plex Mono",
        ]
    }
}

fn is_fallback_only_grid_font(font: &str) -> bool {
    [
        "Apple Color Emoji",
        "Apple Symbols",
        "Heiti SC",
        "Hiragino Sans GB",
        "Kaiti SC",
        "Microsoft YaHei",
        "Noto Color Emoji",
        "Noto Sans CJK SC",
        "Noto Sans Mono CJK SC",
        "Noto Sans SC",
        "Noto Serif CJK SC",
        "PingFang SC",
        "PingFang TC",
        "Segoe UI Emoji",
        "SimSun",
        "Songti SC",
        "Source Han Mono SC",
        "Source Han Sans SC",
        "WenQuanYi Micro Hei",
    ]
    .iter()
    .any(|fallback| fallback.eq_ignore_ascii_case(font.trim()))
}

pub fn is_supported_grid_monospace_font(font: &str) -> bool {
    let font = font.trim();
    !font.is_empty() && !is_fallback_only_grid_font(font)
}

pub fn normalize_grid_monospace_font_family(font: &str) -> String {
    let font = font.trim();
    if is_supported_grid_monospace_font(font) {
        return font.to_string();
    }
    default_grid_monospace_font_family().to_string()
}

pub fn is_installed_font_family(font: &str, installed_font_names: &[String]) -> bool {
    let font = font.trim();
    !font.is_empty()
        && installed_font_names
            .iter()
            .any(|installed| installed.trim().eq_ignore_ascii_case(font))
}

pub fn resolve_installed_grid_monospace_font_family(
    font_family: &str,
    installed_font_names: &[String],
) -> String {
    let normalized = normalize_grid_monospace_font_family(font_family);
    if is_installed_font_family(&normalized, installed_font_names) {
        return normalized;
    }

    grid_monospace_resolution_candidates()
        .iter()
        .copied()
        .find(|candidate| is_installed_font_family(candidate, installed_font_names))
        .unwrap_or(default_grid_monospace_font_family())
        .to_string()
}

pub fn default_grid_font_fallback_families() -> Vec<String> {
    if cfg!(target_os = "macos") {
        vec![
            "PingFang SC",
            "PingFang TC",
            "Hiragino Sans GB",
            "Noto Sans CJK SC",
            "Noto Sans Mono CJK SC",
            "Source Han Sans SC",
            "Source Han Mono SC",
            "Apple Color Emoji",
            "Apple Symbols",
        ]
    } else if cfg!(target_os = "windows") {
        vec![
            "Microsoft YaHei",
            "SimSun",
            "Noto Sans CJK SC",
            "Noto Sans Mono CJK SC",
            "Source Han Sans SC",
            "Source Han Mono SC",
            "Segoe UI Emoji",
        ]
    } else {
        vec![
            "Noto Sans CJK SC",
            "Noto Sans Mono CJK SC",
            "Source Han Sans SC",
            "Source Han Mono SC",
            "WenQuanYi Micro Hei",
            "Noto Color Emoji",
        ]
    }
    .into_iter()
    .map(str::to_string)
    .collect()
}

pub fn grid_monospace_font(font_family: &str) -> Font {
    let mut font = font(normalize_grid_monospace_font_family(font_family));
    font.fallbacks = Some(FontFallbacks::from_fonts(
        default_grid_font_fallback_families(),
    ));
    font
}

pub fn installed_grid_monospace_font(font_family: &str, installed_font_names: &[String]) -> Font {
    let mut font = font(resolve_installed_grid_monospace_font_family(
        font_family,
        installed_font_names,
    ));
    font.fallbacks = Some(FontFallbacks::from_fonts(
        default_grid_font_fallback_families(),
    ));
    font
}

fn default_terminal_font_size() -> f64 {
    15.0
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
            main_window_size: None,
            locale: default_locale(),
            theme_mode: default_theme_mode(),
            auto_switch_theme: false,
            light_theme: default_light_theme(),
            dark_theme: default_dark_theme(),
            window_opacity: default_window_opacity(),
            custom_accent_enabled: false,
            custom_accent_color: default_custom_accent_color(),
            font_family: default_font_family(),
            font_size: default_font_size(),
            sql_editor_font_family: default_monospace_font_family(),
            sql_editor_font_size: default_sql_editor_font_size(),
            table_preview_font_family: default_monospace_font_family(),
            terminal_font_family: default_monospace_font_family(),
            custom_fonts: Vec::new(),
            terminal_font_size: default_terminal_font_size(),
            terminal_auto_copy: default_true(),
            terminal_enable_autocomplete: default_true(),
            terminal_middle_click_paste: default_true(),
            terminal_right_click_paste: false,
            terminal_paste_image_upload: default_true(),
            terminal_sync_path_with_terminal: true,
            terminal_cursor_blink: false,
            terminal_confirm_multiline_paste: default_true(),
            terminal_confirm_high_risk_command: default_true(),
            local_terminal_profile: LocalTerminalProfileSettings::default(),
            log_file_path: String::new(),
            auto_update: true,
            sync_provider: SyncProvider::OnetCloud,
            global_proxy: GlobalProxySettings::default(),
            mcp: McpSettings::default(),
            tool_exposure: ToolExposureSettings::default(),
            ai_chat: AiChatSettings::default(),
            personal_sync: PersonalSyncSettings::default(),
            remote_file_editor: RemoteFileEditorUserSettings::default(),
            database_open_mode: DatabaseOpenMode::default(),
            large_text_cell_editor_open_mode: LargeTextCellEditorOpenMode::default(),
            startup_default_page: StartupDefaultPage::default(),
            require_master_key_on_startup: false,
            home_connection_layout: HomeConnectionLayout::default(),
            home_page_style: HomePageStyle::default(),
            connection_sidebar_expanded: true,
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

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MainWindowSize {
    pub width: f32,
    pub height: f32,
}

impl MainWindowSize {
    pub fn new(width: f32, height: f32) -> Option<Self> {
        (width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0)
            .then_some(Self { width, height })
    }
}

impl Global for AppSettings {}

impl AppSettings {
    pub const MIN_WINDOW_OPACITY: f32 = 0.5;
    pub const MAX_WINDOW_OPACITY: f32 = 1.0;

    fn migrate_legacy_mcp_toolsets(&mut self) {
        let Some(toolsets) = self.mcp.legacy_toolsets.take() else {
            return;
        };
        if self.tool_exposure.mcp == ToolExposureToolsetSettings::public_mcp_default() {
            self.tool_exposure.mcp = toolsets;
        }
    }

    pub fn normalize_font_settings(&mut self) {
        self.sql_editor_font_family =
            normalize_grid_monospace_font_family(&self.sql_editor_font_family);
        self.table_preview_font_family =
            normalize_grid_monospace_font_family(&self.table_preview_font_family);
        self.terminal_font_family =
            normalize_grid_monospace_font_family(&self.terminal_font_family);
    }

    pub fn normalize_appearance_settings(&mut self) {
        if self.auto_switch_theme {
            self.theme_mode = "system".to_string();
        } else if !matches!(self.theme_mode.as_str(), "light" | "system" | "dark") {
            self.theme_mode = default_theme_mode();
        }
        self.window_opacity = self
            .window_opacity
            .clamp(Self::MIN_WINDOW_OPACITY, Self::MAX_WINDOW_OPACITY);
        if self.custom_accent_color.trim().is_empty() {
            self.custom_accent_color = default_custom_accent_color();
        }
    }

    pub fn effective_theme_mode(&self, system_mode: ThemeMode) -> ThemeMode {
        if self.auto_switch_theme || self.theme_mode == "system" {
            system_mode
        } else if self.theme_mode == "dark" {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        }
    }

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
        settings.normalize_font_settings();
        settings.normalize_appearance_settings();
        cx.set_global(settings);
    }

    pub fn update_and_save(cx: &mut App, update: impl FnOnce(&mut AppSettings)) {
        let mut settings = Self::current(cx);
        update(&mut settings);
        settings.normalize_font_settings();
        settings.normalize_appearance_settings();
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
            Ok(content) => match serde_json::from_str::<Self>(&content) {
                Ok(mut settings) => {
                    info!("Settings loaded from {:?}", path);
                    settings.migrate_legacy_mcp_toolsets();
                    settings.normalize_font_settings();
                    settings.normalize_appearance_settings();
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
        gpui_component::set_locale(effective_locale_for_setting(&self.locale));
        crate::themes::apply_appearance(self, cx);
        self.apply_font_size(cx);

        // 同步自动保存配置
        self.sync_auto_save_config(cx);
    }

    pub fn apply_font_size(&self, cx: &mut App) {
        Theme::global_mut(cx).font_size = px(self.font_size as f32);
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
    use gpui::px;
    use gpui_component::{Theme, ThemeMode};

    use super::{
        AppSettings, CustomFont, HomeConnectionLayout, HomePageStyle, LOCALE_SYSTEM,
        LargeTextCellEditorOpenMode, LocalTerminalProfileKind, LocalTerminalProfileSettings,
        McpPermissionMode, McpServerMode, PersonalSyncBackendKind, RemoteFileOpenMode,
        StartupDefaultPage, SyncProvider, default_grid_font_fallback_families,
        default_grid_monospace_font_family, grid_monospace_font, installed_grid_monospace_font,
        is_installed_font_family, resolve_installed_grid_monospace_font_family,
    };

    #[test]
    fn remote_file_editor_settings_default_to_builtin_with_conflict_check() {
        let settings = AppSettings::default();

        assert_eq!(
            RemoteFileOpenMode::BuiltIn,
            settings.remote_file_editor.open_mode
        );
        assert!(
            settings
                .remote_file_editor
                .check_remote_modified_before_upload
        );
        assert!(settings.remote_file_editor.auto_upload_external_changes);
        assert!(
            settings
                .remote_file_editor
                .default_external_editor
                .is_none()
        );
        assert!(settings.remote_file_editor.overrides.is_empty());
    }

    #[test]
    fn app_settings_deserializes_remote_file_editor_defaults() {
        let settings: AppSettings = serde_json::from_value(serde_json::json!({
            "locale": "en",
            "theme_mode": "dark"
        }))
        .expect("legacy settings should deserialize");

        assert_eq!(
            RemoteFileOpenMode::BuiltIn,
            settings.remote_file_editor.open_mode
        );
        assert!(
            settings
                .remote_file_editor
                .check_remote_modified_before_upload
        );
        assert!(settings.remote_file_editor.auto_upload_external_changes);
    }

    #[test]
    fn appearance_settings_migrate_with_safe_defaults() {
        let settings: AppSettings = serde_json::from_value(serde_json::json!({
            "locale": "en",
            "auto_switch_theme": false
        }))
        .expect("legacy appearance settings should deserialize");

        assert_eq!("light", settings.theme_mode);
        assert_eq!("Default Light", settings.light_theme);
        assert_eq!("Default Dark", settings.dark_theme);
        assert_eq!(1.0, settings.window_opacity);
        assert!(!settings.custom_accent_enabled);
        assert_eq!("#3b82f6", settings.custom_accent_color);
    }

    #[test]
    fn appearance_settings_normalize_opacity_and_mode() {
        let mut settings = AppSettings {
            theme_mode: "unknown".to_string(),
            window_opacity: 2.0,
            custom_accent_color: String::new(),
            ..AppSettings::default()
        };

        settings.normalize_appearance_settings();

        assert_eq!("light", settings.theme_mode);
        assert_eq!(1.0, settings.window_opacity);
        assert_eq!("#3b82f6", settings.custom_accent_color);

        settings.window_opacity = 0.1;
        settings.normalize_appearance_settings();
        assert_eq!(AppSettings::MIN_WINDOW_OPACITY, settings.window_opacity);

        settings.auto_switch_theme = true;
        settings.normalize_appearance_settings();
        assert_eq!("system", settings.theme_mode);
    }

    #[test]
    fn appearance_settings_resolve_light_dark_and_system_modes() {
        let mut settings = AppSettings::default();

        assert_eq!(
            ThemeMode::Light,
            settings.effective_theme_mode(ThemeMode::Dark)
        );

        settings.theme_mode = "dark".to_string();
        assert_eq!(
            ThemeMode::Dark,
            settings.effective_theme_mode(ThemeMode::Light)
        );

        settings.theme_mode = "system".to_string();
        assert_eq!(
            ThemeMode::Dark,
            settings.effective_theme_mode(ThemeMode::Dark)
        );

        settings.theme_mode = "light".to_string();
        settings.auto_switch_theme = true;
        assert_eq!(
            ThemeMode::Dark,
            settings.effective_theme_mode(ThemeMode::Dark)
        );
    }

    #[test]
    fn app_settings_deserializes_disabled_external_auto_upload() {
        let settings: AppSettings = serde_json::from_value(serde_json::json!({
            "locale": "en",
            "theme_mode": "dark",
            "remote_file_editor": {
                "auto_upload_external_changes": false
            }
        }))
        .expect("settings should deserialize");

        assert!(!settings.remote_file_editor.auto_upload_external_changes);
    }

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
        assert!(settings.tool_exposure.mcp.terminal);
        assert!(settings.tool_exposure.mcp.terminal_ssh_exec);
        assert!(settings.tool_exposure.mcp.terminal_exec);
        assert!(settings.tool_exposure.mcp.connections);
        assert!(!settings.tool_exposure.mcp.database);
        assert!(!settings.tool_exposure.mcp.redis);
        assert!(!settings.tool_exposure.mcp.sftp);
        assert!(settings.tool_exposure.agent.terminal);
        assert!(settings.tool_exposure.agent.terminal_ssh_exec);
        assert!(settings.tool_exposure.agent.terminal_exec);
        assert!(settings.tool_exposure.agent.connections);
        assert!(settings.tool_exposure.agent.database);
        assert!(settings.tool_exposure.agent.redis);
        assert!(settings.tool_exposure.agent.sftp);
    }

    #[test]
    fn mcp_permission_modes_expose_unified_profile_ids() {
        assert_eq!("safe", McpPermissionMode::Deny.profile_id());
        assert_eq!("confirm", McpPermissionMode::Ask.profile_id());
        assert_eq!("auto", McpPermissionMode::Allow.profile_id());
    }

    #[test]
    fn app_settings_default_sets_sql_query_max_rows() {
        let settings = AppSettings::default();

        assert_eq!(1000, settings.sql_query_max_rows);
    }

    #[test]
    fn app_settings_default_enables_terminal_file_manager_path_sync() {
        let settings = AppSettings::default();

        assert!(settings.terminal_sync_path_with_terminal);
    }

    #[test]
    fn app_settings_deserializes_terminal_file_manager_path_sync_enabled_by_default() {
        let settings: AppSettings = serde_json::from_value(serde_json::json!({
            "locale": "en",
            "theme_mode": "dark"
        }))
        .expect("旧版 settings.json 应能读取");

        assert!(settings.terminal_sync_path_with_terminal);
    }

    #[test]
    fn app_settings_disables_terminal_right_click_paste_by_default() {
        let settings = AppSettings::default();

        assert!(!settings.terminal_right_click_paste);
    }

    #[test]
    fn app_settings_enables_terminal_paste_image_upload_by_default() {
        let settings = AppSettings::default();

        assert!(settings.terminal_paste_image_upload);
    }

    #[test]
    fn app_settings_default_opens_home_on_startup() {
        let settings = AppSettings::default();

        assert_eq!(StartupDefaultPage::Home, settings.startup_default_page);
    }

    #[test]
    fn app_settings_does_not_require_master_key_on_startup_by_default() {
        assert!(!AppSettings::default().require_master_key_on_startup);
    }

    #[test]
    fn app_settings_deserializes_master_key_startup_lock() {
        let settings: AppSettings = serde_json::from_value(serde_json::json!({
            "require_master_key_on_startup": true
        }))
        .expect("require_master_key_on_startup 应能读取");

        assert!(settings.require_master_key_on_startup);
    }

    #[test]
    fn app_settings_defaults_to_modern_home_with_cards_and_expanded_sidebar() {
        let settings = AppSettings::default();

        assert_eq!(HomeConnectionLayout::Card, settings.home_connection_layout);
        assert_eq!(HomePageStyle::Modern, settings.home_page_style);
        assert!(settings.connection_sidebar_expanded);
    }

    #[test]
    fn app_settings_deserializes_connection_display_preferences() {
        let settings: AppSettings = serde_json::from_value(serde_json::json!({
            "home_connection_layout": "list",
            "home_page_style": "legacy",
            "connection_sidebar_expanded": false
        }))
        .expect("connection display preferences should deserialize");

        assert_eq!(HomeConnectionLayout::List, settings.home_connection_layout);
        assert_eq!(HomePageStyle::Legacy, settings.home_page_style);
        assert!(!settings.connection_sidebar_expanded);
    }

    #[test]
    fn app_settings_deserializes_home_startup_default_page_from_legacy_json() {
        let settings: AppSettings = serde_json::from_value(serde_json::json!({
            "locale": "en",
            "theme_mode": "dark"
        }))
        .expect("旧版 settings.json 应能读取");

        assert_eq!(StartupDefaultPage::Home, settings.startup_default_page);
    }

    #[test]
    fn app_settings_deserializes_home_startup_default_page() {
        let settings: AppSettings = serde_json::from_value(serde_json::json!({
            "startup_default_page": "home"
        }))
        .expect("startup_default_page 应能读取");

        assert_eq!(StartupDefaultPage::Home, settings.startup_default_page);
    }

    #[test]
    fn app_settings_default_follows_system_locale() {
        let settings = AppSettings::default();

        assert_eq!(LOCALE_SYSTEM, settings.locale);
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
    fn app_settings_deserializes_missing_locale_as_system_mode() {
        let settings: AppSettings = serde_json::from_value(serde_json::json!({
            "theme_mode": "dark"
        }))
        .expect("缺少 locale 的旧版 settings.json 应能读取");

        assert_eq!(LOCALE_SYSTEM, settings.locale);
    }

    #[test]
    fn app_settings_default_disables_personal_sync() {
        let settings = AppSettings::default();

        assert_eq!(
            PersonalSyncBackendKind::Folder,
            settings.personal_sync.backend
        );
        assert!(settings.personal_sync.path.is_empty());
        assert!(settings.personal_sync.auto_sync);
        assert!(settings.personal_sync.git.auto_push);
    }

    #[test]
    fn app_settings_deserializes_personal_sync_defaults_from_legacy_json() {
        let settings: AppSettings = serde_json::from_value(serde_json::json!({
            "locale": "en",
            "theme_mode": "dark"
        }))
        .expect("旧版 settings.json 应能读取");

        assert!(settings.personal_sync.auto_sync);
    }

    #[test]
    fn app_settings_default_uses_onet_cloud_sync_provider() {
        let settings = AppSettings::default();

        assert_eq!(SyncProvider::OnetCloud, settings.sync_provider);
    }

    #[test]
    fn app_settings_deserializes_personal_sync_provider() {
        let settings: AppSettings = serde_json::from_value(serde_json::json!({
            "sync_provider": "personal"
        }))
        .expect("应能读取个人同步模式");

        assert_eq!(SyncProvider::Personal, settings.sync_provider);
    }

    #[test]
    fn app_settings_round_trip_preserves_personal_sync() {
        let mut settings = AppSettings::default();
        settings.personal_sync.backend = PersonalSyncBackendKind::Git;
        settings.personal_sync.path = "/tmp/repo".to_string();
        settings.personal_sync.git.auto_push = false;

        let json = serde_json::to_string(&settings).expect("应序列化 AppSettings");
        let loaded: AppSettings = serde_json::from_str(&json).expect("应反序列化 AppSettings");

        assert_eq!(settings.personal_sync, loaded.personal_sync);
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
        assert!(settings.tool_exposure.mcp.terminal);
        assert!(settings.tool_exposure.mcp.terminal_ssh_exec);
        assert!(settings.tool_exposure.mcp.terminal_exec);
        assert!(settings.tool_exposure.mcp.connections);
        assert!(settings.tool_exposure.agent.database);
    }

    #[test]
    fn app_settings_migrates_legacy_mcp_toolsets_to_tool_exposure() {
        let mut settings: AppSettings = serde_json::from_value(serde_json::json!({
            "mcp": {
                "toolsets": {
                    "terminal": false,
                    "connections": false,
                    "database": true,
                    "redis": true
                }
            }
        }))
        .expect("旧版 mcp.toolsets 应能读取");

        assert!(settings.mcp.legacy_toolsets.is_some());
        settings.migrate_legacy_mcp_toolsets();

        assert!(settings.mcp.legacy_toolsets.is_none());
        assert!(!settings.tool_exposure.mcp.terminal);
        assert!(!settings.tool_exposure.mcp.connections);
        assert!(settings.tool_exposure.mcp.database);
        assert!(settings.tool_exposure.mcp.redis);
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
            AppSettings::default().sql_editor_font_size,
            settings.sql_editor_font_size
        );
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

    #[gpui::test]
    fn app_settings_apply_updates_theme_font_size(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            cx.set_global(Theme::default());

            let mut settings = AppSettings::default();
            settings.font_size = 18.0;

            settings.apply(cx);

            assert_eq!(px(18.0), Theme::global(cx).font_size);
        });
    }

    #[test]
    fn app_settings_normalizes_grid_font_settings() {
        let mut settings = AppSettings {
            sql_editor_font_family: "Noto Sans Mono CJK SC".to_string(),
            table_preview_font_family: "PingFang SC".to_string(),
            terminal_font_family: "Microsoft YaHei".to_string(),
            ..AppSettings::default()
        };

        settings.normalize_font_settings();

        let default = default_grid_monospace_font_family();
        assert_eq!(default, settings.sql_editor_font_family);
        assert_eq!(default, settings.table_preview_font_family);
        assert_eq!(default, settings.terminal_font_family);
    }

    #[test]
    fn app_settings_keeps_grid_safe_custom_monospace_fonts() {
        let mut settings = AppSettings {
            sql_editor_font_family: "JetBrains Mono".to_string(),
            table_preview_font_family: "Table Safe Mono".to_string(),
            terminal_font_family: "Custom Mono".to_string(),
            ..AppSettings::default()
        };

        settings.normalize_font_settings();

        assert_eq!("JetBrains Mono", settings.sql_editor_font_family);
        assert_eq!("Table Safe Mono", settings.table_preview_font_family);
        assert_eq!("Custom Mono", settings.terminal_font_family);
    }

    #[test]
    fn default_grid_font_fallbacks_include_cjk_before_symbol_fonts() {
        let fallbacks = default_grid_font_fallback_families();
        let cjk_index = fallbacks
            .iter()
            .position(|font| font == "Noto Sans CJK SC" || font == "Microsoft YaHei")
            .expect("grid font fallback should include a CJK font");

        for symbol_font in ["Apple Color Emoji", "Apple Symbols", "Noto Color Emoji"] {
            if let Some(symbol_index) = fallbacks.iter().position(|font| font == symbol_font) {
                assert!(cjk_index < symbol_index);
            }
        }
    }

    #[test]
    fn grid_monospace_font_normalizes_family_and_sets_fallbacks() {
        let font = grid_monospace_font("PingFang SC");

        assert_eq!(default_grid_monospace_font_family(), font.family.as_ref());
        assert!(font.fallbacks.as_ref().is_some_and(|fallbacks| {
            fallbacks.fallback_list().iter().any(|family| {
                family == "Noto Sans CJK SC"
                    || family == "Microsoft YaHei"
                    || family == "PingFang SC"
            })
        }));
    }

    #[test]
    fn installed_font_family_matches_case_insensitively() {
        let installed = vec!["Menlo".to_string(), "JetBrains Mono".to_string()];

        assert!(is_installed_font_family(" jetbrains mono ", &installed));
        assert!(!is_installed_font_family("Fira Code", &installed));
    }

    #[test]
    fn resolve_installed_grid_font_rejects_missing_requested_family() {
        let default = default_grid_monospace_font_family();
        let installed = vec![default.to_string(), "PingFang SC".to_string()];

        assert_eq!(
            default,
            resolve_installed_grid_monospace_font_family("Fira Code", &installed)
        );
        assert_eq!(
            default,
            resolve_installed_grid_monospace_font_family("PingFang SC", &installed)
        );
    }

    #[test]
    fn resolve_installed_grid_font_keeps_installed_requested_family() {
        let installed = vec!["Menlo".to_string(), "JetBrains Mono".to_string()];

        assert_eq!(
            "JetBrains Mono",
            resolve_installed_grid_monospace_font_family("JetBrains Mono", &installed)
        );
    }

    #[test]
    fn installed_grid_monospace_font_uses_effective_installed_family() {
        let default = default_grid_monospace_font_family();
        let installed = vec![default.to_string()];
        let font = installed_grid_monospace_font("Fira Code", &installed);

        assert_eq!(default, font.family.as_ref());
        assert!(font.fallbacks.is_some());
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
        settings.tool_exposure.mcp.connections = false;
        settings.tool_exposure.mcp.database = true;
        settings.tool_exposure.mcp.redis = true;
        settings.tool_exposure.agent.terminal_exec = false;

        let json = serde_json::to_string(&settings).expect("应序列化 AppSettings");
        let loaded: AppSettings = serde_json::from_str(&json).expect("应反序列化 AppSettings");

        assert!(loaded.mcp.server_enabled);
        assert_eq!(loaded.mcp.server_mode, McpServerMode::Persistent);
        assert_eq!(loaded.mcp.permission_mode, McpPermissionMode::Ask);
        assert!(loaded.tool_exposure.mcp.terminal);
        assert!(loaded.tool_exposure.mcp.terminal_ssh_exec);
        assert!(loaded.tool_exposure.mcp.terminal_exec);
        assert!(!loaded.tool_exposure.mcp.connections);
        assert!(loaded.tool_exposure.mcp.database);
        assert!(loaded.tool_exposure.mcp.redis);
        assert!(!loaded.tool_exposure.agent.terminal_exec);
    }

    #[test]
    fn local_terminal_profile_defaults_to_system() {
        let settings = AppSettings::default();

        assert_eq!(
            LocalTerminalProfileKind::System,
            settings.local_terminal_profile.kind
        );
        assert!(settings.local_terminal_profile.custom_program.is_empty());
        assert!(settings.local_terminal_profile.custom_arguments.is_empty());
    }

    #[test]
    fn local_terminal_profile_round_trip_preserves_custom_command() {
        let mut settings = AppSettings::default();
        settings.local_terminal_profile = LocalTerminalProfileSettings {
            kind: LocalTerminalProfileKind::Custom,
            custom_program: "/opt/homebrew/bin/fish".to_string(),
            custom_arguments: "--login -C 'echo ready'".to_string(),
            ..Default::default()
        };

        let json = serde_json::to_string(&settings).unwrap();
        let restored: AppSettings = serde_json::from_str(&json).unwrap();

        assert_eq!(
            LocalTerminalProfileKind::Custom,
            restored.local_terminal_profile.kind
        );
        assert_eq!(
            "/opt/homebrew/bin/fish",
            restored.local_terminal_profile.custom_program
        );
        assert_eq!(
            "--login -C 'echo ready'",
            restored.local_terminal_profile.custom_arguments
        );
        let profile = restored
            .local_terminal_profile
            .selected_custom_profile()
            .unwrap();
        assert_eq!("/opt/homebrew/bin/fish", profile.name);
        assert_eq!(
            "/opt/homebrew/bin/fish --login -C 'echo ready'",
            profile.command
        );
    }
}
