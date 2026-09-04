use one_core::storage::{
    ConnectionType, DatabaseType, PortForwardingKind, RedisMode, StoredConnection,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ConnectionCopyAction {
    BasicInfo,
    FullInfo,
    Name,
    DatabaseAddress,
    SshTarget,
    RedisAddress,
    MongoDbAddress,
    RemoteDesktopAddress,
    TelnetAddress,
    Username,
    SerialPort,
    ForwardingRule,
    SshCommand,
    SftpCommand,
    JdbcUrl,
    CliCommand,
    ConnectionUri,
    SerialConfig,
    ForwardingCommand,
    SentinelConfig,
    ClusterNodes,
    MqttAddress,
}

pub(super) fn connection_copy_actions(
    connection: &StoredConnection,
    can_export_credentials: bool,
    resolved_ssh: Option<&StoredConnection>,
) -> Vec<ConnectionCopyAction> {
    let mut actions = vec![ConnectionCopyAction::BasicInfo];
    if can_export_credentials {
        actions.push(ConnectionCopyAction::FullInfo);
    }
    actions.push(ConnectionCopyAction::Name);

    match connection.connection_type {
        ConnectionType::Database => {
            if database_address(connection).is_some() {
                actions.push(ConnectionCopyAction::DatabaseAddress);
            }
            if connection_username(connection).is_some() {
                actions.push(ConnectionCopyAction::Username);
            }
            if connection
                .to_db_connection()
                .ok()
                .as_ref()
                .and_then(super::connection_command::database_jdbc_url)
                .is_some()
            {
                actions.push(ConnectionCopyAction::JdbcUrl);
            }
            if connection
                .to_db_connection()
                .ok()
                .as_ref()
                .and_then(super::connection_command::database_command)
                .is_some()
            {
                actions.push(ConnectionCopyAction::CliCommand);
            }
        }
        ConnectionType::SshSftp => {
            let has_target = connection_address(connection).is_some();
            if has_target {
                actions.push(ConnectionCopyAction::SshTarget);
            }
            if connection_username(connection).is_some() {
                actions.push(ConnectionCopyAction::Username);
            }
            if has_target && connection.to_ssh_params().is_ok() {
                actions.extend([
                    ConnectionCopyAction::SshCommand,
                    ConnectionCopyAction::SftpCommand,
                ]);
            }
        }
        ConnectionType::Redis => {
            if let Ok(params) = connection.to_redis_params() {
                match params.mode {
                    RedisMode::Standalone => {
                        if connection_address(connection).is_some() {
                            actions.push(ConnectionCopyAction::RedisAddress);
                        }
                        if super::connection_command::redis_uri(&params).is_some() {
                            actions.push(ConnectionCopyAction::ConnectionUri);
                        }
                        if super::connection_command::redis_command(&params).is_some() {
                            actions.push(ConnectionCopyAction::CliCommand);
                        }
                    }
                    RedisMode::Sentinel => {
                        if super::connection_command::redis_sentinel_config(&params).is_some() {
                            actions.push(ConnectionCopyAction::SentinelConfig);
                        }
                    }
                    RedisMode::Cluster => {
                        if super::connection_command::redis_cluster_nodes(&params).is_some() {
                            actions.push(ConnectionCopyAction::ClusterNodes);
                        }
                    }
                }
            }
        }
        ConnectionType::MongoDB => {
            if connection_address(connection).is_some() {
                actions.push(ConnectionCopyAction::MongoDbAddress);
            }
            if connection_username(connection).is_some() {
                actions.push(ConnectionCopyAction::Username);
            }
            if let Ok(params) = connection.to_mongodb_params()
                && super::connection_command::mongodb_safe_uri(&params).is_some()
            {
                actions.extend([
                    ConnectionCopyAction::ConnectionUri,
                    ConnectionCopyAction::CliCommand,
                ]);
            }
        }
        ConnectionType::Mqtt => {
            if connection_address(connection).is_some() {
                actions.push(ConnectionCopyAction::MqttAddress);
            }
            if connection_username(connection).is_some() {
                actions.push(ConnectionCopyAction::Username);
            }
        }
        ConnectionType::Serial => {
            if serial_port(connection).is_some() {
                actions.extend([
                    ConnectionCopyAction::SerialPort,
                    ConnectionCopyAction::SerialConfig,
                ]);
            }
        }
        ConnectionType::Telnet => {
            if connection_address(connection).is_some() {
                actions.push(ConnectionCopyAction::TelnetAddress);
            }
        }
        ConnectionType::PortForwarding => {
            if forwarding_rule(connection).is_some() {
                actions.push(ConnectionCopyAction::ForwardingRule);
            }
            if let Some(ssh) = resolved_forwarding_ssh(connection, resolved_ssh)
                && let Ok(forwarding) = connection.to_port_forwarding_params()
                && let Ok(ssh) = ssh.to_ssh_params()
                && super::connection_command::forwarding_command(&forwarding, &ssh).is_some()
            {
                actions.push(ConnectionCopyAction::ForwardingCommand);
            }
        }
        ConnectionType::Rdp | ConnectionType::Vnc => {
            if connection_address(connection).is_some() {
                actions.push(ConnectionCopyAction::RemoteDesktopAddress);
            }
            if connection_username(connection).is_some() {
                actions.push(ConnectionCopyAction::Username);
            }
        }
        ConnectionType::All | ConnectionType::Extension => {}
    }
    actions
}

pub(super) fn connection_copy_text(
    action: ConnectionCopyAction,
    connection: &StoredConnection,
    resolved_ssh: Option<&StoredConnection>,
) -> Option<String> {
    match action {
        ConnectionCopyAction::BasicInfo => {
            super::connection_share::connection_share_text(connection)
        }
        // Full info is deliberately loaded from storage only after explicit confirmation.
        ConnectionCopyAction::FullInfo => None,
        ConnectionCopyAction::Name => non_empty(connection.name.clone()),
        ConnectionCopyAction::DatabaseAddress => database_address(connection),
        ConnectionCopyAction::SshTarget => connection_address(connection),
        ConnectionCopyAction::RedisAddress => connection_address(connection),
        ConnectionCopyAction::MongoDbAddress => connection_address(connection),
        ConnectionCopyAction::MqttAddress => connection_address(connection),
        ConnectionCopyAction::RemoteDesktopAddress => connection_address(connection),
        ConnectionCopyAction::TelnetAddress => connection_address(connection),
        ConnectionCopyAction::Username => connection_username(connection),
        ConnectionCopyAction::SerialPort => serial_port(connection),
        ConnectionCopyAction::ForwardingRule => forwarding_rule(connection),
        ConnectionCopyAction::SshCommand => {
            connection_address(connection)?;
            connection
                .to_ssh_params()
                .ok()
                .map(|params| super::connection_command::ssh_command(&params))
        }
        ConnectionCopyAction::SftpCommand => {
            connection_address(connection)?;
            connection
                .to_ssh_params()
                .ok()
                .map(|params| super::connection_command::sftp_command(&params))
        }
        ConnectionCopyAction::JdbcUrl => connection
            .to_db_connection()
            .ok()
            .as_ref()
            .and_then(super::connection_command::database_jdbc_url),
        ConnectionCopyAction::CliCommand => match connection.connection_type {
            ConnectionType::Database => connection
                .to_db_connection()
                .ok()
                .as_ref()
                .and_then(super::connection_command::database_command),
            ConnectionType::Redis => connection
                .to_redis_params()
                .ok()
                .as_ref()
                .and_then(super::connection_command::redis_command),
            ConnectionType::MongoDB => connection
                .to_mongodb_params()
                .ok()
                .as_ref()
                .and_then(super::connection_command::mongodb_command),
            _ => None,
        },
        ConnectionCopyAction::ConnectionUri => match connection.connection_type {
            ConnectionType::Redis => connection
                .to_redis_params()
                .ok()
                .as_ref()
                .and_then(super::connection_command::redis_uri),
            ConnectionType::MongoDB => connection
                .to_mongodb_params()
                .ok()
                .as_ref()
                .and_then(super::connection_command::mongodb_safe_uri),
            _ => None,
        },
        ConnectionCopyAction::SerialConfig => connection
            .to_serial_params()
            .ok()
            .map(|params| super::connection_command::serial_config(&params)),
        ConnectionCopyAction::ForwardingCommand => {
            let ssh = resolved_forwarding_ssh(connection, resolved_ssh)?;
            let forwarding = connection.to_port_forwarding_params().ok()?;
            let ssh = ssh.to_ssh_params().ok()?;
            super::connection_command::forwarding_command(&forwarding, &ssh)
        }
        ConnectionCopyAction::SentinelConfig => connection
            .to_redis_params()
            .ok()
            .as_ref()
            .and_then(super::connection_command::redis_sentinel_config),
        ConnectionCopyAction::ClusterNodes => connection
            .to_redis_params()
            .ok()
            .as_ref()
            .and_then(super::connection_command::redis_cluster_nodes),
    }
}

fn connection_address(connection: &StoredConnection) -> Option<String> {
    match connection.connection_type {
        ConnectionType::Database => database_address(connection),
        ConnectionType::SshSftp => connection
            .to_ssh_params()
            .ok()
            .and_then(|params| optional_host_port(&params.host, Some(params.port))),
        ConnectionType::Redis => connection.to_redis_params().ok().and_then(|params| {
            (params.mode == RedisMode::Standalone)
                .then(|| optional_host_port(&params.host, Some(params.port)))
                .flatten()
        }),
        ConnectionType::MongoDB => connection
            .to_mongodb_params()
            .ok()
            .and_then(|params| optional_host_port(&params.host, params.port))
            .or_else(|| {
                connection
                    .to_mongodb_params()
                    .ok()
                    .and_then(|params| super::connection_command::mongodb_safe_uri(&params))
                    .and_then(|uri| url::Url::parse(&uri).ok())
                    .and_then(|uri| {
                        let host = uri.host_str()?.to_string();
                        Some(
                            uri.port()
                                .map_or_else(|| host.clone(), |port| host_port(&host, port)),
                        )
                    })
            }),
        ConnectionType::Mqtt => connection
            .to_mqtt_params()
            .ok()
            .and_then(|params| optional_host_port(&params.host, Some(params.port))),
        ConnectionType::Serial | ConnectionType::PortForwarding | ConnectionType::Extension => None,
        ConnectionType::Telnet => connection
            .to_telnet_params()
            .ok()
            .and_then(|params| optional_host_port(&params.host, Some(params.port))),
        ConnectionType::Rdp | ConnectionType::Vnc => connection
            .to_remote_desktop_params()
            .ok()
            .and_then(|params| optional_host_port(&params.host, Some(params.port))),
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
    optional_host_port(&params.host, Some(params.port))
}

fn forwarding_address(connection: &StoredConnection) -> Option<String> {
    let params = connection.to_port_forwarding_params().ok()?;
    let bind_host = non_empty(params.bind_host)?;
    let address = host_port(&bind_host, params.bind_port);
    match params.kind {
        PortForwardingKind::Local => {
            let target_host = non_empty(params.target_host)?;
            Some(format!(
                "{address} -> {}",
                host_port(&target_host, params.target_port)
            ))
        }
        PortForwardingKind::Remote => {
            let target_host = non_empty(params.target_host)?;
            Some(format!(
                "{address} <- {}",
                host_port(&target_host, params.target_port)
            ))
        }
        PortForwardingKind::Dynamic => Some(address),
    }
}

fn connection_username(connection: &StoredConnection) -> Option<String> {
    let username = match connection.connection_type {
        ConnectionType::Database => connection.to_db_connection().ok()?.username,
        ConnectionType::SshSftp => connection.to_ssh_params().ok()?.username,
        ConnectionType::Redis => connection.to_redis_params().ok()?.username?,
        ConnectionType::MongoDB => connection.to_mongodb_params().ok()?.username?,
        ConnectionType::Mqtt => connection.to_mqtt_params().ok()?.username?,
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

fn resolved_forwarding_ssh<'a>(
    connection: &StoredConnection,
    resolved_ssh: Option<&'a StoredConnection>,
) -> Option<&'a StoredConnection> {
    let forwarding = connection.to_port_forwarding_params().ok()?;
    resolved_ssh.filter(|ssh| {
        ssh.connection_type == ConnectionType::SshSftp
            && ssh.id == Some(forwarding.ssh_connection_id)
    })
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
    use one_core::storage::{
        DatabaseType, DbConnectionConfig, MongoDBParams, PortForwardingKind, PortForwardingParams,
        RedisClusterConfig, RedisMode, RedisParams, RedisSentinelConfig, RemoteDesktopParams,
        RemoteDesktopProtocol, SerialParams, SshAuthMethod, SshParams,
    };

    use super::*;

    fn ssh_connection() -> StoredConnection {
        StoredConnection::new_ssh(
            "SSH".to_string(),
            SshParams {
                sftp_default_directory: None,
                disabled_jump_server: None,
                sftp_account: None,
                host: "2001:db8::1".to_string(),
                port: 2222,
                username: "alice".to_string(),
                auth_method: SshAuthMethod::Password {
                    password: "secret".to_string(),
                },
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
        )
    }

    fn database_connection(database_type: DatabaseType) -> StoredConnection {
        StoredConnection::new_database(
            "Database".to_string(),
            DbConnectionConfig {
                id: String::new(),
                database_type,
                name: String::new(),
                host: "db.example.test".to_string(),
                port: 5432,
                username: "db-user".to_string(),
                password: "db-secret".to_string(),
                credential_reference: None,
                database: Some("app".to_string()),
                service_name: None,
                sid: None,
                workspace_id: None,
                proxy: None,
                extra_params: Default::default(),
            },
            None,
        )
    }

    fn redis_connection(mode: RedisMode) -> StoredConnection {
        let sentinel = (mode == RedisMode::Sentinel).then(|| RedisSentinelConfig {
            master_name: "mymaster".to_string(),
            sentinels: vec!["redis-1:26379".to_string()],
            sentinel_password: Some("sentinel-secret".to_string()),
            credential_reference: None,
        });
        let cluster = (mode == RedisMode::Cluster).then(|| RedisClusterConfig {
            nodes: vec!["redis-1:6379".to_string(), "redis-2:6379".to_string()],
        });
        StoredConnection::new_redis(
            "Redis".to_string(),
            RedisParams {
                host: "redis.example.test".to_string(),
                port: 6379,
                password: Some("redis-secret".to_string()),
                username: Some("redis-user".to_string()),
                credential_reference: None,
                db_index: 2,
                mode,
                use_tls: false,
                connect_timeout: None,
                sentinel,
                cluster,
                ssh_tunnel: None,
            },
            None,
        )
    }

    #[test]
    fn ssh_copy_actions_cover_safe_info_targets_and_commands() {
        assert_eq!(
            connection_copy_actions(&ssh_connection(), true, None),
            vec![
                ConnectionCopyAction::BasicInfo,
                ConnectionCopyAction::FullInfo,
                ConnectionCopyAction::Name,
                ConnectionCopyAction::SshTarget,
                ConnectionCopyAction::Username,
                ConnectionCopyAction::SshCommand,
                ConnectionCopyAction::SftpCommand,
            ]
        );
    }

    #[test]
    fn full_info_action_is_hidden_without_credential_export_permission() {
        let actions = connection_copy_actions(&ssh_connection(), false, None);
        assert!(!actions.contains(&ConnectionCopyAction::FullInfo));
        assert!(actions.contains(&ConnectionCopyAction::BasicInfo));
        assert!(actions.contains(&ConnectionCopyAction::SshCommand));
    }

    #[test]
    fn database_copy_actions_include_jdbc_and_cli_except_for_external_drivers() {
        let builtin =
            connection_copy_actions(&database_connection(DatabaseType::PostgreSQL), true, None);
        assert!(builtin.contains(&ConnectionCopyAction::DatabaseAddress));
        assert!(builtin.contains(&ConnectionCopyAction::Username));
        assert!(builtin.contains(&ConnectionCopyAction::JdbcUrl));
        assert!(builtin.contains(&ConnectionCopyAction::CliCommand));

        let external = connection_copy_actions(
            &database_connection(DatabaseType::external("custom-driver")),
            true,
            None,
        );
        assert!(!external.contains(&ConnectionCopyAction::JdbcUrl));
        assert!(!external.contains(&ConnectionCopyAction::CliCommand));
    }

    #[test]
    fn redis_modes_offer_only_commands_that_match_their_topology() {
        let standalone =
            connection_copy_actions(&redis_connection(RedisMode::Standalone), true, None);
        assert!(standalone.contains(&ConnectionCopyAction::RedisAddress));
        assert!(standalone.contains(&ConnectionCopyAction::ConnectionUri));
        assert!(standalone.contains(&ConnectionCopyAction::CliCommand));

        let sentinel = connection_copy_actions(&redis_connection(RedisMode::Sentinel), true, None);
        assert!(sentinel.contains(&ConnectionCopyAction::SentinelConfig));
        assert!(!sentinel.contains(&ConnectionCopyAction::ConnectionUri));
        assert!(!sentinel.contains(&ConnectionCopyAction::CliCommand));
        assert!(!sentinel.contains(&ConnectionCopyAction::RedisAddress));

        let cluster = connection_copy_actions(&redis_connection(RedisMode::Cluster), true, None);
        assert!(cluster.contains(&ConnectionCopyAction::ClusterNodes));
        assert!(!cluster.contains(&ConnectionCopyAction::ConnectionUri));
        assert!(!cluster.contains(&ConnectionCopyAction::CliCommand));
        assert!(!cluster.contains(&ConnectionCopyAction::RedisAddress));
    }

    #[test]
    fn mongodb_copy_actions_include_safe_uri_and_mongosh_command() {
        let connection = StoredConnection::new_mongodb(
            "MongoDB".to_string(),
            MongoDBParams {
                driver_variant: Default::default(),
                connection_string: "mongodb://admin:secret@mongo.example.test:27017/app"
                    .to_string(),
                host: "mongo.example.test".to_string(),
                port: Some(27017),
                database: Some("app".to_string()),
                username: Some("admin".to_string()),
                password: Some("secret".to_string()),
                credential_reference: None,
                auth_source: Some("admin".to_string()),
                replica_set: None,
                read_preference: None,
                use_srv_record: false,
                direct_connection: false,
                use_tls: false,
                connect_timeout_seconds: None,
                application_name: None,
                ssh_tunnel: None,
            },
            None,
        );

        let actions = connection_copy_actions(&connection, true, None);
        assert!(actions.contains(&ConnectionCopyAction::MongoDbAddress));
        assert!(actions.contains(&ConnectionCopyAction::ConnectionUri));
        assert!(actions.contains(&ConnectionCopyAction::CliCommand));
    }

    #[test]
    fn serial_copy_actions_include_port_and_configuration() {
        let connection = StoredConnection::new_serial(
            "Serial".to_string(),
            SerialParams {
                port_name: "/dev/ttyUSB0".to_string(),
                ..Default::default()
            },
            None,
        );

        assert_eq!(
            connection_copy_actions(&connection, true, None),
            vec![
                ConnectionCopyAction::BasicInfo,
                ConnectionCopyAction::FullInfo,
                ConnectionCopyAction::Name,
                ConnectionCopyAction::SerialPort,
                ConnectionCopyAction::SerialConfig,
            ]
        );
    }

    #[test]
    fn forwarding_command_requires_a_resolved_ssh_connection() {
        let forwarding = StoredConnection::new_port_forwarding(
            "Forward".to_string(),
            PortForwardingParams {
                ssh_connection_id: 42,
                kind: PortForwardingKind::Local,
                bind_host: "127.0.0.1".to_string(),
                bind_port: 3307,
                target_host: "db.internal".to_string(),
                target_port: 3306,
            },
            None,
        );
        let mut ssh = ssh_connection();
        ssh.id = Some(42);

        let unresolved = connection_copy_actions(&forwarding, true, None);
        assert!(unresolved.contains(&ConnectionCopyAction::ForwardingRule));
        assert!(!unresolved.contains(&ConnectionCopyAction::ForwardingCommand));

        let resolved = connection_copy_actions(&forwarding, true, Some(&ssh));
        assert!(resolved.contains(&ConnectionCopyAction::ForwardingCommand));
    }

    #[test]
    fn remote_forwarding_rule_uses_reverse_direction() {
        let forwarding = StoredConnection::new_port_forwarding(
            "Reverse".to_string(),
            PortForwardingParams {
                ssh_connection_id: 42,
                kind: PortForwardingKind::Remote,
                bind_host: "127.0.0.1".to_string(),
                bind_port: 18080,
                target_host: "127.0.0.1".to_string(),
                target_port: 3000,
            },
            None,
        );

        assert_eq!(
            Some("127.0.0.1:18080 <- 127.0.0.1:3000".to_string()),
            connection_copy_text(ConnectionCopyAction::ForwardingRule, &forwarding, None,)
        );
    }

    #[test]
    fn incomplete_forwarding_connections_hide_invalid_rules_and_commands() {
        let mut forwarding_params = PortForwardingParams {
            ssh_connection_id: 42,
            kind: PortForwardingKind::Local,
            bind_host: String::new(),
            bind_port: 3307,
            target_host: "db.internal".to_string(),
            target_port: 3306,
        };
        let mut ssh = ssh_connection();
        ssh.id = Some(42);

        let mut forwarding = StoredConnection::new_port_forwarding(
            "Forward".to_string(),
            forwarding_params.clone(),
            None,
        );
        for action in [
            ConnectionCopyAction::ForwardingRule,
            ConnectionCopyAction::ForwardingCommand,
        ] {
            assert!(!connection_copy_actions(&forwarding, true, Some(&ssh)).contains(&action));
            assert_eq!(None, connection_copy_text(action, &forwarding, Some(&ssh)));
        }

        forwarding_params.bind_host = "127.0.0.1".to_string();
        forwarding_params.target_host.clear();
        forwarding = StoredConnection::new_port_forwarding(
            "Forward".to_string(),
            forwarding_params.clone(),
            None,
        );
        for action in [
            ConnectionCopyAction::ForwardingRule,
            ConnectionCopyAction::ForwardingCommand,
        ] {
            assert!(!connection_copy_actions(&forwarding, true, Some(&ssh)).contains(&action));
            assert_eq!(None, connection_copy_text(action, &forwarding, Some(&ssh)));
        }

        forwarding_params.kind = PortForwardingKind::Dynamic;
        forwarding =
            StoredConnection::new_port_forwarding("Forward".to_string(), forwarding_params, None);
        let dynamic_actions = connection_copy_actions(&forwarding, true, Some(&ssh));
        assert!(dynamic_actions.contains(&ConnectionCopyAction::ForwardingRule));
        assert!(dynamic_actions.contains(&ConnectionCopyAction::ForwardingCommand));

        let mut empty_ssh_params = ssh.to_ssh_params().expect("SSH params");
        empty_ssh_params.host.clear();
        let mut empty_ssh = StoredConnection::new_ssh("SSH".to_string(), empty_ssh_params, None);
        empty_ssh.id = Some(42);
        assert!(
            !connection_copy_actions(&forwarding, true, Some(&empty_ssh))
                .contains(&ConnectionCopyAction::ForwardingCommand)
        );
        assert_eq!(
            None,
            connection_copy_text(
                ConnectionCopyAction::ForwardingCommand,
                &forwarding,
                Some(&empty_ssh),
            )
        );
    }

    #[test]
    fn remote_desktop_copy_actions_include_address_and_username() {
        let connection = StoredConnection::new_remote_desktop(
            "RDP".to_string(),
            RemoteDesktopParams {
                protocol: RemoteDesktopProtocol::Rdp,
                host: "rdp.example.test".to_string(),
                port: 3389,
                username: Some("desktop-user".to_string()),
                password: Some("desktop-secret".to_string()),
                credential_reference: None,
                domain: None,
                read_only: false,
                audio_playback: false,
                proxy: None,
                backend_preference: Default::default(),
                rdp: None,
            },
            None,
        );

        let actions = connection_copy_actions(&connection, true, None);
        assert!(actions.contains(&ConnectionCopyAction::RemoteDesktopAddress));
        assert!(actions.contains(&ConnectionCopyAction::Username));
    }

    #[test]
    fn incomplete_host_based_connections_hide_invalid_targets_and_commands() {
        let mut ssh_params = ssh_connection().to_ssh_params().expect("SSH params");
        ssh_params.host.clear();
        let ssh = StoredConnection::new_ssh("SSH".to_string(), ssh_params, None);
        let ssh_actions = connection_copy_actions(&ssh, true, None);
        for action in [
            ConnectionCopyAction::SshTarget,
            ConnectionCopyAction::SshCommand,
            ConnectionCopyAction::SftpCommand,
        ] {
            assert!(!ssh_actions.contains(&action));
            assert_eq!(None, connection_copy_text(action, &ssh, None));
        }

        let mut database_params = database_connection(DatabaseType::PostgreSQL)
            .to_db_connection()
            .expect("database params");
        database_params.host.clear();
        let database =
            StoredConnection::new_database("Database".to_string(), database_params, None);
        let database_actions = connection_copy_actions(&database, true, None);
        for action in [
            ConnectionCopyAction::DatabaseAddress,
            ConnectionCopyAction::JdbcUrl,
            ConnectionCopyAction::CliCommand,
        ] {
            assert!(!database_actions.contains(&action));
            assert_eq!(None, connection_copy_text(action, &database, None));
        }

        let mut redis_params = redis_connection(RedisMode::Standalone)
            .to_redis_params()
            .expect("Redis params");
        redis_params.host.clear();
        let redis = StoredConnection::new_redis("Redis".to_string(), redis_params, None);
        let redis_actions = connection_copy_actions(&redis, true, None);
        for action in [
            ConnectionCopyAction::RedisAddress,
            ConnectionCopyAction::ConnectionUri,
            ConnectionCopyAction::CliCommand,
        ] {
            assert!(!redis_actions.contains(&action));
            assert_eq!(None, connection_copy_text(action, &redis, None));
        }

        let remote_desktop = StoredConnection::new_remote_desktop(
            "RDP".to_string(),
            RemoteDesktopParams {
                protocol: RemoteDesktopProtocol::Rdp,
                host: String::new(),
                port: 3389,
                username: Some("desktop-user".to_string()),
                password: Some("desktop-secret".to_string()),
                credential_reference: None,
                domain: None,
                read_only: false,
                audio_playback: false,
                proxy: None,
                backend_preference: Default::default(),
                rdp: None,
            },
            None,
        );
        let remote_actions = connection_copy_actions(&remote_desktop, true, None);
        assert!(!remote_actions.contains(&ConnectionCopyAction::RemoteDesktopAddress));
        assert_eq!(
            None,
            connection_copy_text(
                ConnectionCopyAction::RemoteDesktopAddress,
                &remote_desktop,
                None,
            )
        );
    }
}
