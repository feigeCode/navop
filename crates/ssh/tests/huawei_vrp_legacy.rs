//! 复刻华为 S5720（VRP）交换机的 SSH 服务端画像，验证 navop 客户端能否完成协商。
//!
//! 画像来源是客户提供的 SecureCRT 成功登录抓包（10.129.192.1）：
//! - 标识串 `SSH-2.0--`（软件版本为空）
//! - KEX 只有 `diffie-hellman-group-exchange-sha1` / `group14-sha1` / `group1-sha1`
//! - 主机密钥算法只有 `ssh-dss`、`ssh-rsa`（此处只复刻 `ssh-rsa`，即 SHA-1 签名）
//! - cipher `aes256-ctr`/`aes128-ctr`/CBC 系，MAC `hmac-sha2-256`/`hmac-sha1`
//! - 认证方式 password

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

use russh::keys::PrivateKey;
use russh::keys::ssh_key::Algorithm;
use russh::server::Server as _;
use ssh::{HostKeyVerifier, RusshClient, SshAuth, SshClient, SshConnectConfig};
use tokio::net::TcpListener;

const USERNAME: &str = "manager";
const PASSWORD: &str = "password";

#[derive(Clone)]
struct HuaweiVrpServer;

impl russh::server::Server for HuaweiVrpServer {
    type Handler = Self;

    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self::Handler {
        self.clone()
    }
}

impl russh::server::Handler for HuaweiVrpServer {
    type Error = anyhow::Error;

    async fn auth_password(
        &mut self,
        _: &str,
        _: &str,
    ) -> Result<russh::server::Auth, Self::Error> {
        Ok(russh::server::Auth::Accept)
    }
}

fn client_config(host: String, port: u16, allow_legacy_algorithms: bool) -> SshConnectConfig {
    SshConnectConfig {
        host,
        port,
        username: USERNAME.to_string(),
        auth: SshAuth::Password(PASSWORD.to_string()),
        timeout: Some(Duration::from_secs(30)),
        keepalive_interval: None,
        keepalive_max: None,
        jump_server: None,
        proxy: None,
        keyboard_interactive_responder: None,
        host_key_verifier: HostKeyVerifier::insecure(),
        x11_forwarding: false,
        allow_legacy_algorithms,
        forward_agent: false,
        agent_identities: Vec::new(),
    }
}

async fn spawn_huawei_vrp_server() -> (
    std::net::SocketAddr,
    tokio::task::JoinHandle<std::io::Result<()>>,
) {
    let socket = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("huawei vrp test server should bind");
    let address = socket
        .local_addr()
        .expect("huawei vrp test server should have an address");
    let rsa = PrivateKey::random(&mut rand_010::rng(), Algorithm::Rsa { hash: None })
        .expect("RSA host key should be generated");

    let server_config = Arc::new(russh::server::Config {
        server_id: russh::SshId::Standard(Cow::Borrowed("SSH-2.0--")),
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        keys: vec![rsa],
        preferred: russh::Preferred {
            kex: Cow::Owned(vec![
                russh::kex::DH_GEX_SHA1,
                russh::kex::DH_G14_SHA1,
                russh::kex::DH_G1_SHA1,
            ]),
            key: Cow::Owned(vec![Algorithm::Rsa { hash: None }]),
            cipher: Cow::Owned(vec![russh::cipher::AES_256_CTR, russh::cipher::AES_128_CTR]),
            mac: Cow::Owned(vec![russh::mac::HMAC_SHA256, russh::mac::HMAC_SHA1]),
            compression: Cow::Owned(vec![russh::compression::NONE]),
            ..russh::Preferred::default()
        },
        ..Default::default()
    });

    let server_task = tokio::spawn(async move {
        let mut server = HuaweiVrpServer;
        server.run_on_socket(server_config, &socket).await
    });
    (address, server_task)
}

#[tokio::test]
async fn client_connects_to_huawei_vrp_profile_with_legacy_algorithms() {
    let (address, server_task) = spawn_huawei_vrp_server().await;
    let config = client_config(address.ip().to_string(), address.port(), true);

    let result = RusshClient::connect(config).await;
    server_task.abort();

    result.expect("华为 VRP（SHA-1 KEX + ssh-rsa 主机密钥）应能在开启旧版算法后连上");
}

#[tokio::test]
async fn client_rejects_huawei_vrp_profile_without_legacy_algorithms() {
    let (address, server_task) = spawn_huawei_vrp_server().await;
    let config = client_config(address.ip().to_string(), address.port(), false);

    let result = RusshClient::connect(config).await;
    server_task.abort();

    let Err(error) = result else {
        panic!("未开启旧版算法时不应与只提供 SHA-1 KEX 的设备协商成功");
    };
    assert!(
        error.to_string().contains("No common Kex algorithm"),
        "预期无共同 KEX 的错误，实际: {error:?}"
    );
}
