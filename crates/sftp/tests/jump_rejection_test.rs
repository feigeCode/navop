//! 跳板机拒绝 direct-tcpip 时的 SFTP 连接错误链测试。
//!
//! JumpServer 这类堡垒机会默认拒绝 `direct-tcpip` 通道。这里用一个
//! 本地 russh 测试服务器仿真该行为，验证 `RusshSftpClient::connect` 的
//! 错误里带有 JumpChannelRejected 提示与服务器回传的描述文本。

use std::sync::Arc;
use std::time::Duration;

use russh::keys::{Algorithm, PrivateKey};
use russh::server::{Auth, Server as _};
use sftp::{RusshSftpClient, SftpClient};
use tokio::net::TcpListener;

use ssh::{
    BastionUserFormat, HostKeyVerifier, JumpChannelRejected, JumpMode, JumpServerConnectConfig,
    SshAuth, SshConnectConfig,
};

/// 接受登录但像 JumpServer 一样拒绝所有 direct-tcpip 通道的测试服务器。
#[derive(Clone)]
struct ForwardingDenyingJumpServer;

impl russh::server::Server for ForwardingDenyingJumpServer {
    type Handler = Self;

    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self::Handler {
        self.clone()
    }
}

impl russh::server::Handler for ForwardingDenyingJumpServer {
    type Error = anyhow::Error;

    async fn auth_password(&mut self, _: &str, _: &str) -> Result<Auth, Self::Error> {
        Ok(Auth::Accept)
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        _channel: russh::Channel<russh::server::Msg>,
        _host_to_connect: &str,
        _port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: russh::server::ChannelOpenHandle,
        _session: &mut russh::server::Session,
    ) -> Result<(), Self::Error> {
        reply
            .reject(russh::ChannelOpenFailure::Other {
                code: russh::ChannelOpenFailure::AdministrativelyProhibited.code(),
                reason: "port forwarding is disabled".to_string(),
            })
            .await;
        Ok(())
    }
}

async fn spawn_forwarding_denying_jump_server() -> std::io::Result<std::net::SocketAddr> {
    let socket = TcpListener::bind(("127.0.0.1", 0)).await?;
    let address = socket.local_addr()?;
    let ed25519 = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).map_err(|error| {
        std::io::Error::other(format!("failed to generate test host key: {error}"))
    })?;
    let server_config = Arc::new(russh::server::Config {
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        keys: vec![ed25519],
        ..Default::default()
    });
    tokio::spawn(async move {
        let mut server = ForwardingDenyingJumpServer;
        server.run_on_socket(server_config, &socket).await
    });
    Ok(address)
}

#[tokio::test]
async fn sftp_jump_direct_tcpip_rejection_surfaces_hint_and_server_description() {
    let jump_address = spawn_forwarding_denying_jump_server()
        .await
        .expect("test server should bind");

    let config = SshConnectConfig {
        host: "10.255.255.1".to_owned(),
        port: 22,
        username: "tester".to_owned(),
        auth: SshAuth::Password("password".to_owned()),
        timeout: Some(Duration::from_secs(10)),
        keepalive_interval: None,
        keepalive_max: None,
        jump_server: Some(JumpServerConnectConfig {
            host: jump_address.ip().to_string(),
            port: jump_address.port(),
            username: "jump".to_owned(),
            auth: SshAuth::Password("password".to_owned()),
            mode: JumpMode::default(),
            bastion_asset: String::new(),
            bastion_account: String::new(),
            bastion_user_format: BastionUserFormat::default(),
        }),
        proxy: None,
        keyboard_interactive_responder: None,
        host_key_verifier: HostKeyVerifier::insecure(),
        x11_forwarding: false,
        allow_legacy_algorithms: false,
        forward_agent: false,
        agent_identities: Vec::new(),
        proxy_command: None,
    };

    let result = RusshSftpClient::connect(config).await;

    let Err(error) = result else {
        panic!("a jump server rejecting direct-tcpip must fail the SFTP connection");
    };
    assert!(
        error.downcast_ref::<JumpChannelRejected>().is_some(),
        "rejection should be wrapped as JumpChannelRejected, got: {error:#}"
    );
    assert!(format!("{error:#}").contains("Failed to open channel"));
    assert!(
        format!("{error:#}").contains("port forwarding is disabled"),
        "server description should be surfaced, got: {error:#}"
    );
}
