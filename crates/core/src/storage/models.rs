use crate::cloud_sync::sync_type::SyncableItem;
use crate::crypto;
use crate::storage::credential_vault::CredentialReference;
use crate::storage::traits::Entity;
use connection_tunnel::SshTunnelConfig;
use gpui::Global;
use gpui_component::Size::Large;
use gpui_component::{Icon, IconName, Sizable};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex};

use super::rdp_settings::RdpSettings;

/// 活跃连接状态 - 用于跟踪哪些连接当前已打开
#[derive(Clone, Default)]
pub struct ActiveConnections {
    active_ids: Arc<Mutex<HashSet<i64>>>,
}

pub struct ActiveConnectionLease {
    active_ids: Arc<Mutex<HashSet<i64>>>,
    conn_id: i64,
}

impl Global for ActiveConnections {}

impl ActiveConnections {
    pub fn new() -> Self {
        Self {
            active_ids: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub fn add(&mut self, conn_id: i64) {
        self.active_ids
            .lock()
            .expect("active connections poisoned")
            .insert(conn_id);
    }

    pub fn remove(&mut self, conn_id: i64) {
        self.active_ids
            .lock()
            .expect("active connections poisoned")
            .remove(&conn_id);
    }

    pub fn is_active(&self, conn_id: i64) -> bool {
        self.active_ids
            .lock()
            .expect("active connections poisoned")
            .contains(&conn_id)
    }

    pub fn active_count(&self) -> usize {
        self.active_ids
            .lock()
            .expect("active connections poisoned")
            .len()
    }

    pub fn lease(&mut self, conn_id: i64) -> ActiveConnectionLease {
        self.add(conn_id);
        ActiveConnectionLease {
            active_ids: Arc::clone(&self.active_ids),
            conn_id,
        }
    }
}

impl Drop for ActiveConnectionLease {
    fn drop(&mut self) {
        if let Ok(mut active_ids) = self.active_ids.lock() {
            active_ids.remove(&self.conn_id);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ConnectionType {
    All,
    Database,
    SshSftp,
    Redis,
    MongoDB,
    Mqtt,
    Serial,
    Telnet,
    PortForwarding,
    Rdp,
    Vnc,
    Extension,
}

impl fmt::Display for ConnectionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            ConnectionType::All => "All",
            ConnectionType::Database => "Database",
            ConnectionType::SshSftp => "SshSftp",
            ConnectionType::Redis => "Redis",
            ConnectionType::MongoDB => "MongoDB",
            ConnectionType::Mqtt => "Mqtt",
            ConnectionType::Serial => "Serial",
            ConnectionType::Telnet => "Telnet",
            ConnectionType::PortForwarding => "PortForwarding",
            ConnectionType::Rdp => "Rdp",
            ConnectionType::Vnc => "Vnc",
            ConnectionType::Extension => "Extension",
        };
        write!(f, "{}", s)
    }
}

impl ConnectionType {
    pub fn all() -> Vec<ConnectionType> {
        vec![
            ConnectionType::All,
            ConnectionType::SshSftp,
            ConnectionType::Database,
            ConnectionType::Redis,
            ConnectionType::MongoDB,
            ConnectionType::Mqtt,
            ConnectionType::Serial,
            ConnectionType::Telnet,
            ConnectionType::PortForwarding,
            ConnectionType::Rdp,
            ConnectionType::Vnc,
            ConnectionType::Extension,
        ]
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "Database" => ConnectionType::Database,
            "SshSftp" => ConnectionType::SshSftp,
            "Redis" => ConnectionType::Redis,
            "MongoDB" => ConnectionType::MongoDB,
            "Mqtt" => ConnectionType::Mqtt,
            "Serial" => ConnectionType::Serial,
            "Telnet" => ConnectionType::Telnet,
            "PortForwarding" => ConnectionType::PortForwarding,
            "Rdp" => ConnectionType::Rdp,
            "Vnc" => ConnectionType::Vnc,
            "Extension" => ConnectionType::Extension,
            _ => ConnectionType::Database,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ConnectionType::All => "All",
            ConnectionType::Database => "Database",
            ConnectionType::SshSftp => "SSH/SFTP",
            ConnectionType::Redis => "Redis",
            ConnectionType::MongoDB => "MongoDB",
            ConnectionType::Mqtt => "MQTT",
            ConnectionType::Serial => "Serial",
            ConnectionType::Telnet => "Telnet",
            ConnectionType::PortForwarding => "Port Forwarding",
            ConnectionType::Rdp => "RDP",
            ConnectionType::Vnc => "VNC",
            ConnectionType::Extension => "Extension",
        }
    }

    pub fn icon(&self) -> IconName {
        match self {
            ConnectionType::All => IconName::Server,
            ConnectionType::Database => IconName::Database,
            ConnectionType::SshSftp => IconName::TerminalColor,
            ConnectionType::Redis => IconName::Redis,
            ConnectionType::MongoDB => IconName::MongoDB,
            // 外部 gpui-component 未提供 MQTT 品牌图标,
            // 核心层回退通用网络图标;品牌图标经应用 AssetSource 提供
            ConnectionType::Mqtt => IconName::Network,
            ConnectionType::Serial => IconName::SerialPort,
            ConnectionType::Telnet => IconName::SquareTerminalColor,
            ConnectionType::PortForwarding => IconName::PortForwardingColor,
            ConnectionType::Rdp => IconName::Rdp,
            ConnectionType::Vnc => IconName::Vnc,
            ConnectionType::Extension => IconName::ExtensionsColor,
        }
    }
}

/// Navop 自带品牌图标的资源路径。
///
/// 外部 gpui-component 的 `IconName` 由其资产宏生成,无法在本仓库扩展变体;
/// TDengine/MQTT 品牌图标以 SVG 形式内嵌于应用(main 的 `AppAssets`),
/// 通过 `Icon::default().path(...)` 按路径引用。
pub const NAVOP_TDENGINE_COLOR_ICON: &str = "navop/tdengine-color.svg";
pub const NAVOP_TDENGINE_LINE_COLOR_ICON: &str = "navop/tdengine-line-color.svg";
pub const NAVOP_MQTT_COLOR_ICON: &str = "navop/mqtt-color.svg";
pub const NAVOP_MQTT_LINE_ICON: &str = "navop/mqtt-line.svg";

/// Database type enumeration
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DatabaseType {
    MySQL,
    PostgreSQL,
    SQLite,
    DuckDB,
    MSSQL,
    Oracle,
    ClickHouse,
    /// TDengine 时序数据库(官方 taos ws 驱动,经 taosAdapter 连接)
    TDengine,
    External {
        driver_id: String,
    },
}

impl DatabaseType {
    pub fn all() -> &'static [DatabaseType] {
        Self::builtin_all()
    }

    pub fn builtin_all() -> &'static [DatabaseType] {
        &[
            DatabaseType::MySQL,
            DatabaseType::PostgreSQL,
            DatabaseType::SQLite,
            DatabaseType::DuckDB,
            DatabaseType::MSSQL,
            DatabaseType::Oracle,
            DatabaseType::ClickHouse,
            DatabaseType::TDengine,
        ]
    }

    pub fn external(driver_id: impl Into<String>) -> Self {
        Self::External {
            driver_id: driver_id.into(),
        }
    }

    pub fn is_external(&self) -> bool {
        matches!(self, Self::External { .. })
    }

    pub fn external_driver_id(&self) -> Option<&str> {
        match self {
            Self::External { driver_id } => Some(driver_id),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            DatabaseType::MySQL => "MySQL",
            DatabaseType::PostgreSQL => "PostgreSQL",
            DatabaseType::SQLite => "SQLite",
            DatabaseType::DuckDB => "DuckDB",
            DatabaseType::MSSQL => "MSSQL",
            DatabaseType::Oracle => "Oracle",
            DatabaseType::ClickHouse => "ClickHouse",
            DatabaseType::TDengine => "TDengine",
            DatabaseType::External { .. } => "External",
        }
    }

    pub fn storage_key(&self) -> String {
        match self {
            DatabaseType::External { driver_id } => format!("External:{driver_id}"),
            _ => self.as_str().to_string(),
        }
    }

    pub fn path_key(&self) -> String {
        self.storage_key()
            .replace([':', '/', '\\', '<', '>', '|', '?', '*', '.'], "_")
    }

    pub fn from_storage_key(s: &str) -> Option<Self> {
        if let Some(driver_id) = s.strip_prefix("External:") {
            if driver_id.is_empty() {
                return None;
            }
            return Some(DatabaseType::external(driver_id));
        }
        Self::from_str(s)
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "MySQL" => Some(DatabaseType::MySQL),
            "PostgreSQL" => Some(DatabaseType::PostgreSQL),
            "SQLite" => Some(DatabaseType::SQLite),
            "DuckDB" => Some(DatabaseType::DuckDB),
            "MSSQL" => Some(DatabaseType::MSSQL),
            "Oracle" => Some(DatabaseType::Oracle),
            "ClickHouse" => Some(DatabaseType::ClickHouse),
            "TDengine" => Some(DatabaseType::TDengine),
            _ => None,
        }
    }

    pub fn as_icon(&self) -> Icon {
        match self {
            DatabaseType::MySQL => IconName::MySQLColor.color().with_size(Large),
            DatabaseType::PostgreSQL => IconName::PostgreSQLColor.color().with_size(Large),
            DatabaseType::SQLite => IconName::SQLiteColor.color().with_size(Large),
            DatabaseType::DuckDB => IconName::DuckDB.color().with_size(Large),
            DatabaseType::MSSQL => IconName::MSSQLColor.color().with_size(Large),
            DatabaseType::Oracle => IconName::OracleColor.color().with_size(Large),
            DatabaseType::ClickHouse => IconName::ClickHouseColor.color().with_size(Large),
            DatabaseType::TDengine => Icon::default()
                .path(NAVOP_TDENGINE_COLOR_ICON)
                .color()
                .with_size(Large),
            DatabaseType::External { .. } => IconName::Database.color().with_size(Large),
        }
    }
    pub fn as_node_icon(&self) -> Icon {
        match self {
            DatabaseType::MySQL => IconName::MySQLLineColor.color().with_size(Large),
            DatabaseType::PostgreSQL => IconName::PostgreSQLLineColor.color().with_size(Large),
            DatabaseType::SQLite => IconName::SQLiteLineColor.color().with_size(Large),
            DatabaseType::DuckDB => IconName::DuckDB.color().with_size(Large),
            DatabaseType::MSSQL => IconName::MSSQLLineColor.color().with_size(Large),
            DatabaseType::Oracle => IconName::OracleLineColor.color().with_size(Large),
            DatabaseType::ClickHouse => IconName::ClickHouseLineColor.color().with_size(Large),
            DatabaseType::TDengine => Icon::default()
                .path(NAVOP_TDENGINE_LINE_COLOR_ICON)
                .color()
                .with_size(Large),
            DatabaseType::External { .. } => IconName::Database.color().with_size(Large),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoredTerminalEncoding {
    #[default]
    Utf8,
    Gbk,
    Gb18030,
    Big5,
    ShiftJis,
    EucJp,
    EucKr,
    Windows1252,
}

impl StoredTerminalEncoding {
    pub const fn all() -> &'static [Self] {
        &[
            Self::Utf8,
            Self::Gbk,
            Self::Gb18030,
            Self::Big5,
            Self::ShiftJis,
            Self::EucJp,
            Self::EucKr,
            Self::Windows1252,
        ]
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Gbk => "GBK",
            Self::Gb18030 => "GB18030",
            Self::Big5 => "Big5",
            Self::ShiftJis => "Shift_JIS",
            Self::EucJp => "EUC-JP",
            Self::EucKr => "EUC-KR",
            Self::Windows1252 => "Windows-1252",
        }
    }

    fn is_utf8(value: &Self) -> bool {
        *value == Self::Utf8
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum StoredTerminalType {
    #[default]
    #[serde(rename = "xterm-256color")]
    Xterm256Color,
    #[serde(rename = "xterm")]
    Xterm,
}

impl StoredTerminalType {
    pub const fn all() -> &'static [Self] {
        &[Self::Xterm256Color, Self::Xterm]
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Xterm256Color => "xterm-256color",
            Self::Xterm => "xterm",
        }
    }

    pub const fn label(self) -> &'static str {
        self.as_str()
    }

    fn is_default(value: &Self) -> bool {
        *value == Self::default()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalExpectSend {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub expect: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub send: String,
}

impl TerminalExpectSend {
    pub fn is_empty(&self) -> bool {
        self.expect.is_empty() && self.send.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshAccountExpect {
    #[serde(default, skip_serializing_if = "TerminalExpectSend::is_empty")]
    pub username: TerminalExpectSend,
    #[serde(default, skip_serializing_if = "TerminalExpectSend::is_empty")]
    pub password: TerminalExpectSend,
}

impl SshAccountExpect {
    pub fn is_empty(&self) -> bool {
        self.username.is_empty() && self.password.is_empty()
    }
}

/// 独立的 SFTP 账户凭据（可选）。
///
/// 配置后，SFTP 传输与远程文件编辑使用该账户连接远端，
/// SSH 终端仍使用主账户；未配置时 SFTP 与 SSH 共用主账户凭据。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SftpAccount {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshParams {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: SshAuthMethod,
    /// 独立的 SFTP 账户（可选）；`None` 表示 SFTP 复用主账户。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sftp_account: Option<SftpAccount>,
    /// SFTP 面板连接成功后进入的初始目录；`None` 表示使用服务器登录目录。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sftp_default_directory: Option<String>,
    /// Optional field-level reference to the local credential vault.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_reference: Option<CredentialReference>,
    /// 不持久化用户名，每次建立连接前由用户输入。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_username: Option<bool>,
    /// 不持久化密码，每次建立连接前由用户输入。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_password: Option<bool>,
    /// 是否允许服务端发起 keyboard-interactive（常用于 OTP/2FA）认证。
    ///
    /// 旧连接没有该字段时保持历史行为：允许 keyboard-interactive。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyboard_interactive: Option<bool>,
    /// SSH 终端文本编码；旧连接缺少此字段时保持 UTF-8。
    #[serde(default, skip_serializing_if = "StoredTerminalEncoding::is_utf8")]
    pub terminal_encoding: StoredTerminalEncoding,
    /// SSH PTY 终端类型；旧连接缺少此字段时保持 xterm-256color。
    #[serde(default, skip_serializing_if = "StoredTerminalType::is_default")]
    pub terminal_type: StoredTerminalType,
    /// SSH shell/channel 打开后，根据设备 CLI 输出自动应答用户名和密码提示。
    #[serde(default, skip_serializing_if = "SshAccountExpect::is_empty")]
    pub account_expect: SshAccountExpect,
    /// 连接超时（秒）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connect_timeout: Option<u64>,
    /// 心跳间隔（秒）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keepalive_interval: Option<u64>,
    /// 最大心跳失败次数
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keepalive_max: Option<usize>,
    /// 默认工作目录
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_directory: Option<String>,
    /// 初始化脚本
    #[serde(skip_serializing_if = "Option::is_none")]
    pub init_script: Option<String>,
    /// 关闭 shell integration 注入(走裸 request_shell,牺牲 prompt hook / 命令记录)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable_shell_integration: Option<bool>,
    /// 启用 X11 转发（需要本机有可用 X server，如 macOS 的 XQuartz）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x11_forwarding: Option<bool>,
    /// 为旧版 SSH 服务器启用兼容算法；默认关闭
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_legacy_algorithms: Option<bool>,
    /// 跳板机配置
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jump_server: Option<JumpServerConfig>,
    /// 停用保留的跳板机配置：启用/停用只是切换 `jump_server`，已填信息保存在这里以便恢复。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_jump_server: Option<JumpServerConfig>,
    /// 代理配置
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxyConfig>,
    /// 远端操作系统 ID（测试连接时从 /etc/os-release 探测，用于连接图标展示）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_id: Option<String>,
    /// 手动指定的连接图标 ID（None = 按探测到的 os_id 自动选择）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// 本机自定义连接图标的绝对路径（优先于内置图标）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_file_path: Option<String>,
}

impl SshParams {
    pub fn prompts_for_username(&self) -> bool {
        self.prompt_username.unwrap_or(false)
    }

    pub fn prompts_for_password(&self) -> bool {
        self.prompt_password.unwrap_or(false)
    }

    pub fn keyboard_interactive_enabled(&self) -> bool {
        self.keyboard_interactive.unwrap_or(true)
    }

    /// 清除只应存在于当前连接尝试中的凭据，返回可安全持久化的参数。
    pub fn sanitize_for_storage(&mut self) {
        if self.prompts_for_username() {
            self.username.clear();
        }
        if self.prompts_for_password()
            && let SshAuthMethod::Password { password } = &mut self.auth_method
        {
            password.clear();
        }
    }

    /// 选择连接图标：手动指定优先，其次按探测到的操作系统 ID，未识别时默认 Linux 企鹅。
    pub fn os_icon(&self) -> IconName {
        ssh_os_icon(self.icon.as_deref().or(self.os_id.as_deref()))
    }
}

/// SSH 连接可选的图标 ID 列表（"linux" 为默认企鹅）。
pub const SSH_ICON_IDS: &[&str] = &[
    "linux",
    "ubuntu",
    "debian",
    "redhat",
    "centos",
    "rocky",
    "fedora",
    "almalinux",
    "alpine",
    "arch",
    "opensuse",
    "kali",
    "amzn",
    "alinux",
    "openeuler",
    "freebsd",
    "cisco",
    "juniper",
    "huawei",
    "h3c",
    "hpe",
    "mikrotik",
    "fortinet",
    "paloalto",
    "zyxel",
    "macos",
    "windows",
    "docker",
];

/// 根据图标 ID（通常为 /etc/os-release 的 ID 或手动选择值）选择 SSH 连接图标，
/// 未识别时默认 Linux 企鹅。
pub fn ssh_os_icon(os_id: Option<&str>) -> IconName {
    match os_id {
        Some("ubuntu") => IconName::UbuntuColor,
        Some("debian") => IconName::DebianColor,
        Some("centos") => IconName::CentosColor,
        Some("rocky") => IconName::RockyColor,
        Some("fedora") => IconName::FedoraColor,
        Some("almalinux") => IconName::AlmalinuxColor,
        Some("alpine") => IconName::AlpineColor,
        Some("arch" | "archarm" | "manjaro") => IconName::ArchColor,
        Some("rhel" | "redhat" | "ol") => IconName::RedhatColor,
        Some("opensuse" | "opensuse-leap" | "opensuse-tumbleweed" | "sles" | "suse") => {
            IconName::OpensuseColor
        }
        Some("kali") => IconName::KaliColor,
        Some("amzn") => IconName::AmazonColor,
        Some("alinux") => IconName::AlinuxColor,
        Some("openeuler") => IconName::OpenEulerColor,
        Some("freebsd") => IconName::FreebsdColor,
        Some("cisco" | "ios" | "ios-xe") => IconName::CiscoColor,
        Some("juniper" | "junos") => IconName::JuniperColor,
        Some("huawei" | "vrp") => IconName::HuaweiColor,
        Some("h3c" | "comware") => IconName::H3cColor,
        Some("hpe") => IconName::HpeColor,
        Some("mikrotik" | "routeros") => IconName::MikrotikColor,
        Some("fortinet" | "fortios") => IconName::FortinetColor,
        Some("paloalto" | "palo-alto" | "panos") => IconName::PaloaltoColor,
        Some("zyxel") => IconName::ZyxelColor,
        Some("macos" | "darwin") => IconName::MacosColor,
        Some("windows") => IconName::WindowsColor,
        Some("docker") => IconName::DockerColor,
        _ => IconName::LinuxPenguinColor,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteDesktopProtocol {
    Rdp,
    Vnc,
}

impl RemoteDesktopProtocol {
    pub fn connection_type(self) -> ConnectionType {
        match self {
            Self::Rdp => ConnectionType::Rdp,
            Self::Vnc => ConnectionType::Vnc,
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            Self::Rdp => 3389,
            Self::Vnc => 5900,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Rdp => "RDP",
            Self::Vnc => "VNC",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteDesktopBackendPreference {
    Auto,
    WindowsNative,
    #[default]
    Canvas,
}

impl RemoteDesktopBackendPreference {
    fn is_canvas(value: &Self) -> bool {
        *value == Self::Canvas
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteDesktopParams {
    pub protocol: RemoteDesktopProtocol,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_reference: Option<CredentialReference>,
    pub domain: Option<String>,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub audio_playback: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxyConfig>,
    #[serde(
        default,
        skip_serializing_if = "RemoteDesktopBackendPreference::is_canvas"
    )]
    pub backend_preference: RemoteDesktopBackendPreference,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rdp: Option<RdpSettings>,
}

impl RemoteDesktopParams {
    pub fn effective_rdp_settings(&self) -> RdpSettings {
        self.rdp
            .clone()
            .unwrap_or_else(|| RdpSettings::from_legacy_audio_playback(self.audio_playback))
    }
}

/// 跳板机配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JumpServerConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: SshAuthMethod,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_reference: Option<CredentialReference>,
}

/// 代理类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProxyType {
    Socks5,
    Http,
}

/// 代理配置
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyConfig {
    pub proxy_type: ProxyType,
    pub host: String,
    pub port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_reference: Option<CredentialReference>,
}

impl fmt::Debug for ProxyConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProxyConfig")
            .field("proxy_type", &self.proxy_type)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("credential_reference", &self.credential_reference)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SshAuthMethod {
    Password {
        password: String,
    },
    PrivateKey {
        key_path: String,
        passphrase: Option<String>,
    },
    PrivateKeyContent {
        private_key: String,
        passphrase: Option<String>,
    },
    Agent,
    Pageant,
    AutoPublicKey,
}

/// Redis 连接模式
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RedisMode {
    /// 单机模式
    #[default]
    Standalone,
    /// 哨兵模式
    Sentinel,
    /// 集群模式
    Cluster,
}

/// Redis 哨兵配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedisSentinelConfig {
    /// 主节点名称
    pub master_name: String,
    /// 哨兵节点列表（host:port）
    pub sentinels: Vec<String>,
    /// 哨兵密码
    pub sentinel_password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_reference: Option<CredentialReference>,
}

/// Redis 集群节点
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedisClusterConfig {
    /// 集群节点列表（host:port）
    pub nodes: Vec<String>,
}

pub type RedisSshTunnelConfig = SshTunnelConfig;
pub type MongoSshTunnelConfig = SshTunnelConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedisParams {
    pub host: String,
    pub port: u16,
    pub password: Option<String>,
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_reference: Option<CredentialReference>,
    pub db_index: u8,
    /// 连接模式
    #[serde(default)]
    pub mode: RedisMode,
    /// 是否启用 TLS
    #[serde(default)]
    pub use_tls: bool,
    /// 连接超时（秒）
    #[serde(default)]
    pub connect_timeout: Option<u64>,
    /// 哨兵配置
    #[serde(default)]
    pub sentinel: Option<RedisSentinelConfig>,
    /// 集群配置
    #[serde(default)]
    pub cluster: Option<RedisClusterConfig>,
    /// SSH 隧道配置
    #[serde(default)]
    pub ssh_tunnel: Option<RedisSshTunnelConfig>,
}

impl RedisParams {
    pub fn apply_referenced_ssh_tunnel(
        &mut self,
        ssh_connection: &StoredConnection,
    ) -> Result<(), serde_json::Error> {
        let Some(tunnel) = self.ssh_tunnel.as_mut() else {
            return Ok(());
        };
        let Some(ssh_connection_id) = tunnel.connection_id else {
            return Ok(());
        };
        if ssh_connection.id != Some(ssh_connection_id) {
            return Ok(());
        }
        if ssh_connection.connection_type != ConnectionType::SshSftp {
            return Ok(());
        }

        let ssh_params = ssh_connection.to_ssh_params()?;
        tunnel.host = ssh_params.host;
        tunnel.port = ssh_params.port;
        tunnel.username = ssh_params.username;
        tunnel.timeout = ssh_params.connect_timeout;
        tunnel.target_host.get_or_insert_with(|| self.host.clone());
        tunnel.target_port.get_or_insert(self.port);

        match ssh_params.auth_method {
            SshAuthMethod::Password { password } => {
                tunnel.auth_type = "password".to_string();
                tunnel.password = Some(password);
                tunnel.private_key_path = None;
                tunnel.private_key_content = None;
                tunnel.private_key_passphrase = None;
            }
            SshAuthMethod::PrivateKey {
                key_path,
                passphrase,
            } => {
                tunnel.auth_type = "private_key".to_string();
                tunnel.password = None;
                tunnel.private_key_path = Some(key_path);
                tunnel.private_key_content = None;
                tunnel.private_key_passphrase = passphrase;
            }
            SshAuthMethod::PrivateKeyContent {
                private_key,
                passphrase,
            } => {
                tunnel.auth_type = "private_key_content".to_string();
                tunnel.password = None;
                tunnel.private_key_path = None;
                tunnel.private_key_content = Some(private_key);
                tunnel.private_key_passphrase = passphrase;
            }
            SshAuthMethod::Agent => {
                tunnel.auth_type = "agent".to_string();
                tunnel.password = None;
                tunnel.private_key_path = None;
                tunnel.private_key_content = None;
                tunnel.private_key_passphrase = None;
            }
            SshAuthMethod::Pageant => {
                tunnel.auth_type = "pageant".to_string();
                tunnel.password = None;
                tunnel.private_key_path = None;
                tunnel.private_key_content = None;
                tunnel.private_key_passphrase = None;
            }
            SshAuthMethod::AutoPublicKey => {
                tunnel.auth_type = "auto_publickey".to_string();
                tunnel.password = None;
                tunnel.private_key_path = None;
                tunnel.private_key_content = None;
                tunnel.private_key_passphrase = None;
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MongoDriverVariant {
    Modern,
    Legacy,
    Legacy32,
}

impl Default for MongoDriverVariant {
    fn default() -> Self {
        Self::Modern
    }
}

impl MongoDriverVariant {
    pub fn driver_id(&self) -> &'static str {
        match self {
            Self::Modern => "mongodb-modern",
            Self::Legacy => "mongodb-legacy",
            Self::Legacy32 => "mongodb-legacy-3-2",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MongoDBParams {
    #[serde(default)]
    pub driver_variant: MongoDriverVariant,
    #[serde(default)]
    pub connection_string: String,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub database: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_reference: Option<CredentialReference>,
    #[serde(default)]
    pub auth_source: Option<String>,
    #[serde(default)]
    pub replica_set: Option<String>,
    #[serde(default)]
    pub read_preference: Option<String>,
    #[serde(default)]
    pub use_srv_record: bool,
    #[serde(default)]
    pub direct_connection: bool,
    #[serde(default)]
    pub use_tls: bool,
    #[serde(default)]
    pub connect_timeout_seconds: Option<u64>,
    #[serde(default)]
    pub application_name: Option<String>,
    #[serde(default)]
    pub ssh_tunnel: Option<MongoSshTunnelConfig>,
}

impl MongoDBParams {
    pub fn apply_referenced_ssh_tunnel(
        &mut self,
        ssh_connection: &StoredConnection,
    ) -> Result<(), serde_json::Error> {
        let Some(tunnel) = self.ssh_tunnel.as_mut() else {
            return Ok(());
        };
        let Some(ssh_connection_id) = tunnel.connection_id else {
            return Ok(());
        };
        if ssh_connection.id != Some(ssh_connection_id) {
            return Ok(());
        }
        if ssh_connection.connection_type != ConnectionType::SshSftp {
            return Ok(());
        }

        let ssh_params = ssh_connection.to_ssh_params()?;
        tunnel.host = ssh_params.host;
        tunnel.port = ssh_params.port;
        tunnel.username = ssh_params.username;
        tunnel.timeout = ssh_params.connect_timeout;
        tunnel.target_host.get_or_insert_with(|| self.host.clone());
        tunnel.target_port.get_or_insert(self.port.unwrap_or(27017));

        match ssh_params.auth_method {
            SshAuthMethod::Password { password } => {
                tunnel.auth_type = "password".to_string();
                tunnel.password = Some(password);
                tunnel.private_key_path = None;
                tunnel.private_key_content = None;
                tunnel.private_key_passphrase = None;
            }
            SshAuthMethod::PrivateKey {
                key_path,
                passphrase,
            } => {
                tunnel.auth_type = "private_key".to_string();
                tunnel.password = None;
                tunnel.private_key_path = Some(key_path);
                tunnel.private_key_content = None;
                tunnel.private_key_passphrase = passphrase;
            }
            SshAuthMethod::PrivateKeyContent {
                private_key,
                passphrase,
            } => {
                tunnel.auth_type = "private_key_content".to_string();
                tunnel.password = None;
                tunnel.private_key_path = None;
                tunnel.private_key_content = Some(private_key);
                tunnel.private_key_passphrase = passphrase;
            }
            SshAuthMethod::Agent => {
                tunnel.auth_type = "agent".to_string();
                tunnel.password = None;
                tunnel.private_key_path = None;
                tunnel.private_key_content = None;
                tunnel.private_key_passphrase = None;
            }
            SshAuthMethod::Pageant => {
                tunnel.auth_type = "pageant".to_string();
                tunnel.password = None;
                tunnel.private_key_path = None;
                tunnel.private_key_content = None;
                tunnel.private_key_passphrase = None;
            }
            SshAuthMethod::AutoPublicKey => {
                tunnel.auth_type = "auto_publickey".to_string();
                tunnel.password = None;
                tunnel.private_key_path = None;
                tunnel.private_key_content = None;
                tunnel.private_key_passphrase = None;
            }
        }

        Ok(())
    }
}

pub type MqttSshTunnelConfig = SshTunnelConfig;

/// MQTT 协议版本
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MqttVersion {
    /// MQTT 3.1.1
    #[default]
    V311,
    /// MQTT 5(rumqttc 暂不支持,预留)
    V5,
}

impl MqttVersion {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::V311 => "3.1.1",
            Self::V5 => "5.0",
        }
    }
}

/// MQTT 连接参数
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttParams {
    #[serde(default = "default_mqtt_host")]
    pub host: String,
    #[serde(default = "default_mqtt_port")]
    pub port: u16,
    /// 客户端 ID(空串表示连接时自动生成)
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_reference: Option<CredentialReference>,
    /// 是否启用 TLS(默认端口切换为 8883)
    #[serde(default)]
    pub use_tls: bool,
    /// 连接超时(秒)
    #[serde(default)]
    pub connect_timeout: Option<u64>,
    /// keep-alive 间隔(秒,默认 30)
    #[serde(default)]
    pub keep_alive: Option<u64>,
    /// MQTT 协议版本
    #[serde(default)]
    pub mqtt_version: MqttVersion,
    /// 清除会话
    #[serde(default = "default_true")]
    pub clean_session: bool,
    /// SSH 隧道配置
    #[serde(default)]
    pub ssh_tunnel: Option<MqttSshTunnelConfig>,
}

fn default_mqtt_host() -> String {
    "127.0.0.1".to_string()
}

fn default_mqtt_port() -> u16 {
    1883
}

fn default_true() -> bool {
    true
}

impl Default for MqttParams {
    fn default() -> Self {
        Self {
            host: default_mqtt_host(),
            port: default_mqtt_port(),
            client_id: String::new(),
            username: None,
            password: None,
            credential_reference: None,
            use_tls: false,
            connect_timeout: None,
            keep_alive: None,
            mqtt_version: MqttVersion::V311,
            clean_session: true,
            ssh_tunnel: None,
        }
    }
}

impl MqttParams {
    pub fn apply_referenced_ssh_tunnel(
        &mut self,
        ssh_connection: &StoredConnection,
    ) -> Result<(), serde_json::Error> {
        let Some(tunnel) = self.ssh_tunnel.as_mut() else {
            return Ok(());
        };
        let Some(ssh_connection_id) = tunnel.connection_id else {
            return Ok(());
        };
        if ssh_connection.id != Some(ssh_connection_id) {
            return Ok(());
        }
        if ssh_connection.connection_type != ConnectionType::SshSftp {
            return Ok(());
        }

        let ssh_params = ssh_connection.to_ssh_params()?;
        tunnel.host = ssh_params.host;
        tunnel.port = ssh_params.port;
        tunnel.username = ssh_params.username;
        tunnel.timeout = ssh_params.connect_timeout;
        tunnel.target_host.get_or_insert_with(|| self.host.clone());
        tunnel.target_port.get_or_insert(self.port);

        match ssh_params.auth_method {
            SshAuthMethod::Password { password } => {
                tunnel.auth_type = "password".to_string();
                tunnel.password = Some(password);
                tunnel.private_key_path = None;
                tunnel.private_key_content = None;
                tunnel.private_key_passphrase = None;
            }
            SshAuthMethod::PrivateKey {
                key_path,
                passphrase,
            } => {
                tunnel.auth_type = "private_key".to_string();
                tunnel.password = None;
                tunnel.private_key_path = Some(key_path);
                tunnel.private_key_content = None;
                tunnel.private_key_passphrase = passphrase;
            }
            SshAuthMethod::PrivateKeyContent {
                private_key,
                passphrase,
            } => {
                tunnel.auth_type = "private_key_content".to_string();
                tunnel.password = None;
                tunnel.private_key_path = None;
                tunnel.private_key_content = Some(private_key);
                tunnel.private_key_passphrase = passphrase;
            }
            SshAuthMethod::Agent => {
                tunnel.auth_type = "agent".to_string();
                tunnel.password = None;
                tunnel.private_key_path = None;
                tunnel.private_key_content = None;
                tunnel.private_key_passphrase = None;
            }
            SshAuthMethod::Pageant => {
                tunnel.auth_type = "pageant".to_string();
                tunnel.password = None;
                tunnel.private_key_path = None;
                tunnel.private_key_content = None;
                tunnel.private_key_passphrase = None;
            }
            SshAuthMethod::AutoPublicKey => {
                tunnel.auth_type = "auto_publickey".to_string();
                tunnel.password = None;
                tunnel.private_key_path = None;
                tunnel.private_key_content = None;
                tunnel.private_key_passphrase = None;
            }
        }

        Ok(())
    }
}

/// 串口校验位
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SerialParity {
    #[default]
    None,
    Odd,
    Even,
}

impl SerialParity {
    pub fn all() -> &'static [SerialParity] {
        &[SerialParity::None, SerialParity::Odd, SerialParity::Even]
    }

    pub fn label(&self) -> &'static str {
        match self {
            SerialParity::None => "None",
            SerialParity::Odd => "Odd",
            SerialParity::Even => "Even",
        }
    }
}

/// 串口流控
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SerialFlowControl {
    #[default]
    None,
    Software,
    Hardware,
}

impl SerialFlowControl {
    pub fn all() -> &'static [SerialFlowControl] {
        &[
            SerialFlowControl::None,
            SerialFlowControl::Software,
            SerialFlowControl::Hardware,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            SerialFlowControl::None => "None",
            SerialFlowControl::Software => "XON/XOFF",
            SerialFlowControl::Hardware => "RTS/CTS",
        }
    }
}

/// 串口连接参数
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialParams {
    /// 串口设备路径，如 /dev/ttyUSB0 或 COM1
    pub port_name: String,
    /// 波特率
    #[serde(default = "default_baud_rate")]
    pub baud_rate: u32,
    /// 数据位 (5/6/7/8)
    #[serde(default = "default_data_bits")]
    pub data_bits: u8,
    /// 停止位 (1/2)
    #[serde(default = "default_stop_bits")]
    pub stop_bits: u8,
    /// 校验位
    #[serde(default)]
    pub parity: SerialParity,
    /// 流控
    #[serde(default)]
    pub flow_control: SerialFlowControl,
}

fn default_baud_rate() -> u32 {
    115200
}

fn default_data_bits() -> u8 {
    8
}

fn default_stop_bits() -> u8 {
    1
}

impl Default for SerialParams {
    fn default() -> Self {
        Self {
            port_name: String::new(),
            baud_rate: 115200,
            data_bits: 8,
            stop_bits: 1,
            parity: SerialParity::None,
            flow_control: SerialFlowControl::None,
        }
    }
}

/// Telnet 登录脚本步骤：匹配到服务端输出后，自动发送配置的内容。
///
/// 对应 Xshell / SecureCRT 的 expect/send 登录脚本模型。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelnetLoginStep {
    /// 期望匹配的文本；支持 `\r`、`\n`、`\t`、`\xNN` 转义。
    pub expect: String,
    /// 匹配后发送的内容；支持与 `expect` 相同的转义，
    /// 且不以 `\r`/`\n` 结尾时自动补一个回车。
    #[serde(default)]
    pub send: String,
}

/// 按下退格键时发送给 Telnet 服务端的控制字符。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TelnetBackspaceCode {
    /// BS（Backspace，0x08）。
    Backspace,
    /// DEL（Delete，0x7F），保持历史默认行为。
    #[default]
    Delete,
}

impl TelnetBackspaceCode {
    const ALL: [Self; 2] = [Self::Backspace, Self::Delete];

    pub const fn all() -> &'static [Self] {
        &Self::ALL
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Backspace => "BS (0x08)",
            Self::Delete => "DEL (0x7F)",
        }
    }

    fn is_default(value: &Self) -> bool {
        *value == Self::default()
    }
}

/// Telnet 连接参数
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelnetParams {
    /// 主机地址
    pub host: String,
    /// 端口（默认 23）
    #[serde(default = "default_telnet_port")]
    pub port: u16,
    /// Optional field-level reference to the local credential vault.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_reference: Option<CredentialReference>,
    /// 当前设备缺少引用的钥匙串时，仅本次连接提示输入用户名。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_username: Option<bool>,
    /// 当前设备缺少引用的钥匙串时，仅本次连接提示输入密码。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_password: Option<bool>,
    /// 按下退格键时发送的控制字符；旧连接默认继续使用 DEL（0x7F）。
    #[serde(default, skip_serializing_if = "TelnetBackspaceCode::is_default")]
    pub backspace_code: TelnetBackspaceCode,
    /// 可选登录脚本；旧连接没有该字段时保持为空。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub login_script: Vec<TelnetLoginStep>,
}

impl TelnetParams {
    pub fn prompts_for_username(&self) -> bool {
        self.prompt_username.unwrap_or(false)
    }

    pub fn prompts_for_password(&self) -> bool {
        self.prompt_password.unwrap_or(false)
    }

    /// 返回登录脚本中仍需要由运行时凭据填充的字段。
    ///
    /// 只有 `send` 为空且 `expect` 能明确识别为用户名或密码提示时，
    /// 才会把该字段交给临时凭据输入框，避免把敏感信息发送到不明确的
    /// 自定义正则步骤中。
    pub fn login_credential_prompt_fields(&self) -> (bool, bool) {
        self.login_script
            .iter()
            .filter(|step| step.send.is_empty())
            .filter_map(|step| match telnet_expect_credential_kind(&step.expect) {
                Some(TelnetExpectCredentialKind::Username) => Some((true, false)),
                Some(TelnetExpectCredentialKind::Password) => Some((false, true)),
                None => None,
            })
            .fold(
                (false, false),
                |(username, password), (step_username, step_password)| {
                    (username || step_username, password || step_password)
                },
            )
    }

    /// 将临时用户名/密码填入 send 为空的对应 expect 步骤。
    ///
    /// 显式配置的 send 始终优先；无法明确识别为用户名或密码提示的步骤保持原样。
    pub fn apply_login_credentials(&mut self, username: Option<&str>, password: Option<&str>) {
        for step in &mut self.login_script {
            if !step.send.is_empty() {
                continue;
            }
            step.send = match telnet_expect_credential_kind(&step.expect) {
                Some(TelnetExpectCredentialKind::Username) => username
                    .filter(|value| !value.is_empty())
                    .unwrap_or_default(),
                Some(TelnetExpectCredentialKind::Password) => password
                    .filter(|value| !value.is_empty())
                    .unwrap_or_default(),
                None => "",
            }
            .to_string();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelnetExpectCredentialKind {
    Username,
    Password,
}

/// 根据 expect 正则表达式中的提示词判断它需要用户名还是密码。
///
/// 这里识别的是正则源码，而真正的服务端输出匹配仍由 Telnet expect 引擎执行。
/// 同时包含用户名和密码提示词的宽泛规则视为不明确，避免发送错误的敏感信息。
pub fn telnet_expect_credential_kind(expect: &str) -> Option<TelnetExpectCredentialKind> {
    let expect = expect.to_ascii_lowercase();
    let username_markers = ["login", "username", "user name", "account"];
    let password_markers = ["password", "passwd", "passcode"];
    let username = username_markers
        .iter()
        .any(|marker| contains_telnet_marker(&expect, marker));
    let password = password_markers
        .iter()
        .any(|marker| contains_telnet_marker(&expect, marker));
    match (username, password) {
        (true, false)
            if username_markers
                .iter()
                .any(|marker| contains_telnet_prompt_marker(&expect, marker)) =>
        {
            Some(TelnetExpectCredentialKind::Username)
        }
        (false, true)
            if password_markers
                .iter()
                .any(|marker| contains_telnet_prompt_marker(&expect, marker)) =>
        {
            Some(TelnetExpectCredentialKind::Password)
        }
        _ => None,
    }
}

fn contains_telnet_marker(expect: &str, marker: &str) -> bool {
    telnet_marker_occurrences(expect, marker).next().is_some()
}

fn contains_telnet_prompt_marker(expect: &str, marker: &str) -> bool {
    telnet_marker_occurrences(expect, marker).any(|(_, end)| telnet_prompt_suffix(&expect[end..]))
}

fn telnet_marker_occurrences<'a>(
    expect: &'a str,
    marker: &'a str,
) -> impl Iterator<Item = (usize, usize)> + 'a {
    expect.match_indices(marker).filter_map(move |(start, _)| {
        let end = start + marker.len();
        let has_word_boundary_before = start == 0
            || !expect[..start]
                .chars()
                .next_back()
                .is_some_and(|character| character.is_alphanumeric() || character == '_');
        let has_word_boundary_after = end == expect.len()
            || !expect[end..]
                .chars()
                .next()
                .is_some_and(|character| character.is_alphanumeric() || character == '_');
        (has_word_boundary_before && has_word_boundary_after).then_some((start, end))
    })
}

fn telnet_prompt_suffix(mut suffix: &str) -> bool {
    loop {
        suffix = suffix.trim_start();
        let Some(first) = suffix.chars().next() else {
            return true;
        };
        if matches!(first, ':' | '>' | '#') {
            return true;
        }
        if first == '$' {
            return true;
        }
        if first == '\\' {
            let mut indices = suffix.char_indices();
            indices.next();
            indices.next();
            let next_index = indices
                .next()
                .map(|(index, _)| index)
                .unwrap_or(suffix.len());
            suffix = &suffix[next_index..];
            continue;
        }
        if first.is_ascii_punctuation() {
            suffix = &suffix[first.len_utf8()..];
            continue;
        }
        return false;
    }
}

fn default_telnet_port() -> u16 {
    23
}

impl Default for TelnetParams {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 23,
            credential_reference: None,
            prompt_username: None,
            prompt_password: None,
            backspace_code: TelnetBackspaceCode::default(),
            login_script: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PortForwardingKind {
    #[default]
    Local,
    Remote,
    Dynamic,
}

impl PortForwardingKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Local => "Local",
            Self::Remote => "Remote",
            Self::Dynamic => "Dynamic SOCKS",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortForwardingParams {
    pub ssh_connection_id: i64,
    #[serde(default)]
    pub kind: PortForwardingKind,
    #[serde(default = "default_forward_bind_host")]
    pub bind_host: String,
    #[serde(default)]
    pub bind_port: u16,
    #[serde(default)]
    pub target_host: String,
    #[serde(default)]
    pub target_port: u16,
}

fn default_forward_bind_host() -> String {
    "127.0.0.1".to_string()
}

/// Connection configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbConnectionConfig {
    #[serde(skip)]
    pub id: String,
    pub database_type: DatabaseType,
    #[serde(skip)]
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_reference: Option<CredentialReference>,
    pub database: Option<String>,
    pub service_name: Option<String>,
    pub sid: Option<String>,
    #[serde(skip)]
    pub workspace_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxyConfig>,
    #[serde(default)]
    pub extra_params: std::collections::HashMap<String, String>,
}

impl DbConnectionConfig {
    pub fn get_param(&self, key: &str) -> Option<&String> {
        self.extra_params.get(key)
    }

    pub fn get_param_as<T: std::str::FromStr>(&self, key: &str) -> Option<T> {
        self.extra_params.get(key).and_then(|v| v.parse().ok())
    }

    pub fn get_param_bool(&self, key: &str) -> bool {
        self.extra_params
            .get(key)
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false)
    }

    pub fn server_info(&self) -> String {
        match self.database_type {
            DatabaseType::SQLite | DatabaseType::DuckDB => format!("{}", self.host),
            _ => format!("{}:{}", self.host, self.port),
        }
    }

    pub fn is_change(&self, other: &DbConnectionConfig) -> bool {
        self.host != other.host
            || self.port != other.port
            || self.username != other.username
            || self.password != other.password
            || self.database != other.database
            || self.service_name != other.service_name
            || self.sid != other.sid
            || self.proxy != other.proxy
            || self.extra_params != other.extra_params
    }

    pub fn apply_referenced_ssh_tunnel(
        &mut self,
        ssh_connection: &StoredConnection,
    ) -> Result<(), serde_json::Error> {
        let Some(ssh_connection_id) = self.extra_params.get("ssh_connection_id") else {
            return Ok(());
        };
        if ssh_connection.id.map(|id| id.to_string()).as_ref() != Some(ssh_connection_id) {
            return Ok(());
        }
        if ssh_connection.connection_type != ConnectionType::SshSftp {
            return Ok(());
        }

        let ssh_params = ssh_connection.to_ssh_params()?;
        if self.proxy.is_none() {
            self.proxy = ssh_params.proxy.clone();
        }
        self.extra_params
            .insert("ssh_host".to_string(), ssh_params.host);
        self.extra_params
            .insert("ssh_port".to_string(), ssh_params.port.to_string());
        self.extra_params
            .insert("ssh_username".to_string(), ssh_params.username);
        if let Some(timeout) = ssh_params.connect_timeout {
            self.extra_params
                .insert("ssh_timeout".to_string(), timeout.to_string());
        }

        match ssh_params.auth_method {
            SshAuthMethod::Password { password } => {
                self.extra_params
                    .insert("ssh_auth_type".to_string(), "password".to_string());
                self.extra_params
                    .insert("ssh_password".to_string(), password);
            }
            SshAuthMethod::PrivateKey {
                key_path,
                passphrase,
            } => {
                self.extra_params
                    .insert("ssh_auth_type".to_string(), "private_key".to_string());
                self.extra_params
                    .insert("ssh_private_key_path".to_string(), key_path);
                if let Some(passphrase) = passphrase {
                    self.extra_params
                        .insert("ssh_private_key_passphrase".to_string(), passphrase);
                }
            }
            SshAuthMethod::PrivateKeyContent {
                private_key,
                passphrase,
            } => {
                self.extra_params.insert(
                    "ssh_auth_type".to_string(),
                    "private_key_content".to_string(),
                );
                self.extra_params
                    .insert("ssh_private_key_content".to_string(), private_key);
                if let Some(passphrase) = passphrase {
                    self.extra_params
                        .insert("ssh_private_key_passphrase".to_string(), passphrase);
                }
            }
            SshAuthMethod::Agent => {
                self.extra_params
                    .insert("ssh_auth_type".to_string(), "agent".to_string());
            }
            SshAuthMethod::Pageant => {
                self.extra_params
                    .insert("ssh_auth_type".to_string(), "pageant".to_string());
            }
            SshAuthMethod::AutoPublicKey => {
                self.extra_params
                    .insert("ssh_auth_type".to_string(), "auto_publickey".to_string());
            }
        }

        Ok(())
    }
}

/// Workspace for organizing connections
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// 父工作区 ID；为空时表示根分组。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
    /// 云端 ID（用于同步）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cloud_id: Option<String>,
    /// 最后同步时间戳
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_synced_at: Option<i64>,
    /// 手动排序位序，用于跨设备同步工作区列表顺序。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_order: Option<i32>,
    /// 本地侧栏中的折叠状态，不参与云同步。
    #[serde(skip)]
    pub sidebar_collapsed: bool,
}

impl Entity for Workspace {
    fn id(&self) -> Option<i64> {
        self.id
    }

    fn created_at(&self) -> i64 {
        self.created_at
            .expect("created_at 在从数据库读取后应该存在")
    }

    fn updated_at(&self) -> i64 {
        self.updated_at
            .expect("updated_at 在从数据库读取后应该存在")
    }
}

impl Workspace {
    pub fn new(name: String) -> Self {
        Self {
            id: None,
            name,
            color: None,
            icon: None,
            parent_id: None,
            created_at: None,
            updated_at: None,
            cloud_id: None,
            last_synced_at: None,
            sort_order: None,
            sidebar_collapsed: false,
        }
    }
}

impl SyncableItem for Workspace {
    fn local_id(&self) -> Option<i64> {
        self.id
    }

    fn set_local_id(&mut self, id: Option<i64>) {
        self.id = id;
    }

    fn item_name(&self) -> &str {
        &self.name
    }

    fn cloud_id(&self) -> Option<&str> {
        self.cloud_id.as_deref()
    }

    fn set_cloud_id(&mut self, cloud_id: Option<String>) {
        self.cloud_id = cloud_id;
    }

    fn updated_at(&self) -> Option<i64> {
        self.updated_at
    }

    fn last_synced_at(&self) -> Option<i64> {
        self.last_synced_at
    }
}

/// Stored connection with ID
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredConnection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    /// Local, non-secret generation for authentication/session identity.
    ///
    /// The repository assigns this value and advances it on full connection
    /// record rewrites. It is intentionally excluded from cloud/export
    /// serialization because it is meaningful only together with the local
    /// database record ID. Unsaved or imported in-memory records therefore
    /// carry `None` until inserted.
    #[serde(skip)]
    pub credential_revision: Option<i64>,
    pub name: String,
    pub connection_type: ConnectionType,
    pub params: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<i64>,
    /// 已选中的数据库ID列表（JSON数组），None表示全选
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_databases: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remark: Option<String>,
    /// 是否启用云同步（默认 true）
    #[serde(default = "default_sync_enabled")]
    pub sync_enabled: bool,
    /// 云端记录 ID（同步成功后获得）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cloud_id: Option<String>,
    /// 最后同步时间戳
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_synced_at: Option<i64>,
    /// 最近使用时间戳，仅用于本地列表排序，不参与云同步。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<i64>,
    /// 手动排序位序，仅用于本地列表排序，不参与云同步。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_order: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
    /// 团队归属 ID（None = 个人数据）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    /// 连接创建者 ID（用户 UUID，用于权限判断）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<String>,
}

pub const EXTENSION_CONNECTION_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtensionConnectionParams {
    pub schema_version: u32,
    pub extension_id: String,
    pub contribution_id: String,
    #[serde(default)]
    pub config: serde_json::Map<String, Value>,
    #[serde(default)]
    pub secrets: BTreeMap<String, String>,
}

impl ExtensionConnectionParams {
    pub fn new(
        extension_id: impl Into<String>,
        contribution_id: impl Into<String>,
        config: serde_json::Map<String, Value>,
        secrets: BTreeMap<String, String>,
    ) -> anyhow::Result<Self> {
        let params = Self {
            schema_version: EXTENSION_CONNECTION_SCHEMA_VERSION,
            extension_id: extension_id.into(),
            contribution_id: contribution_id.into(),
            config,
            secrets,
        };
        params.validate()?;
        Ok(params)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.schema_version == EXTENSION_CONNECTION_SCHEMA_VERSION,
            "unsupported extension connection schema version {}",
            self.schema_version
        );
        anyhow::ensure!(
            !self.extension_id.trim().is_empty(),
            "extension id is empty"
        );
        anyhow::ensure!(
            !self.contribution_id.trim().is_empty(),
            "connection contribution id is empty"
        );
        anyhow::ensure!(
            self.secrets
                .keys()
                .all(|key| !self.config.contains_key(key)),
            "extension connection config and secrets must not share keys"
        );
        Ok(())
    }
}

fn default_sync_enabled() -> bool {
    true
}

impl Entity for StoredConnection {
    fn id(&self) -> Option<i64> {
        self.id
    }

    fn created_at(&self) -> i64 {
        self.created_at
            .expect("created_at 在从数据库读取后应该存在")
    }

    fn updated_at(&self) -> i64 {
        self.updated_at
            .expect("updated_at 在从数据库读取后应该存在")
    }
}

impl SyncableItem for StoredConnection {
    fn local_id(&self) -> Option<i64> {
        self.id
    }

    fn set_local_id(&mut self, id: Option<i64>) {
        self.id = id;
    }

    fn item_name(&self) -> &str {
        &self.name
    }

    fn cloud_id(&self) -> Option<&str> {
        self.cloud_id.as_deref()
    }

    fn set_cloud_id(&mut self, cloud_id: Option<String>) {
        self.cloud_id = cloud_id;
    }

    fn updated_at(&self) -> Option<i64> {
        self.updated_at
    }

    fn is_sync_enabled(&self) -> bool {
        self.sync_enabled
    }

    fn last_synced_at(&self) -> Option<i64> {
        self.last_synced_at
    }

    fn team_id(&self) -> Option<&str> {
        self.team_id.as_deref()
    }
}

fn trimmed_or_default(name: String, default_name: String) -> String {
    if name.trim().is_empty() {
        default_name
    } else {
        name
    }
}

fn host_port_name(host: &str, port: u16) -> String {
    let host = host.trim();
    if host.is_empty() {
        port.to_string()
    } else {
        format!("{host}:{port}")
    }
}

fn optional_host_port_name(host: &str, port: Option<u16>) -> String {
    match port {
        Some(port) => host_port_name(host, port),
        None => host.trim().to_string(),
    }
}

fn default_database_name(name: String, params: &DbConnectionConfig) -> String {
    trimmed_or_default(name, params.server_info())
}

fn default_ssh_name(name: String, params: &SshParams) -> String {
    let username = params.username.trim();
    let destination = host_port_name(&params.host, params.port);
    let default_name = if username.is_empty() {
        destination
    } else {
        format!("{username}@{destination}")
    };
    trimmed_or_default(name, default_name)
}

fn default_remote_desktop_name(name: String, params: &RemoteDesktopParams) -> String {
    trimmed_or_default(name, host_port_name(&params.host, params.port))
}

fn default_redis_name(name: String, params: &RedisParams) -> String {
    trimmed_or_default(name, host_port_name(&params.host, params.port))
}

fn default_mongodb_name(name: String, params: &MongoDBParams) -> String {
    let default_name = optional_host_port_name(&params.host, params.port);
    let default_name = if default_name.is_empty() {
        params.connection_string.trim().to_string()
    } else {
        default_name
    };
    trimmed_or_default(name, default_name)
}

fn default_mqtt_name(name: String, params: &MqttParams) -> String {
    trimmed_or_default(name, host_port_name(&params.host, params.port))
}

fn default_serial_name(name: String, params: &SerialParams) -> String {
    trimmed_or_default(name, params.port_name.trim().to_string())
}

fn default_telnet_name(name: String, params: &TelnetParams) -> String {
    trimmed_or_default(name, host_port_name(&params.host, params.port))
}

fn default_port_forwarding_name(name: String, params: &PortForwardingParams) -> String {
    let default_name = match params.kind {
        PortForwardingKind::Local => format!(
            "{}:{} -> {}:{}",
            params.bind_host, params.bind_port, params.target_host, params.target_port
        ),
        PortForwardingKind::Remote => format!(
            "{}:{} <- {}:{}",
            params.bind_host, params.bind_port, params.target_host, params.target_port
        ),
        PortForwardingKind::Dynamic => {
            format!("SOCKS {}:{}", params.bind_host, params.bind_port)
        }
    };
    trimmed_or_default(name, default_name)
}

impl StoredConnection {
    pub fn new_extension(
        name: String,
        params: ExtensionConnectionParams,
        workspace_id: Option<i64>,
    ) -> Self {
        let name = trimmed_or_default(name, params.contribution_id.clone());
        Self {
            id: None,
            credential_revision: None,
            name,
            connection_type: ConnectionType::Extension,
            params: serde_json::to_string(&params)
                .expect("ExtensionConnectionParams serialization must succeed"),
            workspace_id,
            selected_databases: None,
            remark: None,
            sync_enabled: true,
            cloud_id: None,
            last_synced_at: None,
            last_used_at: None,
            sort_order: None,
            created_at: None,
            updated_at: None,
            team_id: None,
            owner_id: None,
        }
    }

    pub fn to_extension_params(&self) -> anyhow::Result<ExtensionConnectionParams> {
        anyhow::ensure!(
            self.connection_type == ConnectionType::Extension,
            "connection is not an extension connection"
        );
        let params: ExtensionConnectionParams = serde_json::from_str(&self.params)?;
        params.validate()?;
        Ok(params)
    }

    pub fn new_database(
        name: String,
        params: DbConnectionConfig,
        workspace_id: Option<i64>,
    ) -> Self {
        let name = default_database_name(name, &params);
        Self {
            id: None,
            credential_revision: None,
            name,
            connection_type: ConnectionType::Database,
            params: serde_json::to_string(&params).expect("DbConnectionConfig 序列化不应失败"),
            workspace_id,
            selected_databases: if let Some(database) = &params.database {
                Some(format!("[\"{}\"]", database))
            } else {
                None
            },
            remark: None,
            sync_enabled: true,
            cloud_id: None,
            last_synced_at: None,
            last_used_at: None,
            sort_order: None,
            created_at: None,
            updated_at: None,
            team_id: None,
            owner_id: None,
        }
    }

    pub fn new_ssh(name: String, mut params: SshParams, workspace_id: Option<i64>) -> Self {
        params.sanitize_for_storage();
        let name = default_ssh_name(name, &params);
        Self {
            id: None,
            credential_revision: None,
            name,
            connection_type: ConnectionType::SshSftp,
            params: serde_json::to_string(&params).expect("SshParams 序列化不应失败"),
            workspace_id,
            selected_databases: None,
            remark: None,
            sync_enabled: true,
            cloud_id: None,
            last_synced_at: None,
            last_used_at: None,
            sort_order: None,
            created_at: None,
            updated_at: None,
            team_id: None,
            owner_id: None,
        }
    }

    pub fn new_remote_desktop(
        name: String,
        params: RemoteDesktopParams,
        workspace_id: Option<i64>,
    ) -> Self {
        let name = default_remote_desktop_name(name, &params);
        Self {
            id: None,
            credential_revision: None,
            name,
            connection_type: params.protocol.connection_type(),
            params: serde_json::to_string(&params).expect("RemoteDesktopParams 序列化不应失败"),
            workspace_id,
            selected_databases: None,
            remark: None,
            sync_enabled: true,
            cloud_id: None,
            last_synced_at: None,
            last_used_at: None,
            sort_order: None,
            created_at: None,
            updated_at: None,
            team_id: None,
            owner_id: None,
        }
    }

    pub fn new_redis(name: String, params: RedisParams, workspace_id: Option<i64>) -> Self {
        let name = default_redis_name(name, &params);
        Self {
            id: None,
            credential_revision: None,
            name,
            connection_type: ConnectionType::Redis,
            params: serde_json::to_string(&params).expect("RedisParams 序列化不应失败"),
            workspace_id,
            selected_databases: None,
            remark: None,
            sync_enabled: true,
            cloud_id: None,
            last_synced_at: None,
            last_used_at: None,
            sort_order: None,
            created_at: None,
            updated_at: None,
            team_id: None,
            owner_id: None,
        }
    }

    pub fn new_mongodb(name: String, params: MongoDBParams, workspace_id: Option<i64>) -> Self {
        let name = default_mongodb_name(name, &params);
        Self {
            id: None,
            credential_revision: None,
            name,
            connection_type: ConnectionType::MongoDB,
            params: serde_json::to_string(&params).expect("MongoDBParams 序列化不应失败"),
            workspace_id,
            selected_databases: None,
            remark: None,
            sync_enabled: true,
            cloud_id: None,
            last_synced_at: None,
            last_used_at: None,
            sort_order: None,
            created_at: None,
            updated_at: None,
            team_id: None,
            owner_id: None,
        }
    }

    pub fn new_mqtt(name: String, params: MqttParams, workspace_id: Option<i64>) -> Self {
        let name = default_mqtt_name(name, &params);
        Self {
            id: None,
            credential_revision: None,
            name,
            connection_type: ConnectionType::Mqtt,
            params: serde_json::to_string(&params).expect("MqttParams 序列化不应失败"),
            workspace_id,
            selected_databases: None,
            remark: None,
            sync_enabled: true,
            cloud_id: None,
            last_synced_at: None,
            last_used_at: None,
            sort_order: None,
            created_at: None,
            updated_at: None,
            team_id: None,
            owner_id: None,
        }
    }

    pub fn to_ssh_params(&self) -> Result<SshParams, serde_json::Error> {
        let mut params: SshParams = serde_json::from_str(&self.params)?;
        params.sanitize_for_storage();
        Ok(params)
    }

    pub fn to_remote_desktop_params(&self) -> Result<RemoteDesktopParams, serde_json::Error> {
        serde_json::from_str(&self.params)
    }

    pub fn to_redis_params(&self) -> Result<RedisParams, serde_json::Error> {
        serde_json::from_str(&self.params)
    }

    pub fn to_mongodb_params(&self) -> Result<MongoDBParams, serde_json::Error> {
        serde_json::from_str(&self.params)
    }

    pub fn to_mqtt_params(&self) -> Result<MqttParams, serde_json::Error> {
        serde_json::from_str(&self.params)
    }

    pub fn new_serial(name: String, params: SerialParams, workspace_id: Option<i64>) -> Self {
        let name = default_serial_name(name, &params);
        Self {
            id: None,
            credential_revision: None,
            name,
            connection_type: ConnectionType::Serial,
            params: serde_json::to_string(&params).expect("SerialParams 序列化不应失败"),
            workspace_id,
            selected_databases: None,
            remark: None,
            sync_enabled: true,
            cloud_id: None,
            last_synced_at: None,
            last_used_at: None,
            sort_order: None,
            created_at: None,
            updated_at: None,
            team_id: None,
            owner_id: None,
        }
    }

    pub fn new_telnet(name: String, params: TelnetParams, workspace_id: Option<i64>) -> Self {
        let name = default_telnet_name(name, &params);
        Self {
            id: None,
            credential_revision: None,
            name,
            connection_type: ConnectionType::Telnet,
            params: serde_json::to_string(&params).expect("TelnetParams 序列化不应失败"),
            workspace_id,
            selected_databases: None,
            remark: None,
            sync_enabled: true,
            cloud_id: None,
            last_synced_at: None,
            last_used_at: None,
            sort_order: None,
            created_at: None,
            updated_at: None,
            team_id: None,
            owner_id: None,
        }
    }

    pub fn new_port_forwarding(
        name: String,
        params: PortForwardingParams,
        workspace_id: Option<i64>,
    ) -> Self {
        let name = default_port_forwarding_name(name, &params);
        Self {
            id: None,
            credential_revision: None,
            name,
            connection_type: ConnectionType::PortForwarding,
            params: serde_json::to_string(&params).expect("PortForwardingParams 序列化不应失败"),
            workspace_id,
            selected_databases: None,
            remark: None,
            sync_enabled: true,
            cloud_id: None,
            last_synced_at: None,
            last_used_at: None,
            sort_order: None,
            created_at: None,
            updated_at: None,
            team_id: None,
            owner_id: None,
        }
    }

    pub fn to_serial_params(&self) -> Result<SerialParams, serde_json::Error> {
        serde_json::from_str(&self.params)
    }

    pub fn to_telnet_params(&self) -> Result<TelnetParams, serde_json::Error> {
        serde_json::from_str(&self.params)
    }

    pub fn to_port_forwarding_params(&self) -> Result<PortForwardingParams, serde_json::Error> {
        serde_json::from_str(&self.params)
    }

    pub fn to_db_connection(&self) -> Result<DbConnectionConfig, serde_json::Error> {
        let mut params: DbConnectionConfig = serde_json::from_str(&self.params)?;
        params.name = self.name.clone();
        params.workspace_id = self.workspace_id;
        params.id = self.id.unwrap_or(0).to_string();
        Ok(params)
    }

    pub fn from_db_connection(connection: DbConnectionConfig) -> Self {
        let name = connection.name.clone();
        let workspace_id = connection.workspace_id.clone();
        Self::new_database(name, connection, workspace_id)
    }

    /// 获取已选中的数据库列表，None表示全选
    pub fn get_selected_databases(&self) -> Option<Vec<String>> {
        self.selected_databases
            .as_ref()
            .and_then(|json| serde_json::from_str(json).ok())
    }

    /// 设置已选中的数据库列表，None表示全选
    pub fn set_selected_databases(&mut self, databases: Option<Vec<String>>) {
        self.selected_databases =
            databases.map(|dbs| serde_json::to_string(&dbs).unwrap_or_default());
    }

    /// 对 params 中的敏感字段进行加密，返回加密后的 params 字符串。
    ///
    /// 敏感字段包括：password、passphrase、private_key、private_key_content、
    /// Telnet 登录脚本的 send 值，以及嵌套结构中的同类字段。
    pub fn try_encrypt_params(&self) -> anyhow::Result<String> {
        if self.connection_type != ConnectionType::Extension {
            return Ok(encrypt_json_passwords(&self.params_for_storage()));
        }
        let mut params = self.to_extension_params()?;
        for secret in params.secrets.values_mut() {
            if secret.is_empty() || crypto::is_encrypted(secret) {
                continue;
            }
            anyhow::ensure!(
                crypto::has_master_key(),
                "Cannot persist extension connection secrets without a master key"
            );
            let encrypted = crypto::encrypt_password(secret);
            anyhow::ensure!(
                encrypted != *secret && crypto::is_encrypted(&encrypted),
                "extension connection secret encryption failed"
            );
            *secret = encrypted;
        }
        Ok(serde_json::to_string(&params)?)
    }

    /// 返回适合持久化、同步、分享或导出的参数 JSON。
    ///
    /// SSH 连接若配置为连接时输入用户名或密码，会在此处再次清除对应字段，
    /// 防止绕过 `StoredConnection::new_ssh` 的调用路径意外泄漏临时凭据。
    pub fn params_for_storage(&self) -> String {
        if self.connection_type != ConnectionType::SshSftp {
            return self.params.clone();
        }

        let Ok(mut params) = serde_json::from_str::<SshParams>(&self.params) else {
            return self.params.clone();
        };
        params.sanitize_for_storage();
        serde_json::to_string(&params).unwrap_or_else(|_| self.params.clone())
    }

    /// 对 params 中的加密字段进行解密，返回解密后的 params 字符串。
    pub fn decrypt_params(&self) -> String {
        if self.connection_type == ConnectionType::Extension {
            let Ok(mut params) = self.to_extension_params() else {
                return self.params.clone();
            };
            for secret in params.secrets.values_mut() {
                if crypto::is_encrypted(secret) {
                    *secret = crypto::decrypt_password(secret);
                }
            }
            return serde_json::to_string(&params).unwrap_or_else(|_| self.params.clone());
        }
        decrypt_json_passwords(&self.params)
    }

    /// 返回一个新的 StoredConnection，其 params 中的密码字段已解密
    pub fn with_decrypted_params(&self) -> Self {
        let mut cloned = self.clone();
        cloned.params = cloned.decrypt_params();
        cloned
    }
}

pub fn has_connection_decrypt_failure(connection_type: ConnectionType, params: &str) -> bool {
    if connection_type != ConnectionType::Extension {
        return has_decrypt_failure_in_sensitive_fields(params);
    }
    let Ok(params) = serde_json::from_str::<ExtensionConnectionParams>(params) else {
        return true;
    };
    params.validate().is_err()
        || params.secrets.values().any(|secret| {
            crypto::is_encrypted(secret) && crypto::decrypt_password(secret).is_empty()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn active_connection_lease_cleans_up_on_drop() {
        let mut active = ActiveConnections::new();
        let lease = active.lease(42);

        assert!(active.is_active(42));
        drop(lease);
        assert!(!active.is_active(42));
    }

    #[test]
    fn extension_connection_encrypts_arbitrary_secret_fields() {
        crypto::set_master_key_for_session("extension-connection-test-key").unwrap();
        let params = ExtensionConnectionParams::new(
            "com.example.search",
            "search",
            serde_json::Map::from_iter([("url".into(), Value::String("https://example".into()))]),
            BTreeMap::from([("api_key".into(), "secret-value".into())]),
        )
        .unwrap();
        let connection = StoredConnection::new_extension("Search".into(), params, Some(7));

        let encrypted = connection.try_encrypt_params().unwrap();
        assert!(!encrypted.contains("secret-value"));
        let encrypted: ExtensionConnectionParams = serde_json::from_str(&encrypted).unwrap();
        assert!(crypto::is_encrypted(&encrypted.secrets["api_key"]));
    }

    fn ssh_connection_with_id(id: i64, auth_method: SshAuthMethod) -> StoredConnection {
        let mut connection = StoredConnection::new_ssh(
            "prod-bastion".to_string(),
            SshParams {
                disabled_jump_server: None,
                sftp_default_directory: None,
                sftp_account: None,
                host: "bastion.example.com".to_string(),
                port: 2222,
                username: "deploy".to_string(),
                auth_method,
                credential_reference: None,
                prompt_username: None,
                prompt_password: None,
                keyboard_interactive: None,
                terminal_encoding: Default::default(),
                terminal_type: Default::default(),
                connect_timeout: Some(15),
                keepalive_interval: Some(30),
                keepalive_max: Some(3),
                default_directory: None,
                init_script: None,
                disable_shell_integration: None,
                x11_forwarding: None,
                allow_legacy_algorithms: None,
                jump_server: None,
                proxy: None,
                os_id: None,
                icon: None,
                icon_file_path: None,
                account_expect: Default::default(),
            },
            Some(7),
        );
        connection.id = Some(id);
        connection
    }

    #[test]
    fn stored_connection_credential_revision_is_local_only() {
        let mut connection = ssh_connection_with_id(42, SshAuthMethod::Agent);
        connection.credential_revision = Some(7);

        let serialized = serde_json::to_string(&connection).expect("serialize connection");
        assert!(!serialized.contains("credential_revision"));

        let restored: StoredConnection =
            serde_json::from_str(&serialized).expect("deserialize connection");
        assert_eq!(Some(42), restored.id);
        assert_eq!(None, restored.credential_revision);
    }

    fn database_config_with_ssh_ref(ssh_connection_id: i64) -> DbConnectionConfig {
        let mut extra_params = HashMap::new();
        extra_params.insert("ssh_tunnel_enabled".to_string(), "true".to_string());
        extra_params.insert(
            "ssh_connection_id".to_string(),
            ssh_connection_id.to_string(),
        );

        DbConnectionConfig {
            id: "db-1".to_string(),
            database_type: DatabaseType::MySQL,
            name: "prod mysql".to_string(),
            host: "mysql.internal".to_string(),
            port: 3306,
            username: "root".to_string(),
            password: "secret".to_string(),
            database: None,
            service_name: None,
            sid: None,
            workspace_id: Some(7),
            proxy: None,
            extra_params,
            credential_reference: None,
        }
    }

    #[test]
    fn database_config_deserializes_legacy_json_without_proxy() {
        let json = r#"{
            "database_type":"MySQL",
            "host":"db.internal",
            "port":3306,
            "username":"root",
            "password":"secret",
            "database":null,
            "service_name":null,
            "sid":null,
            "extra_params":{}
        }"#;

        let config: DbConnectionConfig = serde_json::from_str(json).unwrap();

        assert!(config.proxy.is_none());
    }

    #[test]
    fn database_config_round_trip_preserves_proxy() {
        let mut config = database_config_with_ssh_ref(42);
        config.proxy = Some(ProxyConfig {
            proxy_type: ProxyType::Http,
            host: "proxy.example.com".to_string(),
            port: 8080,
            username: Some("alice".to_string()),
            password: Some("secret".to_string()),
            credential_reference: None,
        });

        let json = serde_json::to_string(&config).unwrap();
        let restored: DbConnectionConfig = serde_json::from_str(&json).unwrap();

        let proxy = restored.proxy.expect("proxy should round trip");
        assert_eq!(ProxyType::Http, proxy.proxy_type);
        assert_eq!("proxy.example.com", proxy.host);
        assert_eq!(Some("alice".to_string()), proxy.username);
    }

    #[test]
    fn database_config_change_detection_includes_proxy() {
        let original = database_config_with_ssh_ref(42);
        let mut proxied = original.clone();
        proxied.proxy = Some(ProxyConfig {
            proxy_type: ProxyType::Socks5,
            host: "proxy.example.com".to_string(),
            port: 1080,
            username: None,
            password: None,
            credential_reference: None,
        });

        assert!(original.is_change(&proxied));
    }

    #[test]
    fn referenced_ssh_tunnel_inherits_proxy_when_database_has_none() {
        let mut ssh = ssh_connection_with_id(42, SshAuthMethod::Agent);
        let mut ssh_params = ssh.to_ssh_params().unwrap();
        ssh_params.proxy = Some(ProxyConfig {
            proxy_type: ProxyType::Socks5,
            host: "proxy.example.com".to_string(),
            port: 1080,
            username: Some("alice".to_string()),
            password: Some("secret".to_string()),
            credential_reference: None,
        });
        ssh.params = serde_json::to_string(&ssh_params).unwrap();
        let mut database = database_config_with_ssh_ref(42);

        database.apply_referenced_ssh_tunnel(&ssh).unwrap();

        assert_eq!(
            Some("proxy.example.com"),
            database.proxy.as_ref().map(|proxy| proxy.host.as_str())
        );
    }

    #[test]
    fn proxy_config_debug_redacts_password() {
        let proxy = ProxyConfig {
            proxy_type: ProxyType::Http,
            host: "proxy.example.com".to_string(),
            port: 8080,
            username: Some("alice".to_string()),
            password: Some("proxy-secret".to_string()),
            credential_reference: None,
        };

        let debug = format!("{proxy:?}");

        assert!(debug.contains("proxy.example.com"));
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("proxy-secret"));
    }

    #[test]
    fn empty_connection_names_default_to_target_address() {
        let db = DbConnectionConfig {
            id: String::new(),
            database_type: DatabaseType::MySQL,
            name: String::new(),
            host: "127.0.0.1".to_string(),
            port: 3306,
            username: "root".to_string(),
            password: String::new(),
            database: None,
            service_name: None,
            sid: None,
            workspace_id: None,
            proxy: None,
            extra_params: HashMap::new(),
            credential_reference: None,
        };
        assert_eq!(
            "127.0.0.1:3306",
            StoredConnection::new_database(String::new(), db, None).name
        );

        let ssh = SshParams {
            disabled_jump_server: None,
            sftp_default_directory: None,
            sftp_account: None,
            host: "localhost".to_string(),
            port: 22,
            username: "root".to_string(),
            auth_method: SshAuthMethod::Agent,
            credential_reference: None,
            prompt_username: None,
            prompt_password: None,
            keyboard_interactive: None,
            terminal_encoding: Default::default(),
            terminal_type: Default::default(),
            connect_timeout: None,
            keepalive_interval: None,
            keepalive_max: None,
            default_directory: None,
            init_script: None,
            disable_shell_integration: None,
            x11_forwarding: None,
            allow_legacy_algorithms: None,
            jump_server: None,
            proxy: None,
            os_id: None,
            icon: None,
            icon_file_path: None,
            account_expect: Default::default(),
        };
        assert_eq!(
            "root@localhost:22",
            StoredConnection::new_ssh(String::new(), ssh, None).name
        );

        let redis = RedisParams {
            host: "10.0.0.5".to_string(),
            port: 6379,
            password: None,
            username: None,
            db_index: 0,
            mode: RedisMode::Standalone,
            use_tls: false,
            connect_timeout: None,
            sentinel: None,
            cluster: None,
            ssh_tunnel: None,
            credential_reference: None,
        };
        assert_eq!(
            "10.0.0.5:6379",
            StoredConnection::new_redis(String::new(), redis, None).name
        );

        let mongo = MongoDBParams {
            driver_variant: MongoDriverVariant::Modern,
            connection_string: String::new(),
            host: "mongo.internal".to_string(),
            port: Some(27017),
            database: None,
            username: None,
            password: None,
            auth_source: None,
            replica_set: None,
            read_preference: None,
            use_srv_record: false,
            direct_connection: false,
            use_tls: false,
            connect_timeout_seconds: None,
            application_name: None,
            ssh_tunnel: None,
            credential_reference: None,
        };
        assert_eq!(
            "mongo.internal:27017",
            StoredConnection::new_mongodb(String::new(), mongo, None).name
        );

        let remote = RemoteDesktopParams {
            protocol: RemoteDesktopProtocol::Rdp,
            host: "winhost".to_string(),
            port: 3389,
            username: None,
            password: None,
            domain: None,
            read_only: false,
            audio_playback: false,
            proxy: None,
            credential_reference: None,
            backend_preference: RemoteDesktopBackendPreference::Auto,
            rdp: None,
        };
        assert_eq!(
            "winhost:3389",
            StoredConnection::new_remote_desktop(String::new(), remote, None).name
        );

        let serial = SerialParams {
            port_name: "/dev/tty.usbserial".to_string(),
            baud_rate: 115200,
            data_bits: 8,
            stop_bits: 1,
            parity: SerialParity::None,
            flow_control: SerialFlowControl::None,
        };
        assert_eq!(
            "/dev/tty.usbserial",
            StoredConnection::new_serial(String::new(), serial, None).name
        );

        let forward = PortForwardingParams {
            ssh_connection_id: 42,
            kind: PortForwardingKind::Local,
            bind_host: "127.0.0.1".to_string(),
            bind_port: 15432,
            target_host: "db.internal".to_string(),
            target_port: 5432,
        };
        assert_eq!(
            "127.0.0.1:15432 -> db.internal:5432",
            StoredConnection::new_port_forwarding(String::new(), forward, None).name
        );
    }

    #[test]
    fn explicit_connection_names_are_preserved() {
        let db = database_config_with_ssh_ref(42);
        assert_eq!(
            "  keep spaces  ",
            StoredConnection::new_database("  keep spaces  ".to_string(), db, None).name
        );
    }

    #[test]
    fn db_connection_can_apply_referenced_password_ssh_connection() {
        let ssh = ssh_connection_with_id(
            42,
            SshAuthMethod::Password {
                password: "ssh-secret".to_string(),
            },
        );
        let mut db = database_config_with_ssh_ref(42);

        db.apply_referenced_ssh_tunnel(&ssh)
            .expect("referenced ssh connection should be applied");

        assert_eq!(
            Some(&"bastion.example.com".to_string()),
            db.extra_params.get("ssh_host")
        );
        assert_eq!(Some(&"2222".to_string()), db.extra_params.get("ssh_port"));
        assert_eq!(
            Some(&"deploy".to_string()),
            db.extra_params.get("ssh_username")
        );
        assert_eq!(
            Some(&"password".to_string()),
            db.extra_params.get("ssh_auth_type")
        );
        assert_eq!(
            Some(&"ssh-secret".to_string()),
            db.extra_params.get("ssh_password")
        );
        assert_eq!(Some(&"15".to_string()), db.extra_params.get("ssh_timeout"));
    }

    #[test]
    fn ssh_connection_round_trips_private_key_content() {
        let connection = ssh_connection_with_id(
            42,
            SshAuthMethod::PrivateKeyContent {
                private_key: "-----BEGIN OPENSSH PRIVATE KEY-----\nfixture\n".to_string(),
                passphrase: Some("secret".to_string()),
            },
        );

        let params = connection
            .to_ssh_params()
            .expect("ssh params should decode");

        assert!(matches!(
            params.auth_method,
            SshAuthMethod::PrivateKeyContent {
                private_key,
                passphrase: Some(passphrase),
            } if private_key.contains("OPENSSH PRIVATE KEY") && passphrase == "secret"
        ));
    }

    #[test]
    fn private_key_content_fields_are_sensitive() {
        assert!(is_sensitive_field("private_key"));
        assert!(is_sensitive_field("private_key_content"));
        assert!(is_sensitive_field("ssh_private_key_content"));
    }

    #[test]
    fn stored_db_connection_keeps_only_ssh_reference_before_runtime_resolution() {
        let db = database_config_with_ssh_ref(42);
        let stored = StoredConnection::from_db_connection(db);

        let parsed = stored
            .to_db_connection()
            .expect("stored db connection should parse");

        assert_eq!(
            Some(&"42".to_string()),
            parsed.extra_params.get("ssh_connection_id")
        );
        assert_eq!(None, parsed.extra_params.get("ssh_password"));
        assert_eq!(None, parsed.extra_params.get("ssh_host"));
    }

    #[test]
    fn db_connection_applies_referenced_auto_publickey_ssh_connection() {
        let ssh = ssh_connection_with_id(42, SshAuthMethod::AutoPublicKey);
        let mut db = database_config_with_ssh_ref(42);

        db.apply_referenced_ssh_tunnel(&ssh)
            .expect("auto public key ssh connection should be reusable by db tunnel");

        assert_eq!(
            Some(&"auto_publickey".to_string()),
            db.extra_params.get("ssh_auth_type")
        );
        assert_eq!(None, db.extra_params.get("ssh_password"));
        assert_eq!(None, db.extra_params.get("ssh_private_key_path"));
    }

    #[test]
    fn redis_params_can_apply_referenced_password_ssh_connection() {
        let ssh = ssh_connection_with_id(
            42,
            SshAuthMethod::Password {
                password: "ssh-secret".to_string(),
            },
        );
        let mut redis = RedisParams {
            host: "redis.internal".to_string(),
            port: 6379,
            password: None,
            username: None,
            db_index: 0,
            mode: RedisMode::Standalone,
            use_tls: false,
            connect_timeout: Some(10),
            sentinel: None,
            cluster: None,
            ssh_tunnel: Some(RedisSshTunnelConfig {
                enabled: true,
                connection_id: Some(42),
                ..Default::default()
            }),
            credential_reference: None,
        };

        redis
            .apply_referenced_ssh_tunnel(&ssh)
            .expect("referenced ssh connection should be applied");

        let tunnel = redis
            .ssh_tunnel
            .as_ref()
            .expect("ssh tunnel config should remain present");
        assert_eq!("bastion.example.com", tunnel.host);
        assert_eq!(2222, tunnel.port);
        assert_eq!("deploy", tunnel.username);
        assert_eq!("password", tunnel.auth_type);
        assert_eq!(Some("ssh-secret".to_string()), tunnel.password);
        assert_eq!(Some(15), tunnel.timeout);
        assert_eq!(Some("redis.internal".to_string()), tunnel.target_host);
        assert_eq!(Some(6379), tunnel.target_port);
    }

    #[test]
    fn mongodb_params_deserializes_legacy_json_without_ssh_tunnel() {
        let params: MongoDBParams = serde_json::from_value(serde_json::json!({
            "host": "mongo.internal",
            "port": 27017,
            "database": "app"
        }))
        .expect("legacy mongodb params should parse without ssh_tunnel");

        assert_eq!("mongo.internal", params.host);
        assert_eq!(Some(27017), params.port);
        assert!(matches!(params.driver_variant, MongoDriverVariant::Modern));
        assert_eq!(None, params.ssh_tunnel);
    }

    #[test]
    fn mongodb_params_can_apply_referenced_password_ssh_connection() {
        let ssh = ssh_connection_with_id(
            42,
            SshAuthMethod::Password {
                password: "ssh-secret".to_string(),
            },
        );
        let mut mongo = MongoDBParams {
            driver_variant: MongoDriverVariant::Modern,
            connection_string: String::new(),
            host: "mongo.internal".to_string(),
            port: Some(27018),
            database: Some("app".to_string()),
            username: None,
            password: None,
            auth_source: None,
            replica_set: None,
            read_preference: None,
            use_srv_record: false,
            direct_connection: false,
            use_tls: false,
            connect_timeout_seconds: Some(10),
            application_name: None,
            ssh_tunnel: Some(MongoSshTunnelConfig {
                enabled: true,
                connection_id: Some(42),
                ..Default::default()
            }),
            credential_reference: None,
        };

        mongo
            .apply_referenced_ssh_tunnel(&ssh)
            .expect("referenced ssh connection should be applied");

        let tunnel = mongo
            .ssh_tunnel
            .as_ref()
            .expect("ssh tunnel config should remain present");
        assert_eq!("bastion.example.com", tunnel.host);
        assert_eq!(2222, tunnel.port);
        assert_eq!("deploy", tunnel.username);
        assert_eq!("password", tunnel.auth_type);
        assert_eq!(Some("ssh-secret".to_string()), tunnel.password);
        assert_eq!(Some(15), tunnel.timeout);
        assert_eq!(Some("mongo.internal".to_string()), tunnel.target_host);
        assert_eq!(Some(27018), tunnel.target_port);
    }

    #[test]
    fn external_database_type_carries_driver_identity() {
        let database_type = DatabaseType::external("iotdb");

        assert_eq!("External", database_type.as_str());
        assert_eq!("iotdb", database_type.external_driver_id().unwrap());
        assert_eq!("External:iotdb", database_type.storage_key());
        assert_eq!("External_iotdb", database_type.path_key());
        assert_eq!(
            Some(DatabaseType::external("iotdb")),
            DatabaseType::from_storage_key("External:iotdb")
        );
        assert_eq!(None, DatabaseType::from_str("External"));
        assert!(database_type.is_external());
    }

    #[test]
    fn duckdb_database_type_stays_compatible_with_historical_storage() {
        let json = r#"{
            "database_type": "DuckDB",
            "host": "/tmp/history.duckdb",
            "port": 0,
            "username": "",
            "password": "",
            "database": null,
            "service_name": null,
            "sid": null,
            "extra_params": {}
        }"#;

        let config: DbConnectionConfig =
            serde_json::from_str(json).expect("historical DuckDB config should deserialize");

        assert_eq!(DatabaseType::DuckDB, config.database_type);
        assert_eq!(Some(DatabaseType::DuckDB), DatabaseType::from_str("DuckDB"));
        assert_eq!(
            Some(DatabaseType::DuckDB),
            DatabaseType::from_storage_key("DuckDB")
        );
        assert_eq!("/tmp/history.duckdb", config.server_info());
    }
}

/// 递归加密 JSON 中所有敏感字符串字段
fn encrypt_json_passwords(json_str: &str) -> String {
    match serde_json::from_str::<Value>(json_str) {
        Ok(mut value) => {
            encrypt_value(&mut value);
            serde_json::to_string(&value).unwrap_or_else(|_| json_str.to_string())
        }
        Err(_) => json_str.to_string(),
    }
}

/// 递归解密 JSON 中所有敏感字符串字段
fn decrypt_json_passwords(json_str: &str) -> String {
    match serde_json::from_str::<Value>(json_str) {
        Ok(mut value) => {
            decrypt_value(&mut value);
            serde_json::to_string(&value).unwrap_or_else(|_| json_str.to_string())
        }
        Err(_) => json_str.to_string(),
    }
}

pub(crate) fn re_encrypt_sensitive_json(
    json_str: &str,
    old_key: &str,
    new_key: &str,
) -> anyhow::Result<String> {
    let mut value: Value = serde_json::from_str(json_str)?;
    if value.get("schema_version").and_then(Value::as_u64)
        == Some(u64::from(EXTENSION_CONNECTION_SCHEMA_VERSION))
        && value.get("extension_id").is_some()
        && value.get("contribution_id").is_some()
    {
        if let Some(secrets) = value.get_mut("secrets").and_then(Value::as_object_mut) {
            for secret in secrets.values_mut() {
                re_encrypt_string(secret, old_key, new_key)?;
            }
        }
    } else {
        re_encrypt_value(&mut value, old_key, new_key)?;
    }
    Ok(serde_json::to_string(&value)?)
}

/// 判断字段名是否为敏感字段
fn is_sensitive_field(key: &str) -> bool {
    key == "password"
        || key == "passphrase"
        || key == "private_key"
        || key == "private_key_content"
        // Telnet 登录脚本的自动发送内容通常包含密码/enable 密码/token。
        || key == "send"
        || key.ends_with("_password")
        || key.ends_with("_passphrase")
        || key.ends_with("_private_key")
        || key.ends_with("_private_key_content")
}

fn re_encrypt_value(
    value: &mut Value,
    old_key: &str,
    new_key: &str,
) -> Result<(), crypto::CryptoError> {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if is_sensitive_field(key) {
                    re_encrypt_string(value, old_key, new_key)?;
                } else {
                    re_encrypt_value(value, old_key, new_key)?;
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                re_encrypt_value(value, old_key, new_key)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn re_encrypt_string(
    value: &mut Value,
    old_key: &str,
    new_key: &str,
) -> Result<(), crypto::CryptoError> {
    let Value::String(secret) = value else {
        return Ok(());
    };
    if !secret.is_empty() {
        *secret = crypto::re_encrypt_data(secret, old_key, new_key)?;
    }
    Ok(())
}

/// 递归遍历 JSON Value，加密敏感字段
fn encrypt_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, val) in map.iter_mut() {
                if is_sensitive_field(key) {
                    if let Value::String(s) = val {
                        *s = crypto::encrypt_password(s);
                    }
                } else {
                    encrypt_value(val);
                }
            }
        }
        Value::Array(arr) => {
            for item in arr.iter_mut() {
                encrypt_value(item);
            }
        }
        _ => {}
    }
}

/// 递归遍历 JSON Value，解密敏感字段
fn decrypt_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, val) in map.iter_mut() {
                if is_sensitive_field(key) {
                    if let Value::String(s) = val {
                        *s = crypto::decrypt_password(s);
                    }
                } else {
                    decrypt_value(val);
                }
            }
        }
        Value::Array(arr) => {
            for item in arr.iter_mut() {
                decrypt_value(item);
            }
        }
        _ => {}
    }
}

/// 检测 params 中是否存在“已加密字段解密失败”的情况。
///
/// 规则：敏感字段（password/passphrase）若以 ENC: 开头，且解密结果为空，视为失败。
pub fn has_decrypt_failure_in_sensitive_fields(json_str: &str) -> bool {
    match serde_json::from_str::<Value>(json_str) {
        Ok(value) => has_decrypt_failure_in_value(&value),
        Err(_) => false,
    }
}

fn has_decrypt_failure_in_value(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(key, val)| {
            if is_sensitive_field(key) {
                if let Value::String(s) = val {
                    return crypto::is_encrypted(s) && crypto::decrypt_password(s).is_empty();
                }
                false
            } else {
                has_decrypt_failure_in_value(val)
            }
        }),
        Value::Array(arr) => arr.iter().any(has_decrypt_failure_in_value),
        _ => false,
    }
}

/// Generic key-value storage model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyValue {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    pub key: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
}

impl KeyValue {
    pub fn new(key: String, value: String) -> Self {
        Self {
            id: None,
            key,
            value,
            created_at: None,
            updated_at: None,
        }
    }
}

pub fn parse_db_type(s: &str) -> DatabaseType {
    match s {
        "MySQL" => DatabaseType::MySQL,
        "PostgreSQL" => DatabaseType::PostgreSQL,
        "SQLite" => DatabaseType::SQLite,
        "DuckDB" => DatabaseType::DuckDB,
        _ => DatabaseType::MySQL,
    }
}

#[cfg(test)]
mod serial_tests {
    use super::*;

    #[test]
    fn serial_params_serialize_deserialize() {
        let params = SerialParams {
            port_name: "/dev/ttyUSB0".to_string(),
            baud_rate: 115200,
            data_bits: 8,
            stop_bits: 1,
            parity: SerialParity::None,
            flow_control: SerialFlowControl::None,
        };
        let json = serde_json::to_string(&params).unwrap();
        let p2: SerialParams = serde_json::from_str(&json).unwrap();
        assert_eq!(p2.port_name, "/dev/ttyUSB0");
        assert_eq!(p2.baud_rate, 115200);
        assert_eq!(p2.data_bits, 8);
        assert_eq!(p2.stop_bits, 1);
        assert_eq!(p2.parity, SerialParity::None);
        assert_eq!(p2.flow_control, SerialFlowControl::None);
    }

    #[test]
    fn serial_params_defaults_from_minimal_json() {
        let json = r#"{"port_name":"/dev/tty0"}"#;
        let p: SerialParams = serde_json::from_str(json).unwrap();
        assert_eq!(p.port_name, "/dev/tty0");
        assert_eq!(p.baud_rate, 115200);
        assert_eq!(p.data_bits, 8);
        assert_eq!(p.stop_bits, 1);
        assert_eq!(p.parity, SerialParity::None);
        assert_eq!(p.flow_control, SerialFlowControl::None);
    }

    #[test]
    fn stored_connection_serial_roundtrip() {
        let params = SerialParams {
            port_name: "/dev/cu.usbserial-1420".to_string(),
            baud_rate: 9600,
            data_bits: 7,
            stop_bits: 2,
            parity: SerialParity::Even,
            flow_control: SerialFlowControl::Hardware,
        };
        let conn = StoredConnection::new_serial("我的串口".to_string(), params, Some(42));
        assert_eq!(conn.connection_type, ConnectionType::Serial);
        assert_eq!(conn.name, "我的串口");
        assert_eq!(conn.workspace_id, Some(42));

        let rt = conn.to_serial_params().unwrap();
        assert_eq!(rt.port_name, "/dev/cu.usbserial-1420");
        assert_eq!(rt.baud_rate, 9600);
        assert_eq!(rt.data_bits, 7);
        assert_eq!(rt.stop_bits, 2);
        assert_eq!(rt.parity, SerialParity::Even);
        assert_eq!(rt.flow_control, SerialFlowControl::Hardware);
    }

    #[test]
    fn connection_type_serial_methods() {
        assert_eq!(ConnectionType::Serial.label(), "Serial");
        assert_eq!(ConnectionType::from_str("Serial"), ConnectionType::Serial);
        assert_eq!(format!("{}", ConnectionType::Serial), "Serial");
        assert!(ConnectionType::all().contains(&ConnectionType::Serial));
    }

    #[test]
    fn stored_connection_port_forwarding_roundtrip() {
        let params = PortForwardingParams {
            ssh_connection_id: 7,
            kind: PortForwardingKind::Local,
            bind_host: "127.0.0.1".to_string(),
            bind_port: 15432,
            target_host: "db.internal".to_string(),
            target_port: 5432,
        };
        let conn =
            StoredConnection::new_port_forwarding("postgres tunnel".to_string(), params, Some(42));
        assert_eq!(conn.connection_type, ConnectionType::PortForwarding);
        assert_eq!(conn.name, "postgres tunnel");
        assert_eq!(conn.workspace_id, Some(42));

        let rt = conn.to_port_forwarding_params().unwrap();
        assert_eq!(rt.ssh_connection_id, 7);
        assert_eq!(rt.kind, PortForwardingKind::Local);
        assert_eq!(rt.bind_host, "127.0.0.1");
        assert_eq!(rt.bind_port, 15432);
        assert_eq!(rt.target_host, "db.internal");
        assert_eq!(rt.target_port, 5432);
    }

    #[test]
    fn stored_connection_remote_forwarding_roundtrip() {
        let params = PortForwardingParams {
            ssh_connection_id: 7,
            kind: PortForwardingKind::Remote,
            bind_host: "127.0.0.1".to_string(),
            bind_port: 18080,
            target_host: "127.0.0.1".to_string(),
            target_port: 3000,
        };
        let conn = StoredConnection::new_port_forwarding(String::new(), params, Some(42));

        assert_eq!(conn.name, "127.0.0.1:18080 <- 127.0.0.1:3000");
        let rt = conn.to_port_forwarding_params().unwrap();
        assert_eq!(rt.kind, PortForwardingKind::Remote);
        assert_eq!(rt.bind_port, 18080);
        assert_eq!(rt.target_port, 3000);
    }

    #[test]
    fn connection_type_port_forwarding_methods() {
        assert_eq!(ConnectionType::PortForwarding.label(), "Port Forwarding");
        assert_eq!(
            ConnectionType::from_str("PortForwarding"),
            ConnectionType::PortForwarding
        );
        assert_eq!(
            format!("{}", ConnectionType::PortForwarding),
            "PortForwarding"
        );
        assert!(ConnectionType::all().contains(&ConnectionType::PortForwarding));
    }

    #[test]
    fn connection_type_remote_desktop_methods() {
        assert_eq!(ConnectionType::Rdp.label(), "RDP");
        assert_eq!(ConnectionType::Vnc.label(), "VNC");
        assert_eq!(ConnectionType::from_str("Rdp"), ConnectionType::Rdp);
        assert_eq!(ConnectionType::from_str("Vnc"), ConnectionType::Vnc);
        assert_eq!(format!("{}", ConnectionType::Rdp), "Rdp");
        assert_eq!(format!("{}", ConnectionType::Vnc), "Vnc");
        assert!(ConnectionType::all().contains(&ConnectionType::Rdp));
        assert!(ConnectionType::all().contains(&ConnectionType::Vnc));
    }

    #[test]
    fn stored_connection_remote_desktop_uses_remote_desktop_params_shape() {
        let params = RemoteDesktopParams {
            protocol: RemoteDesktopProtocol::Rdp,
            host: "10.2.178.12".to_string(),
            port: 3389,
            username: Some("administrator".to_string()),
            password: Some("secret".to_string()),
            domain: Some("corp".to_string()),
            read_only: false,
            audio_playback: false,
            proxy: None,
            credential_reference: None,
            backend_preference: RemoteDesktopBackendPreference::Canvas,
            rdp: None,
        };

        let conn = StoredConnection::new_remote_desktop("win-rdp".to_string(), params, Some(42));

        assert_eq!(conn.connection_type, ConnectionType::Rdp);
        assert_eq!(conn.workspace_id, Some(42));
        let parsed = conn
            .to_remote_desktop_params()
            .expect("RDP params parse as RemoteDesktopParams");
        assert_eq!(parsed.protocol, RemoteDesktopProtocol::Rdp);
        assert_eq!(parsed.host, "10.2.178.12");
        assert_eq!(parsed.port, 3389);
        assert_eq!(parsed.username.as_deref(), Some("administrator"));
        assert_eq!(parsed.domain.as_deref(), Some("corp"));
        let raw_params =
            serde_json::from_str::<Value>(&conn.params).expect("RDP params parse as JSON");
        assert!(raw_params.get("width").is_none());
        assert!(raw_params.get("height").is_none());
        assert!(raw_params.get("backend_preference").is_none());
        assert_eq!(RemoteDesktopProtocol::Vnc.default_port(), 5900);
    }

    #[test]
    fn remote_desktop_params_deserialize_legacy_json_without_proxy() {
        let json = r#"{
            "protocol":"Rdp",
            "host":"10.0.0.8",
            "port":3389,
            "username":null,
            "password":null,
            "domain":null,
            "read_only":false
        }"#;

        let params: RemoteDesktopParams = serde_json::from_str(json).unwrap();

        assert!(params.proxy.is_none());
        assert!(!params.audio_playback);
        assert_eq!(
            RemoteDesktopBackendPreference::Canvas,
            params.backend_preference
        );
    }

    #[test]
    fn remote_desktop_params_round_trip_preserves_audio_playback() {
        let json = r#"{
            "protocol":"Rdp",
            "host":"10.0.0.8",
            "port":3389,
            "username":null,
            "password":null,
            "domain":null,
            "read_only":false,
            "audio_playback":true
        }"#;

        let params: RemoteDesktopParams = serde_json::from_str(json).unwrap();
        assert!(params.audio_playback);

        let restored: RemoteDesktopParams =
            serde_json::from_str(&serde_json::to_string(&params).unwrap()).unwrap();
        assert!(restored.audio_playback);
    }

    #[test]
    fn remote_desktop_params_round_trip_preserves_backend_preference() {
        let json = r#"{
            "protocol":"Rdp",
            "host":"10.0.0.8",
            "port":3389,
            "username":null,
            "password":null,
            "domain":null,
            "read_only":false,
            "backend_preference":"windows_native"
        }"#;

        let params: RemoteDesktopParams = serde_json::from_str(json).unwrap();
        assert_eq!(
            RemoteDesktopBackendPreference::WindowsNative,
            params.backend_preference
        );

        let restored: RemoteDesktopParams =
            serde_json::from_str(&serde_json::to_string(&params).unwrap()).unwrap();
        assert_eq!(
            RemoteDesktopBackendPreference::WindowsNative,
            restored.backend_preference
        );
    }

    #[test]
    fn remote_desktop_params_round_trip_preserves_explicit_auto_backend() {
        let json = r#"{
            "protocol":"Rdp",
            "host":"10.0.0.8",
            "port":3389,
            "username":null,
            "password":null,
            "domain":null,
            "read_only":false,
            "backend_preference":"auto"
        }"#;

        let params: RemoteDesktopParams = serde_json::from_str(json).unwrap();
        let serialized = serde_json::to_value(&params).unwrap();
        assert_eq!(Some("auto"), serialized["backend_preference"].as_str());

        let restored: RemoteDesktopParams = serde_json::from_value(serialized).unwrap();
        assert_eq!(
            RemoteDesktopBackendPreference::Auto,
            restored.backend_preference
        );
    }

    #[test]
    fn remote_desktop_params_round_trip_preserves_proxy() {
        let params = RemoteDesktopParams {
            protocol: RemoteDesktopProtocol::Vnc,
            host: "10.0.0.9".to_string(),
            port: 5900,
            username: None,
            password: Some("vnc-secret".to_string()),
            domain: None,
            read_only: true,
            audio_playback: false,
            proxy: Some(ProxyConfig {
                proxy_type: ProxyType::Socks5,
                host: "proxy.example.com".to_string(),
                port: 1080,
                username: Some("alice".to_string()),
                password: Some("proxy-secret".to_string()),
                credential_reference: None,
            }),
            credential_reference: None,
            backend_preference: RemoteDesktopBackendPreference::Auto,
            rdp: None,
        };

        let json = serde_json::to_string(&params).unwrap();
        let restored: RemoteDesktopParams = serde_json::from_str(&json).unwrap();

        assert_eq!(
            Some("proxy.example.com"),
            restored.proxy.as_ref().map(|proxy| proxy.host.as_str())
        );
    }

    #[test]
    fn serial_enums_defaults_and_labels() {
        assert_eq!(SerialParity::default(), SerialParity::None);
        assert_eq!(SerialFlowControl::default(), SerialFlowControl::None);
        assert_eq!(SerialParity::all().len(), 3);
        assert_eq!(SerialFlowControl::all().len(), 3);
        assert_eq!(SerialParity::Odd.label(), "Odd");
        assert_eq!(SerialParity::Even.label(), "Even");
        assert_eq!(SerialFlowControl::Software.label(), "XON/XOFF");
        assert_eq!(SerialFlowControl::Hardware.label(), "RTS/CTS");
    }

    #[test]
    fn ssh_auth_method_agent_serialize_deserialize() {
        let auth = SshAuthMethod::Agent;
        let json = serde_json::to_string(&auth).expect("Agent 认证方式应可序列化");
        let parsed: SshAuthMethod =
            serde_json::from_str(&json).expect("Agent 认证方式应可反序列化");
        assert!(matches!(parsed, SshAuthMethod::Agent));
    }

    #[test]
    fn ssh_auth_method_pageant_serialize_deserialize() {
        let auth = SshAuthMethod::Pageant;
        let json = serde_json::to_string(&auth).expect("Pageant 认证方式应可序列化");
        assert_eq!(json, "\"Pageant\"");

        let parsed: SshAuthMethod =
            serde_json::from_str(&json).expect("Pageant 认证方式应可反序列化");
        assert!(matches!(parsed, SshAuthMethod::Pageant));
    }

    #[test]
    fn ssh_auth_method_auto_publickey_serialize_deserialize() {
        let auth = SshAuthMethod::AutoPublicKey;
        let json = serde_json::to_string(&auth).expect("自动公钥认证方式应可序列化");
        let parsed: SshAuthMethod =
            serde_json::from_str(&json).expect("自动公钥认证方式应可反序列化");
        assert!(matches!(parsed, SshAuthMethod::AutoPublicKey));
    }

    #[test]
    fn ssh_os_icon_maps_known_distros_and_defaults_to_penguin() {
        use super::ssh_os_icon;

        assert!(matches!(ssh_os_icon(Some("ubuntu")), IconName::UbuntuColor));
        assert!(matches!(ssh_os_icon(Some("centos")), IconName::CentosColor));
        assert!(matches!(ssh_os_icon(Some("debian")), IconName::DebianColor));
        assert!(matches!(
            ssh_os_icon(Some("almalinux")),
            IconName::AlmalinuxColor
        ));
        assert!(matches!(
            ssh_os_icon(Some("opensuse-leap")),
            IconName::OpensuseColor
        ));
        assert!(matches!(ssh_os_icon(Some("macos")), IconName::MacosColor));
        assert!(matches!(
            ssh_os_icon(Some("windows")),
            IconName::WindowsColor
        ));
        assert!(matches!(ssh_os_icon(Some("docker")), IconName::DockerColor));
        for (id, expected) in [
            ("rhel", IconName::RedhatColor),
            ("redhat", IconName::RedhatColor),
            ("rocky", IconName::RockyColor),
            ("fedora", IconName::FedoraColor),
            ("alpine", IconName::AlpineColor),
            ("arch", IconName::ArchColor),
            ("kali", IconName::KaliColor),
            ("amzn", IconName::AmazonColor),
            ("alinux", IconName::AlinuxColor),
            ("openeuler", IconName::OpenEulerColor),
            ("freebsd", IconName::FreebsdColor),
            ("cisco", IconName::CiscoColor),
            ("juniper", IconName::JuniperColor),
            ("huawei", IconName::HuaweiColor),
            ("h3c", IconName::H3cColor),
            ("hpe", IconName::HpeColor),
            ("mikrotik", IconName::MikrotikColor),
            ("fortinet", IconName::FortinetColor),
            ("paloalto", IconName::PaloaltoColor),
            ("zyxel", IconName::ZyxelColor),
        ] {
            assert!(
                matches!(ssh_os_icon(Some(id)), x if x == expected),
                "{id} 应映射到 {expected:?} 图标"
            );
        }
        assert!(matches!(
            ssh_os_icon(Some("kylin")),
            IconName::LinuxPenguinColor
        ));
        assert!(matches!(ssh_os_icon(None), IconName::LinuxPenguinColor));
    }

    #[test]
    fn ssh_params_os_id_round_trips_through_json() {
        let mut params = SshParams {
            disabled_jump_server: None,
            sftp_default_directory: None,
            sftp_account: None,
            host: "example.com".to_string(),
            port: 22,
            username: "root".to_string(),
            auth_method: SshAuthMethod::Agent,
            credential_reference: None,
            prompt_username: None,
            prompt_password: None,
            keyboard_interactive: None,
            terminal_encoding: Default::default(),
            terminal_type: Default::default(),
            connect_timeout: None,
            keepalive_interval: None,
            keepalive_max: None,
            default_directory: None,
            init_script: None,
            disable_shell_integration: None,
            x11_forwarding: None,
            allow_legacy_algorithms: None,
            jump_server: None,
            proxy: None,
            os_id: Some("ubuntu".to_string()),
            icon: None,
            icon_file_path: None,
            account_expect: Default::default(),
        };
        let json = serde_json::to_string(&params).expect("SshParams 应可序列化");
        assert!(json.contains("\"os_id\":\"ubuntu\""));
        let parsed: SshParams = serde_json::from_str(&json).expect("SshParams 应可反序列化");
        assert_eq!(Some("ubuntu".to_string()), parsed.os_id);

        params.os_id = None;
        let json = serde_json::to_string(&params).expect("SshParams 应可序列化");
        assert!(!json.contains("os_id"));
    }

    #[test]
    fn ssh_params_custom_icon_path_round_trips_and_defaults_to_none() {
        let mut params: SshParams = serde_json::from_str(
            r#"{"host":"example.com","port":22,"username":"root","auth_method":"Agent"}"#,
        )
        .expect("旧连接缺少自定义图标路径时应可反序列化");
        assert_eq!(params.icon_file_path, None);

        params.icon = Some("ubuntu".to_string());
        params.icon_file_path = Some("/tmp/custom-ssh-icon.svg".to_string());
        let json = serde_json::to_string(&params).expect("SshParams 应可序列化");
        let parsed: SshParams = serde_json::from_str(&json).expect("SshParams 应可反序列化");

        assert_eq!(parsed.icon, Some("ubuntu".to_string()));
        assert_eq!(
            parsed.icon_file_path,
            Some("/tmp/custom-ssh-icon.svg".to_string())
        );
    }

    #[test]
    fn ssh_params_legacy_algorithms_are_opt_in_and_round_trip() {
        let mut params: SshParams = serde_json::from_str(
            r#"{"host":"example.com","port":22,"username":"root","auth_method":"Agent"}"#,
        )
        .expect("旧连接缺少兼容算法字段时应可反序列化");
        assert_eq!(params.allow_legacy_algorithms, None);
        assert_eq!(params.terminal_encoding, StoredTerminalEncoding::Utf8);

        params.allow_legacy_algorithms = Some(true);
        let json = serde_json::to_string(&params).expect("SshParams 应可序列化");
        assert!(json.contains("\"allow_legacy_algorithms\":true"));

        let parsed: SshParams = serde_json::from_str(&json).expect("SshParams 应可反序列化");
        assert_eq!(parsed.allow_legacy_algorithms, Some(true));
    }

    #[test]
    fn ssh_account_expect_round_trips_and_legacy_json_defaults_empty() {
        let params: SshParams = serde_json::from_value(serde_json::json!({
            "host": "example.com",
            "port": 22,
            "username": "root",
            "auth_method": "Agent",
            "account_expect": {
                "username": {
                    "expect": "(?i)login:",
                    "send": "admin"
                },
                "password": {
                    "expect": "(?i)password:",
                    "send": "secret"
                }
            }
        }))
        .expect("SSH expect 配置应可反序列化");

        let json = serde_json::to_string(&params).expect("SSH expect 配置应可序列化");
        let parsed: SshParams =
            serde_json::from_str(&json).expect("SSH expect 配置应可再次反序列化");
        assert_eq!(parsed.account_expect, params.account_expect);
        assert!(json.contains("\"account_expect\""));

        let legacy: SshParams = serde_json::from_str(
            r#"{"host":"example.com","port":22,"username":"root","auth_method":"Agent"}"#,
        )
        .expect("旧 SSH 配置应可反序列化");
        assert!(legacy.account_expect.is_empty());
        let legacy_json = serde_json::to_string(&legacy).expect("旧 SSH 配置应可序列化");
        assert!(!legacy_json.contains("account_expect"));
    }

    #[test]
    fn ssh_params_sftp_account_round_trips_and_legacy_json_defaults_none() {
        let params: SshParams = serde_json::from_value(serde_json::json!({
            "host": "example.com",
            "port": 22,
            "username": "root",
            "auth_method": "Agent",
            "sftp_account": {
                "username": "sftp-user",
                "password": "sftp-secret"
            }
        }))
        .expect("带独立 SFTP 账户的配置应可反序列化");
        assert_eq!(
            params.sftp_account,
            Some(SftpAccount {
                username: "sftp-user".to_string(),
                password: "sftp-secret".to_string(),
            })
        );

        let json = serde_json::to_string(&params).expect("SSH 配置应可序列化");
        assert!(json.contains("\"sftp_account\""));
        let parsed: SshParams = serde_json::from_str(&json).expect("SSH 配置应可再次反序列化");
        assert_eq!(parsed.sftp_account, params.sftp_account);

        let legacy: SshParams = serde_json::from_str(
            r#"{"host":"example.com","port":22,"username":"root","auth_method":"Agent"}"#,
        )
        .expect("旧 SSH 配置应可反序列化");
        assert_eq!(legacy.sftp_account, None);
        let legacy_json = serde_json::to_string(&legacy).expect("旧 SSH 配置应可序列化");
        assert!(!legacy_json.contains("sftp_account"));
    }

    #[test]
    fn ssh_params_sftp_default_directory_round_trips_and_legacy_json_defaults_none() {
        let params: SshParams = serde_json::from_value(serde_json::json!({
            "host": "example.com",
            "port": 22,
            "username": "root",
            "auth_method": "Agent",
            "sftp_default_directory": "/data/upload"
        }))
        .expect("带 SFTP 初始目录的配置应可反序列化");
        assert_eq!(
            params.sftp_default_directory,
            Some("/data/upload".to_string())
        );

        let json = serde_json::to_string(&params).expect("SSH 配置应可序列化");
        assert!(json.contains("\"sftp_default_directory\""));

        let parsed: SshParams = serde_json::from_str(&json).expect("SSH 配置应可再次反序列化");
        assert_eq!(parsed.sftp_default_directory, params.sftp_default_directory);

        let legacy: SshParams = serde_json::from_str(
            r#"{"host":"example.com","port":22,"username":"root","auth_method":"Agent"}"#,
        )
        .expect("旧 SSH 配置应可反序列化");
        assert_eq!(legacy.sftp_default_directory, None);
        let legacy_json = serde_json::to_string(&legacy).expect("旧 SSH 配置应可序列化");
        assert!(!legacy_json.contains("sftp_default_directory"));
    }

    #[test]
    fn ssh_params_disabled_jump_server_round_trips_and_legacy_json_defaults_none() {
        let params: SshParams = serde_json::from_value(serde_json::json!({
            "host": "example.com",
            "port": 22,
            "username": "root",
            "auth_method": "Agent",
            "disabled_jump_server": {
                "host": "jump.example.com",
                "port": 2222,
                "username": "jump",
                "auth_method": {"Password": {"password": "secret"}}
            }
        }))
        .expect("带停用跳板机配置的参数应可反序列化");
        let stash = params
            .disabled_jump_server
            .as_ref()
            .expect("停用跳板机配置应存在");
        assert_eq!(stash.host, "jump.example.com");
        assert_eq!(stash.port, 2222);

        let json = serde_json::to_string(&params).expect("SSH 配置应可序列化");
        assert!(json.contains("\"disabled_jump_server\""));

        let parsed: SshParams = serde_json::from_str(&json).expect("SSH 配置应可再次反序列化");
        let parsed_stash = parsed
            .disabled_jump_server
            .as_ref()
            .expect("序列化往返后停用跳板机配置应保留");
        assert_eq!(parsed_stash.host, stash.host);
        assert_eq!(parsed_stash.port, stash.port);
        assert_eq!(parsed_stash.username, stash.username);

        let legacy: SshParams = serde_json::from_str(
            r#"{"host":"example.com","port":22,"username":"root","auth_method":"Agent"}"#,
        )
        .expect("旧 SSH 配置应可反序列化");
        assert!(legacy.disabled_jump_server.is_none());
        let legacy_json = serde_json::to_string(&legacy).expect("旧 SSH 配置应可序列化");
        assert!(!legacy_json.contains("disabled_jump_server"));
    }

    #[test]
    fn ssh_params_credential_prompt_policy_is_backward_compatible() {
        let mut params: SshParams = serde_json::from_str(
            r#"{"host":"example.com","port":22,"username":"root","auth_method":{"Password":{"password":"secret"}}}"#,
        )
        .expect("旧连接缺少凭据策略字段时应可反序列化");
        assert!(!params.prompts_for_username());
        assert!(!params.prompts_for_password());
        assert!(params.keyboard_interactive_enabled());

        params.prompt_username = Some(true);
        params.prompt_password = Some(false);
        params.keyboard_interactive = Some(false);
        let json = serde_json::to_string(&params).expect("SshParams 应可序列化");
        assert!(json.contains("\"prompt_username\":true"));
        assert!(json.contains("\"prompt_password\":false"));
        assert!(json.contains("\"keyboard_interactive\":false"));

        let parsed: SshParams = serde_json::from_str(&json).expect("SshParams 应可反序列化");
        assert!(parsed.prompts_for_username());
        assert!(!parsed.prompts_for_password());
        assert!(!parsed.keyboard_interactive_enabled());
    }

    #[test]
    fn stored_ssh_connection_clears_prompted_credentials() {
        let params: SshParams = serde_json::from_value(serde_json::json!({
            "host": "example.com",
            "port": 22,
            "username": "temporary-user",
            "auth_method": {
                "Password": {
                    "password": "temporary-password"
                }
            },
            "prompt_username": true,
            "prompt_password": true,
            "keyboard_interactive": true
        }))
        .expect("测试 SSH 参数应有效");

        let connection = StoredConnection::new_ssh(String::new(), params, None);
        assert_eq!(connection.name, "example.com:22");
        assert!(!connection.params.contains("temporary-user"));
        assert!(!connection.params.contains("temporary-password"));

        let stored = connection.to_ssh_params().expect("持久化参数应可读取");
        assert!(stored.username.is_empty());
        assert!(matches!(
            stored.auth_method,
            SshAuthMethod::Password { ref password } if password.is_empty()
        ));
        assert!(stored.prompts_for_username());
        assert!(stored.prompts_for_password());
        assert!(stored.keyboard_interactive_enabled());
    }

    #[test]
    fn ssh_storage_boundary_clears_prompted_credentials_from_raw_params() {
        let mut connection = StoredConnection::new_ssh(
            "example".to_string(),
            SshParams {
                disabled_jump_server: None,
                sftp_default_directory: None,
                sftp_account: None,
                host: "example.com".to_string(),
                port: 22,
                username: "stored-user".to_string(),
                auth_method: SshAuthMethod::Agent,
                credential_reference: None,
                prompt_username: None,
                prompt_password: None,
                keyboard_interactive: None,
                terminal_encoding: Default::default(),
                terminal_type: Default::default(),
                connect_timeout: None,
                keepalive_interval: None,
                keepalive_max: None,
                default_directory: None,
                init_script: None,
                disable_shell_integration: None,
                x11_forwarding: None,
                allow_legacy_algorithms: None,
                jump_server: None,
                proxy: None,
                os_id: None,
                icon: None,
                icon_file_path: None,
                account_expect: Default::default(),
            },
            None,
        );
        connection.params = serde_json::json!({
            "host": "example.com",
            "port": 22,
            "username": "temporary-user",
            "auth_method": {
                "Password": {
                    "password": "temporary-password"
                }
            },
            "prompt_username": true,
            "prompt_password": true
        })
        .to_string();

        let sanitized = connection.params_for_storage();
        assert!(!sanitized.contains("temporary-user"));
        assert!(!sanitized.contains("temporary-password"));

        let parsed = connection
            .to_ssh_params()
            .expect("读取 SSH 参数时也应清除临时凭据");
        assert!(parsed.username.is_empty());
        assert!(matches!(
            parsed.auth_method,
            SshAuthMethod::Password { ref password } if password.is_empty()
        ));
    }

    #[test]
    fn ssh_terminal_encoding_round_trips_through_json() {
        let mut params: SshParams = serde_json::from_str(
            r#"{"host":"example.com","port":22,"username":"root","auth_method":"Agent"}"#,
        )
        .expect("旧连接缺少终端字符集字段时应可反序列化");
        assert_eq!(params.terminal_encoding, StoredTerminalEncoding::Utf8);

        params.terminal_encoding = StoredTerminalEncoding::EucJp;
        let json = serde_json::to_string(&params).expect("SshParams 应可序列化");
        assert!(json.contains("\"terminal_encoding\":\"euc_jp\""));

        let parsed: SshParams = serde_json::from_str(&json).expect("SshParams 应可反序列化");
        assert_eq!(parsed.terminal_encoding, StoredTerminalEncoding::EucJp);
        assert!(StoredTerminalEncoding::all().contains(&StoredTerminalEncoding::EucJp));
        assert_eq!(StoredTerminalEncoding::EucJp.label(), "EUC-JP");
    }

    #[test]
    fn ssh_terminal_type_defaults_and_round_trips_through_json() {
        let mut params: SshParams = serde_json::from_str(
            r#"{"host":"example.com","port":22,"username":"root","auth_method":"Agent"}"#,
        )
        .expect("旧连接缺少终端类型字段时应可反序列化");
        assert_eq!(params.terminal_type, StoredTerminalType::Xterm256Color);

        let default_json = serde_json::to_string(&params).expect("SshParams 应可序列化");
        assert!(!default_json.contains("terminal_type"));

        params.terminal_type = StoredTerminalType::Xterm;
        let json = serde_json::to_string(&params).expect("SshParams 应可序列化");
        assert!(json.contains("\"terminal_type\":\"xterm\""));

        let parsed: SshParams = serde_json::from_str(&json).expect("SshParams 应可反序列化");
        assert_eq!(parsed.terminal_type, StoredTerminalType::Xterm);
        assert_eq!(StoredTerminalType::Xterm.as_str(), "xterm");
    }

    #[test]
    fn ssh_params_manual_icon_overrides_detected_os() {
        let mut params: SshParams = serde_json::from_str(
            r#"{"host":"example.com","port":22,"username":"root","auth_method":"Agent","os_id":"ubuntu"}"#,
        )
        .expect("SshParams 应可反序列化");
        assert!(matches!(params.os_icon(), IconName::UbuntuColor));

        params.icon = Some("docker".to_string());
        assert!(matches!(params.os_icon(), IconName::DockerColor));

        params.icon = None;
        params.os_id = None;
        assert!(matches!(params.os_icon(), IconName::LinuxPenguinColor));
    }

    #[test]
    fn telnet_params_roundtrip_with_login_script() {
        let params = TelnetParams {
            host: "192.168.1.1".to_string(),
            port: 2323,
            credential_reference: None,
            prompt_username: None,
            prompt_password: None,
            backspace_code: Default::default(),
            login_script: vec![
                TelnetLoginStep {
                    expect: "login:".to_string(),
                    send: "admin".to_string(),
                },
                TelnetLoginStep {
                    expect: "Password:".to_string(),
                    send: "secret".to_string(),
                },
            ],
        };
        let json = serde_json::to_string(&params).expect("TelnetParams 应可序列化");
        let parsed: TelnetParams = serde_json::from_str(&json).expect("TelnetParams 应可反序列化");
        assert_eq!(parsed, params);
    }

    #[test]
    fn telnet_params_defaults_login_script_for_legacy_json() {
        let params: TelnetParams = serde_json::from_str(r#"{"host":"10.0.0.1"}"#)
            .expect("旧 Telnet 连接缺少 login_script 时应可反序列化");
        assert_eq!(params.host, "10.0.0.1");
        assert_eq!(params.port, 23);
        assert_eq!(params.backspace_code, TelnetBackspaceCode::Delete);
        assert!(params.login_script.is_empty());

        let json = serde_json::to_string(&params).expect("TelnetParams 应可序列化");
        assert!(!json.contains("backspace_code"));
        assert!(!json.contains("login_script"));
    }

    #[test]
    fn telnet_params_roundtrip_with_backspace_code() {
        let mut params = TelnetParams::default();
        params.host = "switch.example.com".to_string();
        params.backspace_code = TelnetBackspaceCode::Backspace;

        let json = serde_json::to_string(&params).expect("TelnetParams 应可序列化");
        assert!(json.contains(r#""backspace_code":"backspace""#));

        let parsed: TelnetParams = serde_json::from_str(&json).expect("TelnetParams 应可反序列化");
        assert_eq!(parsed.backspace_code, TelnetBackspaceCode::Backspace);
    }

    #[test]
    fn stored_connection_telnet_roundtrip() {
        let params = TelnetParams {
            host: "switch.example.com".to_string(),
            port: 23,
            credential_reference: None,
            prompt_username: None,
            prompt_password: None,
            backspace_code: Default::default(),
            login_script: vec![TelnetLoginStep {
                expect: "Username:".to_string(),
                send: "admin".to_string(),
            }],
        };
        let conn = StoredConnection::new_telnet(String::new(), params, Some(7));
        assert_eq!(conn.connection_type, ConnectionType::Telnet);
        assert_eq!(conn.name, "switch.example.com:23");
        assert_eq!(conn.workspace_id, Some(7));

        let parsed = conn.to_telnet_params().expect("Telnet 参数应可反序列化");
        assert_eq!(parsed.host, "switch.example.com");
        assert_eq!(parsed.port, 23);
        assert_eq!(parsed.login_script.len(), 1);
        assert_eq!(parsed.login_script[0].expect, "Username:");
        assert_eq!(parsed.login_script[0].send, "admin");
    }

    #[test]
    fn telnet_params_credential_reference_and_prompt_policy_roundtrip() {
        let params = TelnetParams {
            host: "switch.example.com".to_string(),
            port: 23,
            credential_reference: Some(CredentialReference {
                credential_id: 42,
                credential_cloud_id: Some("credential-cloud-id".to_string()),
                username: true,
                password: true,
                private_key: false,
                passphrase: false,
            }),
            prompt_username: Some(true),
            prompt_password: Some(true),
            backspace_code: Default::default(),
            login_script: Vec::new(),
        };

        let json = serde_json::to_string(&params).expect("TelnetParams 应可序列化");
        let parsed: TelnetParams = serde_json::from_str(&json).expect("TelnetParams 应可反序列化");
        assert_eq!(parsed, params);
        assert!(parsed.prompts_for_username());
        assert!(parsed.prompts_for_password());
    }

    #[test]
    fn telnet_login_credentials_fill_only_unambiguous_empty_send_steps() {
        let mut params = TelnetParams {
            host: "switch.example.com".to_string(),
            port: 23,
            credential_reference: None,
            prompt_username: None,
            prompt_password: None,
            backspace_code: Default::default(),
            login_script: vec![
                TelnetLoginStep {
                    expect: r"(?i)(?:login|username)\s*:".to_string(),
                    send: String::new(),
                },
                TelnetLoginStep {
                    expect: r"(?i)password\s*:".to_string(),
                    send: String::new(),
                },
                TelnetLoginStep {
                    expect: r"(?i)(?:username|password)\s*:".to_string(),
                    send: String::new(),
                },
                TelnetLoginStep {
                    expect: "token:".to_string(),
                    send: "explicit".to_string(),
                },
            ],
        };

        params.apply_login_credentials(Some("admin"), Some("secret"));

        assert_eq!(params.login_script[0].send, "admin");
        assert_eq!(params.login_script[1].send, "secret");
        assert!(params.login_script[2].send.is_empty());
        assert_eq!(params.login_script[3].send, "explicit");
    }

    #[test]
    fn telnet_login_credential_prompt_fields_only_include_injectable_empty_steps() {
        let params = TelnetParams {
            host: "switch.example.com".to_string(),
            port: 23,
            credential_reference: None,
            prompt_username: None,
            prompt_password: None,
            backspace_code: Default::default(),
            login_script: vec![
                TelnetLoginStep {
                    expect: r"(?i)(?:login|username)\s*:".to_string(),
                    send: String::new(),
                },
                TelnetLoginStep {
                    expect: r"(?i)(?:password|passwd|passcode)\s*:".to_string(),
                    send: String::new(),
                },
                TelnetLoginStep {
                    expect: r"(?i)(?:username|password)\s*:".to_string(),
                    send: String::new(),
                },
                TelnetLoginStep {
                    expect: r"(?i)login\s*:".to_string(),
                    send: "explicit-user".to_string(),
                },
                TelnetLoginStep {
                    expect: "token:".to_string(),
                    send: String::new(),
                },
            ],
        };

        assert_eq!(params.login_credential_prompt_fields(), (true, true));

        let ambiguous_or_explicit_only = TelnetParams {
            login_script: params.login_script[2..].to_vec(),
            ..params
        };
        assert_eq!(
            ambiguous_or_explicit_only.login_credential_prompt_fields(),
            (false, false)
        );
    }

    #[test]
    fn telnet_expect_credential_kind_requires_a_prompt_shaped_expression() {
        assert_eq!(
            telnet_expect_credential_kind(r"(?i)(?:login|username)\s*[:>]\s*$"),
            Some(TelnetExpectCredentialKind::Username)
        );
        assert_eq!(
            telnet_expect_credential_kind(r"(?i)(?:password|passwd)\s*[:>]\s*$"),
            Some(TelnetExpectCredentialKind::Password)
        );
        assert_eq!(telnet_expect_credential_kind(r"(?i)password expired"), None);
        assert_eq!(
            telnet_expect_credential_kind(r"(?i)username and password are required"),
            None
        );
        assert_eq!(telnet_expect_credential_kind("not_a_password_prompt"), None);
    }

    #[test]
    fn telnet_login_script_send_is_a_sensitive_field() {
        // 参数加解密依赖该字段名识别 Telnet 登录脚本中的
        // 自动发送凭据；不要退回到只识别 password/passphrase/private_key。
        assert!(is_sensitive_field("send"));
        assert!(is_sensitive_field("password"));
        assert!(is_sensitive_field("passphrase"));
    }

    #[test]
    fn connection_type_telnet_methods() {
        assert_eq!(ConnectionType::Telnet.label(), "Telnet");
        assert_eq!(ConnectionType::from_str("Telnet"), ConnectionType::Telnet);
        assert_eq!(format!("{}", ConnectionType::Telnet), "Telnet");
        assert!(ConnectionType::all().contains(&ConnectionType::Telnet));
    }
}
