//! P1 堡垒机直连模式（BastionDirect）—— Red 测试先行。
//!
//! 目标：`JumpServerConfig` 增加 `mode`（缺省 `ProxyJump`，旧数据无感）与
//! `bastion`（BastionDirect 时生效），并能按 JumpServer 预设渲染用户名
//! `{user}#{account}#{asset}`。先写测试（Red），再补模型实现（Green）。

use one_core::storage::models::{
    BastionTarget, BastionUserFormat, JumpMode, JumpServerConfig, SshAuthMethod,
};
use serde_json::json;

fn base_jump() -> JumpServerConfig {
    JumpServerConfig {
        host: "10.2.5.220".to_owned(),
        port: 2222,
        username: "hufei".to_owned(),
        auth_method: SshAuthMethod::Password {
            password: "secret".to_owned(),
        },
        credential_reference: None,
        mode: JumpMode::ProxyJump,
        bastion: None,
    }
}

#[test]
fn legacy_jump_json_defaults_to_proxy_jump_mode() {
    // 存量数据没有 mode/bastion 字段：反序列化必须得到 ProxyJump 且无堡垒目标。
    let legacy = json!({
        "host": "10.2.5.220",
        "port": 2222,
        "username": "hufei",
        "auth_method": {"Password": {"password": "secret"}},
    });
    let parsed: JumpServerConfig = serde_json::from_value(legacy).expect("legacy json must parse");
    assert_eq!(parsed.mode, JumpMode::ProxyJump);
    assert!(parsed.bastion.is_none());
}

#[test]
fn proxy_jump_mode_omits_new_fields_when_serializing() {
    // 缺省模式序列化不携带新字段，保证旧版本可读、diff 干净。
    let serialized = serde_json::to_value(base_jump()).expect("serialize");
    assert!(serialized.get("mode").is_none());
    assert!(serialized.get("bastion").is_none());
}

#[test]
fn bastion_direct_round_trips_with_target_and_format() {
    let mut config = base_jump();
    config.mode = JumpMode::BastionDirect;
    config.bastion = Some(BastionTarget {
        asset: "0e85eca7-422a-4cdb-8e4c-8e3ff68eee39".to_owned(),
        account: "root".to_owned(),
        user_format: BastionUserFormat::JumpServer,
    });

    let serialized = serde_json::to_value(&config).expect("serialize");
    assert_eq!(serialized["mode"], "BastionDirect");
    assert_eq!(serialized["bastion"]["account"], "root");

    let parsed: JumpServerConfig =
        serde_json::from_value(serialized).expect("round trip must parse");
    assert_eq!(parsed.mode, JumpMode::BastionDirect);
    let bastion = parsed.bastion.expect("bastion target must survive");
    assert_eq!(bastion.user_format, BastionUserFormat::JumpServer);
    assert_eq!(bastion.asset, "0e85eca7-422a-4cdb-8e4c-8e3ff68eee39");
}

#[test]
fn jump_server_user_format_renders_jumpserver_username() {
    // JumpServer 直连用户名：{user}#{account}#{asset}。
    let mut config = base_jump();
    config.mode = JumpMode::BastionDirect;
    config.bastion = Some(BastionTarget {
        asset: "0e85eca7-422a-4cdb-8e4c-8e3ff68eee39".to_owned(),
        account: "root".to_owned(),
        user_format: BastionUserFormat::JumpServer,
    });
    assert_eq!(
        config.bastion_login_username().as_deref(),
        Some("hufei#root#0e85eca7-422a-4cdb-8e4c-8e3ff68eee39")
    );
}

#[test]
fn jump_server_user_format_returns_none_without_bastion_target() {
    // ProxyJump 或漏配 bastion 时不得臆造用户名。
    assert_eq!(base_jump().bastion_login_username(), None);
}
