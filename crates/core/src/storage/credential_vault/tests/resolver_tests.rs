use std::collections::HashMap;

use crate::storage::traits::Repository;
use crate::storage::{
    ConnectionType, CredentialEntry, CredentialReference, DatabaseType, DbConnectionConfig,
    JumpServerConfig, MongoDBParams, MongoDriverVariant, ProxyConfig, ProxyType, RedisMode,
    RedisParams, RedisSentinelConfig, RemoteDesktopBackendPreference, RemoteDesktopParams,
    RemoteDesktopProtocol, SshAccountExpect, SshAuthMethod, SshParams, StoredConnection,
    TelnetLoginStep, TelnetParams, TerminalExpectSend,
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

fn ssh_params(reference: Option<CredentialReference>) -> SshParams {
    SshParams {
        host: "ssh.example.com".to_string(),
        port: 22,
        username: "manual-user".to_string(),
        auth_method: SshAuthMethod::Password {
            password: "manual-password".to_string(),
        },
        credential_reference: reference,
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
        account_expect: Default::default(),
    }
}

fn insert_password_credential(repository: &super::CredentialRepository) -> i64 {
    let mut credential = CredentialEntry::new("Shared login");
    credential.username = Some("vault-user".to_string());
    credential.password = Some("vault-password".to_string());
    repository
        .insert(&mut credential)
        .expect("insert credential")
}

#[test]
fn resolver_applies_shared_login_to_all_primary_connection_types() {
    with_master_key(|| {
        let (_temp, _connection, repository) = super::test_repository();
        let credential_id = insert_password_credential(&repository);
        let reference = Some(password_reference(credential_id));

        let ssh = repository
            .resolve_ssh(ssh_params(reference.clone()))
            .expect("resolve ssh");
        assert_eq!("vault-user", ssh.username);
        assert!(matches!(
            ssh.auth_method,
            SshAuthMethod::Password { ref password } if password == "vault-password"
        ));

        let database = repository
            .resolve_database(DbConnectionConfig {
                id: String::new(),
                database_type: DatabaseType::MySQL,
                name: "Database".to_string(),
                host: "db.example.com".to_string(),
                port: 3306,
                username: "manual-user".to_string(),
                password: "manual-password".to_string(),
                credential_reference: reference.clone(),
                database: None,
                service_name: None,
                sid: None,
                workspace_id: None,
                proxy: None,
                extra_params: HashMap::new(),
            })
            .expect("resolve database");
        assert_eq!("vault-user", database.username);
        assert_eq!("vault-password", database.password);

        let redis = repository
            .resolve_redis(RedisParams {
                host: "redis.example.com".to_string(),
                port: 6379,
                password: Some("manual-password".to_string()),
                username: Some("manual-user".to_string()),
                credential_reference: reference.clone(),
                db_index: 0,
                mode: RedisMode::Standalone,
                use_tls: false,
                connect_timeout: None,
                sentinel: None,
                cluster: None,
                ssh_tunnel: None,
            })
            .expect("resolve redis");
        assert_eq!(Some("vault-user"), redis.username.as_deref());
        assert_eq!(Some("vault-password"), redis.password.as_deref());

        let mongodb = repository
            .resolve_mongodb(MongoDBParams {
                driver_variant: MongoDriverVariant::Modern,
                connection_string: String::new(),
                host: "mongo.example.com".to_string(),
                port: Some(27017),
                database: None,
                username: Some("manual-user".to_string()),
                password: Some("manual-password".to_string()),
                credential_reference: reference.clone(),
                auth_source: None,
                replica_set: None,
                read_preference: None,
                use_srv_record: false,
                direct_connection: false,
                use_tls: false,
                connect_timeout_seconds: None,
                application_name: None,
                ssh_tunnel: None,
            })
            .expect("resolve mongodb");
        assert_eq!(Some("vault-user"), mongodb.username.as_deref());
        assert_eq!(Some("vault-password"), mongodb.password.as_deref());

        let remote = repository
            .resolve_remote_desktop(RemoteDesktopParams {
                protocol: RemoteDesktopProtocol::Rdp,
                host: "rdp.example.com".to_string(),
                port: 3389,
                username: Some("manual-user".to_string()),
                password: Some("manual-password".to_string()),
                credential_reference: reference,
                domain: None,
                read_only: false,
                audio_playback: false,
                proxy: None,
                backend_preference: RemoteDesktopBackendPreference::Canvas,
                rdp: None,
            })
            .expect("resolve remote desktop");
        assert_eq!(Some("vault-user"), remote.username.as_deref());
        assert_eq!(Some("vault-password"), remote.password.as_deref());
    });
}

#[test]
fn resolver_uses_only_credential_expect_and_ignores_connection_overrides() {
    with_master_key(|| {
        let (_temp, _connection, repository) = super::test_repository();
        let mut credential = CredentialEntry::new("Shared login");
        credential.username = Some("vault-user".to_string());
        credential.password = Some("vault-password".to_string());
        credential.ssh_expect = SshAccountExpect {
            username: TerminalExpectSend {
                expect: "Vault username:".to_string(),
                send: String::new(),
            },
            password: TerminalExpectSend {
                expect: "Vault password:".to_string(),
                send: String::new(),
            },
        };
        let credential_id = repository
            .insert(&mut credential)
            .expect("insert credential with expect rules");

        let mut connection = ssh_params(Some(password_reference(credential_id)));
        connection.account_expect.username = TerminalExpectSend {
            expect: "Connection username:".to_string(),
            send: "override-user".to_string(),
        };

        let resolved = repository.resolve_ssh(connection).expect("resolve ssh");

        assert_eq!(resolved.account_expect.username.expect, "Vault username:");
        assert!(resolved.account_expect.username.send.is_empty());
        assert_eq!(resolved.account_expect.password.expect, "Vault password:");
        assert!(resolved.account_expect.password.send.is_empty());
    });
}

#[test]
fn resolver_applies_keychain_login_to_telnet_expect_steps() {
    with_master_key(|| {
        let (_temp, _connection, repository) = super::test_repository();
        let credential_id = insert_password_credential(&repository);
        let params = TelnetParams {
            host: "switch.example.com".to_string(),
            port: 23,
            credential_reference: Some(password_reference(credential_id)),
            prompt_username: None,
            prompt_password: None,
            login_script: vec![
                TelnetLoginStep {
                    expect: r"(?i)(?:login|username)\s*:".to_string(),
                    send: String::new(),
                },
                TelnetLoginStep {
                    expect: r"(?i)password\s*:".to_string(),
                    send: String::new(),
                },
                TelnetLoginStep {
                    expect: "token:".to_string(),
                    send: "explicit-token".to_string(),
                },
            ],
        };

        let resolved = repository.resolve_telnet(params).expect("resolve telnet");

        assert_eq!(resolved.login_script[0].send, "vault-user");
        assert_eq!(resolved.login_script[1].send, "vault-password");
        assert_eq!(resolved.login_script[2].send, "explicit-token");
    });
}

#[test]
fn resolver_reuses_keychain_expect_for_telnet_without_connection_script() {
    with_master_key(|| {
        let (_temp, _connection, repository) = super::test_repository();
        let mut credential = CredentialEntry::new("Telnet login");
        credential.username = Some("telnet-user".to_string());
        credential.password = Some("telnet-password".to_string());
        credential.ssh_expect = SshAccountExpect {
            username: TerminalExpectSend {
                expect: r"(?i)login\s*:".to_string(),
                send: String::new(),
            },
            password: TerminalExpectSend {
                expect: r"(?i)password\s*:".to_string(),
                send: String::new(),
            },
        };
        let credential_id = repository
            .insert(&mut credential)
            .expect("insert telnet credential");
        let original = StoredConnection::new_telnet(
            "Telnet".to_string(),
            TelnetParams {
                host: "switch.example.com".to_string(),
                port: 23,
                credential_reference: Some(password_reference(credential_id)),
                prompt_username: None,
                prompt_password: None,
                login_script: Vec::new(),
            },
            None,
        );
        let original_params = original.params.clone();

        let resolved = repository
            .resolve_connection(&original)
            .expect("resolve telnet connection");
        let resolved_params = resolved.to_telnet_params().expect("parse resolved telnet");

        assert_eq!(original.params, original_params);
        assert_eq!(resolved_params.login_script.len(), 2);
        assert_eq!(resolved_params.login_script[0].send, "telnet-user");
        assert_eq!(resolved_params.login_script[1].send, "telnet-password");
    });
}

#[test]
fn resolver_clears_legacy_connection_expect_without_a_credential_reference() {
    with_master_key(|| {
        let (_temp, _connection, repository) = super::test_repository();
        let mut connection = ssh_params(None);
        connection.account_expect.username = TerminalExpectSend {
            expect: "Legacy username:".to_string(),
            send: "legacy-user".to_string(),
        };
        connection.account_expect.password = TerminalExpectSend {
            expect: "Legacy password:".to_string(),
            send: "legacy-password".to_string(),
        };

        let resolved = repository.resolve_ssh(connection).expect("resolve ssh");

        assert!(resolved.account_expect.username.is_empty());
        assert!(resolved.account_expect.password.is_empty());
    });
}

#[test]
fn resolver_supports_proxy_jump_server_sentinel_and_private_key_content() {
    with_master_key(|| {
        let (_temp, _connection, repository) = super::test_repository();
        let login_id = insert_password_credential(&repository);
        let login_reference = Some(password_reference(login_id));
        let mut key = CredentialEntry::new("SSH key");
        key.username = Some("key-user".to_string());
        key.private_key_path = Some("/local/key".to_string());
        key.private_key_content = Some("private-key-content".to_string());
        key.passphrase = Some("key-passphrase".to_string());
        let key_id = repository.insert(&mut key).expect("insert key");
        let key_reference = Some(CredentialReference {
            credential_id: key_id,
            credential_cloud_id: None,
            username: true,
            password: false,
            private_key: true,
            passphrase: true,
        });

        let mut ssh = ssh_params(key_reference);
        ssh.proxy = Some(ProxyConfig {
            proxy_type: ProxyType::Socks5,
            host: "proxy.example.com".to_string(),
            port: 1080,
            username: None,
            password: None,
            credential_reference: login_reference.clone(),
        });
        ssh.jump_server = Some(JumpServerConfig {
            host: "jump.example.com".to_string(),
            port: 22,
            username: "manual".to_string(),
            auth_method: SshAuthMethod::Password {
                password: "manual".to_string(),
            },
            credential_reference: login_reference,
        });
        let ssh = repository
            .resolve_ssh(ssh)
            .expect("resolve nested ssh auth");
        assert!(matches!(
            ssh.auth_method,
            SshAuthMethod::PrivateKeyContent {
                ref private_key,
                ref passphrase
            } if private_key == "private-key-content"
                && passphrase.as_deref() == Some("key-passphrase")
        ));
        assert_eq!(
            Some("vault-password"),
            ssh.proxy.as_ref().unwrap().password.as_deref()
        );
        assert_eq!("vault-user", ssh.jump_server.as_ref().unwrap().username);

        let sentinel = repository
            .resolve_redis(RedisParams {
                host: "redis.example.com".to_string(),
                port: 6379,
                password: None,
                username: None,
                credential_reference: None,
                db_index: 0,
                mode: RedisMode::Sentinel,
                use_tls: false,
                connect_timeout: None,
                sentinel: Some(RedisSentinelConfig {
                    master_name: "mymaster".to_string(),
                    sentinels: vec!["localhost:26379".to_string()],
                    sentinel_password: None,
                    credential_reference: Some(CredentialReference {
                        credential_id: login_id,
                        credential_cloud_id: None,
                        username: false,
                        password: true,
                        private_key: false,
                        passphrase: false,
                    }),
                }),
                cluster: None,
                ssh_tunnel: None,
            })
            .expect("resolve sentinel");
        assert_eq!(
            Some("vault-password"),
            sentinel
                .sentinel
                .as_ref()
                .unwrap()
                .sentinel_password
                .as_deref()
        );
    });
}

#[test]
fn resolver_uses_stable_cloud_id_without_falling_back_to_foreign_local_id() {
    with_master_key(|| {
        let (_temp, _connection, repository) = super::test_repository();
        let mut login = CredentialEntry::new("Cloud login");
        login.username = Some("cloud-user".to_string());
        login.password = Some("cloud-password".to_string());
        login.cloud_id = Some("credential-cloud-login".to_string());
        let local_id = repository.insert(&mut login).expect("insert cloud login");

        let cloud_reference = Some(CredentialReference {
            credential_id: local_id + 999,
            credential_cloud_id: Some("credential-cloud-login".to_string()),
            username: true,
            password: true,
            private_key: false,
            passphrase: false,
        });
        let database = repository
            .resolve_database(DbConnectionConfig {
                id: String::new(),
                database_type: DatabaseType::PostgreSQL,
                name: "Database".to_string(),
                host: "db.example.com".to_string(),
                port: 5432,
                username: "manual-user".to_string(),
                password: "manual-password".to_string(),
                credential_reference: cloud_reference,
                database: None,
                service_name: None,
                sid: None,
                workspace_id: None,
                proxy: None,
                extra_params: HashMap::new(),
            })
            .expect("resolve by cloud id");
        assert_eq!("cloud-user", database.username);
        assert_eq!("cloud-password", database.password);

        let missing_cloud_reference = Some(CredentialReference {
            credential_id: local_id,
            credential_cloud_id: Some("credential-cloud-missing".to_string()),
            username: true,
            password: true,
            private_key: false,
            passphrase: false,
        });
        let error = repository
            .resolve_ssh(ssh_params(missing_cloud_reference))
            .expect_err("cloud id must not fall back to a possibly foreign local id");
        assert!(error.to_string().contains("credential"));
    });
}

#[test]
fn resolve_connection_returns_a_temporary_clone_and_rejects_conflicting_ssh_fields() {
    with_master_key(|| {
        let (_temp, _connection, repository) = super::test_repository();
        let credential_id = insert_password_credential(&repository);
        let connection = StoredConnection::new_ssh(
            "SSH".to_string(),
            ssh_params(Some(password_reference(credential_id))),
            None,
        );
        let original_params = connection.params.clone();

        let resolved = repository
            .resolve_connection(&connection)
            .expect("resolve temporary connection");

        assert_eq!(original_params, connection.params);
        assert_ne!(original_params, resolved.params);
        assert_eq!(ConnectionType::SshSftp, resolved.connection_type);
        assert!(resolved.params.contains("vault-password"));

        let mut conflicting = ssh_params(Some(CredentialReference {
            credential_id,
            credential_cloud_id: None,
            username: false,
            password: true,
            private_key: true,
            passphrase: false,
        }));
        conflicting.auth_method = SshAuthMethod::Agent;
        let error = repository
            .resolve_ssh(conflicting)
            .expect_err("password and private key cannot both be selected");
        assert!(error.to_string().contains("password and private key"));
    });
}
