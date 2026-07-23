use anyhow::{Context as _, Result};
use one_core::storage::models::{ProxyType as StorageProxyType, SshAuthMethod, StoredConnection};
use ssh::{JumpServerConnectConfig, ProxyConnectConfig, ProxyType, SshAuth, SshConnectConfig};
use std::time::Duration;

pub(crate) fn ssh_config_for(connection: &StoredConnection) -> Result<SshConnectConfig> {
    let params = connection
        .to_ssh_params()
        .context("connection does not contain valid SSH parameters")?;
    Ok(SshConnectConfig {
        host: params.host,
        port: params.port,
        username: params.username,
        auth: ssh_auth(params.auth_method),
        timeout: params.connect_timeout.map(Duration::from_secs),
        keepalive_interval: params.keepalive_interval.map(Duration::from_secs),
        keepalive_max: params.keepalive_max,
        jump_server: params.jump_server.map(|jump| JumpServerConnectConfig {
            host: jump.host,
            port: jump.port,
            username: jump.username,
            auth: ssh_auth(jump.auth_method),
        }),
        proxy: params.proxy.map(|proxy| ProxyConnectConfig {
            proxy_type: match proxy.proxy_type {
                StorageProxyType::Socks5 => ProxyType::Socks5,
                StorageProxyType::Http => ProxyType::Http,
            },
            host: proxy.host,
            port: proxy.port,
            username: proxy.username,
            password: proxy.password,
        }),
        keyboard_interactive_responder: None,
    })
}

fn ssh_auth(method: SshAuthMethod) -> SshAuth {
    match method {
        SshAuthMethod::Password { password } => SshAuth::Password(password),
        SshAuthMethod::PrivateKey {
            key_path,
            passphrase,
        } => SshAuth::PrivateKey {
            key_path,
            passphrase,
            certificate_path: None,
        },
        SshAuthMethod::PrivateKeyContent {
            private_key,
            passphrase,
        } => SshAuth::PrivateKeyContent {
            private_key,
            passphrase,
            certificate_path: None,
        },
        SshAuthMethod::Agent => SshAuth::Agent,
        SshAuthMethod::AutoPublicKey => SshAuth::AutoPublicKey,
    }
}

#[cfg(test)]
mod tests {
    use super::ssh_config_for;
    use one_core::storage::{SshAuthMethod, SshParams, StoredConnection};
    use ssh::SshAuth;

    #[test]
    fn stored_password_connection_maps_to_runtime_config() {
        let connection = StoredConnection::new_ssh(
            "source".to_string(),
            SshParams {
                host: "source.internal".to_string(),
                port: 2222,
                username: "deploy".to_string(),
                auth_method: SshAuthMethod::Password {
                    password: "secret".to_string(),
                },
                connect_timeout: Some(12),
                keepalive_interval: None,
                keepalive_max: None,
                default_directory: None,
                init_script: None,
                disable_shell_integration: None,
                jump_server: None,
                proxy: None,
                os_id: None,
                icon: None,
            },
            None,
        );

        let config = ssh_config_for(&connection).expect("valid SSH connection");
        assert_eq!(config.host, "source.internal");
        assert_eq!(config.port, 2222);
        assert!(matches!(config.auth, SshAuth::Password(ref value) if value == "secret"));
    }
}
