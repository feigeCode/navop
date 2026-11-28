//! SFTP 目标服务器 keyboard-interactive 认证回归。
//!
//! 这里不实现完整 SFTP 子系统：认证成功后主动拒绝 `sftp` subsystem，
//! 只验证 `RusshSftpClient::connect` 在目标服务器阶段确实把配置里的
//! responder 交给了认证策略，而不是像普通跳板机分支那样忽略它。

use std::borrow::Cow;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use russh::MethodSet;
use russh::keys::{Algorithm, PrivateKey};
use russh::server::{Auth, Response, Server as _};
use sftp::{RusshSftpClient, SftpClient};
use ssh::{
    HostKeyVerifier, KeyboardInteractiveRequest, KeyboardInteractiveResponder, SshAuth,
    SshConnectConfig,
};
use tokio::net::TcpListener;

const USERNAME: &str = "tester";
const PASSWORD: &str = "saved-password";
const VERIFICATION_CODE: &str = "123456";

#[derive(Clone, Debug, PartialEq, Eq)]
enum AuthEvent {
    KeyboardInteractiveStart,
    KeyboardInteractiveAnswers(Vec<String>),
}

#[derive(Clone, Default)]
struct FakeMfaServer {
    events: Arc<Mutex<Vec<AuthEvent>>>,
}

impl FakeMfaServer {
    fn events(&self) -> Vec<AuthEvent> {
        self.events.lock().expect("events lock").clone()
    }
}

#[derive(Clone)]
struct FakeMfaHandler {
    events: Arc<Mutex<Vec<AuthEvent>>>,
}

impl FakeMfaHandler {
    fn record(&self, event: AuthEvent) {
        self.events.lock().expect("events lock").push(event);
    }
}

impl russh::server::Server for FakeMfaServer {
    type Handler = FakeMfaHandler;

    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self::Handler {
        FakeMfaHandler {
            events: self.events.clone(),
        }
    }
}

impl russh::server::Handler for FakeMfaHandler {
    type Error = anyhow::Error;

    async fn auth_none(&mut self, _: &str) -> Result<Auth, Self::Error> {
        Ok(Auth::reject())
    }

    async fn auth_password(&mut self, _: &str, _: &str) -> Result<Auth, Self::Error> {
        Ok(Auth::reject())
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        _: &str,
        _: &str,
        response: Option<Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        let Some(response) = response else {
            self.record(AuthEvent::KeyboardInteractiveStart);
            return Ok(Auth::Partial {
                name: "MFA".into(),
                instructions: "".into(),
                prompts: Cow::Owned(vec![(Cow::Borrowed("Verification code: "), true)]),
            });
        };

        let answers = response
            .map(|answer| String::from_utf8_lossy(&answer).to_string())
            .collect::<Vec<_>>();
        self.record(AuthEvent::KeyboardInteractiveAnswers(answers.clone()));
        if answers == [VERIFICATION_CODE] {
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn channel_open_session(
        &mut self,
        _channel: russh::Channel<russh::server::Msg>,
        reply: russh::server::ChannelOpenHandle,
        _session: &mut russh::server::Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel: russh::ChannelId,
        _name: &str,
        session: &mut russh::server::Session,
    ) -> Result<(), Self::Error> {
        session.channel_failure(channel)?;
        session.close(channel)?;
        Ok(())
    }
}

struct RecordingMfaResponder {
    requests: Mutex<Vec<KeyboardInteractiveRequest>>,
}

impl RecordingMfaResponder {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            requests: Mutex::new(Vec::new()),
        })
    }

    fn prompt_lines(&self) -> Vec<String> {
        self.requests
            .lock()
            .expect("requests lock")
            .iter()
            .flat_map(|request| request.prompts.iter().map(|prompt| prompt.prompt.clone()))
            .collect()
    }
}

#[async_trait]
impl KeyboardInteractiveResponder for RecordingMfaResponder {
    async fn respond(&self, request: KeyboardInteractiveRequest) -> anyhow::Result<Vec<String>> {
        self.requests.lock().expect("requests lock").push(request);
        Ok(vec![VERIFICATION_CODE.to_string()])
    }
}

fn connect_config(
    address: std::net::SocketAddr,
    responder: Arc<RecordingMfaResponder>,
) -> SshConnectConfig {
    SshConnectConfig {
        host: address.ip().to_string(),
        port: address.port(),
        username: USERNAME.to_string(),
        auth: SshAuth::Password(PASSWORD.to_string()),
        timeout: Some(Duration::from_secs(10)),
        keepalive_interval: None,
        keepalive_max: None,
        jump_server: None,
        proxy: None,
        keyboard_interactive_responder: Some(responder),
        host_key_verifier: HostKeyVerifier::insecure(),
        x11_forwarding: false,
        allow_legacy_algorithms: false,
        forward_agent: false,
        agent_identities: Vec::new(),
        proxy_command: None,
    }
}

#[tokio::test]
async fn sftp_target_uses_configured_keyboard_interactive_responder() {
    let socket = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("fake SFTP server should bind");
    let address = socket.local_addr().expect("server address");
    let server = FakeMfaServer::default();
    let server_config = Arc::new(russh::server::Config {
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        methods: MethodSet::from(
            &[
                russh::MethodKind::Password,
                russh::MethodKind::KeyboardInteractive,
            ][..],
        ),
        keys: vec![
            PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)
                .expect("test host key should be generated"),
        ],
        ..Default::default()
    });
    let mut running_server = server.clone();
    let server_task = tokio::spawn(async move {
        let _ = running_server.run_on_socket(server_config, &socket).await;
    });

    let responder = RecordingMfaResponder::new();
    let result = RusshSftpClient::connect(connect_config(address, responder.clone())).await;
    server_task.abort();

    assert_eq!(
        responder.prompt_lines(),
        vec!["Verification code: ".to_string()],
        "目标服务器下发的 keyboard-interactive 提示必须进入 responder；连接错误：{:?}",
        result.as_ref().err()
    );
    assert_eq!(
        server.events(),
        vec![
            AuthEvent::KeyboardInteractiveStart,
            AuthEvent::KeyboardInteractiveAnswers(vec![VERIFICATION_CODE.to_string()]),
        ],
        "responder 的应答必须提交给目标服务器"
    );
    assert!(
        result.is_err(),
        "测试服务器故意拒绝 sftp subsystem，认证之后应返回连接错误"
    );
}
