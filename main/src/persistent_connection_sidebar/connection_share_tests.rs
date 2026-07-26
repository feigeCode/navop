use one_core::storage::{
    JumpServerConfig, MongoDBParams, ProxyConfig, ProxyType, SshAuthMethod, SshParams,
    StoredConnection,
};

use super::{connection_full_info_text_for_locale, connection_share_text_for_locale};

fn ssh_connection() -> StoredConnection {
    StoredConnection::new_ssh(
        "Production SSH".to_string(),
        SshParams {
            host: "ssh.example.test".to_string(),
            port: 2222,
            username: "alice".to_string(),
            auth_method: SshAuthMethod::Password {
                password: "super-secret".to_string(),
            },
            connect_timeout: None,
            keepalive_interval: None,
            keepalive_max: None,
            default_directory: None,
            init_script: None,
            disable_shell_integration: None,
            x11_forwarding: None,
            jump_server: None,
            proxy: None,
            os_id: None,
            icon: None,
        },
        None,
    )
}

#[test]
fn share_template_follows_requested_locale() {
    let connection = ssh_connection();
    let zh = connection_share_text_for_locale(&connection, "zh-CN").unwrap();
    let en = connection_share_text_for_locale(&connection, "en").unwrap();

    assert!(zh.contains("【连接信息】"));
    assert!(zh.contains("类型：SSH/SFTP"));
    assert!(zh.contains("主机：ssh.example.test"));
    assert!(en.contains("[Connection Info]"));
    assert!(en.contains("Host: ssh.example.test"));
    assert!(en.contains("Credentials: Obtain separately through a secure channel"));
    assert!(!zh.contains("super-secret"));
    assert!(!en.contains("super-secret"));
}

#[test]
fn mongodb_template_never_uses_credentialed_connection_string() {
    let connection = StoredConnection::new_mongodb(
        "MongoDB".to_string(),
        MongoDBParams {
            driver_variant: Default::default(),
            connection_string: "mongodb://admin:secret@mongo.test:27017/app".to_string(),
            host: "mongo.test".to_string(),
            port: Some(27017),
            database: Some("app".to_string()),
            username: Some("admin".to_string()),
            password: Some("secret".to_string()),
            auth_source: Some("admin".to_string()),
            replica_set: Some("rs0".to_string()),
            read_preference: None,
            use_srv_record: false,
            direct_connection: false,
            use_tls: true,
            connect_timeout_seconds: None,
            application_name: None,
            ssh_tunnel: None,
        },
        None,
    );
    let text = connection_share_text_for_locale(&connection, "en").unwrap();
    assert!(text.contains("Host: mongo.test"));
    assert!(text.contains("Replica Set: rs0"));
    assert!(!text.contains("mongodb://"));
    assert!(!text.contains("secret"));
}

#[test]
fn basic_info_omits_nested_credentials_and_embedded_private_keys() {
    let connection = StoredConnection::new_ssh(
        "Nested SSH".to_string(),
        SshParams {
            host: "ssh.example.test".to_string(),
            port: 22,
            username: "alice".to_string(),
            auth_method: SshAuthMethod::PrivateKeyContent {
                private_key: "TARGET PRIVATE KEY BODY".to_string(),
                passphrase: Some("target-passphrase".to_string()),
            },
            connect_timeout: None,
            keepalive_interval: None,
            keepalive_max: None,
            default_directory: None,
            init_script: None,
            disable_shell_integration: None,
            x11_forwarding: None,
            jump_server: Some(JumpServerConfig {
                host: "jump.example.test".to_string(),
                port: 22,
                username: "jump".to_string(),
                auth_method: SshAuthMethod::Password {
                    password: "jump-password".to_string(),
                },
            }),
            proxy: Some(ProxyConfig {
                proxy_type: ProxyType::Socks5,
                host: "proxy.example.test".to_string(),
                port: 1080,
                username: Some("proxy-user".to_string()),
                password: Some("proxy-password".to_string()),
            }),
            os_id: None,
            icon: None,
        },
        None,
    );

    let text = connection_share_text_for_locale(&connection, "en").unwrap();
    for secret in [
        "TARGET PRIVATE KEY BODY",
        "target-passphrase",
        "jump-password",
        "proxy-password",
    ] {
        assert!(!text.contains(secret));
    }
}

#[test]
fn full_info_keeps_credentials_but_always_redacts_embedded_private_key_contents() {
    let mut connection = StoredConnection::new_ssh(
        "Sensitive SSH".to_string(),
        SshParams {
            host: "ssh.example.test".to_string(),
            port: 22,
            username: "alice".to_string(),
            auth_method: SshAuthMethod::PrivateKeyContent {
                private_key: "TARGET PRIVATE KEY BODY".to_string(),
                passphrase: Some("target-passphrase".to_string()),
            },
            connect_timeout: None,
            keepalive_interval: None,
            keepalive_max: None,
            default_directory: None,
            init_script: None,
            disable_shell_integration: None,
            x11_forwarding: None,
            jump_server: Some(JumpServerConfig {
                host: "jump.example.test".to_string(),
                port: 22,
                username: "jump".to_string(),
                auth_method: SshAuthMethod::Password {
                    password: "jump-password".to_string(),
                },
            }),
            proxy: Some(ProxyConfig {
                proxy_type: ProxyType::Http,
                host: "proxy.example.test".to_string(),
                port: 8080,
                username: Some("proxy-user".to_string()),
                password: Some("proxy-password".to_string()),
            }),
            os_id: None,
            icon: None,
        },
        Some(17),
    );
    connection.id = Some(42);
    connection.selected_databases = Some("[\"metadata\"]".to_string());
    connection.sync_enabled = false;
    connection.cloud_id = Some("cloud-metadata".to_string());
    connection.last_synced_at = Some(100);
    connection.last_used_at = Some(101);
    connection.sort_order = Some(3);
    connection.created_at = Some(102);
    connection.updated_at = Some(103);
    connection.team_id = Some("team-metadata".to_string());
    connection.owner_id = Some("owner-metadata".to_string());
    connection.remark = Some("production".to_string());

    let text = connection_full_info_text_for_locale(&connection, "en").unwrap();
    assert!(text.contains("target-passphrase"));
    assert!(text.contains("jump-password"));
    assert!(text.contains("proxy-password"));
    assert!(!text.contains("TARGET PRIVATE KEY BODY"));
    assert!(text.contains(
        "Embedded private key configured; the private key content was not copied for security"
    ));
    assert!(text.contains("Sensitive SSH"));
    assert!(text.contains("production"));

    for metadata in [
        "\"id\"",
        "workspace_id",
        "selected_databases",
        "sync_enabled",
        "cloud_id",
        "last_synced_at",
        "last_used_at",
        "sort_order",
        "created_at",
        "updated_at",
        "team_id",
        "owner_id",
        "cloud-metadata",
        "team-metadata",
        "owner-metadata",
    ] {
        assert!(!text.contains(metadata));
    }
}

#[test]
fn full_info_preserves_key_paths_and_mongodb_credentialed_uri() {
    let mut connection = ssh_connection();
    connection.params = serde_json::json!({
        "key_path": "/keys/id_ed25519",
        "private_key_path": "/keys/tunnel_ed25519",
        "private_key": "PRIVATE KEY BODY",
        "private_key_content": "TUNNEL PRIVATE KEY BODY",
        "private_key_passphrase": "keep-this-passphrase",
        "connection_string": "mongodb://admin:uri-secret@mongo.test/app"
    })
    .to_string();

    let text = connection_full_info_text_for_locale(&connection, "en").unwrap();
    assert!(text.contains("/keys/id_ed25519"));
    assert!(text.contains("/keys/tunnel_ed25519"));
    assert!(text.contains("keep-this-passphrase"));
    assert!(text.contains("mongodb://admin:uri-secret@mongo.test/app"));
    assert!(!text.contains("PRIVATE KEY BODY"));
    assert!(!text.contains("TUNNEL PRIVATE KEY BODY"));
}

#[test]
fn full_info_redacts_private_key_payloads_even_when_their_json_shape_is_unexpected() {
    let mut connection = ssh_connection();
    connection.params = serde_json::json!({
        "private_key": {
            "pem": "NESTED PRIVATE KEY BODY",
            "format": "pem"
        },
        "private_key_content": [
            "ARRAY PRIVATE KEY BODY"
        ],
        "PrivateKeyContent": {
            "private_key": "ENUM PRIVATE KEY BODY",
            "passphrase": "keep-enum-passphrase"
        }
    })
    .to_string();

    let text = connection_full_info_text_for_locale(&connection, "en").unwrap();
    assert!(text.contains("keep-enum-passphrase"));
    for private_key_body in [
        "NESTED PRIVATE KEY BODY",
        "ARRAY PRIVATE KEY BODY",
        "ENUM PRIVATE KEY BODY",
    ] {
        assert!(!text.contains(private_key_body));
    }
}
