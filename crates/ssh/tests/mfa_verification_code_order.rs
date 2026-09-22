//! Issue #284 回归：PAM 栈要求「先验证码、后密码」时，客户端不能把保存的密码当成
//! 第一个应答提交给服务器。
//!
//! 假服务器复刻 OpenSSH + pam_google_authenticator 的真实行为：
//! - `password` 方法会把客户端提交的密码直接喂给 PAM 的第一个提示（验证码）。验证码被
//!   拒后该次认证的 PAM 事务作废，接下来的 keyboard-interactive 不再产生任何提示，
//!   于是客户端只会拿到一次 `auth_keyboard_interactive_failed`（issue #284 的现象）。
//! - 只有走 keyboard-interactive 时，客户端才会先看到「Verification code: 」提示，
//!   由用户输入验证码，服务器再要求密码，密码由保存的凭据在 password 提示处自动应答。

use std::borrow::Cow;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::MethodSet;
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
    /// 客户端探测服务器支持哪些认证方法（RFC 4252 的 "none" 探测）。
    NoneProbe,
    /// 客户端提交了密码。
    Password(String),
    /// 客户端发起 keyboard-interactive。
    KeyboardInteractiveStart,
    /// 客户端回答了 keyboard-interactive 的一轮提示。
    KeyboardInteractiveAnswers(Vec<String>),
}

/// 去掉方法探测事件：探测只是客户端用来判断服务器能力的手段，不属于认证动作本身。
fn auth_actions(events: &[AuthEvent]) -> Vec<AuthEvent> {
    events
        .iter()
        .filter(|event| !matches!(event, AuthEvent::NoneProbe))
        .cloned()
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServerMode {
    /// PAM 要求「先验证码后密码」，且 password 方法无法满足（issue #284 场景）。
    VerificationCodeFirst,
    /// 服务器不提供 keyboard-interactive，只能用密码。
    PasswordOnly,
    /// `AuthenticationMethods password,keyboard-interactive`：必须先密码、再验证码。
    PasswordThenVerificationCode,
}

#[derive(Clone)]
struct FakeMfaServer {
    mode: ServerMode,
    events: Arc<Mutex<Vec<AuthEvent>>>,
}

impl FakeMfaServer {
    fn new(mode: ServerMode) -> Self {
        Self {
            mode,
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn events(&self) -> Vec<AuthEvent> {
        self.events.lock().expect("events lock").clone()
    }

    fn advertised_methods(&self) -> MethodSet {
        match self.mode {
            ServerMode::PasswordOnly => MethodSet::from(&[russh::MethodKind::Password][..]),
            ServerMode::VerificationCodeFirst | ServerMode::PasswordThenVerificationCode => {
                MethodSet::from(
                    &[
                        russh::MethodKind::Password,
                        russh::MethodKind::KeyboardInteractive,
                    ][..],
                )
            }
        }
    }
}

#[derive(Clone)]
struct FakeMfaHandler {
    mode: ServerMode,
    events: Arc<Mutex<Vec<AuthEvent>>>,
    /// keyboard-interactive 轮次：0=未开始，1=已提示验证码，2=已提示密码。
    round: u8,
    /// password 方法把密码当成验证码提交后被服务器拒签，PAM 事务作废。
    poisoned: bool,
    /// 密码方法已经通过（partial_success），剩下的验证码只能由 keyboard-interactive 完成。
    password_accepted: bool,
}

impl FakeMfaHandler {
    fn record(&self, event: AuthEvent) {
        self.events.lock().expect("events lock").push(event);
    }

    fn verification_code_prompt() -> Auth {
        Auth::Partial {
            name: "MFA".into(),
            instructions: "".into(),
            prompts: Cow::Owned(vec![(Cow::Borrowed("Verification code: "), true)]),
        }
    }

    fn password_prompt() -> Auth {
        Auth::Partial {
            name: "MFA".into(),
            instructions: "".into(),
            prompts: Cow::Owned(vec![(Cow::Borrowed("Password: "), false)]),
        }
    }

    fn reject_with(methods: &[russh::MethodKind], partial_success: bool) -> Auth {
        Auth::Reject {
            proceed_with_methods: Some(MethodSet::from(methods)),
            partial_success,
        }
    }

    fn answer_keyboard_interactive(&mut self, response: Option<Response<'_>>) -> Auth {
        let Some(response) = response else {
            self.record(AuthEvent::KeyboardInteractiveStart);
            if self.poisoned {
                // PAM 事务已经被密码方法破坏，不再产生任何提示，客户端只会看到认证失败。
                return Self::reject_with(&[russh::MethodKind::Password], false);
            }
            if self.mode == ServerMode::PasswordThenVerificationCode && !self.password_accepted {
                return Self::reject_with(&[russh::MethodKind::Password], false);
            }
            self.round = 1;
            return Self::verification_code_prompt();
        };

        let answers: Vec<String> = response
            .map(|answer| String::from_utf8_lossy(&answer).to_string())
            .collect();
        self.record(AuthEvent::KeyboardInteractiveAnswers(answers.clone()));

        match self.round {
            1 if answers == [VERIFICATION_CODE] => {
                self.round = 2;
                if self.mode == ServerMode::VerificationCodeFirst {
                    Self::password_prompt()
                } else {
                    // 验证码已通过且密码此前已由 password 方法验证，认证完成。
                    Auth::Accept
                }
            }
            2 if answers == [SAVED_PASSWORD] => Auth::Accept,
            _ => Self::reject_with(&[russh::MethodKind::KeyboardInteractive], false),
        }
    }

    fn answer_password(&mut self, password: &str) -> Auth {
        self.record(AuthEvent::Password(password.to_string()));

        if self.mode == ServerMode::PasswordThenVerificationCode {
            if password == SAVED_PASSWORD && !self.password_accepted {
                self.password_accepted = true;
                // 部分成功：密码已过，服务器只接受用户实际输入的验证码。
                return Self::reject_with(&[russh::MethodKind::KeyboardInteractive], true);
            }
            return Self::reject_with(&[russh::MethodKind::KeyboardInteractive], false);
        }

        if password == SAVED_PASSWORD && self.mode == ServerMode::PasswordOnly {
            return Auth::Accept;
        }

        // 密码被送进了 PAM 的验证码提示，服务器拒签并作废本次 PAM 事务。
        self.poisoned = true;
        Self::reject_with(
            &[
                russh::MethodKind::Password,
                russh::MethodKind::KeyboardInteractive,
            ],
            false,
        )
    }
}

impl russh::server::Server for FakeMfaServer {
    type Handler = FakeMfaHandler;

    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self::Handler {
        FakeMfaHandler {
            mode: self.mode,
            events: self.events.clone(),
            round: 0,
            poisoned: false,
            password_accepted: false,
        }
    }
}

impl russh::server::Handler for FakeMfaHandler {
    type Error = anyhow::Error;

    async fn auth_none(&mut self, _: &str) -> Result<Auth, Self::Error> {
        self.record(AuthEvent::NoneProbe);
        Ok(Auth::reject())
    }

    async fn auth_password(&mut self, _: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(self.answer_password(password))
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

/// 复刻 `TerminalMfaResponder`：验证码类提示交给人输入，密码提示用保存的凭据自动应答。
struct RecordingMfaResponder {
    saved_password: String,
    requests: Mutex<Vec<KeyboardInteractiveRequest>>,
}

impl RecordingMfaResponder {
    fn new(saved_password: &str) -> Arc<Self> {
        Arc::new(Self {
            saved_password: saved_password.to_string(),
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
impl KeyboardInteractiveResponder for RecordingMfaResponder {
    async fn respond(&self, request: KeyboardInteractiveRequest) -> anyhow::Result<Vec<String>> {
        let answers = request
            .prompts
            .iter()
            .map(|prompt| {
                if prompt.prompt.to_ascii_lowercase().contains("password") {
                    self.saved_password.clone()
                } else {
                    VERIFICATION_CODE.to_string()
                }
            })
            .collect();
        self.requests.lock().expect("requests lock").push(request);
        Ok(answers)
    }
}

async fn spawn_server(
    server: FakeMfaServer,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let socket = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("fake MFA server should bind");
    let address = socket
        .local_addr()
        .expect("fake MFA server should have an address");
    let methods = server.advertised_methods();
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
    responder: Option<Arc<RecordingMfaResponder>>,
) -> SshConnectConfig {
    SshConnectConfig {
        host: address.ip().to_string(),
        port: address.port(),
        username: USERNAME.to_string(),
        auth: SshAuth::Password(SAVED_PASSWORD.to_string()),
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
    }
}

/// issue #284：服务器要求先验证码后密码时，保存的密码不能被当成第一个应答。
#[tokio::test]
async fn password_is_not_sent_before_verification_code_prompt() {
    let server = FakeMfaServer::new(ServerMode::VerificationCodeFirst);
    let (address, task) = spawn_server(server.clone()).await;
    let responder = RecordingMfaResponder::new(SAVED_PASSWORD);

    let result = RusshClient::connect(connect_config(address, Some(responder.clone()))).await;
    task.abort();

    let events = server.events();
    assert!(
        result.is_ok(),
        "先验证码后密码的服务器应能完成认证，实际错误：{:?}，服务器观察到的动作：{events:?}",
        result.err()
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, AuthEvent::Password(_))),
        "保存的密码不应该作为第一个应答发给服务器，实际动作：{events:?}"
    );
    assert_eq!(
        responder.prompt_lines(),
        vec!["Verification code: ".to_string(), "Password: ".to_string()],
        "客户端应先提示验证码，再提示密码"
    );
    assert_eq!(
        auth_actions(&events),
        vec![
            AuthEvent::KeyboardInteractiveStart,
            AuthEvent::KeyboardInteractiveAnswers(vec![VERIFICATION_CODE.to_string()]),
            AuthEvent::KeyboardInteractiveAnswers(vec![SAVED_PASSWORD.to_string()]),
        ],
        "验证码由用户输入，密码只在 password 提示处用保存的凭据应答"
    );
}

/// 服务器不提供 keyboard-interactive 时必须回退到密码认证。
#[tokio::test]
async fn password_only_server_still_authenticates_with_saved_password() {
    let server = FakeMfaServer::new(ServerMode::PasswordOnly);
    let (address, task) = spawn_server(server.clone()).await;
    let responder = RecordingMfaResponder::new(SAVED_PASSWORD);

    let result = RusshClient::connect(connect_config(address, Some(responder.clone()))).await;
    task.abort();

    let events = server.events();
    assert!(
        result.is_ok(),
        "只支持密码的服务器应能完成认证，实际错误：{:?}，服务器观察到的动作：{events:?}",
        result.err()
    );
    assert_eq!(
        auth_actions(&events),
        vec![AuthEvent::Password(SAVED_PASSWORD.to_string())],
        "没有 keyboard-interactive 时应直接走密码认证"
    );
    assert!(
        responder.prompt_lines().is_empty(),
        "没有 keyboard-interactive 时不应产生任何提示交互"
    );
}

/// `AuthenticationMethods password,keyboard-interactive`：必须先密码后验证码，仍要能连通。
#[tokio::test]
async fn password_then_verification_code_chain_authenticates() {
    let server = FakeMfaServer::new(ServerMode::PasswordThenVerificationCode);
    let (address, task) = spawn_server(server.clone()).await;
    let responder = RecordingMfaResponder::new(SAVED_PASSWORD);

    let result = RusshClient::connect(connect_config(address, Some(responder.clone()))).await;
    task.abort();

    let events = server.events();
    assert!(
        result.is_ok(),
        "先密码后验证码的链路应能完成认证，实际错误：{:?}，服务器观察到的动作：{events:?}",
        result.err()
    );
    assert_eq!(
        auth_actions(&events),
        vec![
            AuthEvent::KeyboardInteractiveStart,
            AuthEvent::Password(SAVED_PASSWORD.to_string()),
            AuthEvent::KeyboardInteractiveStart,
            AuthEvent::KeyboardInteractiveAnswers(vec![VERIFICATION_CODE.to_string()]),
        ],
        "服务器拒绝提前发起的 keyboard-interactive 后，应先补密码认证，再提示验证码"
    );
    assert_eq!(
        responder.prompt_lines(),
        vec!["Verification code: ".to_string()],
        "链路里密码由保存的凭据自动应答，只有验证码需要用户输入"
    );
}

/// 未启用 MFA 回调时保持原有的密码认证顺序（不做任何探测）。
#[tokio::test]
async fn without_responder_password_auth_keeps_original_order() {
    let server = FakeMfaServer::new(ServerMode::PasswordOnly);
    let (address, task) = spawn_server(server.clone()).await;

    let result = RusshClient::connect(connect_config(address, None)).await;
    task.abort();

    assert!(result.is_ok(), "密码认证应成功：{:?}", result.err());
    assert_eq!(
        server.events(),
        vec![AuthEvent::Password(SAVED_PASSWORD.to_string())],
        "没有 MFA 回调时不应出现额外的探测或 keyboard-interactive 请求"
    );
}
