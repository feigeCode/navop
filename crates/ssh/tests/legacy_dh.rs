//! 老旧 SSH 设备（仅提供 1024 位 DH / ssh-dss / CBC / hmac-sha1）连接的验证契约。
//!
//! 这些设备由用户显式提供用于本地回归验证，因此默认 `#[ignore]`，不会在常规 CI
//! 中生效；需要手动验证时用：
//! `cargo test -p ssh --test legacy_dh -- --ignored`
//!
//! 依赖两台 OpenSSH 老服务器的既有行为（见 AGENTS 中“老旧设备 DH 组”经验）：
//! - 172.29.254.120：OpenSSH_5.2，KEX 含 group1-sha1 / group14-sha1 / GEX-sha1，host key 仅 ssh-dss，
//!   CBC 系 cipher，MAC 含 hmac-sha1。
//! - 172.29.254.122：OpenSSH_3.8.1p1，KEX 仅 group-exchange-sha1 / group1-sha1，host key 仅 ssh-dss，
//!   MAC 含 hmac-md5 / hmac-sha1 / hmac-ripemd160。

use std::time::Duration;

use ssh::{
    ChannelEvent, HostKeyVerifier, RusshClient, SshAuth, SshChannel, SshClient, SshConnectConfig,
};

const LEGACY_DEVICES: &[(&str, &str)] = &[
    ("172.29.254.120", "OpenSSH_5.2"),
    ("172.29.254.122", "OpenSSH_3.8.1p1"),
];
const USERNAME: &str = "admin";
const PASSWORD: &str = "password";

fn config_for(host: &str, allow_legacy_algorithms: bool) -> SshConnectConfig {
    SshConnectConfig {
        host: host.to_string(),
        port: 22,
        username: USERNAME.to_string(),
        auth: SshAuth::Password(PASSWORD.to_string()),
        timeout: Some(Duration::from_secs(20)),
        keepalive_interval: None,
        keepalive_max: None,
        jump_server: None,
        proxy: None,
        keyboard_interactive_responder: None,
        host_key_verifier: HostKeyVerifier::insecure(),
        x11_forwarding: false,
        allow_legacy_algorithms,
    }
}

async fn run_echo_check(host: &str) -> anyhow::Result<String> {
    let mut client = RusshClient::connect(config_for(host, true)).await?;
    assert!(client.is_connected(), "legacy client should be connected after handshake");

    let mut channel = client.open_channel().await?;
    channel.exec("echo legacy-dh-${HOSTNAME:-ok}").await?;

    let mut stdout = Vec::new();
    while let Some(event) = channel.recv().await {
        match event {
            ChannelEvent::Data(data) => stdout.extend_from_slice(&data),
            ChannelEvent::Close => break,
            _ => {}
        }
    }
    let _ = client.disconnect().await;
    Ok(String::from_utf8_lossy(&stdout).to_string())
}

#[tokio::test]
#[ignore = "需要用户提供的两台老旧 SSH 设备才能进行真实连接验证"]
async fn legacy_devices_connect_with_1024bit_dh() {
    for (host, banner_hint) in LEGACY_DEVICES {
        let output = run_echo_check(host)
            .await
            .unwrap_or_else(|e| panic!("连接/认证/执行失败 on {host} ({banner_hint}): {e:?}"));
        assert!(
            output.contains("legacy-dh"),
            "预期在 {host} ({banner_hint}) 上通过 1024 位 DH 协商并回显成功，实际输出: {output:?}"
        );
    }
}

#[tokio::test]
#[ignore = "需要用户提供的两台老旧 SSH 设备才能进行真实连接验证"]
async fn stock_algorithms_reject_legacy_devices() {
    // 反向契约：不启用 legacy 算法时，这些设备必须协商失败（只能证明该改动确实起决定作用）。
    for (host, banner_hint) in LEGACY_DEVICES {
        let result = RusshClient::connect(config_for(host, false)).await;
        assert!(
            result.is_err(),
            "{host} ({banner_hint}) 在不启用 legacy 算法时不应连接成功"
        );
    }
}