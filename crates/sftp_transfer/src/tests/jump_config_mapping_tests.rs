//! 统一跳板机/代理映射的单测。
//!
//! 五个历史构造点（terminal / sftp_transfer / runtime_sftp / port_forwarding /
//! ssh_form）各自维护一份「存储 JumpServerConfig → 连接层
//! JumpServerConnectConfig」的映射，bastion 字段加入后重复度进一步扩大。
//! 本模块锁定统一入口 [`crate::ssh_config::jump_connect_config`] 与
//! [`crate::ssh_config::proxy_connect_config`] 的行为，供各构造点共用。

use one_core::storage::models::{
    BastionUserFormat as StorageBastionUserFormat, JumpMode as StorageJumpMode,
    JumpServerConfig, ProxyConfig as StorageProxyConfig, ProxyType as StorageProxyType,
    SshAuthMethod,
};
use ssh::{BastionUserFormat, JumpMode, ProxyType, SshAuth};

use crate::ssh_config::{jump_connect_config, proxy_connect_config};

fn storage_jump(mode: StorageJumpMode) -> JumpServerConfig {
    JumpServerConfig {
        host: "jump.example.com".to_string(),
        port: 2222,
        username: "hufei".to_string(),
        auth_method: SshAuthMethod::Password {
            password: "secret".to_string(),
        },
        credential_reference: None,
        mode,
        bastion: None,
    }
}

#[test]
fn jump_connect_config_maps_proxy_jump_defaults() {
    let jump = storage_jump(StorageJumpMode::ProxyJump);
    let config = jump_connect_config(&jump);
    assert_eq!(config.host, "jump.example.com");
    assert_eq!(config.port, 2222);
    assert_eq!(config.username, "hufei");
    assert_eq!(config.auth, SshAuth::Password("secret".to_string()));
    assert_eq!(config.mode, JumpMode::ProxyJump);
    assert_eq!(config.bastion_asset, "");
    assert_eq!(config.bastion_account, "");
    assert_eq!(config.bastion_user_format, BastionUserFormat::JumpServer);
}

#[test]
fn jump_connect_config_maps_bastion_direct_target() {
    let mut jump = storage_jump(StorageJumpMode::BastionDirect);
    jump.bastion = Some(one_core::storage::models::BastionTarget {
        asset: "0e85eca7-422a-4cdb-8e4c-8e3ff68eee39".to_string(),
        account: "root".to_string(),
        user_format: StorageBastionUserFormat::JumpServer,
    });
    let config = jump_connect_config(&jump);
    assert_eq!(config.mode, JumpMode::BastionDirect);
    assert_eq!(config.bastion_asset, "0e85eca7-422a-4cdb-8e4c-8e3ff68eee39");
    assert_eq!(config.bastion_account, "root");
    assert_eq!(config.bastion_user_format, BastionUserFormat::JumpServer);
    // 渲染出的登录用户名是堡垒机直连的核心语义。
    assert_eq!(
        config.bastion_login_username().as_deref(),
        Some("hufei#root#0e85eca7-422a-4cdb-8e4c-8e3ff68eee39")
    );
}

#[test]
fn proxy_connect_config_maps_types_and_credentials() {
    let proxy = StorageProxyConfig {
        proxy_type: StorageProxyType::Socks5,
        host: "proxy.example.com".to_string(),
        port: 1080,
        username: Some("pu".to_string()),
        password: Some("pp".to_string()),
        credential_reference: None,
    };
    let config = proxy_connect_config(&proxy);
    assert_eq!(config.proxy_type, ProxyType::Socks5);
    assert_eq!(config.host, "proxy.example.com");
    assert_eq!(config.port, 1080);
    assert_eq!(config.username.as_deref(), Some("pu"));
    assert_eq!(config.password.as_deref(), Some("pp"));
}
