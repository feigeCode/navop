//! 隧道继承 jump/proxy 的单测。
//!
//! 历史 bug：引用 SSH 连接建隧道（DB/Redis/Mongo/MQTT）时，
//! `apply_referenced_ssh_tunnel` 只回填 host/port/username/auth/timeout，
//! 跳板机与代理配置被静默丢弃——引用一个带跳板机的 SSH 连接，实际却不走跳板，
//! 直连内网地址导致连接失败。本模块锁定「引用连接的 jump/proxy 必须继承进
//! 隧道配置」这一语义。

use connection_tunnel::{SshTunnelConfig, TunnelJumpMode};
use one_core::storage::models::{
    JumpServerConfig as StorageJumpServerConfig, RedisParams, SshAuthMethod, SshParams,
    StoredConnection,
};

fn referenced_ssh_with_jump() -> StoredConnection {
    let mut connection = StoredConnection::new_ssh(
        "prod-bastion".to_string(),
        SshParams {
            otp: None,
            remote_file: None,
            disabled_jump_server: None,
            sftp_default_directory: None,
            sftp_account: None,
            host: "target.internal".to_string(),
            port: 22,
            username: "deploy".to_string(),
            auth_method: SshAuthMethod::Password {
                password: "ssh-secret".to_string(),
            },
            credential_reference: None,
            prompt_username: None,
            prompt_password: None,
            keyboard_interactive: None,
            terminal_encoding: Default::default(),
            terminal_type: Default::default(),
            connect_timeout: Some(15),
            keepalive_interval: Some(30),
            keepalive_max: Some(3),
            default_directory: None,
            init_script: None,
            disable_shell_integration: None,
            x11_forwarding: None,
            allow_legacy_algorithms: None,
            forward_agent: None,
            agent_forward_key: None,
            jump_server: Some(StorageJumpServerConfig {
                host: "jump.example.com".to_string(),
                port: 2222,
                username: "hufei".to_string(),
                auth_method: SshAuthMethod::Password {
                    password: "jump-secret".to_string(),
                },
                credential_reference: None,
                mode: one_core::storage::models::JumpMode::ProxyJump,
                bastion: None,
            }),
            proxy: None,
            os_id: None,
            icon: None,
            icon_file_path: None,
            account_expect: Default::default(),
            proxy_command: None,
        },
        Some(7),
    );
    connection.id = Some(42);
    connection
}

#[test]
fn referenced_jump_is_inherited_into_tunnel_config() {
    let ssh = referenced_ssh_with_jump();
    let params = ssh.to_ssh_params().expect("ssh params should parse");

    // 统一回填入口：把引用连接的 jump/proxy 带进隧道配置。
    let mut tunnel = SshTunnelConfig {
        enabled: true,
        connection_id: Some(42),
        ..Default::default()
    };
    one_core::storage::models::inherit_tunnel_jump_and_proxy(&mut tunnel, &params);

    let jump = tunnel
        .jump
        .as_ref()
        .expect("referenced jump server should be inherited");
    assert_eq!("jump.example.com", jump.host);
    assert_eq!(2222, jump.port);
    assert_eq!("hufei", jump.username);
    // 认证按隧道凭据子集展开（auth_type + password/密钥材料）。
    assert_eq!("password", jump.auth_type);
    assert_eq!(Some("jump-secret".to_string()), jump.password);
    assert_eq!(TunnelJumpMode::ProxyJump, jump.mode);
    assert_eq!(None, jump.bastion_asset);
    assert_eq!(None, jump.bastion_account);
}

#[test]
fn explicit_tunnel_jump_is_not_overridden() {
    let ssh = referenced_ssh_with_jump();
    let params = ssh.to_ssh_params().expect("ssh params should parse");
    let mut tunnel = SshTunnelConfig::default();
    tunnel.jump = Some(connection_tunnel::TunnelJumpConfig {
        host: "explicit-jump.internal".to_string(),
        port: 2222,
        username: "ops".to_string(),
        auth_type: "password".to_string(),
        password: Some("explicit-secret".to_string()),
        private_key_path: None,
        private_key_content: None,
        private_key_passphrase: None,
        mode: TunnelJumpMode::ProxyJump,
        bastion_asset: None,
        bastion_account: None,
    });

    // 显式配置优先：隧道自己配了 jump 时不被引用连接覆盖。
    one_core::storage::models::inherit_tunnel_jump_and_proxy(&mut tunnel, &params);
    let jump = tunnel.jump.as_ref().expect("explicit jump should remain");
    assert_eq!("explicit-jump.internal", jump.host);
    assert_eq!("ops", jump.username);
}

#[test]
fn redis_apply_referenced_tunnel_inherits_jump() {
    let ssh = referenced_ssh_with_jump();
    let mut redis = RedisParams {
        host: "redis.internal".to_string(),
        port: 6379,
        password: None,
        username: None,
        db_index: 0,
        mode: Default::default(),
        use_tls: false,
        connect_timeout: Some(10),
        sentinel: None,
        cluster: None,
        ssh_tunnel: Some(SshTunnelConfig {
            enabled: true,
            connection_id: Some(42),
            ..Default::default()
        }),
        credential_reference: None,
    };

    redis
        .apply_referenced_ssh_tunnel(&ssh)
        .expect("referenced ssh connection should be applied");

    let tunnel = redis.ssh_tunnel.as_ref().expect("tunnel should remain");
    assert_eq!(
        "jump.example.com",
        tunnel
            .jump
            .as_ref()
            .expect("jump should be inherited into redis tunnel")
            .host
    );
}

#[test]
fn tunnel_jump_serializes_backward_compatibly() {
    // 旧数据没有 jump 字段，必须能无损解析。
    let legacy_json = serde_json::json!({
        "enabled": true,
        "host": "bastion.internal",
        "port": 22,
        "username": "deploy",
        "auth_type": "password",
    });
    let tunnel: SshTunnelConfig =
        serde_json::from_value(legacy_json).expect("legacy tunnel json should parse");
    assert!(tunnel.jump.is_none());

    // 新数据带上 jump 后 round-trip 保持。
    let ssh = referenced_ssh_with_jump();
    let params = ssh.to_ssh_params().unwrap();
    let mut tunnel = SshTunnelConfig::default();
    one_core::storage::models::inherit_tunnel_jump_and_proxy(&mut tunnel, &params);
    let json = serde_json::to_value(&tunnel).expect("tunnel should serialize");
    let parsed: SshTunnelConfig =
        serde_json::from_value(json).expect("tunnel should round-trip");
    assert!(parsed.jump.is_some());
}

