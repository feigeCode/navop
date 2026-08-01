use std::time::Duration;

use serde::{Deserialize, Serialize};
use ssh::{
    HostKeyVerifier, LocalPortForwardTunnel, SshAuth, SshConnectConfig, start_local_port_forward,
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
};

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
        jump_server: None,
        proxy: proxy.map(ProxyTunnelConfig::to_ssh_proxy).transpose()?,
        keyboard_interactive_responder: None,
        host_key_verifier: HostKeyVerifier::default(),
        x11_forwarding: false,
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
        "auto_publickey" | "auto_public_key" => Ok(SshAuth::AutoPublicKey),
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
        _ => Ok(SshAuth::Password(required_value(
            "password",
            config.password.as_deref().unwrap_or(""),
        )?)),
    }
}
