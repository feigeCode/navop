use one_core::storage::{MongoDBParams, SshAuthMethod, SshParams, StoredConnection};

use super::connection_share_text_for_locale;

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
