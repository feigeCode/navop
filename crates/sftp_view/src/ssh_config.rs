use anyhow::{Context as _, Result, anyhow};
use one_core::storage::models::{ProxyType as StorageProxyType, SshAuthMethod, StoredConnection};
use ssh::{
    HostKeyVerifier, JumpServerConnectConfig, ProxyConnectConfig, ProxyType, SshAuth,
    SshConnectConfig,
};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SshCredentialPromptPolicy {
    pub(crate) username: bool,
    pub(crate) password: bool,
}

impl SshCredentialPromptPolicy {
    pub(crate) fn requires_prompt(self) -> bool {
        self.username || self.password
    }
}

pub(crate) struct ResolvedSftpConnection {
    pub(crate) config: SshConnectConfig,
    pub(crate) credential_prompt_policy: SshCredentialPromptPolicy,
}

pub(crate) fn ssh_config_for(connection: &StoredConnection) -> Result<SshConnectConfig> {
    Ok(resolve_ssh_connection(connection)?.config)
}

pub(crate) fn resolve_ssh_connection(
    connection: &StoredConnection,
) -> Result<ResolvedSftpConnection> {
    let params = connection
        .to_ssh_params()
        .context("connection does not contain valid SSH parameters")?;
    let credential_prompt_policy = SshCredentialPromptPolicy {
        username: params.prompts_for_username(),
        password: params.prompts_for_password()
            && matches!(&params.auth_method, SshAuthMethod::Password { .. }),
    };
    let config = SshConnectConfig {
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
        host_key_verifier: HostKeyVerifier::default(),
        x11_forwarding: false,
        allow_legacy_algorithms: params.allow_legacy_algorithms.unwrap_or(false),
    };
    Ok(ResolvedSftpConnection {
        config,
        credential_prompt_policy,
    })
}

pub(crate) fn ssh_config_with_runtime_credentials(
    base_config: &SshConnectConfig,
    username: Option<&str>,
    password: Option<&str>,
) -> Result<SshConnectConfig> {
    let mut config = base_config.clone();

    if let Some(username) = username {
        let username = username.trim();
        if username.is_empty() {
            return Err(anyhow!("SSH username is empty"));
        }
        config.username = username.to_string();
    }

    if let Some(password) = password {
        if password.is_empty() {
            return Err(anyhow!("SSH password is empty"));
        }
        match &mut config.auth {
            SshAuth::Password(configured_password) => {
                *configured_password = password.to_string();
            }
            _ => {
                return Err(anyhow!(
                    "runtime SSH password is only valid for password authentication"
                ));
            }
        }
    }

    Ok(config)
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
        SshAuthMethod::Pageant => SshAuth::Pageant,
        SshAuthMethod::AutoPublicKey => SshAuth::AutoPublicKey,
    }
}

#[cfg(test)]
mod tests {
    use super::{resolve_ssh_connection, ssh_config_for, ssh_config_with_runtime_credentials};
    use one_core::storage::{SshAuthMethod, SshParams, StoredConnection};
    use ssh::SshAuth;

    fn connection_with_auth(auth_method: SshAuthMethod) -> StoredConnection {
        StoredConnection::new_ssh(
            "source".to_string(),
            SshParams {
                host: "source.internal".to_string(),
                port: 2222,
                username: "deploy".to_string(),
                auth_method,
                credential_reference: None,
                prompt_username: None,
                prompt_password: None,
                keyboard_interactive: None,
                terminal_encoding: Default::default(),
                terminal_type: Default::default(),
                connect_timeout: Some(12),
                keepalive_interval: None,
                keepalive_max: None,
                default_directory: None,
                init_script: None,
                disable_shell_integration: None,
                x11_forwarding: None,
                allow_legacy_algorithms: Some(true),
                jump_server: None,
                proxy: None,
                os_id: None,
                icon: None,
                account_expect: Default::default(),
            },
            None,
        )
    }

    #[test]
    fn stored_password_connection_maps_to_runtime_config() {
        let connection = connection_with_auth(SshAuthMethod::Password {
            password: "secret".to_string(),
        });

        let config = ssh_config_for(&connection).expect("valid SSH connection");
        assert_eq!(config.host, "source.internal");
        assert_eq!(config.port, 2222);
        assert!(matches!(config.auth, SshAuth::Password(ref value) if value == "secret"));
        assert!(config.allow_legacy_algorithms);
    }

    #[test]
    fn password_prompt_policy_is_preserved_for_sftp() {
        let mut connection = connection_with_auth(SshAuthMethod::Password {
            password: String::new(),
        });
        let mut params = connection.to_ssh_params().expect("valid SSH params");
        params.prompt_username = Some(true);
        params.prompt_password = Some(true);
        connection.params = serde_json::to_string(&params).expect("serialize SSH params");

        let resolved = resolve_ssh_connection(&connection).expect("valid SSH connection");

        assert!(resolved.credential_prompt_policy.username);
        assert!(resolved.credential_prompt_policy.password);
    }

    #[test]
    fn password_prompt_policy_is_ignored_for_non_password_authentication() {
        let mut connection = connection_with_auth(SshAuthMethod::Agent);
        let mut params = connection.to_ssh_params().expect("valid SSH params");
        params.prompt_password = Some(true);
        connection.params = serde_json::to_string(&params).expect("serialize SSH params");

        let resolved = resolve_ssh_connection(&connection).expect("valid SSH connection");

        assert!(!resolved.credential_prompt_policy.password);
    }

    #[test]
    fn runtime_credentials_are_injected_without_mutating_base_config() {
        let connection = connection_with_auth(SshAuthMethod::Password {
            password: String::new(),
        });
        let base = ssh_config_for(&connection).expect("valid SSH connection");

        let runtime =
            ssh_config_with_runtime_credentials(&base, Some(" runtime-user "), Some("secret"))
                .expect("runtime credentials should be accepted");

        assert_eq!(runtime.username, "runtime-user");
        assert!(matches!(
            runtime.auth,
            SshAuth::Password(ref password) if password == "secret"
        ));
        assert_eq!(base.username, "deploy");
        assert!(matches!(
            base.auth,
            SshAuth::Password(ref password) if password.is_empty()
        ));
    }

    #[test]
    fn empty_runtime_password_is_rejected() {
        let connection = connection_with_auth(SshAuthMethod::Password {
            password: String::new(),
        });
        let base = ssh_config_for(&connection).expect("valid SSH connection");

        let error = match ssh_config_with_runtime_credentials(&base, None, Some("")) {
            Ok(_) => panic!("empty password should be rejected"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("password is empty"));
    }

    #[test]
    fn runtime_password_is_rejected_for_non_password_authentication() {
        let connection = connection_with_auth(SshAuthMethod::Agent);
        let base = ssh_config_for(&connection).expect("valid SSH connection");

        let error = match ssh_config_with_runtime_credentials(&base, None, Some("secret")) {
            Ok(_) => panic!("password should not be accepted for agent authentication"),
            Err(error) => error,
        };

        assert!(
            error
                .to_string()
                .contains("only valid for password authentication")
        );
    }
}
