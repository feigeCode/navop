use one_core::storage::{ConnectionType, DatabaseType, PortForwardingKind, StoredConnection};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ConnectionCopyTarget {
    DatabaseAddress,
    SshTarget,
    RedisAddress,
    MongoDbAddress,
    Username,
    SerialPort,
    ForwardingRule,
    RemoteDesktopAddress,
}

pub(super) fn connection_copy_targets(
    connection: &StoredConnection,
) -> Vec<(ConnectionCopyTarget, String)> {
    let mut targets = Vec::new();
    if let Some(target) = connection_address(connection) {
        targets.push(target);
    }
    if let Some(username) = connection_username(connection) {
        targets.push((ConnectionCopyTarget::Username, username));
    }
    if let Some(port) = serial_port(connection) {
        targets.push((ConnectionCopyTarget::SerialPort, port));
    }
    if let Some(rule) = forwarding_rule(connection) {
        targets.push((ConnectionCopyTarget::ForwardingRule, rule));
    }
    targets
}

fn connection_address(connection: &StoredConnection) -> Option<(ConnectionCopyTarget, String)> {
    match connection.connection_type {
        ConnectionType::Database => {
            database_address(connection).map(|value| (ConnectionCopyTarget::DatabaseAddress, value))
        }
        ConnectionType::SshSftp => connection.to_ssh_params().ok().map(|params| {
            (
                ConnectionCopyTarget::SshTarget,
                host_port(&params.host, params.port),
            )
        }),
        ConnectionType::Redis => connection.to_redis_params().ok().map(|params| {
            (
                ConnectionCopyTarget::RedisAddress,
                host_port(&params.host, params.port),
            )
        }),
        ConnectionType::MongoDB => connection
            .to_mongodb_params()
            .ok()
            .and_then(|params| optional_host_port(&params.host, params.port))
            .map(|value| (ConnectionCopyTarget::MongoDbAddress, value)),
        ConnectionType::Serial | ConnectionType::PortForwarding => None,
        ConnectionType::Rdp | ConnectionType::Vnc => {
            connection.to_remote_desktop_params().ok().map(|params| {
                (
                    ConnectionCopyTarget::RemoteDesktopAddress,
                    host_port(&params.host, params.port),
                )
            })
        }
        ConnectionType::All => None,
    }
}

fn database_address(connection: &StoredConnection) -> Option<String> {
    let params = connection.to_db_connection().ok()?;
    if matches!(
        params.database_type,
        DatabaseType::SQLite | DatabaseType::DuckDB
    ) {
        return non_empty(params.host);
    }
    Some(host_port(&params.host, params.port))
}

fn forwarding_address(connection: &StoredConnection) -> Option<String> {
    let params = connection.to_port_forwarding_params().ok()?;
    let address = host_port(&params.bind_host, params.bind_port);
    match params.kind {
        PortForwardingKind::Local => Some(format!(
            "{address} -> {}",
            host_port(&params.target_host, params.target_port)
        )),
        PortForwardingKind::Dynamic => Some(address),
    }
}

fn connection_username(connection: &StoredConnection) -> Option<String> {
    let username = match connection.connection_type {
        ConnectionType::Database => connection.to_db_connection().ok()?.username,
        ConnectionType::SshSftp => connection.to_ssh_params().ok()?.username,
        ConnectionType::Redis => connection.to_redis_params().ok()?.username?,
        ConnectionType::MongoDB => connection.to_mongodb_params().ok()?.username?,
        ConnectionType::Rdp | ConnectionType::Vnc => {
            connection.to_remote_desktop_params().ok()?.username?
        }
        _ => return None,
    };
    non_empty(username)
}

fn serial_port(connection: &StoredConnection) -> Option<String> {
    (connection.connection_type == ConnectionType::Serial)
        .then(|| connection.to_serial_params().ok())
        .flatten()
        .and_then(|params| non_empty(params.port_name))
}

fn forwarding_rule(connection: &StoredConnection) -> Option<String> {
    (connection.connection_type == ConnectionType::PortForwarding)
        .then(|| forwarding_address(connection))
        .flatten()
}

fn host_port(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

fn optional_host_port(host: &str, port: Option<u16>) -> Option<String> {
    let host = non_empty(host.to_string())?;
    Some(port.map_or(host.clone(), |port| host_port(&host, port)))
}

fn non_empty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use one_core::storage::{SerialParams, SshAuthMethod, SshParams};

    use super::*;

    #[test]
    fn ssh_targets_include_address_and_username_without_password() {
        let connection = StoredConnection::new_ssh(
            "SSH".to_string(),
            SshParams {
                host: "2001:db8::1".to_string(),
                port: 2222,
                username: "alice".to_string(),
                auth_method: SshAuthMethod::Password {
                    password: "secret".to_string(),
                },
                connect_timeout: None,
                keepalive_interval: None,
                keepalive_max: None,
                default_directory: None,
                init_script: None,
                disable_shell_integration: None,
                jump_server: None,
                proxy: None,
                os_id: None,
            },
            None,
        );

        let targets = connection_copy_targets(&connection);
        assert!(targets.contains(&(ConnectionCopyTarget::SshTarget, "[2001:db8::1]:2222".into())));
        assert!(targets.contains(&(ConnectionCopyTarget::Username, "alice".into())));
        assert!(targets.iter().all(|(_, value)| !value.contains("secret")));
    }

    #[test]
    fn serial_target_is_the_device_name() {
        let connection = StoredConnection::new_serial(
            "Serial".to_string(),
            SerialParams {
                port_name: "/dev/ttyUSB0".to_string(),
                ..Default::default()
            },
            None,
        );

        assert_eq!(
            connection_copy_targets(&connection),
            vec![(ConnectionCopyTarget::SerialPort, "/dev/ttyUSB0".into())]
        );
    }
}
