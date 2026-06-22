use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use one_core::storage::{
    ConnectionType, PortForwardingKind, ProxyType as StorageProxyType, SshAuthMethod, SshParams,
    StoredConnection,
};
use ssh::{
    DynamicSocksConfig, DynamicSocksTunnel, JumpServerConnectConfig, LocalPortForwardConfig,
    LocalPortForwardTunnel, ProxyConnectConfig, ProxyType, SshAuth, SshConnectConfig,
    start_dynamic_socks_forward, start_local_port_forward_with_config,
};

pub struct LocalForwardingRequest {
    pub ssh_config: SshConnectConfig,
    pub bind_host: String,
    pub bind_port: u16,
    pub target_host: String,
    pub target_port: u16,
}

pub struct DynamicForwardingRequest {
    pub ssh_config: SshConnectConfig,
    pub bind_host: String,
    pub bind_port: u16,
}

#[derive(Default)]
pub struct PortForwardingRuntime {
    local_tunnels: HashMap<i64, LocalPortForwardTunnel>,
    dynamic_tunnels: HashMap<i64, DynamicSocksTunnel>,
}

impl PortForwardingRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn start_local(
        &mut self,
        connection_id: i64,
        request: LocalForwardingRequest,
    ) -> Result<SocketAddr> {
        if self.is_running(connection_id) {
            bail!("Port Forwarding connection is already running");
        }
        let tunnel = start_local_port_forward_with_config(
            request.ssh_config,
            LocalPortForwardConfig {
                bind_host: request.bind_host,
                bind_port: request.bind_port,
                target_host: request.target_host,
                target_port: request.target_port,
            },
        )
        .await?;
        let local_addr = tunnel.local_addr();
        self.local_tunnels.insert(connection_id, tunnel);
        Ok(local_addr)
    }

    pub fn is_running(&self, connection_id: i64) -> bool {
        self.local_tunnels.contains_key(&connection_id)
            || self.dynamic_tunnels.contains_key(&connection_id)
    }

    pub async fn start_dynamic(
        &mut self,
        connection_id: i64,
        request: DynamicForwardingRequest,
    ) -> Result<SocketAddr> {
        if self.is_running(connection_id) {
            bail!("Port Forwarding connection is already running");
        }
        let tunnel = start_dynamic_socks_forward(
            request.ssh_config,
            DynamicSocksConfig {
                bind_host: request.bind_host,
                bind_port: request.bind_port,
            },
        )
        .await?;
        let local_addr = tunnel.local_addr();
        self.dynamic_tunnels.insert(connection_id, tunnel);
        Ok(local_addr)
    }
}

pub fn build_local_forwarding_request(
    forwarding_connection: &StoredConnection,
    ssh_connection: &StoredConnection,
) -> Result<LocalForwardingRequest> {
    if forwarding_connection.connection_type != ConnectionType::PortForwarding {
        bail!("connection is not a Port Forwarding connection");
    }
    if ssh_connection.connection_type != ConnectionType::SshSftp {
        bail!("referenced connection is not an SSH/SFTP connection");
    }

    let params = forwarding_connection
        .to_port_forwarding_params()
        .context("failed to parse Port Forwarding params")?;
    if params.kind != PortForwardingKind::Local {
        bail!("only local Port Forwarding is supported by this runtime entrypoint");
    }
    if ssh_connection.id != Some(params.ssh_connection_id) {
        bail!("referenced SSH connection id does not match Port Forwarding params");
    }

    let ssh_params = ssh_connection
        .to_ssh_params()
        .context("failed to parse referenced SSH params")?;

    Ok(LocalForwardingRequest {
        ssh_config: build_ssh_connect_config(&ssh_params),
        bind_host: params.bind_host,
        bind_port: params.bind_port,
        target_host: params.target_host,
        target_port: params.target_port,
    })
}

pub fn build_dynamic_forwarding_request(
    forwarding_connection: &StoredConnection,
    ssh_connection: &StoredConnection,
) -> Result<DynamicForwardingRequest> {
    if forwarding_connection.connection_type != ConnectionType::PortForwarding {
        bail!("connection is not a Port Forwarding connection");
    }
    if ssh_connection.connection_type != ConnectionType::SshSftp {
        bail!("referenced connection is not an SSH/SFTP connection");
    }

    let params = forwarding_connection
        .to_port_forwarding_params()
        .context("failed to parse Port Forwarding params")?;
    if params.kind != PortForwardingKind::Dynamic {
        bail!("connection is not Dynamic SOCKS Port Forwarding");
    }
    if ssh_connection.id != Some(params.ssh_connection_id) {
        bail!("referenced SSH connection id does not match Port Forwarding params");
    }

    let ssh_params = ssh_connection
        .to_ssh_params()
        .context("failed to parse referenced SSH params")?;

    Ok(DynamicForwardingRequest {
        ssh_config: build_ssh_connect_config(&ssh_params),
        bind_host: params.bind_host,
        bind_port: params.bind_port,
    })
}

fn build_ssh_connect_config(params: &SshParams) -> SshConnectConfig {
    SshConnectConfig {
        host: params.host.clone(),
        port: params.port,
        username: params.username.clone(),
        auth: build_ssh_auth(&params.auth_method),
        timeout: params.connect_timeout.map(Duration::from_secs),
        keepalive_interval: params.keepalive_interval.map(Duration::from_secs),
        keepalive_max: params.keepalive_max,
        jump_server: params
            .jump_server
            .as_ref()
            .map(|jump| JumpServerConnectConfig {
                host: jump.host.clone(),
                port: jump.port,
                username: jump.username.clone(),
                auth: build_ssh_auth(&jump.auth_method),
            }),
        proxy: params.proxy.as_ref().map(|proxy| ProxyConnectConfig {
            proxy_type: match proxy.proxy_type {
                StorageProxyType::Socks5 => ProxyType::Socks5,
                StorageProxyType::Http => ProxyType::Http,
            },
            host: proxy.host.clone(),
            port: proxy.port,
            username: proxy.username.clone(),
            password: proxy.password.clone(),
        }),
        keyboard_interactive_responder: None,
    }
}

fn build_ssh_auth(auth_method: &SshAuthMethod) -> SshAuth {
    match auth_method {
        SshAuthMethod::Password { password } => SshAuth::Password(password.clone()),
        SshAuthMethod::PrivateKey {
            key_path,
            passphrase,
        } => SshAuth::PrivateKey {
            key_path: key_path.clone(),
            passphrase: passphrase.clone(),
            certificate_path: None,
        },
        SshAuthMethod::Agent => SshAuth::Agent,
        SshAuthMethod::AutoPublicKey => SshAuth::AutoPublicKey,
    }
}
