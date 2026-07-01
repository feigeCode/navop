use super::tool_error;
use agent_runtime::{ResourceKind, ToolError, tools::ToolInvocation};
use one_core::storage::traits::Repository;
use one_core::storage::{
    ConnectionRepository, ConnectionType, JumpServerConfig, ProxyConfig,
    ProxyType as StorageProxyType, SshAuthMethod, SshParams, StoredConnection,
};
use ssh::{JumpServerConnectConfig, ProxyConnectConfig, ProxyType, SshAuth, SshConnectConfig};
use std::sync::Arc;
use std::time::Duration;

pub(super) fn resolve_ssh_config(
    repo: &Arc<ConnectionRepository>,
    invocation: &ToolInvocation,
) -> Result<SshConnectConfig, ToolError> {
    let connection_id = resolve_ssh_connection_id(invocation)?;
    let stored = find_connection(repo, &connection_id)?;
    if stored.connection_type != ConnectionType::SshSftp {
        return Err(ToolError::MissingResource(format!(
            "connection is not SSH/SFTP: {connection_id}"
        )));
    }
    let params = stored.to_ssh_params().map_err(tool_error)?;
    Ok(ssh_config_from_params(&params))
}

fn resolve_ssh_connection_id(invocation: &ToolInvocation) -> Result<String, ToolError> {
    let resource = invocation.target_resource();
    if let Some(resource) = resource
        && resource.kind != ResourceKind::Ssh
    {
        return Err(ToolError::MissingResource(format!(
            "current Agent resource is not an SSH/SFTP connection: {}",
            resource.id
        )));
    }
    invocation
        .arg_str("connection")
        .map(ToString::to_string)
        .or_else(|| resource.map(|resource| resource.id.to_string()))
        .ok_or_else(|| {
            ToolError::MissingResource(
                "please select an SSH/SFTP connection in the sidebar first".into(),
            )
        })
}

fn find_connection(
    repo: &Arc<ConnectionRepository>,
    connection: &str,
) -> Result<StoredConnection, ToolError> {
    if let Ok(id) = connection.parse::<i64>() {
        return repo
            .get(id)
            .map_err(tool_error)?
            .ok_or_else(|| unknown_connection(connection));
    }
    repo.list()
        .map_err(tool_error)?
        .into_iter()
        .find(|stored| stored.name == connection)
        .ok_or_else(|| unknown_connection(connection))
}

fn ssh_config_from_params(params: &SshParams) -> SshConnectConfig {
    SshConnectConfig {
        host: params.host.clone(),
        port: params.port,
        username: params.username.clone(),
        auth: auth_from_method(&params.auth_method),
        timeout: params.connect_timeout.map(Duration::from_secs),
        keepalive_interval: params.keepalive_interval.map(Duration::from_secs),
        keepalive_max: params.keepalive_max,
        jump_server: params.jump_server.as_ref().map(jump_config),
        proxy: params.proxy.as_ref().map(proxy_config),
        keyboard_interactive_responder: None,
    }
}

fn auth_from_method(auth: &SshAuthMethod) -> SshAuth {
    match auth {
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

fn jump_config(jump: &JumpServerConfig) -> JumpServerConnectConfig {
    JumpServerConnectConfig {
        host: jump.host.clone(),
        port: jump.port,
        username: jump.username.clone(),
        auth: auth_from_method(&jump.auth_method),
    }
}

fn proxy_config(proxy: &ProxyConfig) -> ProxyConnectConfig {
    let proxy_type = match proxy.proxy_type {
        StorageProxyType::Socks5 => ProxyType::Socks5,
        StorageProxyType::Http => ProxyType::Http,
    };
    ProxyConnectConfig {
        proxy_type,
        host: proxy.host.clone(),
        port: proxy.port,
        username: proxy.username.clone(),
        password: proxy.password.clone(),
    }
}

fn unknown_connection(connection: &str) -> ToolError {
    ToolError::MissingResource(format!("unknown SSH/SFTP connection: {connection}"))
}
