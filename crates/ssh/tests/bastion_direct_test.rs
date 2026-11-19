use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::Duration;

use russh::keys::{Algorithm, PrivateKey};
use russh::server::{Auth, Server as _};
use tokio::net::TcpListener;

use ssh::{
    BastionUserFormat, JumpMode, JumpServerConnectConfig, RusshClient, SshAuth, SshClient,
    SshConnectConfig,
};

/// 直连型堡垒机测试服务器：记录登录用户名，接受密码认证，
/// 像真实堡垒机一样提供 shell 会话（会话即目标）。
#[derive(Clone)]
struct BastionLikeServer {
    seen_usernames: Arc<StdMutex<Vec<String>>>,
}

impl russh::server::Server for BastionLikeServer {
    type Handler = Self;

    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self::Handler {
        self.clone()
    }
}

impl russh::server::Handler for BastionLikeServer {
    type Error = anyhow::Error;

    async fn auth_password(&mut self, user: &str, _: &str) -> Result<Auth, Self::Error> {
        self.seen_usernames
            .lock()
            .expect("username log mutex")
            .push(user.to_owned());
        Ok(Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        _channel: russh::Channel<russh::server::Msg>,
        _reply: russh::server::ChannelOpenHandle,
        _session: &mut russh::server::Session,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

async fn spawn_bastion_like_server()
-> std::io::Result<(std::net::SocketAddr, Arc<StdMutex<Vec<String>>>)> {
    let socket = TcpListener::bind(("127.0.0.1", 0)).await?;
    let address = socket.local_addr()?;
    let seen = Arc::new(StdMutex::new(Vec::new()));
    let mut server = BastionLikeServer {
        seen_usernames: seen.clone(),
    };
    let ed25519 = PrivateKey::random(&mut rand_010::rng(), Algorithm::Ed25519)
        .map_err(std::io::Error::other)?;
    let server_config = Arc::new(russh::server::Config {
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        keys: vec![ed25519],
        ..Default::default()
    });
    tokio::spawn(async move { server.run_on_socket(server_config, &socket).await });
    Ok((address, seen))
}

#[tokio::test]
async fn bastion_direct_connects_without_direct_tcpip_channel() {
    let (bastion_address, seen_usernames) = spawn_bastion_like_server()
        .await
        .expect("server should bind");

    let config = SshConnectConfig {
        host: bastion_address.ip().to_string(),
        port: bastion_address.port(),
        username: "ignored-target-user".to_owned(),
        auth: SshAuth::Password("password".to_owned()),
        timeout: Some(Duration::from_secs(10)),
        keepalive_interval: None,
        keepalive_max: None,
        jump_server: Some(JumpServerConnectConfig {
            host: bastion_address.ip().to_string(),
            port: bastion_address.port(),
            username: "hufei".to_owned(),
            auth: SshAuth::Password("password".to_owned()),
            mode: JumpMode::BastionDirect,
            bastion_asset: "0e85eca7-422a-4cdb-8e4c-8e3ff68eee39".to_owned(),
            bastion_account: "root".to_owned(),
            bastion_user_format: BastionUserFormat::JumpServer,
        }),
        proxy: None,
        keyboard_interactive_responder: None,
        host_key_verifier: ssh::HostKeyVerifier::insecure(),
        x11_forwarding: false,
        allow_legacy_algorithms: false,
        forward_agent: false,
        agent_identities: Vec::new(),
    };

    let mut client = RusshClient::connect(config)
        .await
        .expect("bastion direct connect must succeed without direct-tcpip");

    // 登录用户名必须渲染为 {user}#{account}#{asset}，而不是表单里的目标用户名。
    let usernames = seen_usernames.lock().expect("username log").clone();
    assert_eq!(
        usernames,
        vec!["hufei#root#0e85eca7-422a-4cdb-8e4c-8e3ff68eee39".to_owned()],
        "bastion login username must follow the JumpServer format"
    );

    // 会话即堡垒机：断开会话即断开堡垒机连接。
    client.disconnect().await.expect("disconnect");
}
