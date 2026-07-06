use connection_tunnel::{SshTunnelConfig, resolve_connection_target as resolve_tunnel_target};
use one_core::storage::DbConnectionConfig;
use ssh::LocalPortForwardTunnel;

use crate::connection::DbError;

const SSH_TUNNEL_ENABLED: &str = "ssh_tunnel_enabled";
const SSH_HOST: &str = "ssh_host";
const SSH_PORT: &str = "ssh_port";
const SSH_USERNAME: &str = "ssh_username";
const SSH_AUTH_TYPE: &str = "ssh_auth_type";
const SSH_PASSWORD: &str = "ssh_password";
const SSH_PRIVATE_KEY_PATH: &str = "ssh_private_key_path";
const SSH_PRIVATE_KEY_PASSPHRASE: &str = "ssh_private_key_passphrase";
const SSH_TARGET_HOST: &str = "ssh_target_host";
const SSH_TARGET_PORT: &str = "ssh_target_port";
const SSH_TIMEOUT: &str = "ssh_timeout";

pub struct ResolvedConnectionTarget {
    pub host: String,
    pub port: u16,
    pub tunnel: Option<LocalPortForwardTunnel>,
}

pub struct TunnelDestination {
    pub host: String,
    pub port: u16,
}

pub fn resolve_tunnel_destination(config: &DbConnectionConfig) -> TunnelDestination {
    let tunnel = tunnel_config_from_db_config(config);
    let (host, port) =
        connection_tunnel::resolve_tunnel_destination(&config.host, config.port, tunnel.as_ref());

    TunnelDestination { host, port }
}

pub async fn resolve_connection_target(
    config: &DbConnectionConfig,
) -> Result<ResolvedConnectionTarget, DbError> {
    let tunnel = tunnel_config_from_db_config(config);
    let target = resolve_tunnel_target(&config.host, config.port, tunnel.as_ref())
        .await
        .map_err(|error| DbError::connection(error.to_string()))?;

    Ok(ResolvedConnectionTarget {
        host: target.host,
        port: target.port,
        tunnel: target.tunnel,
    })
}

fn tunnel_config_from_db_config(config: &DbConnectionConfig) -> Option<SshTunnelConfig> {
    if !config.get_param_bool(SSH_TUNNEL_ENABLED) {
        return None;
    }

    Some(SshTunnelConfig {
        enabled: true,
        connection_id: optional_i64_param(config, "ssh_connection_id"),
        host: string_param(config, SSH_HOST),
        port: optional_u16_param(config, SSH_PORT).unwrap_or(22),
        username: string_param(config, SSH_USERNAME),
        auth_type: string_param(config, SSH_AUTH_TYPE),
        password: optional_string_param(config, SSH_PASSWORD),
        private_key_path: optional_string_param(config, SSH_PRIVATE_KEY_PATH),
        private_key_passphrase: optional_string_param(config, SSH_PRIVATE_KEY_PASSPHRASE),
        target_host: optional_string_param(config, SSH_TARGET_HOST),
        target_port: optional_u16_param(config, SSH_TARGET_PORT),
        timeout: config.get_param_as::<u64>(SSH_TIMEOUT),
    })
}

fn string_param(config: &DbConnectionConfig, key: &str) -> String {
    config
        .get_param(key)
        .map(|value| value.trim().to_string())
        .unwrap_or_default()
}

fn optional_string_param(config: &DbConnectionConfig, key: &str) -> Option<String> {
    config
        .get_param(key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn optional_u16_param(config: &DbConnectionConfig, key: &str) -> Option<u16> {
    config
        .get_param(key)
        .and_then(|value| value.trim().parse::<u16>().ok())
}

fn optional_i64_param(config: &DbConnectionConfig, key: &str) -> Option<i64> {
    config
        .get_param(key)
        .and_then(|value| value.trim().parse::<i64>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_core::storage::DatabaseType;
    use std::collections::HashMap;

    fn build_config(extra_params: HashMap<String, String>) -> DbConnectionConfig {
        DbConnectionConfig {
            id: "test".to_string(),
            database_type: DatabaseType::MySQL,
            name: "test".to_string(),
            host: "127.0.0.1".to_string(),
            port: 3306,
            username: "root".to_string(),
            password: "password".to_string(),
            database: None,
            service_name: None,
            sid: None,
            workspace_id: None,
            extra_params,
        }
    }

    #[test]
    fn tunnel_config_maps_agent_auth_type() {
        let mut extra_params = HashMap::new();
        extra_params.insert(SSH_TUNNEL_ENABLED.to_string(), "true".to_string());
        extra_params.insert(SSH_AUTH_TYPE.to_string(), "agent".to_string());
        let config = build_config(extra_params);

        let tunnel = tunnel_config_from_db_config(&config).expect("tunnel should be enabled");

        assert_eq!("agent", tunnel.auth_type);
    }

    #[test]
    fn tunnel_config_maps_auto_publickey_auth_type() {
        let mut extra_params = HashMap::new();
        extra_params.insert(SSH_TUNNEL_ENABLED.to_string(), "true".to_string());
        extra_params.insert(SSH_AUTH_TYPE.to_string(), "auto_publickey".to_string());
        let config = build_config(extra_params);

        let tunnel = tunnel_config_from_db_config(&config).expect("tunnel should be enabled");

        assert_eq!("auto_publickey", tunnel.auth_type);
    }

    #[test]
    fn resolve_tunnel_destination_uses_explicit_target_host_and_port() {
        let mut extra_params = HashMap::new();
        extra_params.insert(SSH_TUNNEL_ENABLED.to_string(), "true".to_string());
        extra_params.insert(SSH_TARGET_HOST.to_string(), "db.internal".to_string());
        extra_params.insert(SSH_TARGET_PORT.to_string(), "3307".to_string());
        let config = build_config(extra_params);

        let destination = resolve_tunnel_destination(&config);

        assert_eq!("db.internal", destination.host);
        assert_eq!(3307, destination.port);
    }

    #[test]
    fn resolve_tunnel_destination_falls_back_to_database_host_and_port() {
        let config = build_config(HashMap::new());

        let destination = resolve_tunnel_destination(&config);

        assert_eq!("127.0.0.1", destination.host);
        assert_eq!(3306, destination.port);
    }
}
