use crate::cloud_sync::sync_type::SyncableItem;
use crate::crypto;
use crate::storage::traits::Entity;
use connection_tunnel::SshTunnelConfig;
use gpui::Global;
use gpui_component::Size::Large;
use gpui_component::{Icon, IconName, Sizable};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::fmt;

/// 活跃连接状态 - 用于跟踪哪些连接当前已打开
#[derive(Default)]
pub struct ActiveConnections {
    active_ids: HashSet<i64>,
}

impl Global for ActiveConnections {}

impl ActiveConnections {
    pub fn new() -> Self {
        Self {
            active_ids: HashSet::new(),
        }
    }

    pub fn add(&mut self, conn_id: i64) {
        self.active_ids.insert(conn_id);
    }

    pub fn remove(&mut self, conn_id: i64) {
        self.active_ids.remove(&conn_id);
    }

    pub fn is_active(&self, conn_id: i64) -> bool {
        self.active_ids.contains(&conn_id)
    }

    pub fn active_count(&self) -> usize {
        self.active_ids.len()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ConnectionType {
    All,
    Database,
    SshSftp,
    Redis,
    MongoDB,
    Serial,
    PortForwarding,
    Rdp,
    Vnc,
}

impl fmt::Display for ConnectionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            ConnectionType::All => "All",
            ConnectionType::Database => "Database",
            ConnectionType::SshSftp => "SshSftp",
            ConnectionType::Redis => "Redis",
            ConnectionType::MongoDB => "MongoDB",
            ConnectionType::Serial => "Serial",
            ConnectionType::PortForwarding => "PortForwarding",
            ConnectionType::Rdp => "Rdp",
            ConnectionType::Vnc => "Vnc",
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
            ConnectionType::Serial,
            ConnectionType::PortForwarding,
            ConnectionType::Rdp,
            ConnectionType::Vnc,
        ]
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "Database" => ConnectionType::Database,
            "SshSftp" => ConnectionType::SshSftp,
            "Redis" => ConnectionType::Redis,
            "MongoDB" => ConnectionType::MongoDB,
            "Serial" => ConnectionType::Serial,
            "PortForwarding" => ConnectionType::PortForwarding,
            "Rdp" => ConnectionType::Rdp,
            "Vnc" => ConnectionType::Vnc,
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
            ConnectionType::Serial => "Serial",
            ConnectionType::PortForwarding => "Port Forwarding",
            ConnectionType::Rdp => "RDP",
            ConnectionType::Vnc => "VNC",
        }
    }

    pub fn icon(&self) -> IconName {
        match self {
            ConnectionType::All => IconName::Server,
            ConnectionType::Database => IconName::Database,
            ConnectionType::SshSftp => IconName::TerminalColor,
            ConnectionType::Redis => IconName::Redis,
            ConnectionType::MongoDB => IconName::MongoDB,
            ConnectionType::Serial => IconName::SerialPort,
            ConnectionType::PortForwarding => IconName::PortForwardingColor,
            ConnectionType::Rdp => IconName::Rdp,
            ConnectionType::Vnc => IconName::Vnc,
        }
    }
}

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
    External { driver_id: String },
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
            DatabaseType::External { .. } => IconName::Database.color().with_size(Large),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshParams {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: SshAuthMethod,
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
    /// 跳板机配置
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jump_server: Option<JumpServerConfig>,
    /// 代理配置
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxyConfig>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteDesktopParams {
    pub protocol: RemoteDesktopProtocol,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub domain: Option<String>,
    #[serde(default)]
    pub read_only: bool,
}

/// 跳板机配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JumpServerConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: SshAuthMethod,
}

/// 代理类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProxyType {
    Socks5,
    Http,
}

/// 代理配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    pub proxy_type: ProxyType,
    pub host: String,
    pub port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MongoDBParams {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PortForwardingKind {
    #[default]
    Local,
    Dynamic,
}

impl PortForwardingKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Local => "Local",
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
    pub database: Option<String>,
    pub service_name: Option<String>,
    pub sid: Option<String>,
    #[serde(skip)]
    pub workspace_id: Option<i64>,
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
            created_at: None,
            updated_at: None,
            cloud_id: None,
            last_synced_at: None,
            sort_order: None,
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

fn default_serial_name(name: String, params: &SerialParams) -> String {
    trimmed_or_default(name, params.port_name.trim().to_string())
}

fn default_port_forwarding_name(name: String, params: &PortForwardingParams) -> String {
    let default_name = match params.kind {
        PortForwardingKind::Local => format!(
            "{}:{} -> {}:{}",
            params.bind_host, params.bind_port, params.target_host, params.target_port
        ),
        PortForwardingKind::Dynamic => {
            format!("SOCKS {}:{}", params.bind_host, params.bind_port)
        }
    };
    trimmed_or_default(name, default_name)
}

impl StoredConnection {
    pub fn new_database(
        name: String,
        params: DbConnectionConfig,
        workspace_id: Option<i64>,
    ) -> Self {
        let name = default_database_name(name, &params);
        Self {
            id: None,
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

    pub fn new_ssh(name: String, params: SshParams, workspace_id: Option<i64>) -> Self {
        let name = default_ssh_name(name, &params);
        Self {
            id: None,
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

    pub fn to_ssh_params(&self) -> Result<SshParams, serde_json::Error> {
        serde_json::from_str(&self.params)
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

    pub fn new_serial(name: String, params: SerialParams, workspace_id: Option<i64>) -> Self {
        let name = default_serial_name(name, &params);
        Self {
            id: None,
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

    pub fn new_port_forwarding(
        name: String,
        params: PortForwardingParams,
        workspace_id: Option<i64>,
    ) -> Self {
        let name = default_port_forwarding_name(name, &params);
        Self {
            id: None,
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
    /// 敏感字段包括：password、passphrase、private_key、private_key_content 以及嵌套结构中的同类字段。
    pub fn encrypt_params(&self) -> String {
        encrypt_json_passwords(&self.params)
    }

    /// 对 params 中的加密字段进行解密，返回解密后的 params 字符串。
    pub fn decrypt_params(&self) -> String {
        decrypt_json_passwords(&self.params)
    }

    /// 返回一个新的 StoredConnection，其 params 中的密码字段已解密
    pub fn with_decrypted_params(&self) -> Self {
        let mut cloned = self.clone();
        cloned.params = cloned.decrypt_params();
        cloned
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn ssh_connection_with_id(id: i64, auth_method: SshAuthMethod) -> StoredConnection {
        let mut connection = StoredConnection::new_ssh(
            "prod-bastion".to_string(),
            SshParams {
                host: "bastion.example.com".to_string(),
                port: 2222,
                username: "deploy".to_string(),
                auth_method,
                connect_timeout: Some(15),
                keepalive_interval: Some(30),
                keepalive_max: Some(3),
                default_directory: None,
                init_script: None,
                disable_shell_integration: None,
                jump_server: None,
                proxy: None,
            },
            Some(7),
        );
        connection.id = Some(id);
        connection
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
            extra_params,
        }
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
            extra_params: HashMap::new(),
        };
        assert_eq!(
            "127.0.0.1:3306",
            StoredConnection::new_database(String::new(), db, None).name
        );

        let ssh = SshParams {
            host: "localhost".to_string(),
            port: 22,
            username: "root".to_string(),
            auth_method: SshAuthMethod::Agent,
            connect_timeout: None,
            keepalive_interval: None,
            keepalive_max: None,
            default_directory: None,
            init_script: None,
            disable_shell_integration: None,
            jump_server: None,
            proxy: None,
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
        };
        assert_eq!(
            "10.0.0.5:6379",
            StoredConnection::new_redis(String::new(), redis, None).name
        );

        let mongo = MongoDBParams {
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

/// 判断字段名是否为敏感字段
fn is_sensitive_field(key: &str) -> bool {
    key == "password"
        || key == "passphrase"
        || key == "private_key"
        || key == "private_key_content"
        || key.ends_with("_password")
        || key.ends_with("_passphrase")
        || key.ends_with("_private_key")
        || key.ends_with("_private_key_content")
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
        assert_eq!(RemoteDesktopProtocol::Vnc.default_port(), 5900);
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
    fn ssh_auth_method_auto_publickey_serialize_deserialize() {
        let auth = SshAuthMethod::AutoPublicKey;
        let json = serde_json::to_string(&auth).expect("自动公钥认证方式应可序列化");
        let parsed: SshAuthMethod =
            serde_json::from_str(&json).expect("自动公钥认证方式应可反序列化");
        assert!(matches!(parsed, SshAuthMethod::AutoPublicKey));
    }
}
