use std::time::Duration;

use serde::{Deserialize, Serialize};
use ssh::{
    BastionUserFormat, HostKeyVerifier, JumpMode, JumpServerConnectConfig, LocalPortForwardTunnel,
    SshAuth, SshConnectConfig, start_local_port_forward,
};
use thiserror::Error;
use tokio::time::timeout;

mod proxy;
#[cfg(test)]
mod proxy_tests;
#[cfg(test)]
mod tunnel_tests;

pub use proxy::{
    ProxyTunnel, ProxyTunnelConfig, ProxyTunnelError, ProxyTunnelType, start_proxy_tunnel,
    test_proxy_reachability,
};

#[derive(Debug, Error)]
pub enum TcpReachabilityError {
    #[error("target host is required")]
    MissingHost,
    #[error("target port is required")]
    MissingPort,
    #[error("failed to create TCP reachability runtime: {0}")]
    Runtime(String),
    #[error("could not connect to {host}:{port}: {detail}")]
    Connect {
        host: String,
        port: u16,
        detail: String,
    },
    #[error(
        "connection to {host}:{port} timed out after {seconds:.1} seconds",
        seconds = timeout.as_secs_f64()
    )]
    Timeout {
        host: String,
        port: u16,
        timeout: Duration,
    },
}

pub fn test_tcp_reachability(
    host: &str,
    port: u16,
    timeout: Duration,
) -> Result<(), TcpReachabilityError> {
    let host = host.trim();
    if host.is_empty() {
        return Err(TcpReachabilityError::MissingHost);
    }
    if port == 0 {
        return Err(TcpReachabilityError::MissingPort);
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| TcpReachabilityError::Runtime(error.to_string()))?;
    let result = runtime.block_on(async {
        tokio::time::timeout(timeout, async {
            let socket_addrs = tokio::net::lookup_host((host, port))
                .await
                .map_err(|error| error.to_string())?;
            let mut last_error = None;
            for socket_addr in socket_addrs {
                match tokio::net::TcpStream::connect(socket_addr).await {
                    Ok(_) => return Ok(()),
                    Err(error) => last_error = Some(format!("{socket_addr}: {error}")),
                }
            }
            Err(last_error.unwrap_or_else(|| "no address resolved".to_string()))
        })
        .await
    });

    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(detail)) => Err(TcpReachabilityError::Connect {
            host: host.to_string(),
            port,
            detail,
        }),
        Err(_) => Err(TcpReachabilityError::Timeout {
            host: host.to_string(),
            port,
            timeout,
        }),
    }
}

const DEFAULT_SSH_PORT: u16 = 22;
const DEFAULT_SSH_AUTH_TYPE: &str = "password";
const DEFAULT_SSH_TIMEOUT_SECS: u64 = 30;

pub enum TunnelGuard {
    Ssh(LocalPortForwardTunnel),
    Proxy(ProxyTunnel),
}

impl TunnelGuard {
    pub fn local_addr(&self) -> std::net::SocketAddr {
        match self {
            Self::Ssh(tunnel) => tunnel.local_addr(),
            Self::Proxy(tunnel) => tunnel.local_addr(),
        }
    }
}

impl From<LocalPortForwardTunnel> for TunnelGuard {
    fn from(tunnel: LocalPortForwardTunnel) -> Self {
        Self::Ssh(tunnel)
    }
}

impl From<ProxyTunnel> for TunnelGuard {
    fn from(tunnel: ProxyTunnel) -> Self {
        Self::Proxy(tunnel)
    }
}

fn default_ssh_port() -> u16 {
    DEFAULT_SSH_PORT
}

fn default_ssh_auth_type() -> String {
    DEFAULT_SSH_AUTH_TYPE.to_string()
}

#[derive(Debug, Error)]
pub enum TunnelError {
    #[error("ssh tunnel enabled but `{0}` is missing")]
    MissingField(&'static str),
    #[error("failed to establish ssh tunnel: {0}")]
    Establish(String),
    #[error("ssh tunnel connection timed out after {0}s")]
    Timeout(u64),
    #[error(transparent)]
    Proxy(#[from] ProxyTunnelError),
}

/// Reusable SSH tunnel configuration for connection types that need TCP forwarding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshTunnelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub connection_id: Option<i64>,
    #[serde(default)]
    pub host: String,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    #[serde(default)]
    pub username: String,
    #[serde(default = "default_ssh_auth_type")]
    pub auth_type: String,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub private_key_path: Option<String>,
    #[serde(default)]
    pub private_key_content: Option<String>,
    #[serde(default)]
    pub private_key_passphrase: Option<String>,
    #[serde(default)]
    pub target_host: Option<String>,
    #[serde(default)]
    pub target_port: Option<u16>,
    #[serde(default)]
    pub timeout: Option<u64>,
    /// 引用 SSH 连接继承的跳板机配置（隧道也走跳板，语义不丢失）。
    ///
    /// 存储层（one-core）定义的 `JumpServerConfig` 不能直接出现在这里
    /// （connection_tunnel 是 one-core 的依赖，反向引用会成环），因此
    /// 用一个隧道本地的可序列化子集，由
    /// `one_core::storage::models::inherit_tunnel_jump_and_proxy` 回填。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jump: Option<TunnelJumpConfig>,
    /// 引用 SSH 连接继承的代理配置（SOCKS5/HTTP）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<TunnelProxyConfig>,
}

/// 隧道继承的跳板机配置（`one_core::storage::models::JumpServerConfig`
/// 的可序列化子集；凭据引用不进隧道，隧道始终用已解析的具体凭据）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunnelJumpConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_key_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_key_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_key_passphrase: Option<String>,
    /// 工作模式：proxy_jump / bastion_direct。缺省 proxy_jump（旧数据）。
    #[serde(default)]
    pub mode: TunnelJumpMode,
    /// 堡垒机直连目标资产 ID。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bastion_asset: Option<String>,
    /// 堡垒机直连资产登录账号。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bastion_account: Option<String>,
}

/// 隧道跳板工作模式，与存储层 `JumpMode` 一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TunnelJumpMode {
    #[default]
    ProxyJump,
    BastionDirect,
}

/// 隧道继承的代理配置（`one_core::storage::models::ProxyConfig` 的子集）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunnelProxyConfig {
    pub proxy_type: String,
    pub host: String,
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

impl Default for SshTunnelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            connection_id: None,
            host: String::new(),
            port: DEFAULT_SSH_PORT,
            username: String::new(),
            auth_type: DEFAULT_SSH_AUTH_TYPE.to_string(),
            password: None,
            private_key_path: None,
            private_key_content: None,
            private_key_passphrase: None,
            target_host: None,
            target_port: None,
            timeout: None,
            jump: None,
            proxy: None,
        }
    }
}

pub struct ResolvedConnectionTarget {
    pub host: String,
    pub port: u16,
    pub tunnel: Option<TunnelGuard>,
}

fn normalize_direct_host(host: &str) -> String {
    if host.eq_ignore_ascii_case("localhost") {
        return "127.0.0.1".to_string();
    }

    host.to_string()
}

pub fn resolve_tunnel_destination(
    direct_host: &str,
    direct_port: u16,
    tunnel: Option<&SshTunnelConfig>,
) -> (String, u16) {
    let Some(tunnel) = tunnel else {
        return (direct_host.to_string(), direct_port);
    };
    let host = tunnel
        .target_host
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(direct_host)
        .to_string();
    let port = tunnel.target_port.unwrap_or(direct_port);

    (host, port)
}

pub async fn resolve_connection_target(
    direct_host: &str,
    direct_port: u16,
    tunnel: Option<&SshTunnelConfig>,
) -> Result<ResolvedConnectionTarget, TunnelError> {
    resolve_connection_target_with_proxy(direct_host, direct_port, tunnel, None).await
}

pub async fn resolve_connection_target_with_proxy(
    direct_host: &str,
    direct_port: u16,
    tunnel: Option<&SshTunnelConfig>,
    proxy: Option<&ProxyTunnelConfig>,
) -> Result<ResolvedConnectionTarget, TunnelError> {
    if let Some(tunnel) = tunnel.filter(|config| config.enabled) {
        return resolve_ssh_target(direct_host, direct_port, tunnel, proxy).await;
    }
    if let Some(proxy) = proxy {
        let tunnel = start_proxy_tunnel(proxy.clone(), direct_host, direct_port)?;
        let local_addr = tunnel.local_addr();
        return Ok(ResolvedConnectionTarget {
            host: local_addr.ip().to_string(),
            port: local_addr.port(),
            tunnel: Some(tunnel.into()),
        });
    }
    Ok(ResolvedConnectionTarget {
        host: normalize_direct_host(direct_host),
        port: direct_port,
        tunnel: None,
    })
}

async fn resolve_ssh_target(
    direct_host: &str,
    direct_port: u16,
    tunnel: &SshTunnelConfig,
    proxy: Option<&ProxyTunnelConfig>,
) -> Result<ResolvedConnectionTarget, TunnelError> {
    let ssh_host = required_value("host", &tunnel.host)?;
    let ssh_username = required_value("username", &tunnel.username)?;
    let auth = build_auth(tunnel)?;
    let (target_host, target_port) =
        resolve_tunnel_destination(direct_host, direct_port, Some(tunnel));
    let timeout_secs = tunnel.timeout.unwrap_or(DEFAULT_SSH_TIMEOUT_SECS);
    let ssh_config = SshConnectConfig {
        host: ssh_host,
        port: tunnel.port,
        username: ssh_username,
        auth,
        timeout: Some(Duration::from_secs(timeout_secs)),
        keepalive_interval: None,
        keepalive_max: None,
        jump_server: tunnel_jump_connect_config(tunnel),
        proxy: proxy.map(ProxyTunnelConfig::to_ssh_proxy).transpose()?,
        keyboard_interactive_responder: None,
        host_key_verifier: HostKeyVerifier::default(),
        x11_forwarding: false,
        allow_legacy_algorithms: false,
        forward_agent: false,
        agent_identities: Vec::new(),
    };

    let tunnel_result = timeout(
        Duration::from_secs(timeout_secs),
        start_local_port_forward(ssh_config, target_host, target_port),
    )
    .await;
    let tunnel = match tunnel_result {
        Ok(Ok(tunnel)) => tunnel,
        Ok(Err(error)) => return Err(TunnelError::Establish(error.to_string())),
        Err(_) => return Err(TunnelError::Timeout(timeout_secs)),
    };
    let local_addr = tunnel.local_addr();

    Ok(ResolvedConnectionTarget {
        host: local_addr.ip().to_string(),
        port: local_addr.port(),
        tunnel: Some(tunnel.into()),
    })
}

/// 隧道继承的跳板配置 → 连接层跳板配置。
///
/// 凭据字段由 `tunnel_auth_fields` 同款规则从 auth_type + 密钥材料还原；
/// 未配置跳板时返回 `None`，行为与历史版本一致。
fn tunnel_jump_connect_config(tunnel: &SshTunnelConfig) -> Option<JumpServerConnectConfig> {
    let jump = tunnel.jump.as_ref()?;
    let auth = match jump.auth_type.trim().to_ascii_lowercase().as_str() {
        "agent" => SshAuth::Agent,
        "private_key" => SshAuth::PrivateKey {
            key_path: jump.private_key_path.clone().unwrap_or_default(),
            passphrase: jump.private_key_passphrase.clone(),
            certificate_path: None,
        },
        "private_key_content" => SshAuth::PrivateKeyContent {
            private_key: jump.private_key_content.clone().unwrap_or_default(),
            passphrase: jump.private_key_passphrase.clone(),
            certificate_path: None,
        },
        _ => SshAuth::Password(jump.password.clone().unwrap_or_default()),
    };
    Some(JumpServerConnectConfig {
        host: jump.host.clone(),
        port: jump.port,
        username: jump.username.clone(),
        auth,
        mode: match jump.mode {
            TunnelJumpMode::ProxyJump => JumpMode::ProxyJump,
            TunnelJumpMode::BastionDirect => JumpMode::BastionDirect,
        },
        bastion_asset: jump.bastion_asset.clone().unwrap_or_default(),
        bastion_account: jump.bastion_account.clone().unwrap_or_default(),
        bastion_user_format: BastionUserFormat::JumpServer,
    })
}

fn required_value(key: &'static str, value: &str) -> Result<String, TunnelError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(TunnelError::MissingField(key));
    }
    Ok(value.to_string())
}

fn optional_value(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn build_auth(config: &SshTunnelConfig) -> Result<SshAuth, TunnelError> {
    match config.auth_type.trim().to_ascii_lowercase().as_str() {
        "agent" => Ok(SshAuth::Agent),
        "pageant" => Ok(SshAuth::Pageant),
        "auto_publickey" | "auto_public_key" => Ok(SshAuth::AutoPublicKey),
        "none" => Ok(SshAuth::None),
        "private_key" => Ok(SshAuth::PrivateKey {
            key_path: required_value(
                "private_key_path",
                config.private_key_path.as_deref().unwrap_or(""),
            )?,
            passphrase: optional_value(&config.private_key_passphrase),
            certificate_path: None,
        }),
        "private_key_content" | "private_key_material" => Ok(SshAuth::PrivateKeyContent {
            private_key: required_value(
                "private_key_content",
                config.private_key_content.as_deref().unwrap_or(""),
            )?,
            passphrase: optional_value(&config.private_key_passphrase),
            certificate_path: None,
        }),
        // 组合认证：一个隧道配置里同时带密码与私钥两个因素。
        "password_and_private_key" => {
            let password = required_value("password", config.password.as_deref().unwrap_or(""))?;
            let passphrase = optional_value(&config.private_key_passphrase);
            let key = match (
                optional_value(&config.private_key_content),
                optional_value(&config.private_key_path),
            ) {
                (Some(private_key), _) => SshAuth::PrivateKeyContent {
                    private_key,
                    passphrase,
                    certificate_path: None,
                },
                (None, Some(key_path)) => SshAuth::PrivateKey {
                    key_path,
                    passphrase,
                    certificate_path: None,
                },
                (None, None) => return Err(TunnelError::MissingField("private_key_path")),
            };
            Ok(SshAuth::Chain(vec![SshAuth::Password(password), key]))
        }
        _ => Ok(SshAuth::Password(required_value(
            "password",
            config.password.as_deref().unwrap_or(""),
        )?)),
    }
}
