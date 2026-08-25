use std::collections::HashMap;

use crate::storage::traits::Repository;
use crate::storage::{
    ConnectionRepository, CredentialEntry, CredentialReference, CredentialReferenceLocation,
    DatabaseType, DbConnectionConfig, JumpServerConfig, MongoDBParams, MongoDriverVariant,
    PortForwardingKind, PortForwardingParams, ProxyConfig, ProxyType, RedisMode, RedisParams,
    RedisSentinelConfig, RemoteDesktopBackendPreference, RemoteDesktopParams,
    RemoteDesktopProtocol, SshAuthMethod, SshParams, StoredConnection, TelnetParams,
};

fn reference(id: i64) -> CredentialReference {
    CredentialReference {
        credential_id: id,
        credential_cloud_id: None,
        username: true,
        password: true,
        private_key: false,
        passphrase: false,
    }
}

fn proxy(id: i64) -> ProxyConfig {
    ProxyConfig {
        proxy_type: ProxyType::Socks5,
        host: "proxy.example.com".to_string(),
        port: 1080,
        username: None,
        password: None,
        credential_reference: Some(reference(id)),
    }
}

pub(super) fn ssh_connection(id: i64) -> StoredConnection {
    StoredConnection::new_ssh(
        "SSH".to_string(),
        SshParams {
            sftp_account: None,
            host: "ssh.example.com".to_string(),
            port: 22,
            username: String::new(),
            auth_method: SshAuthMethod::Password {
                password: String::new(),
            },
            credential_reference: Some(reference(id)),
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
            jump_server: Some(JumpServerConfig {
                host: "jump.example.com".to_string(),
                port: 22,
                username: String::new(),
                auth_method: SshAuthMethod::Password {
                    password: String::new(),
                },
                credential_reference: Some(reference(id)),
            }),
            proxy: Some(proxy(id)),
            os_id: None,
            icon: None,
            account_expect: Default::default(),
        },
        None,
    )
}

pub(super) fn database_connection(id: i64) -> StoredConnection {
    StoredConnection::new_database(
        "Database".to_string(),
        DbConnectionConfig {
            id: String::new(),
            database_type: DatabaseType::MySQL,
            name: String::new(),
            host: "db.example.com".to_string(),
            port: 3306,
            username: String::new(),
            password: String::new(),
            credential_reference: Some(reference(id)),
            database: None,
            service_name: None,
            sid: None,
            workspace_id: None,
            proxy: Some(proxy(id)),
            extra_params: HashMap::new(),
        },
        None,
    )
}

pub(super) fn redis_connection(id: i64) -> StoredConnection {
    StoredConnection::new_redis(
        "Redis".to_string(),
        RedisParams {
            host: "redis.example.com".to_string(),
            port: 6379,
            password: None,
            username: None,
            credential_reference: Some(reference(id)),
            db_index: 0,
            mode: RedisMode::Sentinel,
            use_tls: false,
            connect_timeout: None,
            sentinel: Some(RedisSentinelConfig {
                master_name: "primary".to_string(),
                sentinels: vec!["sentinel.example.com:26379".to_string()],
                sentinel_password: None,
                credential_reference: Some(reference(id)),
            }),
            cluster: None,
            ssh_tunnel: None,
        },
        None,
    )
}

pub(super) fn mongodb_connection(id: i64) -> StoredConnection {
    StoredConnection::new_mongodb(
        "MongoDB".to_string(),
        MongoDBParams {
            driver_variant: MongoDriverVariant::Modern,
            connection_string: String::new(),
            host: "mongo.example.com".to_string(),
            port: Some(27017),
            database: None,
            username: None,
            password: None,
            credential_reference: Some(reference(id)),
            auth_source: None,
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
    )
}

pub(super) fn remote_desktop_connection(id: i64) -> StoredConnection {
    StoredConnection::new_remote_desktop(
        "RDP".to_string(),
        RemoteDesktopParams {
            protocol: RemoteDesktopProtocol::Rdp,
            host: "desktop.example.com".to_string(),
            port: 3389,
            username: None,
            password: None,
            credential_reference: Some(reference(id)),
            domain: None,
            read_only: false,
            audio_playback: false,
            proxy: Some(proxy(id)),
            backend_preference: RemoteDesktopBackendPreference::Canvas,
            rdp: None,
        },
        None,
    )
}

pub(super) fn telnet_connection(id: i64) -> StoredConnection {
    StoredConnection::new_telnet(
        "Telnet".to_string(),
        TelnetParams {
            host: "telnet.example.com".to_string(),
            port: 23,
            credential_reference: Some(reference(id)),
            prompt_username: None,
            prompt_password: None,
            backspace_code: Default::default(),
            login_script: Vec::new(),
        },
        None,
    )
}

pub(super) fn tunnel_connection(kind: &str, ssh_id: i64) -> StoredConnection {
    let tunnel = connection_tunnel::SshTunnelConfig {
        enabled: true,
        connection_id: Some(ssh_id),
        ..Default::default()
    };
    match kind {
        "database" => {
            let mut connection = database_connection(i64::MAX);
            let mut params = connection.to_db_connection().unwrap();
            params.credential_reference = None;
            params.proxy = None;
            params.extra_params = HashMap::from([
                ("ssh_tunnel_enabled".to_string(), "true".to_string()),
                ("ssh_connection_id".to_string(), ssh_id.to_string()),
            ]);
            connection.params = serde_json::to_string(&params).unwrap();
            connection
        }
        "redis" => {
            let mut connection = redis_connection(i64::MAX);
            let mut params = connection.to_redis_params().unwrap();
            params.credential_reference = None;
            params.sentinel = None;
            params.ssh_tunnel = Some(tunnel);
            connection.params = serde_json::to_string(&params).unwrap();
            connection
        }
        "mongodb" => {
            let mut connection = mongodb_connection(i64::MAX);
            let mut params = connection.to_mongodb_params().unwrap();
            params.credential_reference = None;
            params.ssh_tunnel = Some(tunnel);
            connection.params = serde_json::to_string(&params).unwrap();
            connection
        }
        _ => unreachable!("unsupported tunnel fixture"),
    }
}

pub(super) fn port_forwarding_connection(ssh_id: i64) -> StoredConnection {
    StoredConnection::new_port_forwarding(
        "Port forwarding".to_string(),
        PortForwardingParams {
            ssh_connection_id: ssh_id,
            kind: PortForwardingKind::Local,
            bind_host: "127.0.0.1".to_string(),
            bind_port: 13306,
            target_host: "db.example.com".to_string(),
            target_port: 3306,
        },
        None,
    )
}

pub(super) fn repositories() -> (
    tempfile::TempDir,
    crate::storage::connection::SqliteConnection,
    ConnectionRepository,
) {
    let (temp, connection, _) = super::test_repository();
    let repository = ConnectionRepository::new(connection.clone());
    (temp, connection, repository)
}

pub(super) fn insert_credential(repository: &ConnectionRepository) -> i64 {
    let mut credential = CredentialEntry::new("Shared");
    repository
        .credential_repository()
        .insert(&mut credential)
        .expect("insert credential")
}

#[test]
fn reference_scanner_finds_all_direct_and_tunnel_locations() {
    let (_temp, _connection, repository) = repositories();
    let credential_id = insert_credential(&repository);
    for mut connection in [
        ssh_connection(credential_id),
        database_connection(credential_id),
        redis_connection(credential_id),
        mongodb_connection(credential_id),
        remote_desktop_connection(credential_id),
    ] {
        repository
            .insert(&mut connection)
            .expect("insert connection");
    }
    let mut ssh = ssh_connection(credential_id);
    let ssh_id = repository.insert(&mut ssh).expect("insert tunnel ssh");
    for kind in ["database", "redis", "mongodb"] {
        let mut connection = tunnel_connection(kind, ssh_id);
        repository
            .insert(&mut connection)
            .expect("insert tunnel connection");
    }
    let mut forwarding = port_forwarding_connection(ssh_id);
    repository
        .insert(&mut forwarding)
        .expect("insert port forwarding connection");

    let hits = repository
        .credential_repository()
        .referencing_connections(credential_id)
        .expect("scan references");
    assert_eq!(25, hits.len());
    assert_eq!(
        10,
        hits.iter()
            .filter(|hit| hit.location == CredentialReferenceLocation::Primary)
            .count()
    );
    assert_eq!(
        12,
        hits.iter()
            .filter(|hit| hit.via_ssh_connection_id.is_some())
            .count()
    );
}

#[test]
fn reference_scanner_finds_telnet_primary_reference() {
    let (_temp, _connection, repository) = repositories();
    let credential_id = insert_credential(&repository);
    let mut connection = telnet_connection(credential_id);
    let connection_id = repository
        .insert(&mut connection)
        .expect("insert Telnet connection");

    let hits = repository
        .credential_repository()
        .referencing_connections(credential_id)
        .expect("scan Telnet references");

    assert_eq!(1, hits.len());
    assert_eq!(connection_id, hits[0].connection_id);
    assert_eq!(
        crate::storage::ConnectionType::Telnet,
        hits[0].connection_type
    );
    assert_eq!(CredentialReferenceLocation::Primary, hits[0].location);
    assert_eq!(None, hits[0].via_ssh_connection_id);
}
