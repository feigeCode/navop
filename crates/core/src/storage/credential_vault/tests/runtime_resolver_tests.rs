use std::collections::HashMap;

use crate::storage::traits::Repository;
use crate::storage::{
    ConnectionRepository, CredentialEntry, CredentialReference, DatabaseType, DbConnectionConfig,
    MongoDBParams, MongoDriverVariant, RedisMode, RedisParams, SshAuthMethod, SshParams,
    StoredConnection,
};

use super::with_master_key;

fn password_reference(id: i64) -> CredentialReference {
    CredentialReference {
        credential_id: id,
        credential_cloud_id: None,
        username: true,
        password: true,
        private_key: false,
        passphrase: false,
    }
}

fn insert_vault_ssh(repository: &ConnectionRepository) -> i64 {
    let credentials = repository.credential_repository();
    let mut credential = CredentialEntry::new("Bastion login");
    credential.username = Some("vault-bastion-user".to_string());
    credential.password = Some("vault-bastion-password".to_string());
    let credential_id = credentials
        .insert(&mut credential)
        .expect("insert credential");

    let mut ssh = StoredConnection::new_ssh(
        "Shared bastion".to_string(),
        SshParams {
            sftp_default_directory: None,
            sftp_account: None,
            host: "bastion.example.com".to_string(),
            port: 2222,
            username: "manual-user".to_string(),
            auth_method: SshAuthMethod::Password {
                password: "manual-password".to_string(),
            },
            credential_reference: Some(password_reference(credential_id)),
            prompt_username: None,
            prompt_password: None,
            keyboard_interactive: None,
            terminal_encoding: Default::default(),
            terminal_type: Default::default(),
            connect_timeout: Some(15),
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
    );
    repository.insert(&mut ssh).expect("insert ssh connection")
}

fn database_connection(ssh_id: i64) -> StoredConnection {
    let extra_params = HashMap::from([
        ("ssh_tunnel_enabled".to_string(), "true".to_string()),
        ("ssh_connection_id".to_string(), ssh_id.to_string()),
    ]);
    StoredConnection::new_database(
        "Database".to_string(),
        DbConnectionConfig {
            id: String::new(),
            database_type: DatabaseType::MySQL,
            name: "Database".to_string(),
            host: "db.internal".to_string(),
            port: 3306,
            username: "db-user".to_string(),
            password: "db-password".to_string(),
            credential_reference: None,
            database: None,
            service_name: None,
            sid: None,
            workspace_id: None,
            proxy: None,
            extra_params,
        },
        None,
    )
}

fn redis_connection(ssh_id: i64) -> StoredConnection {
    StoredConnection::new_redis(
        "Redis".to_string(),
        RedisParams {
            host: "redis.internal".to_string(),
            port: 6379,
            password: None,
            username: None,
            credential_reference: None,
            db_index: 0,
            mode: RedisMode::Standalone,
            use_tls: false,
            connect_timeout: None,
            sentinel: None,
            cluster: None,
            ssh_tunnel: Some(connection_tunnel::SshTunnelConfig {
                enabled: true,
                connection_id: Some(ssh_id),
                ..Default::default()
            }),
        },
        None,
    )
}

fn mongodb_connection(ssh_id: i64) -> StoredConnection {
    StoredConnection::new_mongodb(
        "MongoDB".to_string(),
        MongoDBParams {
            driver_variant: MongoDriverVariant::Modern,
            connection_string: String::new(),
            host: "mongo.internal".to_string(),
            port: Some(27017),
            database: None,
            username: None,
            password: None,
            credential_reference: None,
            auth_source: None,
            replica_set: None,
            read_preference: None,
            use_srv_record: false,
            direct_connection: false,
            use_tls: false,
            connect_timeout_seconds: None,
            application_name: None,
            ssh_tunnel: Some(connection_tunnel::SshTunnelConfig {
                enabled: true,
                connection_id: Some(ssh_id),
                ..Default::default()
            }),
        },
        None,
    )
}

#[test]
fn runtime_resolver_applies_vault_credentials_to_referenced_ssh_tunnels() {
    with_master_key(|| {
        let (_temp, connection, _) = super::test_repository();
        let repository = ConnectionRepository::new(connection);
        let ssh_id = insert_vault_ssh(&repository);

        let database = repository
            .resolve_runtime_connection(&database_connection(ssh_id))
            .expect("resolve database runtime connection")
            .to_db_connection()
            .expect("parse resolved database");
        assert_eq!(
            Some(&"vault-bastion-user".to_string()),
            database.extra_params.get("ssh_username")
        );
        assert_eq!(
            Some(&"vault-bastion-password".to_string()),
            database.extra_params.get("ssh_password")
        );

        let redis = repository
            .resolve_runtime_connection(&redis_connection(ssh_id))
            .expect("resolve redis runtime connection")
            .to_redis_params()
            .expect("parse resolved redis");
        let redis_tunnel = redis.ssh_tunnel.expect("redis tunnel");
        assert_eq!("vault-bastion-user", redis_tunnel.username);
        assert_eq!(
            Some("vault-bastion-password"),
            redis_tunnel.password.as_deref()
        );

        let mongodb = repository
            .resolve_runtime_connection(&mongodb_connection(ssh_id))
            .expect("resolve mongodb runtime connection")
            .to_mongodb_params()
            .expect("parse resolved mongodb");
        let mongo_tunnel = mongodb.ssh_tunnel.expect("mongodb tunnel");
        assert_eq!("vault-bastion-user", mongo_tunnel.username);
        assert_eq!(
            Some("vault-bastion-password"),
            mongo_tunnel.password.as_deref()
        );
    });
}

#[test]
fn runtime_resolver_keeps_original_connection_unmodified_and_fails_closed() {
    with_master_key(|| {
        let (_temp, connection, _) = super::test_repository();
        let repository = ConnectionRepository::new(connection);
        let ssh_id = insert_vault_ssh(&repository);
        let original = redis_connection(ssh_id);
        let original_params = original.params.clone();

        let resolved = repository
            .resolve_runtime_connection(&original)
            .expect("resolve temporary connection");

        assert_eq!(original_params, original.params);
        assert_ne!(original.params, resolved.params);

        let missing = database_connection(ssh_id + 999);
        let error = repository
            .resolve_runtime_connection(&missing)
            .expect_err("missing referenced ssh connection must fail");
        assert!(error.to_string().contains("referenced SSH connection"));
    });
}
