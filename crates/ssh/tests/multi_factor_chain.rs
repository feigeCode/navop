//! 多因素认证链回归：防火墙、交换机等设备会要求 `AuthenticationMethods password,publickey`
//! （或反过来），一次登录必须依次通过多个认证因素。
//!
//! 假服务器复刻设备侧的三种常见形态：
//! - 先密码后公钥 / 先公钥后密码：服务器在拒绝时给出还没满足的方法。
//! - 公钥被拒但服务器仍把公钥列在方法表里（真实设备常见的「组合要求未满足」形态），
//!   客户端必须在密码通过后回头再试公钥。
//! - 两个因素之后还要求动态验证码（keyboard-interactive）。

use std::borrow::Cow;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::MethodSet;
use russh::keys::ssh_key::LineEnding;
use russh::keys::{Algorithm, PrivateKey};
use russh::server::{Auth, Response, Server as _};
use ssh::{
    HostKeyVerifier, KeyboardInteractiveRequest, KeyboardInteractiveResponder, RusshClient,
    SshAuth, SshClient, SshConnectConfig,
};
use tokio::net::TcpListener;

const SAVED_PASSWORD: &str = "saved-password";
const VERIFICATION_CODE: &str = "123456";
const USERNAME: &str = "tester";

/// 服务器观察到的客户端认证动作，按发生顺序记录。
#[derive(Debug, Clone, PartialEq, Eq)]
enum AuthEvent {
    /// RFC 4252 的 "none" 方法探测，不算认证动作。
    NoneProbe,
    Password(String),
    /// 客户端提交了带签名的公钥认证请求。
    PublicKey,
    KeyboardInteractiveStart,
    KeyboardInteractiveAnswers(Vec<String>),
}

/// 去掉探测事件，只看真实的认证动作。
fn auth_actions(events: &[AuthEvent]) -> Vec<AuthEvent> {
    events
        .iter()
        .filter(|event| !matches!(event, AuthEvent::NoneProbe))
        .cloned()
        .collect()
}

fn count_passwords(events: &[AuthEvent]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, AuthEvent::Password(_)))
        .count()
}

/// 服务器要求的认证阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Password,
    PublicKey,
    VerificationCode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChainMode {
    /// 服务器只要求密码。
    PasswordOnly,
    /// `AuthenticationMethods password,publickey`
    PasswordThenPublicKey,
    /// `AuthenticationMethods publickey,password`
    PublicKeyThenPassword,
    /// 要求先密码后公钥，但方法表里把公钥列在前面，且拒绝公钥时仍列出公钥。
    MessyPublicKeyListedWhilePasswordPending,
    /// `AuthenticationMethods password,publickey,keyboard-interactive`
    PasswordPublicKeyThenVerificationCode,
}

impl ChainMode {
    /// 服务器实际要求的阶段，按顺序排列。
    fn required_stages(self) -> Vec<Stage> {
        match self {
            ChainMode::PasswordOnly => vec![Stage::Password],
            ChainMode::PasswordThenPublicKey
            | ChainMode::MessyPublicKeyListedWhilePasswordPending => {
                vec![Stage::Password, Stage::PublicKey]
            }
            ChainMode::PublicKeyThenPassword => vec![Stage::PublicKey, Stage::Password],
            ChainMode::PasswordPublicKeyThenVerificationCode => {
                vec![Stage::Password, Stage::PublicKey, Stage::VerificationCode]
            }
        }
    }

    /// 服务器在方法表里公布的方法列表。
    fn advertised_methods(self) -> MethodSet {
        let kinds: Vec<russh::MethodKind> = match self {
            ChainMode::PasswordOnly => vec![russh::MethodKind::Password],
            ChainMode::PasswordThenPublicKey => {
                vec![russh::MethodKind::Password, russh::MethodKind::PublicKey]
            }
            ChainMode::PublicKeyThenPassword
            | ChainMode::MessyPublicKeyListedWhilePasswordPending => {
                vec![russh::MethodKind::PublicKey, russh::MethodKind::Password]
            }
            ChainMode::PasswordPublicKeyThenVerificationCode => vec![
                russh::MethodKind::Password,
                russh::MethodKind::PublicKey,
                russh::MethodKind::KeyboardInteractive,
            ],
        };
        MethodSet::from(&kinds[..])
    }
}

fn stage_method_kind(stage: Stage) -> russh::MethodKind {
    match stage {
        Stage::Password => russh::MethodKind::Password,
        Stage::PublicKey => russh::MethodKind::PublicKey,
        Stage::VerificationCode => russh::MethodKind::KeyboardInteractive,
    }
}

#[derive(Clone)]
struct FakeChainServer {
    mode: ChainMode,
    public_key: russh::keys::PublicKey,
    events: Arc<Mutex<Vec<AuthEvent>>>,
}

impl FakeChainServer {
    fn new(mode: ChainMode, public_key: russh::keys::PublicKey) -> Self {
        Self {
            mode,
            public_key,
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn events(&self) -> Vec<AuthEvent> {
        self.events.lock().expect("events lock").clone()
    }
}

#[derive(Clone)]
struct FakeChainHandler {
    mode: ChainMode,
    public_key: russh::keys::PublicKey,
    events: Arc<Mutex<Vec<AuthEvent>>>,
    /// 已完成的认证阶段。
    completed: Vec<Stage>,
}

impl FakeChainHandler {
    fn record(&self, event: AuthEvent) {
        self.events.lock().expect("events lock").push(event);
    }

    fn next_stage(&self) -> Option<Stage> {
        self.mode
            .required_stages()
            .into_iter()
            .find(|stage| !self.completed.contains(stage))
    }

    /// 还没满足的阶段对应的方法列表，按服务器要求的顺序给出。
    fn remaining_methods(&self) -> Vec<russh::MethodKind> {
        self.mode
            .required_stages()
            .into_iter()
            .filter(|stage| !self.completed.contains(stage))
            .map(stage_method_kind)
            .collect()
    }

    fn reject_with(methods: &[russh::MethodKind], partial_success: bool) -> Auth {
        Auth::Reject {
            proceed_with_methods: Some(MethodSet::from(methods)),
            partial_success,
        }
    }

    /// 服务器公布的方法表，用于「要求的阶段还没轮到这个方法」时的拒绝。
    fn reject_listing_advertised(&self) -> Auth {
        Self::reject_with(&self.mode.advertised_methods(), false)
    }

    /// 当前阶段通过：还有后续阶段时给部分成功，否则完成认证。
    fn accept_stage(&mut self, stage: Stage) -> Auth {
        self.completed.push(stage);
        match self.next_stage() {
            None => Auth::Accept,
            Some(_) => {
                let remaining = self.remaining_methods();
                Self::reject_with(&remaining, true)
            }
        }
    }

    fn answer_password(&mut self, password: &str) -> Auth {
        self.record(AuthEvent::Password(password.to_string()));

        if self.next_stage() != Some(Stage::Password) {
            // 还没轮到密码（例如设备要求先公钥），按公布的方法表拒绝。
            return self.reject_listing_advertised();
        }
        if password != SAVED_PASSWORD {
            let remaining = self.remaining_methods();
            return Self::reject_with(&remaining, false);
        }
        self.accept_stage(Stage::Password)
    }

    fn answer_public_key(&mut self, public_key: &russh::keys::PublicKey) -> Auth {
        self.record(AuthEvent::PublicKey);

        if public_key != &self.public_key {
            let remaining = self.remaining_methods();
            return Self::reject_with(&remaining, false);
        }
        if self.next_stage() != Some(Stage::PublicKey) {
            return self.reject_listing_advertised();
        }
        self.accept_stage(Stage::PublicKey)
    }

    fn answer_keyboard_interactive(&mut self, response: Option<Response<'_>>) -> Auth {
        match response {
            None => {
                self.record(AuthEvent::KeyboardInteractiveStart);
                if self.next_stage() != Some(Stage::VerificationCode) {
                    let remaining = self.remaining_methods();
                    return Self::reject_with(&remaining, false);
                }
                Auth::Partial {
                    name: "MFA".into(),
                    instructions: "".into(),
                    prompts: Cow::Owned(vec![(Cow::Borrowed("Verification code: "), true)]),
                }
            }
            Some(response) => {
                let answers: Vec<String> = response
                    .map(|answer| String::from_utf8_lossy(&answer).to_string())
                    .collect();
                self.record(AuthEvent::KeyboardInteractiveAnswers(answers.clone()));
                if self.next_stage() == Some(Stage::VerificationCode)
                    && answers == [VERIFICATION_CODE]
                {
                    return self.accept_stage(Stage::VerificationCode);
                }
                let remaining = self.remaining_methods();
                Self::reject_with(&remaining, false)
            }
        }
    }
}

impl russh::server::Server for FakeChainServer {
    type Handler = FakeChainHandler;

    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self::Handler {
        FakeChainHandler {
            mode: self.mode,
            public_key: self.public_key.clone(),
            events: self.events.clone(),
            completed: Vec::new(),
        }
    }
}

impl russh::server::Handler for FakeChainHandler {
    type Error = anyhow::Error;

    async fn auth_none(&mut self, _: &str) -> Result<Auth, Self::Error> {
        self.record(AuthEvent::NoneProbe);
        Ok(Self::reject_with(&self.mode.advertised_methods(), false))
    }

    async fn auth_password(&mut self, _: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(self.answer_password(password))
    }

    async fn auth_publickey(
        &mut self,
        _: &str,
        public_key: &russh::keys::PublicKey,
    ) -> Result<Auth, Self::Error> {
        Ok(self.answer_public_key(public_key))
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        _: &str,
        _: &str,
        response: Option<Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        Ok(self.answer_keyboard_interactive(response))
    }
}

/// 只回答动态验证码的 MFA 回调；密码由认证链自己提交。
struct VerificationCodeResponder {
    requests: Mutex<Vec<KeyboardInteractiveRequest>>,
}

impl VerificationCodeResponder {
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
            .map(|request| {
                request
                    .prompts
                    .iter()
                    .map(|prompt| prompt.prompt.clone())
                    .collect::<Vec<_>>()
                    .join("|")
            })
            .collect()
    }
}

#[async_trait::async_trait]
impl KeyboardInteractiveResponder for VerificationCodeResponder {
    async fn respond(&self, request: KeyboardInteractiveRequest) -> anyhow::Result<Vec<String>> {
        let answers = request
            .prompts
            .iter()
            .map(|_| VERIFICATION_CODE.to_string())
            .collect();
        self.requests.lock().expect("requests lock").push(request);
        Ok(answers)
    }
}

/// 生成一对测试密钥，返回（客户端私钥 PEM，服务器侧公钥）。
fn test_key_pair() -> (String, russh::keys::PublicKey) {
    let private_key = PrivateKey::random(&mut rand_010::rng(), Algorithm::Ed25519)
        .expect("test client key should be generated");
    let pem = private_key
        .to_openssh(LineEnding::LF)
        .expect("test client key should encode as OpenSSH PEM")
        .to_string();
    (pem, private_key.public_key().clone())
}

async fn spawn_server(
    server: FakeChainServer,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let socket = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("fake chain server should bind");
    let address = socket
        .local_addr()
        .expect("fake chain server should have an address");
    let methods = server.mode.advertised_methods();
    let server_config = Arc::new(russh::server::Config {
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        methods,
        keys: vec![
            PrivateKey::random(&mut rand_010::rng(), Algorithm::Ed25519)
                .expect("test host key should be generated"),
        ],
        ..Default::default()
    });

    let task = tokio::spawn(async move {
        let mut server = server;
        let _ = server.run_on_socket(server_config, &socket).await;
    });

    (address, task)
}

fn connect_config(
    address: std::net::SocketAddr,
    auth: SshAuth,
    responder: Option<Arc<VerificationCodeResponder>>,
) -> SshConnectConfig {
    SshConnectConfig {
        host: address.ip().to_string(),
        port: address.port(),
        username: USERNAME.to_string(),
        auth,
        timeout: Some(Duration::from_secs(30)),
        keepalive_interval: None,
        keepalive_max: None,
        jump_server: None,
        proxy: None,
        keyboard_interactive_responder: responder
            .map(|responder| responder as Arc<dyn KeyboardInteractiveResponder>),
        host_key_verifier: HostKeyVerifier::insecure(),
        x11_forwarding: false,
        allow_legacy_algorithms: false,
        forward_agent: false,
        agent_identities: Vec::new(),
    }
}

/// 「密码 + 密钥」组合：设备要求先密码后公钥时能完成认证。
#[tokio::test]
async fn password_then_public_key_chain_authenticates() {
    let (private_key, public_key) = test_key_pair();
    let server = FakeChainServer::new(ChainMode::PasswordThenPublicKey, public_key);
    let (address, task) = spawn_server(server.clone()).await;

    let result = RusshClient::connect(connect_config(
        address,
        SshAuth::Chain(vec![
            SshAuth::Password(SAVED_PASSWORD.to_string()),
            SshAuth::PrivateKeyContent {
                private_key,
                passphrase: None,
                certificate_path: None,
            },
        ]),
        None,
    ))
    .await;
    task.abort();

    let events = server.events();
    assert!(
        result.is_ok(),
        "先密码后公钥的设备应能完成认证，实际错误：{:?}，服务器观察到的动作：{events:?}",
        result.err()
    );
    assert_eq!(
        auth_actions(&events),
        vec![
            AuthEvent::Password(SAVED_PASSWORD.to_string()),
            AuthEvent::PublicKey,
        ],
        "配置顺序与设备要求一致时应依次提交密码与公钥"
    );
}

/// 「密码 + 密钥」组合：设备要求先公钥后密码时，客户端要按服务器给出的顺序提交。
#[tokio::test]
async fn public_key_then_password_chain_authenticates() {
    let (private_key, public_key) = test_key_pair();
    let server = FakeChainServer::new(ChainMode::PublicKeyThenPassword, public_key);
    let (address, task) = spawn_server(server.clone()).await;

    let result = RusshClient::connect(connect_config(
        address,
        SshAuth::Chain(vec![
            SshAuth::Password(SAVED_PASSWORD.to_string()),
            SshAuth::PrivateKeyContent {
                private_key,
                passphrase: None,
                certificate_path: None,
            },
        ]),
        None,
    ))
    .await;
    task.abort();

    let events = server.events();
    assert!(
        result.is_ok(),
        "先公钥后密码的设备应能完成认证，实际错误：{:?}，服务器观察到的动作：{events:?}",
        result.err()
    );
    assert_eq!(
        auth_actions(&events),
        vec![
            AuthEvent::PublicKey,
            AuthEvent::Password(SAVED_PASSWORD.to_string()),
        ],
        "提交顺序应跟随服务器的 remaining_methods，而不是配置顺序"
    );
}

/// 设备在密码通过前拒绝公钥、却仍把公钥列在方法表里时，客户端要在密码通过后回头再试公钥。
#[tokio::test]
async fn public_key_is_retried_after_password_succeeds() {
    let (private_key, public_key) = test_key_pair();
    let server = FakeChainServer::new(
        ChainMode::MessyPublicKeyListedWhilePasswordPending,
        public_key,
    );
    let (address, task) = spawn_server(server.clone()).await;

    let result = RusshClient::connect(connect_config(
        address,
        SshAuth::Chain(vec![
            SshAuth::Password(SAVED_PASSWORD.to_string()),
            SshAuth::PrivateKeyContent {
                private_key,
                passphrase: None,
                certificate_path: None,
            },
        ]),
        None,
    ))
    .await;
    task.abort();

    let events = server.events();
    assert!(
        result.is_ok(),
        "方法表顺序与真实要求不一致时仍应完成认证，实际错误：{:?}，服务器观察到的动作：{events:?}",
        result.err()
    );
    assert_eq!(
        auth_actions(&events),
        vec![
            AuthEvent::PublicKey,
            AuthEvent::Password(SAVED_PASSWORD.to_string()),
            AuthEvent::PublicKey,
        ],
        "公钥被拒后应先补密码，密码通过后再回头提交公钥"
    );
}

/// 密码错误时每个因素只提交一次，不反复试错触发设备的失败次数限制。
#[tokio::test]
async fn wrong_password_does_not_repeat_attempts() {
    let (private_key, public_key) = test_key_pair();
    let server = FakeChainServer::new(ChainMode::PasswordThenPublicKey, public_key);
    let (address, task) = spawn_server(server.clone()).await;

    let result = RusshClient::connect(connect_config(
        address,
        SshAuth::Chain(vec![
            SshAuth::Password("wrong-password".to_string()),
            SshAuth::PrivateKeyContent {
                private_key,
                passphrase: None,
                certificate_path: None,
            },
        ]),
        None,
    ))
    .await;
    task.abort();

    let events = server.events();
    assert!(result.is_err(), "凭据错误时必须报认证失败");
    assert_eq!(
        count_passwords(&events),
        1,
        "密码被拒后不应重复提交同一个因素，实际动作：{events:?}"
    );
    assert_eq!(
        auth_actions(&events),
        vec![
            AuthEvent::Password("wrong-password".to_string()),
            AuthEvent::PublicKey,
        ],
        "每个因素在一次无进展期间只提交一次，实际动作：{events:?}"
    );
}

/// 组合认证之后的动态验证码仍由 MFA 回调完成。
#[tokio::test]
async fn chain_then_verification_code_authenticates() {
    let (private_key, public_key) = test_key_pair();
    let server = FakeChainServer::new(ChainMode::PasswordPublicKeyThenVerificationCode, public_key);
    let (address, task) = spawn_server(server.clone()).await;
    let responder = VerificationCodeResponder::new();

    let result = RusshClient::connect(connect_config(
        address,
        SshAuth::Chain(vec![
            SshAuth::Password(SAVED_PASSWORD.to_string()),
            SshAuth::PrivateKeyContent {
                private_key,
                passphrase: None,
                certificate_path: None,
            },
        ]),
        Some(responder.clone()),
    ))
    .await;
    task.abort();

    let events = server.events();
    assert!(
        result.is_ok(),
        "密码 + 密钥 + 验证码的链路应能完成认证，实际错误：{:?}，服务器观察到的动作：{events:?}",
        result.err()
    );
    assert_eq!(
        auth_actions(&events),
        vec![
            AuthEvent::Password(SAVED_PASSWORD.to_string()),
            AuthEvent::PublicKey,
            AuthEvent::KeyboardInteractiveStart,
            AuthEvent::KeyboardInteractiveAnswers(vec![VERIFICATION_CODE.to_string()]),
        ],
        "两个静态因素之后应继续用 MFA 回调完成动态验证码"
    );
    assert_eq!(
        responder.prompt_lines(),
        vec!["Verification code: ".to_string()],
        "验证码提示应交给用户，密码由认证链自己提交"
    );
}

/// 链里只剩一个因素时退化为普通认证，不做任何探测。
#[tokio::test]
async fn single_factor_chain_delegates_to_regular_authentication() {
    let (_, public_key) = test_key_pair();
    let server = FakeChainServer::new(ChainMode::PasswordOnly, public_key);
    let (address, task) = spawn_server(server.clone()).await;

    let result = RusshClient::connect(connect_config(
        address,
        SshAuth::Chain(vec![SshAuth::Password(SAVED_PASSWORD.to_string())]),
        None,
    ))
    .await;
    task.abort();

    assert!(result.is_ok(), "单因素链应直接完成认证：{:?}", result.err());
    assert_eq!(
        server.events(),
        vec![AuthEvent::Password(SAVED_PASSWORD.to_string())],
        "单因素链不应产生额外的探测请求"
    );
}
