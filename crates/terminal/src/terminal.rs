//! Terminal 模型层
//!
//! 独立的 Terminal Entity，负责：
//! - PTY/SSH 后端通信
//! - 终端状态管理（Term grid、选择、滚动）
//! - 事件发送（Title、Bell、ChildExit 等）
//!
//! 与 TerminalView 分离，TerminalView 只负责视图逻辑。

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point as AlacPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config as TermConfig, Term, TermMode};
use alacritty_terminal::tty::{self, Options as PtyOptions};
use alacritty_terminal::vte::ansi::{Color, Processor, StdSyncHandler};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use futures::StreamExt;
use gpui::*;
use one_core::app_dirs;
use one_core::gpui_tokio::Tokio;
use one_core::settings::AppSettings;
use one_core::storage::models::{
    ActiveConnections, ProxyType as StorageProxyType, SerialParams, SshAccountExpect,
    SshAuthMethod, StoredConnection, TelnetParams,
};
use one_core::storage::{
    GlobalStorageState, TerminalCommandHistoryRepository, TerminalCommandHistorySort,
    TerminalHistoryScope,
};
use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;
use tokio::time::interval;
use uuid::Uuid;

#[cfg(any(test, target_os = "windows"))]
use std::ffi::OsStr;
#[cfg(any(test, target_os = "windows"))]
use std::path::Path;

use crate::encoding::TerminalEncoding;
use crate::history::{
    HistoryEntry, PERSISTED_HISTORY_LIMIT, SESSION_HISTORY_LIMIT, ShellHistoryFormat,
    collect_history_search_results, collect_history_suggestions_with_cwd, collect_recent_history,
    normalize_recorded_command, parse_shell_history, push_rich_history_entry,
};
use crate::pty_backend::{GpuiEventProxy, LocalPtyBackend};
use crate::recording::{
    RecordingArtifactKind, RecordingBackend, RecordingCompleteness, RecordingMetadata,
    RecordingPlayback, RecordingPlaybackError, RecordingPlaybackSearchIndexStatus,
    RecordingPlaybackSearchResults, RecordingPlaybackState, RecordingPlaybackTransition,
    RecordingRuntime, RecordingRuntimeConfig, RecordingRuntimeError, RecordingSessionMetadata,
    RecordingSnapshot, RecordingStartRequest, RecordingTap, RecordingTransition,
    TerminalPlaybackRuntime,
};
use crate::session_logging::{
    AutomaticSessionLogRequestInput, application_version, build_automatic_session_log_request,
    local_recording_session_metadata, output_only_recording_config,
    serial_recording_session_metadata, ssh_recording_session_metadata,
    telnet_recording_session_metadata,
};
#[cfg(not(target_os = "windows"))]
use crate::shell_integration::embedded_shell_integration_script;
use crate::ssh_backend::SshBackendConnect;
#[cfg(target_os = "windows")]
use crate::windows_environment::{
    environment_value, merge_environment_overrides, refreshed_windows_environment,
};
use crate::zmodem::{
    ZmodemPickerClaim, ZmodemPickerRequest, ZmodemPickerResponse, ZmodemResponder,
    ZmodemTransferId, ZmodemTransferOutcome, ZmodemTransferProgress,
};

use crate::{
    LocalConfig, SerialBackend, SshBackend, TelnetBackend, TerminalBackend, TerminalControlHandle,
    TerminalEvent, TerminalExecHandle, TerminalInputHandle, TerminalPerformanceMetrics,
    TerminalPerformanceSnapshot, TerminalPerformanceWindow, TerminalSize,
    TerminalTransferCancelHandle,
};
use ssh::{
    ChannelEvent, HostKeyDetails, HostKeyIdentity, HostKeyRejection, HostKeyVerifier,
    KeyboardInteractiveRequest, KeyboardInteractiveResponder, KeyboardInteractiveTarget,
    SshChannel, SshSessionManager,
};
pub use ssh::{
    JumpServerConnectConfig, ProxyConnectConfig, ProxyType, PtyConfig, SshAuth, SshConnectConfig,
};

/// Terminal 发出的事件，供 TerminalView 订阅
#[derive(Debug, Clone)]
pub enum TerminalModelEvent {
    /// 终端内容已更新，需要重新渲染
    Wakeup,
    /// SSH 服务端主机指纹需要用户确认
    HostKeyVerificationRequired,
    /// SSH 临时用户名/密码请求状态变化
    SshCredentialChanged,
    /// Telnet 临时用户名/密码请求状态变化
    TelnetCredentialChanged,
    /// SSH keyboard-interactive/MFA 请求状态变化
    SshMfaChanged,
    /// SSH ZMODEM 文件选择请求状态变化
    ZmodemRequestChanged,
    /// SSH ZMODEM 文件传输进度变化
    ZmodemProgressChanged(ZmodemTransferProgress),
    /// SSH ZMODEM 文件传输结束
    ZmodemTransferFinished {
        transfer_id: ZmodemTransferId,
        outcome: ZmodemTransferOutcome,
        progress: Option<ZmodemTransferProgress>,
    },
    /// shell 开始渲染新的 prompt（OSC 133;A）
    PromptStart,
    /// shell prompt 已渲染完成，用户可以输入（OSC 133;B）
    InputStart,
    /// shell 命令开始执行（OSC 133;C）
    CommandStart,
    /// 终端标题已更改
    TitleChanged(String),
    /// 终端响铃
    Bell,
    /// 子进程已退出
    ChildExit(i32),
    /// 成功命令历史已写入数据库
    CommandHistoryChanged,
    /// 终端程序请求存储到剪贴板
    ClipboardStore(String),
    /// 远程工作目录变更（OSC 7）
    WorkingDirChanged(String),
    /// 会话锁定/解锁状态变化
    LockStateChanged,
}

/// 终端连接状态
#[derive(Clone, PartialEq, Debug)]
pub enum ConnectionState {
    Connected,
    Connecting,
    Disconnected { error: Option<String> },
}

fn should_install_connected_backend(state: &ConnectionState) -> bool {
    !matches!(state, ConnectionState::Disconnected { .. })
}

/// 会话锁定状态（仅内存中，不持久化）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionLockState {
    pub password_hash: String,
    pub hide_output: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostKeyVerificationRequest {
    pub identity: HostKeyIdentity,
    pub presented: HostKeyDetails,
    pub reason: HostKeyVerificationReason,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostKeyVerificationReason {
    Unknown,
    Changed { expected: Vec<HostKeyDetails> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostKeyVerificationDecision {
    AcceptAndSave,
    AcceptOnce,
    Reject,
}

/// 终端连接类型
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TerminalConnectionKind {
    Local,
    Ssh,
    Serial,
    Telnet,
}

/// Capability mode for a terminal surface.
///
/// A stored artifact may describe output originally produced by SSH, but
/// rendering it must never recreate the source session's live capabilities.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TerminalSessionMode {
    Live,
    RecordingPlayback,
    SessionLog,
}

const SSH_CLEAR_SCREEN_REDRAW_BYTES: &[u8] = b"\x0c";

fn clear_screen_remote_redraw_bytes(kind: TerminalConnectionKind) -> Option<&'static [u8]> {
    match kind {
        TerminalConnectionKind::Ssh => Some(SSH_CLEAR_SCREEN_REDRAW_BYTES),
        TerminalConnectionKind::Local
        | TerminalConnectionKind::Serial
        | TerminalConnectionKind::Telnet => None,
    }
}

#[derive(Default)]
struct CommandRecordGate {
    command_started: bool,
    exit_code: Option<i32>,
    recorded_command: Option<String>,
}

impl CommandRecordGate {
    fn command_started(&mut self) {
        self.command_started = true;
        self.exit_code = None;
        self.recorded_command = None;
    }

    fn command_finished(&mut self, exit_code: i32) -> Option<(String, i32)> {
        if !self.command_started {
            self.clear();
            return None;
        }
        self.exit_code = Some(exit_code);
        if exit_code != 0 {
            self.clear();
            return None;
        }
        self.recorded_command
            .take()
            .map(|command| self.accept(command, exit_code))
    }

    fn command_recorded(&mut self, command: String) -> Option<(String, i32)> {
        if !self.command_started {
            return None;
        }
        match self.exit_code {
            Some(0) => Some(self.accept(command, 0)),
            Some(_) => {
                self.clear();
                None
            }
            None => {
                self.recorded_command = Some(command);
                None
            }
        }
    }

    fn accept(&mut self, command: String, exit_code: i32) -> (String, i32) {
        self.clear();
        (command, exit_code)
    }

    fn clear(&mut self) {
        self.command_started = false;
        self.exit_code = None;
        self.recorded_command = None;
    }
}

/// SSH 终端配置
#[derive(Clone)]
pub struct SshTerminalConfig {
    pub ssh_config: SshConnectConfig,
    pub pty_config: PtyConfig,
    pub terminal_encoding: TerminalEncoding,
    pub account_expect: SshAccountExpect,
    /// 关闭 shell integration 注入:走裸 request_shell,失去 OSC 集成。
    pub disable_shell_integration: bool,
}

pub struct SshConnectionUpdate {
    pub connection: StoredConnection,
    pub working_dir: Option<String>,
    pub sync_path_with_terminal: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct SshCredentialPromptPolicy {
    username: bool,
    password: bool,
}

impl SshCredentialPromptPolicy {
    fn requires_credentials(self) -> bool {
        self.username || self.password
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSshCredentialRequest {
    generation: u64,
    pub username: bool,
    pub password: bool,
}

impl TerminalSshCredentialRequest {
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalSshCredentials {
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalTelnetCredentialRequest {
    generation: u64,
    pub username: bool,
    pub password: bool,
}

impl TerminalTelnetCredentialRequest {
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalTelnetCredentials {
    pub username: Option<String>,
    pub password: Option<String>,
}

struct ResolvedSshConnection {
    config: SshTerminalConfig,
    credential_prompt_policy: SshCredentialPromptPolicy,
    keyboard_interactive_enabled: bool,
    init_commands: Option<String>,
    connection_id: Option<i64>,
    connection_name: String,
}

struct SshConnectTask {
    session_manager: Arc<SshSessionManager>,
    config: SshTerminalConfig,
    term: Arc<FairMutex<Term<GpuiEventProxy>>>,
    event_proxy: GpuiEventProxy,
    event_tx: UnboundedSender<TerminalEvent>,
    zmodem_responder: ZmodemResponder,
    connection_id: Option<i64>,
    on_disconnect: Option<tokio::sync::oneshot::Sender<Option<String>>>,
    init_commands: Option<String>,
    recording_tap: Option<RecordingTap>,
    generation: u64,
}

fn ssh_auth_from_storage(auth: SshAuthMethod) -> SshAuth {
    match auth {
        SshAuthMethod::Password { password } => SshAuth::Password(password),
        SshAuthMethod::PrivateKey {
            key_path,
            passphrase,
        } => SshAuth::PrivateKey {
            key_path,
            passphrase,
            certificate_path: None,
        },
        SshAuthMethod::PrivateKeyContent {
            private_key,
            passphrase,
        } => SshAuth::PrivateKeyContent {
            private_key,
            passphrase,
            certificate_path: None,
        },
        SshAuthMethod::Agent => SshAuth::Agent,
        SshAuthMethod::Pageant => SshAuth::Pageant,
        SshAuthMethod::AutoPublicKey => SshAuth::AutoPublicKey,
    }
}

fn password_from_ssh_auth(auth: &SshAuth) -> Option<String> {
    match auth {
        SshAuth::Password(password) => Some(password.clone()),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalMfaPrompt {
    pub prompt: String,
    pub echo: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalMfaRequest {
    pub name: String,
    pub instructions: String,
    pub prompts: Vec<TerminalMfaPrompt>,
}

#[derive(Clone, Default)]
pub struct TerminalMfaResponder {
    state: Arc<StdMutex<TerminalMfaState>>,
    event_tx: Option<UnboundedSender<TerminalEvent>>,
    jump_password: Option<String>,
    target_password: Option<String>,
}

#[derive(Default)]
struct TerminalMfaState {
    pending: Option<TerminalMfaPending>,
}

struct TerminalMfaPending {
    request: TerminalMfaRequest,
    response_tx: Option<oneshot::Sender<Vec<String>>>,
}

impl TerminalMfaResponder {
    pub fn new(
        event_tx: UnboundedSender<TerminalEvent>,
        jump_password: Option<String>,
        target_password: Option<String>,
    ) -> Self {
        Self {
            state: Arc::new(StdMutex::new(TerminalMfaState::default())),
            event_tx: Some(event_tx.clone()),
            jump_password,
            target_password,
        }
    }

    pub fn pending_request(&self) -> Option<TerminalMfaRequest> {
        self.state
            .lock()
            .ok()?
            .pending
            .as_ref()
            .map(|pending| pending.request.clone())
    }

    pub fn submit(&self, responses: Vec<String>) -> bool {
        let Some(mut pending) = self
            .state
            .lock()
            .ok()
            .and_then(|mut state| state.pending.take())
        else {
            return false;
        };

        let sent = pending
            .response_tx
            .take()
            .is_some_and(|tx| tx.send(responses).is_ok());
        self.notify_changed();
        sent
    }

    pub fn cancel(&self) -> bool {
        let cleared = self
            .state
            .lock()
            .ok()
            .and_then(|mut state| state.pending.take())
            .is_some();
        if cleared {
            self.notify_changed();
        }
        cleared
    }

    fn notify_changed(&self) {
        if let Some(event_tx) = &self.event_tx {
            let _ = event_tx.send(TerminalEvent::SshMfaChanged);
        }
    }
}

#[async_trait]
impl KeyboardInteractiveResponder for TerminalMfaResponder {
    async fn respond(&self, request: KeyboardInteractiveRequest) -> Result<Vec<String>> {
        let terminal_prompts = request
            .prompts
            .iter()
            .filter(|prompt| !is_ssh_password_prompt(&prompt.prompt))
            .map(|prompt| TerminalMfaPrompt {
                prompt: prompt.prompt.clone(),
                echo: prompt.echo,
            })
            .collect::<Vec<_>>();

        if terminal_prompts.is_empty() {
            return keyboard_interactive_answers_for_terminal(
                &request,
                &[],
                self.jump_password.as_deref(),
                self.target_password.as_deref(),
            );
        }

        let (response_tx, response_rx) = oneshot::channel();
        let terminal_request = TerminalMfaRequest {
            name: request.name.clone(),
            instructions: request.instructions.clone(),
            prompts: terminal_prompts,
        };

        if let Ok(mut state) = self.state.lock() {
            state.pending = Some(TerminalMfaPending {
                request: terminal_request,
                response_tx: Some(response_tx),
            });
        } else {
            return Err(anyhow!("failed to store SSH MFA request"));
        }
        self.notify_changed();

        let responses = response_rx
            .await
            .map_err(|_| anyhow!("SSH MFA response was cancelled"))?;

        keyboard_interactive_answers_for_terminal(
            &request,
            &responses,
            self.jump_password.as_deref(),
            self.target_password.as_deref(),
        )
    }
}

fn keyboard_interactive_answers_for_terminal(
    request: &KeyboardInteractiveRequest,
    responses: &[String],
    jump_password: Option<&str>,
    target_password: Option<&str>,
) -> Result<Vec<String>> {
    let mut response_index = 0;
    let mut answers = Vec::with_capacity(request.prompts.len());

    for prompt in &request.prompts {
        if is_ssh_password_prompt(&prompt.prompt) {
            let password = match request.target {
                KeyboardInteractiveTarget::JumpServer => jump_password,
                KeyboardInteractiveTarget::TargetServer => target_password,
            };
            answers.push(
                password
                    .ok_or_else(|| anyhow!("SSH password prompt has no configured password"))?
                    .to_string(),
            );
        } else {
            let response = responses
                .get(response_index)
                .ok_or_else(|| anyhow!("SSH MFA response is missing"))?;
            if response.trim().is_empty() {
                return Err(anyhow!("SSH MFA response is empty"));
            }
            answers.push(response.clone());
            response_index += 1;
        }
    }

    if response_index == responses.len() {
        Ok(answers)
    } else {
        Err(anyhow!("SSH MFA response count does not match prompts"))
    }
}

fn is_ssh_password_prompt(prompt: &str) -> bool {
    let prompt = prompt.trim().trim_end_matches(':').trim();
    let prompt = prompt.to_ascii_lowercase();

    if [
        "one-time password",
        "one time password",
        "otp",
        "verification code",
        "security code",
        "authentication code",
        "passcode",
        "token",
    ]
    .iter()
    .any(|marker| prompt.contains(marker))
    {
        return false;
    }

    prompt == "password"
        || prompt.ends_with("'s password")
        || prompt.starts_with("password for ")
        || prompt == "enter password"
}

fn resolve_ssh_connection(
    update: SshConnectionUpdate,
    _event_tx: UnboundedSender<TerminalEvent>,
) -> Result<ResolvedSshConnection> {
    let params = update.connection.to_ssh_params()?;
    let credential_prompt_policy = SshCredentialPromptPolicy {
        username: params.prompts_for_username(),
        password: params.prompts_for_password()
            && matches!(&params.auth_method, SshAuthMethod::Password { .. }),
    };
    let keyboard_interactive_enabled = params.keyboard_interactive_enabled();
    let terminal_encoding = params.terminal_encoding.into();
    let account_expect = params.account_expect.clone();
    let mut pty_config = PtyConfig::default();
    pty_config.term = params.terminal_type.as_str().to_string();
    let init_commands = build_ssh_init_commands(
        update.working_dir.as_deref(),
        params.default_directory.as_deref(),
        params.init_script.as_deref(),
        update.sync_path_with_terminal,
    );
    let ssh_config = SshConnectConfig {
        host: params.host,
        port: params.port,
        username: params.username,
        auth: ssh_auth_from_storage(params.auth_method),
        timeout: params.connect_timeout.map(Duration::from_secs),
        keepalive_interval: params.keepalive_interval.map(Duration::from_secs),
        keepalive_max: params.keepalive_max,
        jump_server: params.jump_server.map(|jump| JumpServerConnectConfig {
            host: jump.host,
            port: jump.port,
            username: jump.username,
            auth: ssh_auth_from_storage(jump.auth_method),
        }),
        proxy: params.proxy.map(|proxy| ProxyConnectConfig {
            proxy_type: match proxy.proxy_type {
                StorageProxyType::Socks5 => ProxyType::Socks5,
                StorageProxyType::Http => ProxyType::Http,
            },
            host: proxy.host,
            port: proxy.port,
            username: proxy.username,
            password: proxy.password,
        }),
        keyboard_interactive_responder: None,
        host_key_verifier: HostKeyVerifier::default(),
        x11_forwarding: params.x11_forwarding.unwrap_or(false),
        allow_legacy_algorithms: params.allow_legacy_algorithms.unwrap_or(false),
    };
    Ok(ResolvedSshConnection {
        config: SshTerminalConfig {
            ssh_config,
            pty_config,
            terminal_encoding,
            account_expect,
            // 运行时注入不写远端文件，默认启用；仅显式存储的禁用值才关闭集成。
            disable_shell_integration: params.disable_shell_integration.unwrap_or(false),
        },
        credential_prompt_policy,
        keyboard_interactive_enabled,
        init_commands,
        connection_id: update.connection.id,
        connection_name: update.connection.name,
    })
}

fn ssh_config_with_runtime_credentials(
    base_config: &SshTerminalConfig,
    credentials: &TerminalSshCredentials,
    event_tx: UnboundedSender<TerminalEvent>,
    keyboard_interactive_enabled: bool,
) -> Result<(SshTerminalConfig, TerminalMfaResponder)> {
    let mut config = base_config.clone();

    if let Some(username) = credentials.username.as_deref() {
        let username = username.trim();
        if username.is_empty() {
            return Err(anyhow!("SSH username is empty"));
        }
        config.ssh_config.username = username.to_string();
    }

    if let Some(password) = credentials.password.as_deref() {
        if password.is_empty() {
            return Err(anyhow!("SSH password is empty"));
        }
        match &mut config.ssh_config.auth {
            SshAuth::Password(configured_password) => {
                *configured_password = password.to_string();
            }
            _ => {
                return Err(anyhow!(
                    "runtime SSH password is only valid for password authentication"
                ));
            }
        }
    }

    let jump_password = config
        .ssh_config
        .jump_server
        .as_ref()
        .and_then(|jump| password_from_ssh_auth(&jump.auth));
    let target_password = password_from_ssh_auth(&config.ssh_config.auth);
    let responder = TerminalMfaResponder::new(event_tx, jump_password, target_password);
    config.ssh_config.keyboard_interactive_responder = keyboard_interactive_enabled
        .then(|| Arc::new(responder.clone()) as Arc<dyn KeyboardInteractiveResponder>);

    Ok((config, responder))
}

fn ssh_config_with_confirmed_host_key(
    runtime_config: &SshTerminalConfig,
    request: &HostKeyVerificationRequest,
    persist: bool,
) -> SshTerminalConfig {
    let mut config = runtime_config.clone();
    let verifier = config.ssh_config.host_key_verifier.clone();
    config.ssh_config.host_key_verifier = match &request.reason {
        HostKeyVerificationReason::Unknown => verifier.with_confirmed_key(
            request.identity.clone(),
            request.presented.clone(),
            persist,
        ),
        HostKeyVerificationReason::Changed { .. } => verifier.with_confirmed_changed_key(
            request.identity.clone(),
            request.presented.clone(),
            persist,
        ),
    };
    config
}

const DEFAULT_COLS: usize = 80;
const DEFAULT_ROWS: usize = 24;

/// 将路径安全地转为 POSIX shell 单参数，避免命令注入。
pub(crate) fn shell_escape_arg(arg: &str) -> String {
    if arg.is_empty() {
        return "''".to_string();
    }

    let mut escaped = String::with_capacity(arg.len() + 2);
    escaped.push('\'');
    for ch in arg.chars() {
        if ch == '\'' {
            escaped.push_str("'\"'\"'");
        } else {
            escaped.push(ch);
        }
    }
    escaped.push('\'');
    escaped
}

fn build_cd_command(dir: &str) -> String {
    format!("cd -- {}", shell_escape_arg(dir))
}

fn build_ssh_base_init_commands(
    working_dir: Option<&str>,
    default_directory: Option<&str>,
    init_script: Option<&str>,
) -> Option<String> {
    let mut commands = Vec::new();

    if let Some(work_dir) = working_dir {
        commands.push(build_cd_command(work_dir));
    } else {
        if let Some(dir) = default_directory.filter(|dir| !dir.is_empty()) {
            commands.push(build_cd_command(dir));
        }
        if let Some(script) = init_script.filter(|script| !script.is_empty()) {
            commands.push(script.to_string());
        }
    }

    (!commands.is_empty()).then(|| commands.join("\n"))
}

fn compose_ssh_init_commands(
    base_init_commands: Option<&str>,
    _sync_path_with_terminal: bool,
) -> Option<String> {
    base_init_commands
        .filter(|commands| !commands.is_empty())
        .map(str::to_string)
}

fn build_ssh_init_commands(
    working_dir: Option<&str>,
    default_directory: Option<&str>,
    init_script: Option<&str>,
    sync_path_with_terminal: bool,
) -> Option<String> {
    let base_init_commands =
        build_ssh_base_init_commands(working_dir, default_directory, init_script);
    compose_ssh_init_commands(base_init_commands.as_deref(), sync_path_with_terminal)
}

#[cfg(any(test, target_os = "windows"))]
fn path_if_file(path: impl Into<PathBuf>) -> Option<String> {
    let path = path.into();
    path.is_file().then(|| path.to_string_lossy().into_owned())
}

#[cfg(any(test, target_os = "windows"))]
fn find_executable_in_path(path_env: Option<&OsStr>, program: &str) -> Option<String> {
    let path_env = path_env?;
    std::env::split_paths(path_env)
        .map(|dir| dir.join(program))
        .find_map(path_if_file)
}

#[cfg(any(test, target_os = "windows"))]
fn resolve_default_windows_shell_from_env(
    path_env: Option<&OsStr>,
    system_root: Option<&OsStr>,
    comspec: Option<&OsStr>,
) -> String {
    if let Some(pwsh) = find_executable_in_path(path_env, "pwsh.exe") {
        return pwsh;
    }

    if let Some(system_root) = system_root {
        let powershell = Path::new(system_root)
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe");
        if let Some(powershell) = path_if_file(powershell) {
            return powershell;
        }
    }

    if let Some(powershell) = find_executable_in_path(path_env, "powershell.exe") {
        return powershell;
    }

    if let Some(comspec) = comspec.and_then(path_if_file) {
        return comspec;
    }

    if let Some(system_root) = system_root {
        let cmd = Path::new(system_root).join("System32").join("cmd.exe");
        if let Some(cmd) = path_if_file(cmd) {
            return cmd;
        }
    }

    "cmd.exe".to_string()
}

#[cfg(target_os = "windows")]
fn build_local_shell(shell: Option<String>, extra_args: Vec<String>) -> Option<tty::Shell> {
    // `new_local` resolves the default shell from the freshly read Windows
    // environment before reaching this helper.
    let program = shell.unwrap_or_else(|| "cmd.exe".to_string());
    Some(tty::Shell::new(program, extra_args))
}

#[cfg(not(target_os = "windows"))]
fn build_local_shell(shell: Option<String>, extra_args: Vec<String>) -> Option<tty::Shell> {
    if extra_args.is_empty() {
        shell.map(|program| tty::Shell::new(program, vec![]))
    } else {
        // 有额外参数（如 --rcfile）时需显式指定 shell 程序
        let program = shell.or_else(|| std::env::var("SHELL").ok())?;
        Some(tty::Shell::new(program, extra_args))
    }
}

fn default_local_working_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

pub fn resolve_local_working_dir(working_dir: Option<String>) -> Option<PathBuf> {
    match working_dir {
        Some(dir) if dir.trim().is_empty() => default_local_working_dir(),
        Some(dir) => Some(PathBuf::from(dir)),
        None => default_local_working_dir(),
    }
}

/// 准备本地终端的 Shell Integration 环境
///
/// 将 `shell_integration.sh` 写入进程级临时目录 `/tmp/onetcli-<pid>/`，
/// 仅对当前 OnetCli 进程内的终端会话生效，不污染全局配置。
/// 返回 `(额外环境变量, shell 额外参数)`。
#[cfg(not(target_os = "windows"))]
fn prepare_shell_integration(shell: Option<&str>) -> (Vec<(String, String)>, Vec<String>) {
    // 使用进程级临时目录，确保不影响其他会话或工具
    let session_dir = std::env::temp_dir().join(format!("onetcli-{}", std::process::id()));
    if fs::create_dir_all(&session_dir).is_err() {
        tracing::warn!(
            "无法创建临时目录 {}，跳过 Shell Integration",
            session_dir.display()
        );
        return (vec![], vec![]);
    }

    // 写入 shell_integration.sh（含交互式守卫，不影响 rsync/scp 等非交互通道）
    let integration_path = session_dir.join("shell_integration.sh");
    if let Err(e) = fs::write(&integration_path, embedded_shell_integration_script()) {
        tracing::warn!("写入 shell_integration.sh 失败: {e}");
        return (vec![], vec![]);
    }

    let mut extra_env: Vec<(String, String)> =
        vec![("ONETCLI_SHELL_INTEGRATION".into(), "1".into())];
    let mut extra_args: Vec<String> = Vec::new();

    // 判断 shell 类型：优先用显式参数，否则读 $SHELL
    let shell_name = shell
        .map(|s| s.to_ascii_lowercase())
        .or_else(|| std::env::var("SHELL").ok().map(|s| s.to_ascii_lowercase()))
        .unwrap_or_default();

    if shell_name.contains("zsh") {
        // zsh: 通过 ZDOTDIR 注入集成脚本
        let zsh_dir = session_dir.join("zsh");
        if fs::create_dir_all(&zsh_dir).is_err() {
            return (extra_env, extra_args);
        }

        let script = shell_escape_arg(&integration_path.to_string_lossy());
        let onetcli_zdotdir = shell_escape_arg(&zsh_dir.to_string_lossy());

        // .zshenv — 恢复原始 ZDOTDIR 并 source 用户的 .zshenv
        let zshenv = format!(
            "_ONETCLI_ZDOTDIR={onetcli_zdotdir}\n\
             _ONETCLI_USER_ZDOTDIR=\"${{_ONETCLI_ORIG_ZDOTDIR:-$HOME}}\"\n\
             ZDOTDIR=\"$_ONETCLI_USER_ZDOTDIR\"\n\
             [[ -f \"$ZDOTDIR/.zshenv\" ]] && source \"$ZDOTDIR/.zshenv\"\n\
             _ONETCLI_USER_ZDOTDIR=\"${{ZDOTDIR:-$_ONETCLI_USER_ZDOTDIR}}\"\n\
             export _ONETCLI_USER_ZDOTDIR\n\
             export ZDOTDIR=\"$_ONETCLI_ZDOTDIR\"\n"
        );
        let _ = fs::write(zsh_dir.join(".zshenv"), zshenv);

        // .zshrc — 恢复 ZDOTDIR，source 用户 .zshrc，再 source 集成脚本
        let zshrc = format!(
            "ZDOTDIR=\"${{_ONETCLI_USER_ZDOTDIR:-${{_ONETCLI_ORIG_ZDOTDIR:-$HOME}}}}\"\n\
             [[ -f \"$ZDOTDIR/.zshrc\" ]] && source \"$ZDOTDIR/.zshrc\"\n\
             source {script}\n"
        );
        let _ = fs::write(zsh_dir.join(".zshrc"), zshrc);

        let orig = std::env::var("ZDOTDIR").unwrap_or_default();
        extra_env.push(("_ONETCLI_ORIG_ZDOTDIR".into(), orig));
        extra_env.push(("ZDOTDIR".into(), zsh_dir.display().to_string()));

        tracing::debug!(
            "已配置 zsh Shell Integration (ZDOTDIR={})",
            zsh_dir.display()
        );
    } else if shell_name.contains("bash") {
        // bash: 通过 --rcfile 注入集成脚本
        let bash_rc = session_dir.join("bash_integration.sh");
        let script = integration_path.display();
        let content = format!(
            "[[ -f \"$HOME/.bashrc\" ]] && source \"$HOME/.bashrc\"\n\
             source \"{script}\"\n"
        );
        let _ = fs::write(&bash_rc, content);

        extra_args.push("--rcfile".into());
        extra_args.push(bash_rc.display().to_string());

        tracing::debug!(
            "已配置 bash Shell Integration (--rcfile={})",
            bash_rc.display()
        );
    } else {
        tracing::debug!("未知 shell 类型 '{shell_name}'，跳过 Shell Integration 注入");
    }

    (extra_env, extra_args)
}

#[cfg(target_os = "windows")]
fn prepare_shell_integration(shell: Option<&str>) -> (Vec<(String, String)>, Vec<String>) {
    let program = shell.unwrap_or("cmd.exe");
    crate::windows_shell_integration::prepare(program)
}

fn history_file_candidates(preferred_shell: Option<&str>) -> Vec<(PathBuf, ShellHistoryFormat)> {
    let Some(home_dir) = dirs::home_dir() else {
        return Vec::new();
    };

    let mut candidates = Vec::new();
    let lower_shell = preferred_shell.unwrap_or_default().to_ascii_lowercase();
    let prefer_zsh = lower_shell.contains("zsh");

    let bash = (home_dir.join(".bash_history"), ShellHistoryFormat::Bash);
    let zsh = (home_dir.join(".zsh_history"), ShellHistoryFormat::Zsh);

    if prefer_zsh {
        candidates.push(bash);
        candidates.push(zsh);
    } else {
        candidates.push(zsh);
        candidates.push(bash);
    }

    candidates
}

fn load_local_history(preferred_shell: Option<&str>) -> Vec<String> {
    history_file_candidates(preferred_shell)
        .into_iter()
        .filter_map(|(path, format)| fs::read_to_string(path).ok().map(|text| (text, format)))
        .flat_map(|(text, format)| parse_shell_history(&text, format))
        .collect()
}

#[cfg(target_os = "macos")]
fn with_local_terminal_default_env(mut env: Vec<(String, String)>) -> Vec<(String, String)> {
    const GUI_FALLBACK_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";
    const DEFAULT_LANG: &str = "en_US.UTF-8";
    const DEFAULT_LC_CTYPE: &str = "UTF-8";
    const MACOS_CLI_PATHS: &[&str] = &[
        "/opt/homebrew/bin",
        "/opt/homebrew/sbin",
        "/usr/local/bin",
        "/usr/local/sbin",
    ];

    let path = env
        .iter()
        .find_map(|(key, value)| (key == "PATH").then_some(value.clone()))
        .or_else(|| std::env::var("PATH").ok())
        .unwrap_or_else(|| GUI_FALLBACK_PATH.to_string());
    let mut path_parts: Vec<String> = path
        .split(':')
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect();

    for cli_path in MACOS_CLI_PATHS.iter().rev() {
        if !path_parts.iter().any(|part| part == cli_path) {
            path_parts.insert(0, (*cli_path).to_string());
        }
    }

    let merged_path = path_parts.join(":");
    if let Some((_, value)) = env.iter_mut().find(|(key, _)| key == "PATH") {
        *value = merged_path;
    } else {
        env.push(("PATH".to_string(), merged_path));
    }
    push_env_if_missing(&mut env, "LANG", DEFAULT_LANG);
    push_env_if_missing(&mut env, "LC_CTYPE", DEFAULT_LC_CTYPE);
    env
}

#[cfg(target_os = "macos")]
fn push_env_if_missing(env: &mut Vec<(String, String)>, key: &str, value: &str) {
    if !env.iter().any(|(name, _)| name == key) {
        env.push((key.to_string(), value.to_string()));
    }
}

#[cfg(not(target_os = "macos"))]
fn with_local_terminal_default_env(env: Vec<(String, String)>) -> Vec<(String, String)> {
    env
}

fn build_remote_history_load_command() -> String {
    [
        "sh -lc",
        "'",
        "if [ -f \"$HOME/.bash_history\" ]; then tail -n 512 \"$HOME/.bash_history\" 2>/dev/null || true; fi;",
        "printf \"\\n__ONETCLI_HISTORY_SPLIT__\\n\";",
        "if [ -f \"$HOME/.zsh_history\" ]; then tail -n 512 \"$HOME/.zsh_history\" 2>/dev/null || true; fi",
        "'",
    ]
    .join(" ")
}

fn parse_remote_history_output(output: &str) -> Vec<String> {
    let (bash_history, zsh_history) = output
        .split_once("\n__ONETCLI_HISTORY_SPLIT__\n")
        .unwrap_or((output, ""));

    let mut commands = parse_shell_history(bash_history, ShellHistoryFormat::Bash);
    commands.extend(parse_shell_history(zsh_history, ShellHistoryFormat::Zsh));
    commands
}

async fn load_ssh_history(manager: Arc<SshSessionManager>) -> anyhow::Result<Vec<String>> {
    let mut channel = manager.open_channel().await?;
    let command = build_remote_history_load_command();
    channel.exec(&command).await?;

    let mut stdout = Vec::new();
    let mut exit_code = None;

    loop {
        match channel.recv().await {
            Some(ChannelEvent::Data(data)) => stdout.extend(data),
            Some(ChannelEvent::ExitStatus(code)) => exit_code = Some(code),
            Some(ChannelEvent::Eof) | Some(ChannelEvent::Close) | None => break,
            _ => {}
        }
    }

    let _ = channel.close().await;

    if let Some(code) = exit_code {
        anyhow::ensure!(code == 0, "ssh history loader exited with status {code}");
    }

    Ok(parse_remote_history_output(&String::from_utf8_lossy(
        &stdout,
    )))
}

/// 终端模型 Entity
///
/// 负责管理终端的核心状态，包括：
/// - alacritty Term grid
/// - PTY/SSH 后端
/// - 连接状态
/// - 标题
pub struct Terminal {
    /// alacritty 终端状态
    term: Arc<FairMutex<Term<GpuiEventProxy>>>,
    /// Whether this surface owns a live connection or renders an untrusted,
    /// read-only artifact.
    session_mode: TerminalSessionMode,
    /// 当前终端实例的共享性能指标。
    performance_metrics: Arc<TerminalPerformanceMetrics>,
    /// PTY/SSH 后端
    backend: Option<Box<dyn TerminalBackend>>,
    /// 与终端实例同生命周期的录制运行时；重连只会克隆新的 tap，不会替换时间线。
    recording_runtime: std::result::Result<RecordingRuntime, RecordingRuntimeError>,
    /// 自动会话日志使用独立运行时，与手工录制互不影响并跨重连保留时间线。
    session_log_runtime: Option<RecordingRuntime>,
    /// Read-only artifacts own a separate fail-closed parser and grid. Live
    /// terminals never populate this field. Playback controls remain gated by
    /// `TerminalSessionMode::RecordingPlayback`.
    playback_runtime: Option<TerminalPlaybackRuntime>,
    /// 只用于录制文件关联的随机逻辑会话 ID；不包含连接名称、地址或凭据。
    recording_session_id: String,
    /// 允许持久化到录制 header 的安全会话字段快照。
    recording_session_metadata: RecordingSessionMetadata,

    /// 终端标题
    title: String,
    /// 当前工作目录（由 OSC 7 更新，仅 SSH 终端）
    current_working_dir: Option<String>,
    /// 子进程退出码
    child_exited: Option<i32>,
    /// 连接状态
    connection_state: ConnectionState,
    /// 会话锁定状态；锁定后禁止输入，可选隐藏输出。
    session_lock: Option<SessionLockState>,

    /// 终端尺寸
    cols: usize,
    rows: usize,
    /// 最近一次同步给 PTY 的像素尺寸,用于 nudge_resize 重发 SIGWINCH
    pixel_width: u16,
    pixel_height: u16,

    /// 当前 SSH 运行时配置；临时凭据仅存在于内存中，不会写回 StoredConnection。
    ssh_config: Option<SshTerminalConfig>,
    /// 可安全用于重连的 SSH 模板配置，不包含要求每次输入的用户名或密码。
    ssh_base_config: Option<SshTerminalConfig>,
    /// SSH 会话管理器（同一 SSH tab 共享底层连接）
    ssh_session_manager: Option<Arc<SshSessionManager>>,
    /// 当前连接的临时凭据策略。
    ssh_credential_prompt_policy: SshCredentialPromptPolicy,
    /// 等待用户输入的临时 SSH 用户名/密码。
    ssh_credential_request: Option<TerminalSshCredentialRequest>,
    /// 是否允许 keyboard-interactive（OTP/2FA）认证。
    ssh_keyboard_interactive_enabled: bool,
    /// SSH keyboard-interactive/MFA 输入响应器
    ssh_mfa_responder: Option<TerminalMfaResponder>,
    /// SSH ZMODEM 文件选择请求协调器
    zmodem_responder: Option<ZmodemResponder>,
    /// 等待用户确认的未知 SSH 主机指纹
    pending_host_key_verification: Option<HostKeyVerificationRequest>,
    /// 串口参数（用于重连）
    serial_params: Option<SerialParams>,
    /// Telnet 参数（用于重连）
    telnet_params: Option<TelnetParams>,
    /// 未注入本次临时用户名/密码的 Telnet 参数模板。
    telnet_base_params: Option<TelnetParams>,
    /// 等待用户输入的临时 Telnet 用户名/密码。
    telnet_credential_request: Option<TerminalTelnetCredentialRequest>,
    /// 完整事件链路中是否已有尚未被 GPUI 消费的 Wakeup。
    wakeup_pending: Arc<AtomicBool>,
    /// 事件发送器（用于 SSH 重连）
    event_tx: Option<UnboundedSender<TerminalEvent>>,
    /// 事件代理（用于设置 PtyWrite 回写通道）
    event_proxy: Option<GpuiEventProxy>,
    /// 连接 ID
    connection_id: Option<i64>,
    /// 连接名称
    connection_name: Option<String>,
    /// 初始化命令（连接成功后执行）
    init_commands: Option<String>,
    /// 当前 OnetCli 会话内记录的命令历史（富条目，含 frecency 元数据）
    session_history: VecDeque<HistoryEntry>,
    /// 从 shell 历史文件加载的持久化历史
    persisted_history: Vec<String>,
    /// 成功命令历史数据库仓库
    history_repository: Option<Arc<TerminalCommandHistoryRepository>>,
    /// 当前终端对应的历史 scope
    history_scope: Option<TerminalHistoryScope>,
    /// OSC 命令记录 gate：只有 exit_code=0 的命令会被接收
    command_record_gate: CommandRecordGate,
    /// 当前连接尝试代次，用于忽略过期的异步回调
    connection_generation: u64,

    /// 连接类型
    connection_kind: TerminalConnectionKind,
    /// 终端滚屏历史最多保留的行数
    scrollback_lines: usize,
}

#[derive(Clone)]
pub struct TerminalScrollProxy {
    term: Arc<FairMutex<Term<GpuiEventProxy>>>,
    event_tx: Option<UnboundedSender<TerminalEvent>>,
    wakeup_pending: Arc<AtomicBool>,
}

/// Snapshot of terminal scroll state, captured in a single lock acquisition
/// to ensure consistency.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalScrollSnapshot {
    pub display_offset: usize,
    pub history_size: usize,
    pub screen_lines: usize,
    pub columns: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalTextSnapshot {
    pub text: String,
    pub requested_lines: usize,
    pub returned_lines: usize,
    pub available_lines: usize,
    pub history_size: usize,
    pub screen_lines: usize,
    pub columns: usize,
}

impl TerminalScrollProxy {
    fn snapshot_from_term(term: &Term<GpuiEventProxy>) -> TerminalScrollSnapshot {
        TerminalScrollSnapshot {
            display_offset: term.grid().display_offset(),
            history_size: term.history_size(),
            screen_lines: term.screen_lines(),
            columns: term.columns(),
        }
    }

    /// Snapshot all scroll-related state in a single lock acquisition
    /// to avoid inconsistency from multiple separate locks.
    pub fn snapshot(&self) -> TerminalScrollSnapshot {
        let term = self.term.lock();
        Self::snapshot_from_term(&term)
    }

    /// Try to capture scroll state without waiting for the parser.
    ///
    /// GPUI layout/paint paths must use this instead of [`Self::snapshot`], so a
    /// large PTY parse cannot block the Windows message pump.
    pub fn try_snapshot(&self) -> Option<TerminalScrollSnapshot> {
        let term = self.term.try_lock_unfair()?;
        Some(Self::snapshot_from_term(&term))
    }

    /// Try to move the viewport to an exact display offset without waiting for
    /// the parser. Returns `false` when the terminal is currently busy.
    pub fn try_set_display_offset(&self, display_offset: usize) -> bool {
        let Some(mut term) = self.term.try_lock_unfair() else {
            return false;
        };
        let current = term.grid().display_offset();
        let delta = display_offset as i64 - current as i64;
        if delta != 0 {
            let delta = delta.clamp(i32::MIN as i64, i32::MAX as i64) as i32;
            term.scroll_display(alacritty_terminal::grid::Scroll::Delta(delta));
        }
        drop(term);

        if delta != 0 {
            if let Some(tx) = &self.event_tx {
                send_coalesced_wakeup(tx, &self.wakeup_pending);
            }
        }
        true
    }

    /// Try to move the viewport by a relative number of lines without waiting
    /// for the parser. Returns `false` when the terminal is currently busy.
    pub fn try_scroll_display_delta(&self, delta: i32) -> bool {
        let Some(mut term) = self.term.try_lock_unfair() else {
            return false;
        };
        if delta != 0 {
            term.scroll_display(alacritty_terminal::grid::Scroll::Delta(delta));
        }
        drop(term);

        if delta != 0 {
            if let Some(tx) = &self.event_tx {
                send_coalesced_wakeup(tx, &self.wakeup_pending);
            }
        }
        true
    }

    pub fn display_offset(&self) -> usize {
        self.term.lock().grid().display_offset()
    }

    pub fn history_size(&self) -> usize {
        self.term.lock().history_size()
    }

    pub fn screen_lines(&self) -> usize {
        self.term.lock().screen_lines()
    }

    pub fn columns(&self) -> usize {
        self.term.lock().columns()
    }

    pub fn mode(&self) -> TermMode {
        *self.term.lock().mode()
    }

    /// Read the most recent physical PTY rows from scrollback and the current
    /// screen. This ignores the user's viewport scroll offset and always tails
    /// the live terminal buffer.
    pub fn recent_text(&self, max_lines: usize) -> TerminalTextSnapshot {
        recent_text_from_term(&self.term, max_lines)
    }

    pub fn scroll_display_delta(&self, delta: i32) {
        if delta == 0 {
            return;
        }
        self.term
            .lock()
            .scroll_display(alacritty_terminal::grid::Scroll::Delta(delta));
        if let Some(tx) = &self.event_tx {
            send_coalesced_wakeup(tx, &self.wakeup_pending);
        }
    }
}

fn visible_text_from_term(term: &Arc<FairMutex<Term<GpuiEventProxy>>>) -> String {
    let term = term.lock();
    let grid = term.grid();
    let display_offset = grid.display_offset();
    let mut lines = Vec::with_capacity(term.screen_lines());

    for screen_line in 0..term.screen_lines() {
        let grid_line = screen_line as i32 - display_offset as i32;
        if grid_line < -(term.history_size() as i32) || grid_line >= term.screen_lines() as i32 {
            lines.push(String::new());
            continue;
        }

        let line = &grid[Line(grid_line)];
        let text: String = line[..].iter().map(|cell| cell.c).collect();
        lines.push(
            text.trim_end_matches(|ch: char| ch == ' ' || ch == '\0')
                .to_string(),
        );
    }

    lines.join("\n")
}

fn recent_text_from_term(
    term: &Arc<FairMutex<Term<GpuiEventProxy>>>,
    max_lines: usize,
) -> TerminalTextSnapshot {
    let term = term.lock();
    let grid = term.grid();
    let history_size = term.history_size();
    let screen_lines = term.screen_lines();
    let columns = term.columns();
    let top_line = -(history_size as i32);
    let bottom_line = screen_lines.saturating_sub(1) as i32;
    let cursor_line = grid.cursor.point.line.0.clamp(top_line, bottom_line);
    let line_text = |line: i32| {
        let text: String = grid[Line(line)][..].iter().map(|cell| cell.c).collect();
        text.trim_end_matches(|ch: char| ch == ' ' || ch == '\0')
            .to_string()
    };

    let mut last_content_line = cursor_line;
    for line in (cursor_line + 1)..=bottom_line {
        if !line_text(line).is_empty() {
            last_content_line = line;
        }
    }
    let available_lines = (last_content_line - top_line + 1).max(0) as usize;
    let take = max_lines.min(available_lines);
    let start_line = last_content_line - take.saturating_sub(1) as i32;
    let lines = if take == 0 {
        Vec::new()
    } else {
        (start_line..=last_content_line)
            .map(line_text)
            .collect::<Vec<_>>()
    };

    TerminalTextSnapshot {
        text: lines.join("\n"),
        requested_lines: max_lines,
        returned_lines: lines.len(),
        available_lines,
        history_size,
        screen_lines,
        columns,
    }
}

fn merge_history_matches(primary: Vec<String>, fallback: Vec<String>, limit: usize) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut merged = Vec::new();
    for command in primary.into_iter().chain(fallback) {
        if seen.insert(command.clone()) {
            merged.push(command);
        }
        if merged.len() >= limit {
            break;
        }
    }
    merged
}

fn normalize_history_matches(
    commands: Vec<String>,
    history_user: Option<&str>,
    limit: usize,
) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut normalized = Vec::new();
    for command in commands {
        let Some(command) = normalize_recorded_command(&command, history_user) else {
            continue;
        };
        if seen.insert(command.clone()) {
            normalized.push(command);
        }
        if normalized.len() >= limit {
            break;
        }
    }
    normalized
}

fn is_reconnect_generation(generation: u64) -> bool {
    generation > 1
}

struct PendingPlaybackEventLoop {
    event_rx: UnboundedReceiver<TerminalEvent>,
    wakeup_pending: Arc<AtomicBool>,
}

struct AutomaticSessionLogRuntimeInput {
    enabled: bool,
    event_tx: UnboundedSender<TerminalEvent>,
    wakeup_pending: Arc<AtomicBool>,
    backend: RecordingBackend,
    session_id: String,
    initial_size: TerminalSize,
    session: RecordingSessionMetadata,
}

fn send_coalesced_wakeup(event_tx: &UnboundedSender<TerminalEvent>, wakeup_pending: &AtomicBool) {
    if wakeup_pending.swap(true, Ordering::AcqRel) {
        return;
    }

    if event_tx.send(TerminalEvent::Wakeup).is_err() {
        wakeup_pending.store(false, Ordering::Release);
    }
}

impl Terminal {
    fn new_recording_session_id() -> String {
        Uuid::new_v4().to_string()
    }

    fn create_recording_runtime(
        event_tx: UnboundedSender<TerminalEvent>,
        wakeup_pending: Arc<AtomicBool>,
    ) -> std::result::Result<RecordingRuntime, RecordingRuntimeError> {
        RecordingRuntime::with_observer(RecordingRuntimeConfig::default(), move |_| {
            // Recording control transitions and asynchronous failures must
            // invalidate the pane, but they must never block the recording
            // worker when the terminal has already gone away.
            send_coalesced_wakeup(&event_tx, &wakeup_pending);
        })
    }

    fn recording_tap(&self) -> Option<RecordingTap> {
        if self.is_read_only() {
            return None;
        }
        Self::recording_tap_for_runtimes(&self.recording_runtime, &self.session_log_runtime)
    }

    fn recording_tap_for_runtimes(
        recording_runtime: &std::result::Result<RecordingRuntime, RecordingRuntimeError>,
        session_log_runtime: &Option<RecordingRuntime>,
    ) -> Option<RecordingTap> {
        RecordingTap::fan_out(
            [
                recording_runtime.as_ref().ok().map(RecordingRuntime::tap),
                session_log_runtime.as_ref().map(RecordingRuntime::tap),
            ]
            .into_iter()
            .flatten(),
        )
    }

    fn start_automatic_session_log(
        input: AutomaticSessionLogRuntimeInput,
    ) -> Option<RecordingRuntime> {
        if !input.enabled {
            return None;
        }
        let Some(data_directory) = app_dirs::data_dir() else {
            tracing::warn!(
                "automatic session logging disabled: application data directory missing"
            );
            return None;
        };
        let started_at_unix_ms = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(duration) => u64::try_from(duration.as_millis()).ok(),
            Err(_) => None,
        };
        let Some(started_at_unix_ms) = started_at_unix_ms else {
            tracing::warn!("automatic session logging disabled: invalid system clock");
            return None;
        };
        let request = build_automatic_session_log_request(AutomaticSessionLogRequestInput {
            data_directory,
            backend: input.backend,
            session_id: input.session_id,
            initial_size: input.initial_size,
            session: input.session,
            started_at_unix_ms,
            recording_id: Uuid::new_v4().to_string(),
        });
        Self::start_automatic_session_log_request(input.event_tx, input.wakeup_pending, request)
    }

    fn start_automatic_session_log_request(
        event_tx: UnboundedSender<TerminalEvent>,
        wakeup_pending: Arc<AtomicBool>,
        request: std::result::Result<RecordingStartRequest, RecordingRuntimeError>,
    ) -> Option<RecordingRuntime> {
        let request = match request {
            Ok(request) => request,
            Err(error) => {
                tracing::warn!(%error, "failed to build automatic terminal session log request");
                return None;
            }
        };
        let runtime = match Self::create_recording_runtime(event_tx, wakeup_pending) {
            Ok(runtime) => runtime,
            Err(error) => {
                tracing::warn!(%error, "failed to create automatic terminal session log runtime");
                return None;
            }
        };
        if let Err(error) = runtime.start(request) {
            tracing::warn!(%error, "failed to start automatic terminal session log");
            let _ = runtime.shutdown();
            return None;
        }
        Some(runtime)
    }

    fn record_connection_generation_marker(&self, generation: u64) {
        if let Some(tap) = self.recording_tap() {
            let _ = tap.record_marker(&format!("connection_generation:{generation}"));
        }
    }

    fn recording_runtime(&self) -> std::result::Result<&RecordingRuntime, RecordingRuntimeError> {
        if self.is_read_only() {
            return Err(RecordingRuntimeError::ReadOnlyPlayback);
        }
        self.recording_runtime
            .as_ref()
            .map_err(RecordingRuntimeError::clone)
    }

    fn new_local_disconnected(error: String, cx: &mut Context<Self>) -> Self {
        let (event_tx, event_rx) = unbounded_channel::<TerminalEvent>();
        let recording_session_id = Self::new_recording_session_id();
        let scrollback_lines = AppSettings::current(cx).terminal_scrollback_lines;
        let (term, event_proxy, _colors, performance_metrics) = Self::create_term(
            DEFAULT_COLS,
            DEFAULT_ROWS,
            scrollback_lines,
            event_tx.clone(),
        );
        let wakeup_pending = event_proxy.wakeup_pending_handle();
        let recording_runtime =
            Self::create_recording_runtime(event_tx.clone(), wakeup_pending.clone());

        Self::spawn_event_loop(event_rx, wakeup_pending.clone(), cx);

        Self {
            term,
            session_mode: TerminalSessionMode::Live,
            performance_metrics,
            backend: None,
            recording_runtime,
            session_log_runtime: None,
            playback_runtime: None,
            recording_session_id,
            recording_session_metadata: local_recording_session_metadata(),
            title: String::new(),
            current_working_dir: None,
            child_exited: None,
            connection_state: ConnectionState::Disconnected { error: Some(error) },
            session_lock: None,
            cols: DEFAULT_COLS,
            rows: DEFAULT_ROWS,
            pixel_width: 0,
            pixel_height: 0,
            ssh_config: None,
            ssh_base_config: None,
            ssh_session_manager: None,
            ssh_credential_prompt_policy: SshCredentialPromptPolicy::default(),
            ssh_credential_request: None,
            ssh_keyboard_interactive_enabled: false,
            ssh_mfa_responder: None,
            zmodem_responder: None,
            pending_host_key_verification: None,
            serial_params: None,
            telnet_params: None,
            telnet_base_params: None,
            telnet_credential_request: None,
            wakeup_pending,
            event_tx: Some(event_tx.clone()),
            event_proxy: None,
            connection_id: None,
            connection_name: None,
            init_commands: None,
            session_history: VecDeque::new(),
            persisted_history: Vec::new(),
            history_repository: Self::history_repository(cx),
            history_scope: Some(TerminalHistoryScope::local()),
            command_record_gate: CommandRecordGate::default(),
            connection_generation: 0,
            connection_kind: TerminalConnectionKind::Local,
            scrollback_lines,
        }
    }

    pub fn new_local_or_disconnected(
        config: LocalConfig,
        cx: &mut Context<Self>,
    ) -> (Self, Option<String>) {
        match Self::new_local(config, cx) {
            Ok(terminal) => (terminal, None),
            Err(error) => {
                let message = error.to_string();
                (
                    Self::new_local_disconnected(message.clone(), cx),
                    Some(message),
                )
            }
        }
    }

    /// 创建本地终端
    pub fn new_local(config: LocalConfig, cx: &mut Context<Self>) -> Result<Self> {
        let (event_tx, event_rx) = unbounded_channel::<TerminalEvent>();
        let app_settings = AppSettings::current(cx);
        let scrollback_lines = app_settings.terminal_scrollback_lines;
        let automatic_logging = app_settings.terminal_auto_session_logging;
        let (term, event_proxy, _colors, performance_metrics) = Self::create_term(
            DEFAULT_COLS,
            DEFAULT_ROWS,
            scrollback_lines,
            event_tx.clone(),
        );
        let wakeup_pending = event_proxy.wakeup_pending_handle();
        let recording_runtime =
            Self::create_recording_runtime(event_tx.clone(), wakeup_pending.clone());
        let recording_session_id = Self::new_recording_session_id();
        let recording_session_metadata = local_recording_session_metadata();
        let session_log_runtime =
            Self::start_automatic_session_log(AutomaticSessionLogRuntimeInput {
                enabled: automatic_logging,
                event_tx: event_tx.clone(),
                wakeup_pending: wakeup_pending.clone(),
                backend: RecordingBackend::Local,
                session_id: recording_session_id.clone(),
                initial_size: TerminalSize {
                    rows: DEFAULT_ROWS as u16,
                    cols: DEFAULT_COLS as u16,
                    pixel_width: 0,
                    pixel_height: 0,
                },
                session: recording_session_metadata.clone(),
            });
        let recording_tap =
            Self::recording_tap_for_runtimes(&recording_runtime, &session_log_runtime);
        let LocalConfig {
            shell,
            args,
            working_dir,
            env,
        } = config;
        #[cfg(target_os = "windows")]
        let refreshed_env = refreshed_windows_environment();
        #[cfg(target_os = "windows")]
        let shell = shell.or_else(|| {
            let path = environment_value(&refreshed_env, "PATH").map(OsStr::new);
            let system_root = environment_value(&refreshed_env, "SystemRoot").map(OsStr::new);
            let comspec = environment_value(&refreshed_env, "COMSPEC").map(OsStr::new);
            Some(resolve_default_windows_shell_from_env(
                path,
                system_root,
                comspec,
            ))
        });
        let history_shell = shell.clone();
        let working_directory = resolve_local_working_dir(working_dir);

        // 准备 Shell Integration 环境（写入集成脚本、生成 wrapper 配置）
        let (integration_env, integration_args) = prepare_shell_integration(shell.as_deref());
        let mut shell_args = args;
        shell_args.extend(integration_args);

        // 合并用户环境变量与 Shell Integration 环境变量
        #[cfg(target_os = "windows")]
        let mut env_pairs = merge_environment_overrides(refreshed_env, env);
        #[cfg(not(target_os = "windows"))]
        let mut env_pairs = env;
        env_pairs.extend(integration_env);
        env_pairs = with_local_terminal_default_env(env_pairs);

        let pty_options = PtyOptions {
            shell: build_local_shell(shell, shell_args),
            working_directory,
            env: env_pairs.into_iter().collect(),
            drain_on_exit: true,
            #[cfg(target_os = "windows")]
            escape_args: true,
        };
        let local_backend = LocalPtyBackend::new_with_recording(
            term.clone(),
            event_proxy.clone(),
            pty_options,
            recording_tap,
        )?;

        Self::spawn_event_loop(event_rx, wakeup_pending.clone(), cx);
        Self::spawn_local_history_loader(history_shell.as_deref(), cx);
        let history_repository = Self::history_repository(cx);

        Ok(Self {
            term,
            session_mode: TerminalSessionMode::Live,
            performance_metrics,
            backend: Some(Box::new(local_backend)),
            recording_runtime,
            session_log_runtime,
            playback_runtime: None,
            recording_session_id,
            recording_session_metadata,
            title: String::new(),
            current_working_dir: None,
            child_exited: None,
            connection_state: ConnectionState::Connected,
            session_lock: None,
            cols: DEFAULT_COLS,
            rows: DEFAULT_ROWS,
            pixel_width: 0,
            pixel_height: 0,
            ssh_config: None,
            ssh_base_config: None,
            ssh_session_manager: None,
            ssh_credential_prompt_policy: SshCredentialPromptPolicy::default(),
            ssh_credential_request: None,
            ssh_keyboard_interactive_enabled: false,
            ssh_mfa_responder: None,
            zmodem_responder: None,
            pending_host_key_verification: None,
            serial_params: None,
            telnet_params: None,
            telnet_base_params: None,
            telnet_credential_request: None,
            wakeup_pending,
            event_tx: Some(event_tx),
            event_proxy: None, // 本地终端的 event_proxy 已在 LocalPtyBackend 中设置
            connection_id: None,
            connection_name: None,
            init_commands: None,
            session_history: VecDeque::new(),
            persisted_history: Vec::new(),
            history_repository,
            history_scope: Some(TerminalHistoryScope::local()),
            command_record_gate: CommandRecordGate::default(),
            connection_generation: 0,
            connection_kind: TerminalConnectionKind::Local,
            scrollback_lines,
        })
    }

    pub fn clear_screen(&mut self, cx: &mut Context<Self>) {
        if self.is_read_only() {
            return;
        }
        let mut term = self.term.lock();
        term.grid_mut().reset::<Color>();
        term.selection = None;
        drop(term);
        if let Some(bytes) = clear_screen_remote_redraw_bytes(self.connection_kind) {
            self.write(bytes);
        }
        cx.emit(TerminalModelEvent::Wakeup);
    }

    /// Try to clear the terminal without waiting for the parser.
    ///
    /// GPUI event handlers should queue and retry this operation when it
    /// returns `false`, keeping the Windows message loop responsive while the
    /// parser owns the terminal lock.
    pub fn try_clear_screen(&mut self, cx: &mut Context<Self>) -> bool {
        if self.is_read_only() {
            return true;
        }
        let Some(mut term) = self.term.try_lock_unfair() else {
            return false;
        };
        term.grid_mut().reset::<Color>();
        term.selection = None;
        drop(term);
        if let Some(bytes) = clear_screen_remote_redraw_bytes(self.connection_kind) {
            self.write(bytes);
        }
        cx.emit(TerminalModelEvent::Wakeup);
        true
    }

    /// 创建 SSH 终端
    pub fn new_ssh(
        conn: StoredConnection,
        cx: &mut Context<Self>,
        working_dir: Option<&str>,
        sync_path_with_terminal: bool,
    ) -> Self {
        let (event_tx, event_rx) = unbounded_channel::<TerminalEvent>();
        let resolved = resolve_ssh_connection(
            SshConnectionUpdate {
                connection: conn,
                working_dir: working_dir.map(str::to_string),
                sync_path_with_terminal,
            },
            event_tx.clone(),
        )
        .expect("StoredConnection should contain valid SSH params");
        let recording_session_metadata = ssh_recording_session_metadata(
            resolved.connection_id,
            resolved.connection_name.clone(),
            resolved.config.ssh_config.username.clone(),
            resolved.config.ssh_config.host.clone(),
            resolved.config.ssh_config.port,
        );
        let base_config = resolved.config;

        let cols = base_config.pty_config.width as usize;
        let rows = base_config.pty_config.height as usize;

        let app_settings = AppSettings::current(cx);
        let scrollback_lines = app_settings.terminal_scrollback_lines;
        let automatic_logging = app_settings.terminal_auto_session_logging;
        let (term, event_proxy, _colors, performance_metrics) =
            Self::create_term(cols, rows, scrollback_lines, event_tx.clone());
        let wakeup_pending = event_proxy.wakeup_pending_handle();
        let recording_runtime =
            Self::create_recording_runtime(event_tx.clone(), wakeup_pending.clone());
        let connection_generation = 1;
        let zmodem_responder = ZmodemResponder::new(event_tx.clone());

        Self::spawn_event_loop(event_rx, wakeup_pending.clone(), cx);
        let history_repository = Self::history_repository(cx);
        let history_scope = resolved.connection_id.map(TerminalHistoryScope::ssh);
        let recording_session_id = Self::new_recording_session_id();
        let session_log_runtime =
            Self::start_automatic_session_log(AutomaticSessionLogRuntimeInput {
                enabled: automatic_logging,
                event_tx: event_tx.clone(),
                wakeup_pending: wakeup_pending.clone(),
                backend: RecordingBackend::Ssh,
                session_id: recording_session_id.clone(),
                initial_size: TerminalSize {
                    rows: u16::try_from(base_config.pty_config.height).unwrap_or(u16::MAX),
                    cols: u16::try_from(base_config.pty_config.width).unwrap_or(u16::MAX),
                    pixel_width: 0,
                    pixel_height: 0,
                },
                session: recording_session_metadata.clone(),
            });

        let mut terminal = Self {
            term,
            session_mode: TerminalSessionMode::Live,
            performance_metrics,
            backend: None,
            recording_runtime,
            session_log_runtime,
            playback_runtime: None,
            recording_session_id,
            recording_session_metadata,
            title: String::new(),
            current_working_dir: None,
            child_exited: None,
            connection_state: ConnectionState::Connecting,
            session_lock: None,
            cols,
            rows,
            pixel_width: 0,
            pixel_height: 0,
            ssh_config: Some(base_config.clone()),
            ssh_base_config: Some(base_config.clone()),
            ssh_session_manager: None,
            ssh_credential_prompt_policy: resolved.credential_prompt_policy,
            ssh_credential_request: None,
            ssh_keyboard_interactive_enabled: resolved.keyboard_interactive_enabled,
            ssh_mfa_responder: None,
            zmodem_responder: Some(zmodem_responder),
            pending_host_key_verification: None,
            serial_params: None,
            telnet_params: None,
            telnet_base_params: None,
            telnet_credential_request: None,
            wakeup_pending,
            event_tx: Some(event_tx.clone()),
            event_proxy: Some(event_proxy),
            connection_id: resolved.connection_id,
            connection_name: Some(resolved.connection_name),
            init_commands: resolved.init_commands,
            session_history: VecDeque::new(),
            persisted_history: Vec::new(),
            history_repository,
            history_scope,
            command_record_gate: CommandRecordGate::default(),
            connection_generation,
            connection_kind: TerminalConnectionKind::Ssh,
            scrollback_lines,
        };

        if terminal.ssh_credential_prompt_policy.requires_credentials() {
            terminal.ssh_credential_request = Some(TerminalSshCredentialRequest {
                generation: connection_generation,
                username: terminal.ssh_credential_prompt_policy.username,
                password: terminal.ssh_credential_prompt_policy.password,
            });
        } else {
            let (runtime_config, responder) = ssh_config_with_runtime_credentials(
                &base_config,
                &TerminalSshCredentials::default(),
                event_tx,
                resolved.keyboard_interactive_enabled,
            )
            .expect("stored SSH config should produce a runtime config");
            assert!(
                terminal.start_ssh_connection_attempt(
                    runtime_config,
                    responder,
                    connection_generation,
                    cx,
                ),
                "SSH terminal runtime should be available during construction"
            );
        }

        terminal
    }

    /// 创建串口终端
    pub fn new_serial(conn: StoredConnection, cx: &mut Context<Self>) -> Self {
        let serial_params = conn
            .to_serial_params()
            .expect("StoredConnection 应包含有效的 SerialParams");
        let recording_session_metadata = serial_recording_session_metadata(
            conn.id,
            conn.name.clone(),
            serial_params.port_name.clone(),
        );

        let (event_tx, event_rx) = unbounded_channel::<TerminalEvent>();
        let app_settings = AppSettings::current(cx);
        let scrollback_lines = app_settings.terminal_scrollback_lines;
        let automatic_logging = app_settings.terminal_auto_session_logging;
        let (term, event_proxy, _colors, performance_metrics) = Self::create_term(
            DEFAULT_COLS,
            DEFAULT_ROWS,
            scrollback_lines,
            event_tx.clone(),
        );
        let wakeup_pending = event_proxy.wakeup_pending_handle();
        let recording_runtime =
            Self::create_recording_runtime(event_tx.clone(), wakeup_pending.clone());
        let recording_session_id = Self::new_recording_session_id();
        let session_log_runtime =
            Self::start_automatic_session_log(AutomaticSessionLogRuntimeInput {
                enabled: automatic_logging,
                event_tx: event_tx.clone(),
                wakeup_pending: wakeup_pending.clone(),
                backend: RecordingBackend::Serial,
                session_id: recording_session_id.clone(),
                initial_size: TerminalSize {
                    rows: DEFAULT_ROWS as u16,
                    cols: DEFAULT_COLS as u16,
                    pixel_width: 0,
                    pixel_height: 0,
                },
                session: recording_session_metadata.clone(),
            });
        let recording_tap =
            Self::recording_tap_for_runtimes(&recording_runtime, &session_log_runtime);
        let (disconnect_tx, disconnect_rx) = tokio::sync::oneshot::channel::<()>();
        let connection_generation = 1;

        Self::spawn_disconnect_handler(disconnect_rx, connection_generation, cx);
        Self::spawn_event_loop(event_rx, wakeup_pending.clone(), cx);
        Self::spawn_serial_connect(
            serial_params.clone(),
            term.clone(),
            event_proxy.clone(),
            performance_metrics.clone(),
            Some(disconnect_tx),
            recording_tap,
            connection_generation,
            cx,
        );

        Self {
            term,
            session_mode: TerminalSessionMode::Live,
            performance_metrics,
            backend: None,
            recording_runtime,
            session_log_runtime,
            playback_runtime: None,
            recording_session_id,
            recording_session_metadata,
            title: String::new(),
            current_working_dir: None,
            child_exited: None,
            connection_state: ConnectionState::Connecting,
            session_lock: None,
            cols: DEFAULT_COLS,
            rows: DEFAULT_ROWS,
            pixel_width: 0,
            pixel_height: 0,
            ssh_config: None,
            ssh_base_config: None,
            ssh_session_manager: None,
            ssh_credential_prompt_policy: SshCredentialPromptPolicy::default(),
            ssh_credential_request: None,
            ssh_keyboard_interactive_enabled: false,
            ssh_mfa_responder: None,
            zmodem_responder: None,
            pending_host_key_verification: None,
            serial_params: Some(serial_params),
            telnet_params: None,
            telnet_base_params: None,
            telnet_credential_request: None,
            wakeup_pending,
            event_tx: Some(event_tx),
            event_proxy: Some(event_proxy),
            connection_id: conn.id,
            connection_name: Some(conn.name),
            init_commands: None,
            session_history: VecDeque::new(),
            persisted_history: Vec::new(),
            history_repository: None,
            history_scope: None,
            command_record_gate: CommandRecordGate::default(),
            connection_generation,
            connection_kind: TerminalConnectionKind::Serial,
            scrollback_lines,
        }
    }

    pub fn new_telnet(conn: StoredConnection, cx: &mut Context<Self>) -> Self {
        let telnet_params = match parse_stored_telnet_params(&conn) {
            Ok(params) => params,
            Err(message) => {
                let mut terminal = Self::new_local_disconnected(message, cx);
                terminal.connection_kind = TerminalConnectionKind::Telnet;
                terminal.connection_id = conn.id;
                terminal.connection_name = Some(conn.name.clone());
                terminal.title = conn.name;
                return terminal;
            }
        };
        let recording_session_metadata = telnet_recording_session_metadata(
            conn.id,
            conn.name.clone(),
            telnet_params.host.clone(),
            telnet_params.port,
        );

        let (event_tx, event_rx) = unbounded_channel::<TerminalEvent>();
        let app_settings = AppSettings::current(cx);
        let scrollback_lines = app_settings.terminal_scrollback_lines;
        let automatic_logging = app_settings.terminal_auto_session_logging;
        let (term, event_proxy, _colors, performance_metrics) = Self::create_term(
            DEFAULT_COLS,
            DEFAULT_ROWS,
            scrollback_lines,
            event_tx.clone(),
        );
        let wakeup_pending = event_proxy.wakeup_pending_handle();
        let recording_runtime =
            Self::create_recording_runtime(event_tx.clone(), wakeup_pending.clone());
        let recording_session_id = Self::new_recording_session_id();
        let session_log_runtime =
            Self::start_automatic_session_log(AutomaticSessionLogRuntimeInput {
                enabled: automatic_logging,
                event_tx: event_tx.clone(),
                wakeup_pending: wakeup_pending.clone(),
                backend: RecordingBackend::Telnet,
                session_id: recording_session_id.clone(),
                initial_size: TerminalSize {
                    rows: DEFAULT_ROWS as u16,
                    cols: DEFAULT_COLS as u16,
                    pixel_width: 0,
                    pixel_height: 0,
                },
                session: recording_session_metadata.clone(),
            });
        let recording_tap =
            Self::recording_tap_for_runtimes(&recording_runtime, &session_log_runtime);
        let connection_generation = 1;
        let prompt_username = telnet_params.prompts_for_username();
        let prompt_password = telnet_params.prompts_for_password();
        let requires_credentials = prompt_username || prompt_password;

        Self::spawn_event_loop(event_rx, wakeup_pending.clone(), cx);
        if !requires_credentials {
            let (disconnect_tx, disconnect_rx) = tokio::sync::oneshot::channel::<Option<String>>();
            Self::spawn_telnet_disconnect_handler(disconnect_rx, connection_generation, cx);
            Self::spawn_telnet_connect(
                telnet_params.clone(),
                term.clone(),
                event_proxy.clone(),
                performance_metrics.clone(),
                Some(disconnect_tx),
                recording_tap,
                connection_generation,
                cx,
            );
        }

        Self {
            term,
            session_mode: TerminalSessionMode::Live,
            performance_metrics,
            backend: None,
            recording_runtime,
            session_log_runtime,
            playback_runtime: None,
            recording_session_id,
            recording_session_metadata,
            title: String::new(),
            current_working_dir: None,
            child_exited: None,
            connection_state: ConnectionState::Connecting,
            session_lock: None,
            cols: DEFAULT_COLS,
            rows: DEFAULT_ROWS,
            pixel_width: 0,
            pixel_height: 0,
            ssh_config: None,
            ssh_base_config: None,
            ssh_session_manager: None,
            ssh_credential_prompt_policy: SshCredentialPromptPolicy::default(),
            ssh_credential_request: None,
            ssh_keyboard_interactive_enabled: false,
            ssh_mfa_responder: None,
            zmodem_responder: None,
            pending_host_key_verification: None,
            serial_params: None,
            telnet_params: Some(telnet_params.clone()),
            telnet_base_params: Some(telnet_params),
            telnet_credential_request: requires_credentials.then_some(
                TerminalTelnetCredentialRequest {
                    generation: connection_generation,
                    username: prompt_username,
                    password: prompt_password,
                },
            ),
            wakeup_pending,
            event_tx: Some(event_tx),
            event_proxy: Some(event_proxy),
            connection_id: conn.id,
            connection_name: Some(conn.name),
            init_commands: None,
            session_history: VecDeque::new(),
            persisted_history: Vec::new(),
            history_repository: None,
            history_scope: None,
            command_record_gate: CommandRecordGate::default(),
            connection_generation,
            connection_kind: TerminalConnectionKind::Telnet,
            scrollback_lines,
        }
    }

    /// Creates a terminal surface that renders an untrusted recording without
    /// recreating any live PTY, SSH, serial, input, exec, or control capability.
    pub fn new_recording_playback(playback: RecordingPlayback, cx: &mut Context<Self>) -> Self {
        let scrollback_lines = AppSettings::current(cx).terminal_scrollback_lines;
        let (terminal, event_loop) = Self::build_recording_playback(playback, scrollback_lines);
        Self::spawn_event_loop(event_loop.event_rx, event_loop.wakeup_pending, cx);
        terminal
    }

    /// Creates a static, read-only terminal history from a session-log
    /// artifact. The complete output stream is materialized once and no
    /// playback timeline or live terminal capability is exposed.
    pub fn new_session_log(playback: RecordingPlayback, cx: &mut Context<Self>) -> Self {
        let scrollback_lines = AppSettings::current(cx).terminal_scrollback_lines;
        let (terminal, event_loop) = Self::build_session_log(playback, scrollback_lines);
        Self::spawn_event_loop(event_loop.event_rx, event_loop.wakeup_pending, cx);
        terminal
    }

    /// Builds the capability-free playback model independently from its GPUI
    /// event-loop attachment. Keeping this step synchronous makes the security
    /// boundary unit-testable without starting a real Tokio worker inside
    /// GPUI's deterministic test scheduler.
    fn build_recording_playback(
        playback: RecordingPlayback,
        scrollback_lines: usize,
    ) -> (Self, PendingPlaybackEventLoop) {
        Self::build_read_only_recording_surface(
            playback,
            scrollback_lines,
            TerminalSessionMode::RecordingPlayback,
            false,
        )
    }

    fn build_session_log(
        playback: RecordingPlayback,
        scrollback_lines: usize,
    ) -> (Self, PendingPlaybackEventLoop) {
        Self::build_read_only_recording_surface(
            playback,
            scrollback_lines,
            TerminalSessionMode::SessionLog,
            true,
        )
    }

    fn build_read_only_recording_surface(
        playback: RecordingPlayback,
        scrollback_lines: usize,
        session_mode: TerminalSessionMode,
        materialize_all: bool,
    ) -> (Self, PendingPlaybackEventLoop) {
        debug_assert!(session_mode != TerminalSessionMode::Live);
        let source_backend = playback.recording().header.navop.backend;
        let connection_kind = match source_backend {
            RecordingBackend::Local => TerminalConnectionKind::Local,
            RecordingBackend::Ssh => TerminalConnectionKind::Ssh,
            RecordingBackend::Serial => TerminalConnectionKind::Serial,
            RecordingBackend::Telnet => TerminalConnectionKind::Telnet,
        };
        let scrollback_lines = AppSettings::normalize_terminal_scrollback_lines(scrollback_lines);
        let (event_tx, event_rx) = unbounded_channel::<TerminalEvent>();
        let performance_metrics = Arc::new(TerminalPerformanceMetrics::default());
        let mut playback_runtime = TerminalPlaybackRuntime::new(
            playback,
            scrollback_lines,
            event_tx.clone(),
            performance_metrics.clone(),
        );
        let initial_size = playback_runtime.initial_size();
        if materialize_all {
            playback_runtime.materialize_all();
        }
        let term = playback_runtime.term().clone();
        let (cols, rows) = if materialize_all {
            let term = term.lock();
            (term.columns(), term.screen_lines())
        } else {
            (
                usize::from(initial_size.cols),
                usize::from(initial_size.rows),
            )
        };
        let wakeup_pending = playback_runtime.wakeup_pending_handle();

        (
            Self {
                term,
                session_mode,
                performance_metrics,
                backend: None,
                recording_runtime: Err(RecordingRuntimeError::ReadOnlyPlayback),
                session_log_runtime: None,
                playback_runtime: Some(playback_runtime),
                recording_session_id: Self::new_recording_session_id(),
                recording_session_metadata: RecordingSessionMetadata::default(),
                title: String::new(),
                current_working_dir: None,
                child_exited: None,
                // Connected suppresses the live reconnect overlay. Capability
                // checks still fail closed through `session_mode`.
                connection_state: ConnectionState::Connected,
                session_lock: None,
                cols,
                rows,
                pixel_width: 0,
                pixel_height: 0,
                ssh_config: None,
                ssh_base_config: None,
                ssh_session_manager: None,
                ssh_credential_prompt_policy: SshCredentialPromptPolicy::default(),
                ssh_credential_request: None,
                ssh_keyboard_interactive_enabled: false,
                ssh_mfa_responder: None,
                zmodem_responder: None,
                pending_host_key_verification: None,
                serial_params: None,
                telnet_params: None,
                telnet_base_params: None,
                telnet_credential_request: None,
                wakeup_pending: wakeup_pending.clone(),
                event_tx: Some(event_tx),
                event_proxy: None,
                connection_id: None,
                connection_name: None,
                init_commands: None,
                session_history: VecDeque::new(),
                persisted_history: Vec::new(),
                history_repository: None,
                history_scope: None,
                command_record_gate: CommandRecordGate::default(),
                connection_generation: 0,
                // This is source-format metadata for display only. It does not
                // confer live capabilities; use `live_connection_kind`.
                connection_kind,
                scrollback_lines,
            },
            PendingPlaybackEventLoop {
                event_rx,
                wakeup_pending,
            },
        )
    }

    fn history_repository(cx: &mut Context<Self>) -> Option<Arc<TerminalCommandHistoryRepository>> {
        cx.try_global::<GlobalStorageState>()
            .and_then(|state| state.storage.get::<TerminalCommandHistoryRepository>())
    }

    fn next_connection_generation(&mut self) -> u64 {
        self.connection_generation = self.connection_generation.saturating_add(1).max(1);
        self.connection_generation
    }

    fn is_current_connection_generation(&self, generation: u64) -> bool {
        self.connection_generation == generation
    }

    fn create_term(
        cols: usize,
        rows: usize,
        scrollback_lines: usize,
        event_tx: UnboundedSender<TerminalEvent>,
    ) -> (
        Arc<FairMutex<Term<GpuiEventProxy>>>,
        GpuiEventProxy,
        alacritty_terminal::term::color::Colors,
        Arc<TerminalPerformanceMetrics>,
    ) {
        Self::create_term_with_metrics(
            cols,
            rows,
            scrollback_lines,
            event_tx,
            Arc::new(TerminalPerformanceMetrics::for_runtime()),
        )
    }

    fn create_term_with_metrics(
        cols: usize,
        rows: usize,
        scrollback_lines: usize,
        event_tx: UnboundedSender<TerminalEvent>,
        performance_metrics: Arc<TerminalPerformanceMetrics>,
    ) -> (
        Arc<FairMutex<Term<GpuiEventProxy>>>,
        GpuiEventProxy,
        alacritty_terminal::term::color::Colors,
        Arc<TerminalPerformanceMetrics>,
    ) {
        let term_config = TermConfig {
            scrolling_history: scrollback_lines,
            ..Default::default()
        };
        let event_proxy = GpuiEventProxy::with_metrics(event_tx, performance_metrics.clone());
        let term = Term::new(
            term_config,
            &TermDimensions { cols, rows },
            event_proxy.clone(),
        );
        let colors = term.colors().clone();
        (
            Arc::new(FairMutex::new(term)),
            event_proxy,
            colors,
            performance_metrics,
        )
    }

    pub fn set_scrollback_lines(&mut self, lines: usize) {
        let lines = AppSettings::normalize_terminal_scrollback_lines(lines);
        if self.scrollback_lines == lines {
            return;
        }

        self.scrollback_lines = lines;
        if let Some(playback_runtime) = self.playback_runtime.as_mut() {
            playback_runtime.set_scrollback_lines(lines);
        } else {
            self.term.lock().set_options(TermConfig {
                scrolling_history: lines,
                ..Default::default()
            });
        }
    }

    pub fn scrollback_lines(&self) -> usize {
        self.scrollback_lines
    }

    fn spawn_local_history_loader(preferred_shell: Option<&str>, cx: &mut Context<Self>) {
        let preferred_shell = preferred_shell.map(str::to_string);
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let history = load_local_history(preferred_shell.as_deref());
            let _ = this.update(cx, |terminal, cx| {
                terminal.set_persisted_history(history, cx);
            });
        })
        .detach();
    }

    fn spawn_ssh_history_loader(manager: Arc<SshSessionManager>, cx: &mut Context<Self>) {
        let task = Tokio::spawn(cx, async move { load_ssh_history(manager).await });

        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let Ok(Ok(history)) = task.await else {
                return;
            };
            let _ = this.update(cx, |terminal, cx| {
                terminal.set_persisted_history(history, cx);
            });
        })
        .detach();
    }

    fn spawn_event_loop(
        mut event_rx: UnboundedReceiver<TerminalEvent>,
        wakeup_pending: Arc<AtomicBool>,
        cx: &mut Context<Self>,
    ) {
        let (render_tx, mut render_rx) = futures::channel::mpsc::unbounded::<TerminalEvent>();

        // 后台事件聚合任务 - 8ms 节流
        Tokio::spawn(cx, async move {
            let mut render_interval = interval(Duration::from_millis(8));
            let mut pending_wakeup = false;
            let mut pending_events: Vec<TerminalEvent> = Vec::new();

            loop {
                tokio::select! {
                    result = event_rx.recv() => {
                        match result {
                            None => {
                                // The producer can close between render ticks.
                                // Forward the final semantic events and Wakeup
                                // instead of silently dropping the tail.
                                let _ = flush_pending_terminal_events(
                                    &render_tx,
                                    &mut pending_events,
                                    &mut pending_wakeup,
                                );
                                break;
                            }
                            Some(event) => {
                                match &event {
                                    TerminalEvent::Wakeup => pending_wakeup = true,
                                    _ => pending_events.push(event),
                                }
                            }
                        }
                    }
                    _ = render_interval.tick() => {
                        if !flush_pending_terminal_events(
                            &render_tx,
                            &mut pending_events,
                            &mut pending_wakeup,
                        ) {
                            return;
                        }
                    }
                }
            }
        })
        .detach();

        // GPUI 线程事件处理
        cx.spawn(async move |this, cx| {
            while let Some(event) =
                receive_terminal_event_for_gpui(&mut render_rx, &wakeup_pending).await
            {
                if this
                    .update(cx, |this, cx| {
                        this.handle_terminal_event(event, cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn spawn_disconnect_handler(
        disconnect_rx: tokio::sync::oneshot::Receiver<()>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        let entity = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let _ = disconnect_rx.await;
            let _ = entity.update(cx, |this, cx| {
                if !this.is_current_connection_generation(generation) {
                    return;
                }
                this.connection_state = ConnectionState::Disconnected { error: None };
                this.backend = None;
                this.set_connection_active(false, cx);
                cx.emit(TerminalModelEvent::Wakeup);
            });
        })
        .detach();
    }

    fn spawn_telnet_disconnect_handler(
        disconnect_rx: tokio::sync::oneshot::Receiver<Option<String>>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        let entity = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let Ok(error) = disconnect_rx.await else {
                // Backend creation failed before the worker was installed.
                // handle_telnet_result owns that error and must not be overwritten.
                return;
            };
            let _ = entity.update(cx, |this, cx| {
                if !this.is_current_connection_generation(generation) {
                    return;
                }
                this.connection_state = ConnectionState::Disconnected { error };
                this.backend = None;
                this.set_connection_active(false, cx);
                cx.emit(TerminalModelEvent::Wakeup);
            });
        })
        .detach();
    }

    fn spawn_ssh_disconnect_handler(
        disconnect_rx: tokio::sync::oneshot::Receiver<Option<String>>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        let entity = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let Ok(error) = disconnect_rx.await else {
                // Backend creation failed before the runtime actor was installed.
                // handle_ssh_result owns that error and must not be overwritten.
                return;
            };
            let _ = entity.update(cx, |this, cx| {
                if !this.is_current_connection_generation(generation) {
                    return;
                }
                this.connection_state = ConnectionState::Disconnected { error };
                this.backend = None;
                this.set_connection_active(false, cx);
                cx.emit(TerminalModelEvent::Wakeup);
            });
        })
        .detach();
    }

    fn spawn_ssh_connect(task: SshConnectTask, cx: &mut Context<Self>) {
        let SshConnectTask {
            session_manager,
            config,
            term,
            event_proxy,
            event_tx,
            zmodem_responder,
            connection_id,
            on_disconnect,
            init_commands,
            recording_tap,
            generation,
        } = task;
        let task = Tokio::spawn(cx, async move {
            let expect_username = config.ssh_config.username.clone();
            let expect_password = password_from_ssh_auth(&config.ssh_config.auth);
            let disconnect_tx = on_disconnect.map(|tx| {
                let (sender, mut receiver) = unbounded_channel::<Option<String>>();
                tokio::spawn(async move {
                    if let Some(error) = receiver.recv().await {
                        let _ = tx.send(error);
                    }
                });
                sender
            });
            SshBackend::connect_with_recording(
                SshBackendConnect {
                    session_manager,
                    pty_config: config.pty_config,
                    terminal_encoding: config.terminal_encoding,
                    connection_id,
                    term,
                    event_proxy,
                    event_tx,
                    on_disconnect: disconnect_tx,
                    init_commands,
                    account_expect: config.account_expect,
                    expect_username,
                    expect_password,
                    disable_shell_integration: config.disable_shell_integration,
                },
                recording_tap,
                zmodem_responder,
            )
            .await
        });

        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.handle_ssh_result(result, generation, cx);
            });
        })
        .detach();
    }

    fn queue_ssh_credential_request(&mut self, generation: u64, cx: &mut Context<Self>) {
        self.ssh_credential_request = Some(TerminalSshCredentialRequest {
            generation,
            username: self.ssh_credential_prompt_policy.username,
            password: self.ssh_credential_prompt_policy.password,
        });
        cx.emit(TerminalModelEvent::SshCredentialChanged);
        cx.emit(TerminalModelEvent::Wakeup);
    }

    fn queue_telnet_credential_request(&mut self, generation: u64, cx: &mut Context<Self>) -> bool {
        let Some(params) = self.telnet_base_params.as_ref() else {
            return false;
        };
        let username = params.prompts_for_username();
        let password = params.prompts_for_password();
        if !username && !password {
            return false;
        }

        self.telnet_credential_request = Some(TerminalTelnetCredentialRequest {
            generation,
            username,
            password,
        });
        cx.emit(TerminalModelEvent::TelnetCredentialChanged);
        cx.emit(TerminalModelEvent::Wakeup);
        true
    }

    fn start_telnet_connection_attempt(
        &mut self,
        params: TelnetParams,
        generation: u64,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(event_proxy) = self.event_proxy.clone() else {
            return false;
        };

        let (disconnect_tx, disconnect_rx) = tokio::sync::oneshot::channel::<Option<String>>();
        Self::spawn_telnet_disconnect_handler(disconnect_rx, generation, cx);
        Self::spawn_telnet_connect(
            params.clone(),
            self.term.clone(),
            event_proxy,
            self.performance_metrics.clone(),
            Some(disconnect_tx),
            self.recording_tap(),
            generation,
            cx,
        );

        self.telnet_params = Some(params);
        self.telnet_credential_request = None;
        cx.emit(TerminalModelEvent::TelnetCredentialChanged);
        cx.emit(TerminalModelEvent::Wakeup);
        true
    }

    fn start_ssh_connection_attempt(
        &mut self,
        config: SshTerminalConfig,
        responder: TerminalMfaResponder,
        generation: u64,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(event_tx) = self.event_tx.clone() else {
            return false;
        };
        let Some(event_proxy) = self.event_proxy.clone() else {
            return false;
        };
        let Some(zmodem_responder) = self.zmodem_responder.clone() else {
            return false;
        };

        let session_manager = Arc::new(SshSessionManager::new(config.ssh_config.clone()));
        let (disconnect_tx, disconnect_rx) = tokio::sync::oneshot::channel::<Option<String>>();
        Self::spawn_ssh_disconnect_handler(disconnect_rx, generation, cx);

        self.ssh_config = Some(config.clone());
        self.ssh_session_manager = Some(session_manager.clone());
        self.ssh_mfa_responder = Some(responder);
        let credential_request_cleared = self.ssh_credential_request.take().is_some();

        Self::spawn_ssh_connect(
            SshConnectTask {
                session_manager: session_manager.clone(),
                config,
                term: self.term.clone(),
                event_proxy,
                event_tx,
                zmodem_responder,
                connection_id: self.connection_id,
                on_disconnect: Some(disconnect_tx),
                init_commands: self.init_commands.clone(),
                recording_tap: self.recording_tap(),
                generation,
            },
            cx,
        );
        Self::spawn_ssh_history_loader(session_manager, cx);

        if credential_request_cleared {
            cx.emit(TerminalModelEvent::SshCredentialChanged);
        }
        cx.emit(TerminalModelEvent::Wakeup);
        true
    }

    fn handle_ssh_result(
        &mut self,
        result: Result<Result<SshBackend, anyhow::Error>, tokio::task::JoinError>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if !self.is_current_connection_generation(generation) {
            if let Ok(Ok(backend)) = result {
                backend.shutdown();
            }
            return;
        }

        match result {
            Ok(Ok(backend)) => {
                if !should_install_connected_backend(&self.connection_state) {
                    // worker 已在 backend 安装前上报断开：不得把状态覆盖回 Connected。
                    backend.shutdown();
                    return;
                }
                self.pending_host_key_verification = None;
                self.connection_state = ConnectionState::Connected;
                self.performance_metrics
                    .record_ssh_connect(is_reconnect_generation(generation));
                self.set_connection_active(true, cx);
                // 连接后重新调整终端大小
                self.term.lock().resize(TermDimensions {
                    cols: self.cols,
                    rows: self.rows,
                });
                // 重要：将当前终端尺寸同步到新连接的 SSH 后端
                // 因为远程 PTY 是用 PtyConfig 默认尺寸（80x24）创建的，
                // 需要调整到当前实际尺寸
                tracing::info!(
                    "SSH 连接成功，同步终端尺寸到远程: {}x{}",
                    self.cols,
                    self.rows
                );
                backend.resize(TerminalSize {
                    rows: self.rows as u16,
                    cols: self.cols as u16,
                    pixel_width: 0,
                    pixel_height: 0,
                });
                self.backend = Some(Box::new(backend));
            }
            Ok(Err(e)) => {
                if let Some(responder) = &self.ssh_mfa_responder {
                    responder.cancel();
                }
                if let Some(request) = host_key_verification_request(&e) {
                    self.pending_host_key_verification = Some(request);
                    self.connection_state = ConnectionState::Disconnected { error: None };
                    cx.emit(TerminalModelEvent::HostKeyVerificationRequired);
                } else {
                    self.pending_host_key_verification = None;
                    let detail = format_connection_error(&e);
                    tracing::error!(
                        target: "terminal.ssh.connect",
                        error = %detail,
                        error_debug = ?e,
                        "SSH connection failed"
                    );
                    self.connection_state = ConnectionState::Disconnected {
                        error: Some(detail),
                    };
                }
                self.set_connection_active(false, cx);
            }
            Err(e) => {
                if let Some(responder) = &self.ssh_mfa_responder {
                    responder.cancel();
                }
                self.pending_host_key_verification = None;
                let detail = format!("{e:#}");
                tracing::error!(
                    target: "terminal.ssh.connect",
                    error = %detail,
                    error_debug = ?e,
                    "SSH connection task failed"
                );
                self.connection_state = ConnectionState::Disconnected {
                    error: Some(detail),
                };
                self.set_connection_active(false, cx);
            }
        }
        cx.emit(TerminalModelEvent::Wakeup);
    }

    fn spawn_serial_connect(
        params: SerialParams,
        term: Arc<FairMutex<Term<GpuiEventProxy>>>,
        event_proxy: GpuiEventProxy,
        performance_metrics: Arc<TerminalPerformanceMetrics>,
        on_disconnect: Option<tokio::sync::oneshot::Sender<()>>,
        recording_tap: Option<RecordingTap>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        let disconnect_tx = on_disconnect.map(|tx| {
            let (sender, mut receiver) = unbounded_channel::<()>();
            Tokio::spawn(cx, async move {
                if receiver.recv().await.is_some() {
                    let _ = tx.send(());
                }
            })
            .detach();
            sender
        });

        let result = SerialBackend::connect_with_metrics_and_recording(
            params,
            term,
            event_proxy,
            disconnect_tx,
            performance_metrics,
            recording_tap,
        );

        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let _ = this.update(cx, |this, cx| {
                this.handle_serial_result(result, generation, cx);
            });
        })
        .detach();
    }

    fn handle_serial_result(
        &mut self,
        result: anyhow::Result<SerialBackend>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if !self.is_current_connection_generation(generation) {
            if let Ok(backend) = result {
                backend.shutdown();
            }
            return;
        }

        match result {
            Ok(backend) => {
                self.connection_state = ConnectionState::Connected;
                self.set_connection_active(true, cx);
                self.backend = Some(Box::new(backend));
                tracing::info!("串口连接成功");
            }
            Err(e) => {
                self.connection_state = ConnectionState::Disconnected {
                    error: Some(e.to_string()),
                };
                self.set_connection_active(false, cx);
            }
        }
        cx.emit(TerminalModelEvent::Wakeup);
    }

    fn spawn_telnet_connect(
        params: TelnetParams,
        term: Arc<FairMutex<Term<GpuiEventProxy>>>,
        event_proxy: GpuiEventProxy,
        performance_metrics: Arc<TerminalPerformanceMetrics>,
        on_disconnect: Option<tokio::sync::oneshot::Sender<Option<String>>>,
        recording_tap: Option<RecordingTap>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        let disconnect_tx = on_disconnect.map(|tx| {
            let (sender, mut receiver) = unbounded_channel::<Option<String>>();
            Tokio::spawn(cx, async move {
                if let Some(error) = receiver.recv().await {
                    let _ = tx.send(error);
                }
            })
            .detach();
            sender
        });

        // 与 SSH 一致：TCP 连接在后台完成，成功后 Terminal 才进入
        // Connected 状态，避免“先显示已连接、随后又变成断开”。
        let task = Tokio::spawn(cx, async move {
            TelnetBackend::connect_with_metrics_and_recording(
                params,
                term,
                event_proxy,
                disconnect_tx,
                performance_metrics,
                recording_tap,
            )
            .await
        });

        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.handle_telnet_result(result, generation, cx);
            });
        })
        .detach();
    }

    fn handle_telnet_result(
        &mut self,
        result: Result<Result<TelnetBackend, anyhow::Error>, tokio::task::JoinError>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if !self.is_current_connection_generation(generation) {
            if let Ok(Ok(backend)) = result {
                backend.shutdown();
            }
            return;
        }

        match result {
            Ok(Ok(backend)) => {
                if !should_install_connected_backend(&self.connection_state) {
                    // worker 已在 backend 安装前上报断开：不得把状态覆盖回 Connected。
                    backend.shutdown();
                    return;
                }
                self.connection_state = ConnectionState::Connected;
                self.set_connection_active(true, cx);
                // 连接过程是异步的，连接完成时终端尺寸可能已经变化；
                // 与 SSH 一样把当前尺寸同步到新后端。
                self.term.lock().resize(TermDimensions {
                    cols: self.cols,
                    rows: self.rows,
                });
                backend.resize(TerminalSize {
                    rows: self.rows as u16,
                    cols: self.cols as u16,
                    pixel_width: self.pixel_width,
                    pixel_height: self.pixel_height,
                });
                self.backend = Some(Box::new(backend));
                tracing::info!("Telnet 连接成功");
            }
            Ok(Err(error)) => {
                tracing::error!(
                    target: "terminal.telnet.connect",
                    error = %error,
                    "Telnet connection failed"
                );
                self.connection_state = ConnectionState::Disconnected {
                    error: Some(error.to_string()),
                };
                self.set_connection_active(false, cx);
            }
            Err(error) => {
                tracing::error!(
                    target: "terminal.telnet.connect",
                    error = %error,
                    "Telnet connect task failed"
                );
                self.connection_state = ConnectionState::Disconnected {
                    error: Some(error.to_string()),
                };
                self.set_connection_active(false, cx);
            }
        }
        cx.emit(TerminalModelEvent::Wakeup);
    }

    fn set_connection_active(&self, active: bool, cx: &mut Context<Self>) {
        let Some(connection_id) = self.connection_id else {
            return;
        };

        let global_state = cx.global_mut::<ActiveConnections>();
        if active {
            global_state.add(connection_id);
        } else {
            global_state.remove(connection_id);
        }
    }

    fn record_successful_history_entry(
        &mut self,
        command: &str,
        exit_code: i32,
        cx: &mut Context<Self>,
    ) {
        let history_user = self.history_record_user();
        let Some(command) = normalize_recorded_command(command, history_user.as_deref()) else {
            return;
        };
        let cwd = self.current_working_dir.clone();
        let entry = HistoryEntry::new(command.clone())
            .with_cwd(cwd.clone())
            .with_exit_code(Some(exit_code));
        let changed =
            push_rich_history_entry(&mut self.session_history, entry, SESSION_HISTORY_LIMIT);
        let mut command_history_changed = false;
        if let (Some(repo), Some(scope)) = (&self.history_repository, &self.history_scope) {
            match repo.record_success(scope, &command, cwd.as_deref(), Some(exit_code)) {
                Ok(Some(_)) => command_history_changed = true,
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(%error, "failed to record terminal command history");
                }
            }
        }
        if command_history_changed {
            cx.emit(TerminalModelEvent::CommandHistoryChanged);
        }
        if changed {
            cx.emit(TerminalModelEvent::Wakeup);
        }
    }

    fn history_record_user(&self) -> Option<String> {
        match self.connection_kind {
            TerminalConnectionKind::Ssh => self
                .ssh_config
                .as_ref()
                .map(|config| config.ssh_config.username.clone()),
            TerminalConnectionKind::Local => std::env::var("USER").ok(),
            TerminalConnectionKind::Serial | TerminalConnectionKind::Telnet => None,
        }
    }

    fn accept_recorded_command(&mut self, accepted: Option<(String, i32)>, cx: &mut Context<Self>) {
        if let Some((command, exit_code)) = accepted {
            self.record_successful_history_entry(&command, exit_code, cx);
        }
    }

    fn set_persisted_history(&mut self, history: Vec<String>, cx: &mut Context<Self>) {
        let history = history
            .into_iter()
            .filter_map(|command| crate::history::normalize_history_command(&command))
            .rev()
            .take(PERSISTED_HISTORY_LIMIT)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>();

        if self.persisted_history != history {
            self.persisted_history = history;
            cx.emit(TerminalModelEvent::Wakeup);
        }
    }

    fn handle_terminal_event(&mut self, event: TerminalEvent, cx: &mut Context<Self>) {
        match event {
            TerminalEvent::Wakeup => {
                cx.emit(TerminalModelEvent::Wakeup);
            }
            TerminalEvent::SshMfaChanged => {
                cx.emit(TerminalModelEvent::SshMfaChanged);
            }
            TerminalEvent::ZmodemRequestChanged => {
                cx.emit(TerminalModelEvent::ZmodemRequestChanged);
            }
            TerminalEvent::ZmodemProgressChanged(progress) => {
                cx.emit(TerminalModelEvent::ZmodemProgressChanged(progress));
            }
            TerminalEvent::ZmodemTransferFinished {
                transfer_id,
                outcome,
                progress,
            } => {
                cx.emit(TerminalModelEvent::ZmodemTransferFinished {
                    transfer_id,
                    outcome,
                    progress,
                });
            }
            TerminalEvent::PromptStart => {
                cx.emit(TerminalModelEvent::PromptStart);
            }
            TerminalEvent::InputStart => {
                cx.emit(TerminalModelEvent::InputStart);
            }
            TerminalEvent::CommandStart => {
                self.command_record_gate.command_started();
                cx.emit(TerminalModelEvent::CommandStart);
            }
            TerminalEvent::TitleChanged(title) => {
                self.title = title.clone();
                cx.emit(TerminalModelEvent::TitleChanged(title));
            }
            TerminalEvent::Bell => {
                cx.emit(TerminalModelEvent::Bell);
            }
            TerminalEvent::ChildExit(code) => {
                self.child_exited = Some(code);
                cx.emit(TerminalModelEvent::ChildExit(code));
            }
            TerminalEvent::ClipboardStore(_ty, data) => {
                cx.emit(TerminalModelEvent::ClipboardStore(data));
            }
            TerminalEvent::ClipboardLoad(_ty) => {
                // 剪贴板加载由 TerminalView 处理
            }
            TerminalEvent::WorkingDirChanged(path) => {
                self.current_working_dir = Some(path.clone());
                cx.emit(TerminalModelEvent::WorkingDirChanged(path));
            }
            TerminalEvent::CommandFinished { exit_code } => {
                tracing::debug!("命令执行完毕，退出码: {}", exit_code);
                let accepted = self.command_record_gate.command_finished(exit_code);
                self.accept_recorded_command(accepted, cx);
            }
            TerminalEvent::CommandRecorded(command) => {
                let accepted = self.command_record_gate.command_recorded(command);
                self.accept_recorded_command(accepted, cx);
            }
        }
    }

    // ========== 公共 API ==========

    /// 获取 Term 的共享引用
    pub fn term(&self) -> &Arc<FairMutex<Term<GpuiEventProxy>>> {
        &self.term
    }

    /// 获取终端标题
    pub fn title(&self) -> &str {
        &self.title
    }

    /// 获取子进程退出码
    pub fn child_exited(&self) -> Option<i32> {
        self.child_exited
    }

    /// 获取连接状态
    pub fn connection_state(&self) -> &ConnectionState {
        &self.connection_state
    }

    /// 获取连接名称
    pub fn connection_name(&self) -> Option<&str> {
        self.connection_name.as_deref()
    }

    /// 获取连接 ID
    pub fn connection_id(&self) -> Option<i64> {
        self.connection_id
    }

    /// 获取当前工作目录（由 OSC 7 更新，仅 SSH 终端）
    pub fn current_working_dir(&self) -> Option<&str> {
        self.current_working_dir.as_deref()
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    /// 获取当前可见屏幕文本，供外部只读集成使用。
    pub fn visible_text(&self) -> String {
        visible_text_from_term(&self.term)
    }

    pub fn history_suggestions(&self, prefix: &str, limit: usize) -> Vec<String> {
        let history_user = self.history_record_user();
        let db_matches = self
            .history_repository
            .as_ref()
            .zip(self.history_scope.as_ref())
            .and_then(|(repo, scope)| repo.suggestions(scope, prefix, limit).ok())
            .unwrap_or_default();
        let db_matches = normalize_history_matches(db_matches, history_user.as_deref(), limit);
        let fallback = collect_history_suggestions_with_cwd(
            &self.session_history,
            &self.persisted_history,
            prefix,
            limit,
            self.current_working_dir.as_deref(),
        );
        merge_history_matches(db_matches, fallback, limit)
    }

    pub fn recent_history(&self, limit: usize) -> Vec<String> {
        collect_recent_history(&self.session_history, &self.persisted_history, limit)
    }

    pub fn history_search_results(&self, query: &str, limit: usize) -> Vec<String> {
        let history_user = self.history_record_user();
        let db_matches = self
            .history_repository
            .as_ref()
            .zip(self.history_scope.as_ref())
            .and_then(|(repo, scope)| {
                repo.list(
                    scope,
                    TerminalCommandHistorySort::Latest,
                    (!query.trim().is_empty()).then_some(query),
                    limit,
                )
                .ok()
            })
            .unwrap_or_default()
            .into_iter()
            .map(|item| item.command)
            .collect();
        let db_matches = normalize_history_matches(db_matches, history_user.as_deref(), limit);
        let fallback = collect_history_search_results(
            &self.session_history,
            &self.persisted_history,
            query,
            limit,
        );
        merge_history_matches(db_matches, fallback, limit)
    }

    pub fn record_command(&mut self, command: &str, cx: &mut Context<Self>) {
        if self.is_read_only() {
            return;
        }
        self.record_successful_history_entry(command, 0, cx);
    }

    /// 获取 SSH 连接配置（仅 SSH 终端）
    pub fn ssh_config(&self) -> Option<&SshTerminalConfig> {
        self.ssh_config.as_ref()
    }

    pub fn ssh_session_manager(&self) -> Option<&Arc<SshSessionManager>> {
        self.ssh_session_manager.as_ref()
    }

    pub fn apply_ssh_connection_update(&mut self, update: SshConnectionUpdate) -> Result<()> {
        if self.is_read_only() {
            return Err(anyhow!(
                "recording playback sessions cannot update SSH connections"
            ));
        }
        let event_tx = self
            .event_tx
            .clone()
            .ok_or_else(|| anyhow!("SSH terminal event channel is unavailable"))?;
        let resolved = resolve_ssh_connection(update, event_tx)?;

        if let Some(responder) = &self.ssh_mfa_responder {
            responder.cancel();
        }
        self.ssh_credential_request = None;
        self.ssh_credential_prompt_policy = resolved.credential_prompt_policy;
        self.ssh_keyboard_interactive_enabled = resolved.keyboard_interactive_enabled;
        self.ssh_base_config = Some(resolved.config.clone());

        if resolved.credential_prompt_policy.requires_credentials() {
            self.ssh_config = Some(resolved.config);
            self.ssh_mfa_responder = None;
        } else {
            let event_tx = self
                .event_tx
                .clone()
                .ok_or_else(|| anyhow!("SSH terminal event channel is unavailable"))?;
            let (runtime_config, responder) = ssh_config_with_runtime_credentials(
                &resolved.config,
                &TerminalSshCredentials::default(),
                event_tx,
                resolved.keyboard_interactive_enabled,
            )?;
            if let Some(session_manager) = &self.ssh_session_manager {
                session_manager.replace_config(runtime_config.ssh_config.clone());
            }
            self.ssh_config = Some(runtime_config);
            self.ssh_mfa_responder = Some(responder);
        }
        self.connection_id = resolved.connection_id;
        self.connection_name = Some(resolved.connection_name);
        self.init_commands = resolved.init_commands;
        self.history_scope = resolved.connection_id.map(TerminalHistoryScope::ssh);
        Ok(())
    }

    pub fn ssh_credential_request(&self) -> Option<TerminalSshCredentialRequest> {
        self.ssh_credential_request.clone()
    }

    pub fn submit_ssh_credentials(
        &mut self,
        generation: u64,
        credentials: TerminalSshCredentials,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(request) = self.ssh_credential_request.clone() else {
            return false;
        };
        if generation != request.generation
            || !self.is_current_connection_generation(request.generation)
        {
            return false;
        }

        let username = if request.username {
            let Some(username) = credentials.username.as_deref().map(str::trim) else {
                return false;
            };
            if username.is_empty() {
                return false;
            }
            Some(username.to_string())
        } else {
            None
        };
        let password = if request.password {
            let Some(password) = credentials.password else {
                return false;
            };
            if password.is_empty() {
                return false;
            }
            Some(password)
        } else {
            None
        };

        let Some(base_config) = self.ssh_base_config.clone() else {
            return false;
        };
        let Some(event_tx) = self.event_tx.clone() else {
            return false;
        };
        let Ok((runtime_config, responder)) = ssh_config_with_runtime_credentials(
            &base_config,
            &TerminalSshCredentials { username, password },
            event_tx,
            self.ssh_keyboard_interactive_enabled,
        ) else {
            return false;
        };

        self.connection_state = ConnectionState::Connecting;
        self.set_connection_active(false, cx);
        self.start_ssh_connection_attempt(runtime_config, responder, generation, cx)
    }

    pub fn telnet_credential_request(&self) -> Option<TerminalTelnetCredentialRequest> {
        self.telnet_credential_request.clone()
    }

    pub fn submit_telnet_credentials(
        &mut self,
        generation: u64,
        credentials: TerminalTelnetCredentials,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(request) = self.telnet_credential_request.clone() else {
            return false;
        };
        if generation != request.generation
            || !self.is_current_connection_generation(request.generation)
        {
            return false;
        }

        let username = if request.username {
            let Some(username) = credentials.username.as_deref().map(str::trim) else {
                return false;
            };
            if username.is_empty() {
                return false;
            }
            Some(username.to_string())
        } else {
            None
        };
        let password = if request.password {
            let Some(password) = credentials.password else {
                return false;
            };
            if password.is_empty() {
                return false;
            }
            Some(password)
        } else {
            None
        };

        let Some(mut params) = self.telnet_base_params.clone() else {
            return false;
        };
        params.apply_login_credentials(username.as_deref(), password.as_deref());
        params.credential_reference = None;
        params.prompt_username = None;
        params.prompt_password = None;

        self.connection_state = ConnectionState::Connecting;
        self.set_connection_active(false, cx);
        if self.start_telnet_connection_attempt(params, generation, cx) {
            true
        } else {
            self.telnet_credential_request = None;
            self.connection_state = ConnectionState::Disconnected {
                error: Some("Telnet connection runtime is unavailable".to_string()),
            };
            cx.emit(TerminalModelEvent::TelnetCredentialChanged);
            cx.emit(TerminalModelEvent::Wakeup);
            false
        }
    }

    pub fn ssh_mfa_request(&self) -> Option<TerminalMfaRequest> {
        self.ssh_mfa_responder
            .as_ref()
            .and_then(TerminalMfaResponder::pending_request)
    }

    pub fn submit_ssh_mfa(&self, responses: Vec<String>) -> bool {
        self.ssh_mfa_responder
            .as_ref()
            .is_some_and(|responder| responder.submit(responses))
    }

    /// 取消等待中的 SSH keyboard-interactive/MFA 输入；返回是否有请求被清除。
    pub fn cancel_ssh_mfa(&self) -> bool {
        self.ssh_mfa_responder
            .as_ref()
            .is_some_and(|responder| responder.cancel())
    }

    pub fn zmodem_picker_request(&self) -> Option<ZmodemPickerRequest> {
        self.zmodem_responder
            .as_ref()
            .and_then(ZmodemResponder::pending_request)
    }

    pub fn zmodem_transfer_progress(&self) -> Option<ZmodemTransferProgress> {
        self.zmodem_responder
            .as_ref()
            .and_then(ZmodemResponder::transfer_progress)
    }

    /// Request cancellation of the active ZMODEM transfer.
    ///
    /// Returns `true` only for a backend that accepted the request; the final
    /// A `ZmodemTransferFinished { outcome: Cancelled, .. }` event remains
    /// the source of truth.
    pub fn cancel_zmodem_transfer(&self) -> bool {
        self.backend
            .as_deref()
            .is_some_and(TerminalBackend::cancel_transfer)
    }

    pub fn zmodem_transfer_cancel_handle(&self) -> Option<TerminalTransferCancelHandle> {
        self.backend
            .as_deref()
            .and_then(TerminalBackend::transfer_cancel_handle)
    }

    pub fn submit_zmodem_picker(&self, response: ZmodemPickerResponse) -> bool {
        self.zmodem_responder
            .as_ref()
            .is_some_and(|responder| responder.submit(response))
    }

    pub fn claim_zmodem_picker(&self, request_id: u64) -> Option<ZmodemPickerClaim> {
        self.zmodem_responder
            .as_ref()
            .and_then(|responder| responder.try_claim_picker(request_id))
    }

    /// 获取连接类型
    pub fn connection_kind(&self) -> TerminalConnectionKind {
        self.connection_kind
    }

    pub fn session_mode(&self) -> TerminalSessionMode {
        self.session_mode
    }

    pub fn is_recording_playback(&self) -> bool {
        self.session_mode == TerminalSessionMode::RecordingPlayback
    }

    pub fn is_session_log(&self) -> bool {
        self.session_mode == TerminalSessionMode::SessionLog
    }

    pub fn is_read_only(&self) -> bool {
        matches!(
            self.session_mode,
            TerminalSessionMode::RecordingPlayback | TerminalSessionMode::SessionLog
        )
    }

    /// 会话是否处于锁定状态。
    pub fn is_locked(&self) -> bool {
        self.session_lock.is_some()
    }

    /// 锁定状态下是否隐藏输出。
    pub fn hide_output(&self) -> bool {
        self.session_lock
            .as_ref()
            .map(|lock| lock.hide_output)
            .unwrap_or(false)
    }

    /// 锁定密码哈希是否匹配（用于解锁全部会话时校验）。
    pub fn lock_password_matches(&self, password_hash: &str) -> bool {
        self.session_lock
            .as_ref()
            .is_some_and(|lock| lock.password_hash == password_hash)
    }

    /// 锁定会话；密码仅保存在内存中。
    pub fn lock_session(
        &mut self,
        password_hash: String,
        hide_output: bool,
        cx: &mut Context<Self>,
    ) {
        self.session_lock = Some(SessionLockState {
            password_hash,
            hide_output,
        });
        cx.emit(TerminalModelEvent::LockStateChanged);
        cx.notify();
    }

    /// 密码哈希匹配时解锁会话。
    pub fn unlock_session(&mut self, password_hash: &str, cx: &mut Context<Self>) -> bool {
        let matches = self.lock_password_matches(password_hash);
        if matches {
            self.session_lock = None;
            cx.emit(TerminalModelEvent::LockStateChanged);
            cx.notify();
        }
        matches
    }

    /// Returns a connection kind only when the surface owns live connection
    /// capabilities. Recording metadata must not be treated as a live SSH or
    /// serial session by Public MCP or other integrations.
    pub fn live_connection_kind(&self) -> Option<TerminalConnectionKind> {
        (!self.is_read_only()).then_some(self.connection_kind)
    }

    fn playback_runtime(
        &self,
    ) -> std::result::Result<&TerminalPlaybackRuntime, RecordingPlaybackError> {
        if !self.is_recording_playback() {
            return Err(RecordingPlaybackError::NotPlaybackSession);
        }
        self.playback_runtime
            .as_ref()
            .ok_or(RecordingPlaybackError::NotPlaybackSession)
    }

    fn playback_runtime_mut(
        &mut self,
    ) -> std::result::Result<&mut TerminalPlaybackRuntime, RecordingPlaybackError> {
        if !self.is_recording_playback() {
            return Err(RecordingPlaybackError::NotPlaybackSession);
        }
        self.playback_runtime
            .as_mut()
            .ok_or(RecordingPlaybackError::NotPlaybackSession)
    }

    fn sync_playback_dimensions(&mut self) {
        let term = self.term.lock();
        self.cols = term.columns();
        self.rows = term.screen_lines();
    }

    pub fn recording_playback_state(&self) -> Option<RecordingPlaybackState> {
        self.playback_runtime()
            .ok()
            .map(|runtime| runtime.timeline().state())
    }

    pub fn recording_playback_elapsed(&self) -> Option<Duration> {
        self.playback_runtime()
            .ok()
            .map(|runtime| runtime.timeline().elapsed())
    }

    pub fn recording_playback_duration(&self) -> Option<Duration> {
        self.playback_runtime()
            .ok()
            .map(|runtime| runtime.timeline().duration())
    }

    pub fn recording_playback_speed(&self) -> Option<f64> {
        self.playback_runtime()
            .ok()
            .map(|runtime| runtime.timeline().speed())
    }

    pub fn recording_playback_completeness(&self) -> Option<&RecordingCompleteness> {
        self.playback_runtime()
            .ok()
            .map(|runtime| runtime.timeline().completeness())
    }

    pub fn recording_playback_search_index_status(
        &self,
    ) -> Option<RecordingPlaybackSearchIndexStatus> {
        self.playback_runtime()
            .ok()
            .map(|runtime| runtime.timeline().search_index_status())
    }

    pub fn resume_recording_playback(
        &mut self,
    ) -> std::result::Result<RecordingPlaybackTransition, RecordingPlaybackError> {
        Ok(self.playback_runtime_mut()?.resume())
    }

    pub fn pause_recording_playback(
        &mut self,
    ) -> std::result::Result<RecordingPlaybackTransition, RecordingPlaybackError> {
        Ok(self.playback_runtime_mut()?.pause())
    }

    pub fn set_recording_playback_speed(
        &mut self,
        speed: f64,
    ) -> std::result::Result<RecordingPlaybackTransition, RecordingPlaybackError> {
        self.playback_runtime_mut()?.set_speed(speed)
    }

    pub fn advance_recording_playback(
        &mut self,
        elapsed: Duration,
    ) -> std::result::Result<(), RecordingPlaybackError> {
        self.playback_runtime_mut()?.advance(elapsed);
        self.sync_playback_dimensions();
        Ok(())
    }

    pub fn seek_recording_playback(
        &mut self,
        target: Duration,
    ) -> std::result::Result<(), RecordingPlaybackError> {
        self.playback_runtime_mut()?.seek(target);
        self.sync_playback_dimensions();
        Ok(())
    }

    pub fn search_recording_playback(
        &self,
        query: &str,
        requested_results: usize,
    ) -> std::result::Result<RecordingPlaybackSearchResults, RecordingPlaybackError> {
        self.playback_runtime()?
            .timeline()
            .search(query, requested_results)
    }

    /// 获取当前终端实例共享的性能指标。
    pub fn performance_metrics(&self) -> Arc<TerminalPerformanceMetrics> {
        self.performance_metrics.clone()
    }

    /// 获取当前终端性能指标的 best-effort 快照。
    pub fn performance_snapshot(&self) -> TerminalPerformanceSnapshot {
        self.performance_metrics.snapshot()
    }

    /// 计算相对前一个快照的窗口指标。
    pub fn performance_window(
        &self,
        previous: &TerminalPerformanceSnapshot,
        elapsed: Duration,
    ) -> TerminalPerformanceWindow {
        self.performance_snapshot().delta_since(previous, elapsed)
    }

    /// Returns the current recording state, including initialization or
    /// asynchronous persistence failures.
    pub fn recording_snapshot(
        &self,
    ) -> std::result::Result<RecordingSnapshot, RecordingRuntimeError> {
        Ok(self.recording_runtime()?.snapshot())
    }

    /// Builds the privacy-preserving request used by the pane recording UI.
    ///
    /// This path intentionally records terminal output, resize events and
    /// lifecycle markers only. Input capture requires a separate, explicit
    /// disclosure flow and must not be enabled by mutating UI defaults.
    pub fn build_output_recording_start_request(
        &self,
        final_path: PathBuf,
    ) -> std::result::Result<RecordingStartRequest, RecordingRuntimeError> {
        // Surface a runtime initialization failure before generating metadata
        // or touching the requested destination.
        self.recording_runtime()?;

        let rows = u16::try_from(self.rows).map_err(|_| {
            RecordingRuntimeError::InvalidConfig(format!(
                "terminal row count does not fit recording format: {}",
                self.rows
            ))
        })?;
        let cols = u16::try_from(self.cols).map_err(|_| {
            RecordingRuntimeError::InvalidConfig(format!(
                "terminal column count does not fit recording format: {}",
                self.cols
            ))
        })?;
        if rows == 0 || cols == 0 {
            return Err(RecordingRuntimeError::InvalidConfig(
                "terminal dimensions must be non-zero".to_string(),
            ));
        }

        let started_at = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| {
            RecordingRuntimeError::InvalidConfig("system clock predates the Unix epoch".to_string())
        })?;
        let started_at_unix_ms = u64::try_from(started_at.as_millis()).map_err(|_| {
            RecordingRuntimeError::InvalidConfig(
                "recording start timestamp exceeds the supported range".to_string(),
            )
        })?;
        let backend = match self.connection_kind {
            TerminalConnectionKind::Local => RecordingBackend::Local,
            TerminalConnectionKind::Ssh => RecordingBackend::Ssh,
            TerminalConnectionKind::Serial => RecordingBackend::Serial,
            TerminalConnectionKind::Telnet => RecordingBackend::Telnet,
        };

        Ok(RecordingStartRequest {
            final_path,
            metadata: RecordingMetadata {
                recording_id: Uuid::new_v4().to_string(),
                session_id: self.recording_session_id.clone(),
                backend,
                artifact_kind: RecordingArtifactKind::Recording,
                application_version: application_version(),
                started_at_unix_ms,
                capture_input: false,
                session: Some(self.recording_session_metadata.clone()),
            },
            initial_size: TerminalSize {
                rows,
                cols,
                pixel_width: self.pixel_width,
                pixel_height: self.pixel_height,
            },
            recording: output_only_recording_config(),
        })
    }

    /// Starts an output-only recording using metadata generated by the active
    /// terminal. Repeated calls after a completed stop create a fresh file and
    /// recording ID while retaining the pane's logical session ID.
    pub fn start_output_recording(
        &self,
        final_path: PathBuf,
    ) -> std::result::Result<RecordingTransition, RecordingRuntimeError> {
        let request = self.build_output_recording_start_request(final_path)?;
        self.start_recording(request)
    }

    /// Starts a recording on this terminal's stable runtime. Reconnects reuse
    /// the same runtime and therefore cannot silently replace the timeline.
    pub fn start_recording(
        &self,
        request: RecordingStartRequest,
    ) -> std::result::Result<RecordingTransition, RecordingRuntimeError> {
        self.recording_runtime()?.start(request)
    }

    pub fn pause_recording(
        &self,
    ) -> std::result::Result<RecordingTransition, RecordingRuntimeError> {
        self.recording_runtime()?.pause()
    }

    pub fn resume_recording(
        &self,
    ) -> std::result::Result<RecordingTransition, RecordingRuntimeError> {
        self.recording_runtime()?.resume()
    }

    pub fn stop_recording(
        &self,
    ) -> std::result::Result<RecordingTransition, RecordingRuntimeError> {
        self.recording_runtime()?.stop()
    }

    /// 是否可以重连
    pub fn can_reconnect(&self) -> bool {
        !self.is_read_only()
            && (self.ssh_config.is_some()
                || self.serial_params.is_some()
                || self.telnet_params.is_some())
    }

    /// 当前会话的退格键编码；非 Telnet 会话保持历史默认 DEL（0x7F）。
    pub fn telnet_backspace_code(&self) -> one_core::storage::TelnetBackspaceCode {
        self.telnet_params
            .as_ref()
            .map(|params| params.backspace_code)
            .unwrap_or_default()
    }

    /// 写入数据到终端
    pub fn write(&self, data: &[u8]) {
        if self.is_read_only() || self.is_locked() {
            return;
        }
        if let Some(ref backend) = self.backend {
            backend.write(data.to_vec());
        }
    }

    /// 向终端网格注入一段模型侧合成文本（连接状态提示、内联凭据回显等）。
    ///
    /// 只应在 PTY 不再产出内容的场景使用（断开、等待凭据/MFA），否则注入
    /// 内容可能与远端输出互相覆盖。注入后自动滚动到底部并触发重绘。
    pub fn inject_system_message(&self, text: &str, cx: &mut Context<Self>) {
        if self.inject_system_text(text) {
            cx.emit(TerminalModelEvent::Wakeup);
        }
    }

    fn inject_system_text(&self, text: &str) -> bool {
        if self.is_read_only() || text.is_empty() {
            return false;
        }
        let mut term = self.term.lock();
        let mut processor: Processor<StdSyncHandler> = Processor::new();
        processor.advance(&mut *term, text.as_bytes());
        term.scroll_display(alacritty_terminal::grid::Scroll::Bottom);
        drop(term);
        true
    }

    /// 写入来自外部集成的输入，例如 Public MCP。
    pub fn write_external_input(&self, data: &[u8]) {
        if let Some(handle) = self.external_input_handle() {
            handle.write(data.to_vec());
        }
    }

    pub fn external_input_handle(&self) -> Option<TerminalInputHandle> {
        if self.is_read_only() || self.is_locked() {
            return None;
        }
        self.backend
            .as_ref()
            .and_then(|backend| backend.input_handle())
    }

    pub fn external_exec_handle(&self) -> Option<TerminalExecHandle> {
        if self.is_read_only() || self.is_locked() {
            return None;
        }
        self.backend
            .as_ref()
            .and_then(|backend| backend.exec_handle())
    }

    pub fn external_control_handle(&self) -> Option<TerminalControlHandle> {
        if self.is_read_only() || self.is_locked() {
            return None;
        }
        self.backend
            .as_ref()
            .and_then(|backend| backend.control_handle())
    }

    /// 调整终端大小
    pub fn resize(&mut self, cols: usize, rows: usize, pixel_width: u16, pixel_height: u16) {
        if self.is_read_only() {
            // The artifact header and Resize events are authoritative for the
            // read-only grid. Canvas layout may update only cached pixel size.
            self.pixel_width = pixel_width;
            self.pixel_height = pixel_height;
            return;
        }

        if self.cols == cols && self.rows == rows {
            // 单元格行列数未变,但仍记录最新像素尺寸,供 nudge_resize 复用
            self.pixel_width = pixel_width;
            self.pixel_height = pixel_height;
            tracing::debug!(
                target: "terminal_residue",
                cols, rows, pixel_width, pixel_height,
                "Terminal::resize noop (cells unchanged, pixels cached)"
            );
            return;
        }

        tracing::info!(
            target: "terminal_residue",
            "Terminal::resize: {}x{} -> {}x{}, pixel={}x{}, backend={}",
            self.cols,
            self.rows,
            cols,
            rows,
            pixel_width,
            pixel_height,
            self.backend.is_some()
        );

        self.cols = cols;
        self.rows = rows;
        self.pixel_width = pixel_width;
        self.pixel_height = pixel_height;

        self.term.lock().resize(TermDimensions { cols, rows });

        if let Some(ref backend) = self.backend {
            let size = TerminalSize {
                rows: rows as u16,
                cols: cols as u16,
                pixel_width,
                pixel_height,
            };
            backend.resize(size);
            if let Some(tap) = self.recording_tap() {
                let _ = tap.record_resize(size);
            }
        }
    }

    /// 重新向 PTY 后端发送当前尺寸,不修改 alacritty grid。
    ///
    /// 用于在 alt screen 切换等场景下触发 SIGWINCH,
    /// 让 TUI 应用(opencode/lazygit/vim 等)重新查询尺寸并刷新整屏画面,
    /// 避免出现底部残留旧画面的问题。
    pub fn nudge_resize(&self) {
        if self.is_recording_playback() {
            return;
        }
        let Some(ref backend) = self.backend else {
            tracing::warn!(target: "terminal_residue", "nudge_resize skipped: no backend");
            return;
        };
        tracing::info!(
            target: "terminal_residue",
            cols = self.cols,
            rows = self.rows,
            pixel_width = self.pixel_width,
            pixel_height = self.pixel_height,
            "Terminal::nudge_resize -> backend.resize"
        );
        backend.resize(TerminalSize {
            rows: self.rows as u16,
            cols: self.cols as u16,
            pixel_width: self.pixel_width,
            pixel_height: self.pixel_height,
        });
    }

    /// 重新连接 SSH 或串口
    pub fn reconnect(&mut self, cx: &mut Context<Self>) -> bool {
        if self.is_read_only() {
            return false;
        }
        if let Some(base_config) = self.ssh_base_config.clone() {
            let Some(event_tx) = self.event_tx.clone() else {
                return false;
            };

            self.connection_state = ConnectionState::Connecting;
            self.set_connection_active(false, cx);
            if let Some(backend) = self.backend.take() {
                backend.shutdown();
            }
            self.prepare_surface_for_reconnect();

            let generation = self.next_connection_generation();
            self.record_connection_generation_marker(generation);
            if let Some(responder) = &self.ssh_mfa_responder {
                responder.cancel();
            }
            let Some(zmodem_responder) = self.zmodem_responder.clone() else {
                return false;
            };
            zmodem_responder.cancel();
            self.ssh_mfa_responder = None;
            self.ssh_credential_request = None;
            self.ssh_config = Some(base_config.clone());

            if let Some(session_manager) = self.ssh_session_manager.take() {
                cx.spawn(async move |_, _| {
                    let _ = session_manager.disconnect().await;
                })
                .detach();
            }

            if self.ssh_credential_prompt_policy.requires_credentials() {
                self.queue_ssh_credential_request(generation, cx);
            } else {
                let Ok((runtime_config, responder)) = ssh_config_with_runtime_credentials(
                    &base_config,
                    &TerminalSshCredentials::default(),
                    event_tx,
                    self.ssh_keyboard_interactive_enabled,
                ) else {
                    return false;
                };
                if !self.start_ssh_connection_attempt(runtime_config, responder, generation, cx) {
                    return false;
                }
            }
        } else if let Some(params) = self.serial_params.clone() {
            let Some(event_proxy) = self.event_proxy.clone() else {
                return false;
            };

            self.connection_state = ConnectionState::Connecting;
            self.set_connection_active(false, cx);
            if let Some(backend) = self.backend.take() {
                backend.shutdown();
            }
            self.prepare_surface_for_reconnect();
            let generation = self.next_connection_generation();
            self.record_connection_generation_marker(generation);
            let recording_tap = self.recording_tap();

            let (disconnect_tx, disconnect_rx) = tokio::sync::oneshot::channel::<()>();
            Self::spawn_disconnect_handler(disconnect_rx, generation, cx);
            Self::spawn_serial_connect(
                params,
                self.term.clone(),
                event_proxy,
                self.performance_metrics.clone(),
                Some(disconnect_tx),
                recording_tap,
                generation,
                cx,
            );
        } else if let Some(params) = self
            .telnet_base_params
            .clone()
            .or_else(|| self.telnet_params.clone())
        {
            self.connection_state = ConnectionState::Connecting;
            self.set_connection_active(false, cx);
            if let Some(backend) = self.backend.take() {
                backend.shutdown();
            }
            self.prepare_surface_for_reconnect();
            let generation = self.next_connection_generation();
            self.record_connection_generation_marker(generation);
            self.telnet_credential_request = None;

            if params.prompts_for_username() || params.prompts_for_password() {
                if !self.queue_telnet_credential_request(generation, cx) {
                    return false;
                }
            } else if !self.start_telnet_connection_attempt(params, generation, cx) {
                return false;
            }
        } else {
            return false;
        }

        cx.emit(TerminalModelEvent::Wakeup);
        true
    }

    fn prepare_surface_for_reconnect(&mut self) {
        // Keep the primary grid and scrollback, but never carry a stale full-screen
        // alternate-screen application (such as Vim) into the replacement backend.
        let mut term = self.term.lock();
        if term.mode().contains(TermMode::ALT_SCREEN) {
            let mut processor: Processor<StdSyncHandler> = Processor::new();
            processor.advance(&mut *term, b"\x1b[?1049l");
            term.scroll_display(alacritty_terminal::grid::Scroll::Bottom);
        }
        drop(term);
        self.child_exited = None;
        self.current_working_dir = None;
    }

    pub fn host_key_verification_request(&self) -> Option<HostKeyVerificationRequest> {
        self.pending_host_key_verification.clone()
    }

    pub fn respond_to_host_key_verification(
        &mut self,
        decision: HostKeyVerificationDecision,
        rejection_message: String,
        cx: &mut Context<Self>,
    ) {
        let Some(request) = self.pending_host_key_verification.take() else {
            return;
        };

        if decision == HostKeyVerificationDecision::Reject {
            self.connection_state = ConnectionState::Disconnected {
                error: Some(rejection_message),
            };
            cx.emit(TerminalModelEvent::Wakeup);
            return;
        }

        let Some(runtime_config) = self.ssh_config.clone() else {
            self.connection_state = ConnectionState::Disconnected {
                error: Some(rejection_message),
            };
            cx.emit(TerminalModelEvent::Wakeup);
            return;
        };

        let persist = decision == HostKeyVerificationDecision::AcceptAndSave;
        let retry_config = ssh_config_with_confirmed_host_key(&runtime_config, &request, persist);
        let Some(event_tx) = self.event_tx.clone() else {
            self.connection_state = ConnectionState::Disconnected {
                error: Some(rejection_message),
            };
            cx.emit(TerminalModelEvent::Wakeup);
            return;
        };
        let Ok((retry_config, responder)) = ssh_config_with_runtime_credentials(
            &retry_config,
            &TerminalSshCredentials::default(),
            event_tx,
            self.ssh_keyboard_interactive_enabled,
        ) else {
            self.connection_state = ConnectionState::Disconnected {
                error: Some(rejection_message),
            };
            cx.emit(TerminalModelEvent::Wakeup);
            return;
        };

        self.connection_state = ConnectionState::Connecting;
        self.set_connection_active(false, cx);
        if let Some(backend) = self.backend.take() {
            backend.shutdown();
        }
        self.prepare_surface_for_reconnect();

        let generation = self.next_connection_generation();
        self.record_connection_generation_marker(generation);
        if let Some(responder) = &self.ssh_mfa_responder {
            responder.cancel();
        }
        if let Some(zmodem_responder) = &self.zmodem_responder {
            zmodem_responder.cancel();
        }
        self.ssh_mfa_responder = None;
        self.ssh_credential_request = None;

        if let Some(session_manager) = self.ssh_session_manager.take() {
            cx.spawn(async move |_, _| {
                let _ = session_manager.disconnect().await;
            })
            .detach();
        }

        if !self.start_ssh_connection_attempt(retry_config, responder, generation, cx) {
            self.connection_state = ConnectionState::Disconnected {
                error: Some(rejection_message),
            };
            cx.emit(TerminalModelEvent::Wakeup);
        }
    }

    /// 更新 SSH 终端的路径同步设置。
    ///
    /// 路径同步由 shell 集成脚本发出 OSC 7，这里保留接口以兼容设置同步流程。
    pub fn set_sync_path_with_terminal(&mut self, _enabled: bool) {
        if self.connection_kind != TerminalConnectionKind::Ssh {
            return;
        }
    }

    /// 关闭终端
    pub fn shutdown(&self) {
        if let Some(responder) = &self.ssh_mfa_responder {
            responder.cancel();
        }
        if let Some(responder) = &self.zmodem_responder {
            responder.cancel();
        }
        if let Some(ref backend) = self.backend {
            backend.shutdown();
        }
        if let Ok(recording_runtime) = &self.recording_runtime {
            if let Err(error) = recording_runtime.shutdown() {
                tracing::warn!(%error, "failed to shut down terminal recording runtime");
            }
        }
        if let Some(session_log_runtime) = &self.session_log_runtime {
            if let Err(error) = session_log_runtime.shutdown() {
                tracing::warn!(
                    %error,
                    "failed to shut down automatic terminal session log runtime"
                );
            }
        }
    }

    // ========== 选择操作 ==========

    /// 获取选中的文本
    pub fn selection_text(&self) -> Option<String> {
        crate::selection_text_from_term(&self.term.lock())
    }

    /// 清除选择
    pub fn clear_selection(&mut self) {
        self.term.lock().selection = None;
    }

    /// Try to clear the selection without waiting for the parser.
    ///
    /// UI event handlers should use this and retry later when it returns
    /// `false`, so mouse input never blocks the GPUI/Windows message loop.
    pub fn try_clear_selection(&mut self) -> bool {
        let Some(mut term) = self.term.try_lock_unfair() else {
            return false;
        };
        term.selection = None;
        true
    }

    /// 全选
    pub fn select_all(&mut self) {
        let mut term = self.term.lock();
        let start = AlacPoint::new(Line(-(term.history_size() as i32)), Column(0));
        let end = AlacPoint::new(
            Line(term.screen_lines() as i32 - 1),
            Column(term.columns() - 1),
        );
        term.selection = Some(Selection::new(SelectionType::Simple, start, Side::Left));
        if let Some(selection) = &mut term.selection {
            selection.update(end, Side::Right);
        }
    }

    /// 开始选择
    pub fn start_selection(&mut self, selection_type: SelectionType, point: AlacPoint, side: Side) {
        let mut term = self.term.lock();
        let point_with_offset = AlacPoint::new(
            point.line - term.grid().display_offset() as i32,
            point.column,
        );
        term.selection = Some(Selection::new(selection_type, point_with_offset, side));
    }

    /// Try to start a selection without waiting for the parser.
    pub fn try_start_selection(
        &mut self,
        selection_type: SelectionType,
        point: AlacPoint,
        side: Side,
    ) -> bool {
        let Some(mut term) = self.term.try_lock_unfair() else {
            return false;
        };
        let point_with_offset = AlacPoint::new(
            point.line - term.grid().display_offset() as i32,
            point.column,
        );
        term.selection = Some(Selection::new(selection_type, point_with_offset, side));
        true
    }

    /// 更新选择
    pub fn update_selection(&mut self, point: AlacPoint, side: Side) {
        let mut term = self.term.lock();
        let point_with_offset = AlacPoint::new(
            point.line - term.grid().display_offset() as i32,
            point.column,
        );
        if let Some(selection) = &mut term.selection {
            selection.update(point_with_offset, side);
        }
    }

    /// Try to update a selection without waiting for the parser.
    pub fn try_update_selection(&mut self, point: AlacPoint, side: Side) -> bool {
        let Some(mut term) = self.term.try_lock_unfair() else {
            return false;
        };
        let point_with_offset = AlacPoint::new(
            point.line - term.grid().display_offset() as i32,
            point.column,
        );
        if let Some(selection) = &mut term.selection {
            selection.update(point_with_offset, side);
        }
        true
    }

    // ========== 滚动操作 ==========

    /// 滚动终端
    pub fn scroll(&mut self, delta: i32) {
        self.term
            .lock()
            .scroll_display(alacritty_terminal::grid::Scroll::Delta(delta));
    }

    /// 获取滚动代理（供视图层的滚动条使用）
    pub fn scroll_proxy(&self) -> TerminalScrollProxy {
        TerminalScrollProxy {
            term: self.term.clone(),
            event_tx: self.event_tx.clone(),
            wakeup_pending: self.wakeup_pending.clone(),
        }
    }

    // ========== Vi 模式 ==========

    /// 切换 Vi 模式
    pub fn toggle_vi_mode(&mut self) {
        self.term.lock().toggle_vi_mode();
    }

    /// 是否处于 Vi 模式
    pub fn in_vi_mode(&self) -> bool {
        self.term.lock().mode().contains(TermMode::VI)
    }

    /// 获取终端模式
    pub fn mode(&self) -> TermMode {
        *self.term.lock().mode()
    }
}

/// 解析持久化边界上的 Telnet 参数。
///
/// `StoredConnection.params` 可能来自旧版本迁移、云同步、导入或数据库损坏，
/// 调用方必须处理解析失败，不能直接 expect/panic。
fn parse_stored_telnet_params(conn: &StoredConnection) -> Result<TelnetParams, String> {
    conn.to_telnet_params()
        .map_err(|error| format!("Telnet 连接参数损坏或不兼容: {error}"))
}

fn format_connection_error(err: &anyhow::Error) -> String {
    format!("{err:#}")
}

fn host_key_verification_request(error: &anyhow::Error) -> Option<HostKeyVerificationRequest> {
    error
        .downcast_ref::<HostKeyRejection>()
        .and_then(|rejection| match rejection {
            HostKeyRejection::Unknown {
                identity,
                presented,
            } => Some(HostKeyVerificationRequest {
                identity: identity.clone(),
                presented: presented.clone(),
                reason: HostKeyVerificationReason::Unknown,
            }),
            HostKeyRejection::Changed {
                identity,
                presented,
                expected,
            } => Some(HostKeyVerificationRequest {
                identity: identity.clone(),
                presented: presented.clone(),
                reason: HostKeyVerificationReason::Changed {
                    expected: expected.clone(),
                },
            }),
            HostKeyRejection::Revoked { .. } | HostKeyRejection::StoreUnavailable { .. } => None,
        })
}

impl EventEmitter<TerminalModelEvent> for Terminal {}

fn flush_pending_terminal_events(
    render_tx: &futures::channel::mpsc::UnboundedSender<TerminalEvent>,
    pending_events: &mut Vec<TerminalEvent>,
    pending_wakeup: &mut bool,
) -> bool {
    // Preserve non-render events and finish each batch with a single render
    // invalidation. Do not acknowledge the Wakeup here: render_tx is also
    // unbounded, so clearing the end-to-end gate before GPUI consumes the
    // event would allow stale Wakeups to accumulate while the UI is busy.
    for event in pending_events.drain(..) {
        if render_tx.unbounded_send(event).is_err() {
            return false;
        }
    }

    if *pending_wakeup {
        *pending_wakeup = false;
        if render_tx.unbounded_send(TerminalEvent::Wakeup).is_err() {
            return false;
        }
    }

    true
}

async fn receive_terminal_event_for_gpui(
    render_rx: &mut futures::channel::mpsc::UnboundedReceiver<TerminalEvent>,
    wakeup_pending: &AtomicBool,
) -> Option<TerminalEvent> {
    let event = render_rx.next().await?;
    if matches!(&event, TerminalEvent::Wakeup) {
        // Reopen only after GPUI has dequeued the sole outstanding invalidation.
        // Doing this before the entity update also ensures output produced while
        // the handler runs can queue the next Wakeup instead of being lost.
        wakeup_pending.store(false, Ordering::Release);
    }
    Some(event)
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    use super::with_local_terminal_default_env;
    use super::{
        AutomaticSessionLogRequestInput, CommandRecordGate, ConnectionState,
        HostKeyVerificationReason, HostKeyVerificationRequest, SessionLockState,
        SshConnectionUpdate, SshCredentialPromptPolicy, TermDimensions, Terminal,
        TerminalConnectionKind, TerminalMfaPrompt, TerminalMfaRequest, TerminalMfaResponder,
        TerminalScrollProxy, TerminalSessionMode, TerminalSshCredentials,
        build_automatic_session_log_request, build_cd_command, build_ssh_base_init_commands,
        build_ssh_init_commands, clear_screen_remote_redraw_bytes, compose_ssh_init_commands,
        flush_pending_terminal_events, format_connection_error, host_key_verification_request,
        is_reconnect_generation, is_ssh_password_prompt, keyboard_interactive_answers_for_terminal,
        merge_history_matches, normalize_history_matches, parse_stored_telnet_params,
        receive_terminal_event_for_gpui, recent_text_from_term,
        resolve_default_windows_shell_from_env, resolve_local_working_dir, resolve_ssh_connection,
        send_coalesced_wakeup, shell_escape_arg, should_install_connected_backend,
        ssh_config_with_confirmed_host_key, ssh_config_with_runtime_credentials,
    };
    use crate::history::{
        HistoryEntry, ShellHistoryFormat, collect_history_suggestions, normalize_history_command,
        parse_shell_history, push_history_entry,
    };
    use crate::recording::{
        ASCIICAST_VERSION, NAVOP_EVENT_STREAM, NAVOP_RECORDING_FORMAT_VERSION, ParsedRecording,
        RecordingArtifactKind, RecordingBackend, RecordingCompleteness, RecordingConfig,
        RecordingEvent, RecordingEventKind, RecordingFileLimits, RecordingHeader,
        RecordingHeaderMetadata, RecordingMetadata, RecordingPlayback, RecordingPlaybackError,
        RecordingPlaybackLimits, RecordingPlaybackSearchKind, RecordingPlaybackState,
        RecordingPlaybackTransition, RecordingRuntime, RecordingRuntimeConfig,
        RecordingRuntimeError, RecordingSessionMetadata, RecordingStartRequest, RecordingState,
        RecordingTapOutcome, RecordingTransition, read_recording,
    };
    use crate::{
        TerminalBackend, TerminalControlHandle, TerminalEvent, TerminalExecHandle,
        TerminalInputHandle, TerminalPerformanceMetrics, TerminalSize,
    };
    use alacritty_terminal::event::{Event as AlacTermEvent, EventListener};
    use alacritty_terminal::grid::Dimensions;
    use alacritty_terminal::index::{Column, Line, Point, Side};
    use alacritty_terminal::selection::SelectionType;
    use alacritty_terminal::term::cell::Flags;
    use alacritty_terminal::term::{TermDamage, TermMode};
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
    use anyhow::anyhow;
    use one_core::storage::models::{SshAuthMethod, SshParams, StoredConnection};
    use ssh::{
        HostKeyDetails, HostKeyIdentity, HostKeyRejection, HostKeyRoute,
        KeyboardInteractiveRequest, KeyboardInteractiveResponder, SshAuth,
    };
    use std::collections::VecDeque;
    use std::fs;
    use std::path::PathBuf;
    #[cfg(not(target_os = "windows"))]
    use std::process::Command;
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio::sync::mpsc::unbounded_channel;

    #[test]
    fn connected_backend_installation_rejects_a_prior_disconnect() {
        assert!(should_install_connected_backend(
            &ConnectionState::Connecting
        ));
        assert!(should_install_connected_backend(
            &ConnectionState::Connected
        ));
        assert!(!should_install_connected_backend(
            &ConnectionState::Disconnected { error: None }
        ));
    }

    struct ResizeProbe {
        sizes: Arc<Mutex<Vec<TerminalSize>>>,
    }

    impl TerminalBackend for ResizeProbe {
        fn write(&self, _data: Vec<u8>) {}

        fn resize(&self, size: TerminalSize) {
            self.sizes
                .lock()
                .expect("resize probe should lock")
                .push(size);
        }

        fn shutdown(&self) {}
    }

    struct InputRouteProbe {
        direct_writes: Arc<Mutex<Vec<Vec<u8>>>>,
        external_writes: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    struct CancelTransferProbe {
        requests: Arc<Mutex<usize>>,
    }

    impl TerminalBackend for CancelTransferProbe {
        fn write(&self, _data: Vec<u8>) {}

        fn resize(&self, _size: TerminalSize) {}

        fn shutdown(&self) {}

        fn cancel_transfer(&self) -> bool {
            let mut requests = self.requests.lock().expect("cancel probe should lock");
            *requests += 1;
            true
        }
    }

    impl TerminalBackend for InputRouteProbe {
        fn write(&self, data: Vec<u8>) {
            self.direct_writes
                .lock()
                .expect("direct input probe should lock")
                .push(data);
        }

        fn resize(&self, _size: TerminalSize) {}

        fn shutdown(&self) {}

        fn input_handle(&self) -> Option<TerminalInputHandle> {
            let external_writes = self.external_writes.clone();
            Some(TerminalInputHandle::new(move |data| {
                external_writes
                    .lock()
                    .expect("external input probe should lock")
                    .push(data);
            }))
        }

        fn exec_handle(&self) -> Option<TerminalExecHandle> {
            Some(TerminalExecHandle::new(|_, _| {
                Box::pin(async { unreachable!("read-only test must not invoke exec") })
            }))
        }

        fn control_handle(&self) -> Option<TerminalControlHandle> {
            Some(TerminalControlHandle::new(|_, _| {
                Box::pin(async { unreachable!("read-only test must not invoke control") })
            }))
        }
    }

    fn test_terminal_with_recording_runtime(
        recording_runtime: std::result::Result<RecordingRuntime, RecordingRuntimeError>,
    ) -> Terminal {
        let (event_tx, _event_rx) = unbounded_channel();
        let (term, event_proxy, _colors, performance_metrics) =
            Terminal::create_term(80, 24, 10_000, event_tx.clone());
        let wakeup_pending = event_proxy.wakeup_pending_handle();

        Terminal {
            term,
            session_mode: TerminalSessionMode::Live,
            performance_metrics,
            backend: None,
            recording_runtime,
            session_log_runtime: None,
            playback_runtime: None,
            recording_session_id: "terminal-runtime-test-session".to_string(),
            recording_session_metadata: RecordingSessionMetadata {
                connection_id: Some(1),
                connection_name: Some("test SSH".to_string()),
                remote_user: Some("tester".to_string()),
                remote_host: Some("example.test".to_string()),
                remote_port: Some(22),
                ..RecordingSessionMetadata::default()
            },
            title: String::new(),
            current_working_dir: None,
            child_exited: None,
            connection_state: ConnectionState::Connected,
            session_lock: None,
            cols: 80,
            rows: 24,
            pixel_width: 640,
            pixel_height: 480,
            ssh_config: None,
            ssh_base_config: None,
            ssh_session_manager: None,
            ssh_credential_prompt_policy: SshCredentialPromptPolicy::default(),
            ssh_credential_request: None,
            ssh_keyboard_interactive_enabled: false,
            ssh_mfa_responder: None,
            zmodem_responder: None,
            pending_host_key_verification: None,
            serial_params: None,
            telnet_params: None,
            telnet_base_params: None,
            telnet_credential_request: None,
            wakeup_pending,
            event_tx: Some(event_tx),
            event_proxy: Some(event_proxy),
            connection_id: Some(1),
            connection_name: Some("test SSH".to_string()),
            init_commands: None,
            session_history: VecDeque::new(),
            persisted_history: Vec::new(),
            history_repository: None,
            history_scope: None,
            command_record_gate: CommandRecordGate::default(),
            connection_generation: 1,
            connection_kind: TerminalConnectionKind::Ssh,
            scrollback_lines: 10_000,
        }
    }

    fn test_recording_start_request(final_path: PathBuf) -> RecordingStartRequest {
        RecordingStartRequest {
            final_path,
            metadata: RecordingMetadata {
                recording_id: "terminal-runtime-test-recording".to_string(),
                session_id: "terminal-runtime-test-session".to_string(),
                backend: RecordingBackend::Ssh,
                artifact_kind: RecordingArtifactKind::Recording,
                application_version: "0.1.0-test".to_string(),
                started_at_unix_ms: 1_700_000_000_123,
                capture_input: false,
                session: None,
            },
            initial_size: TerminalSize {
                rows: 24,
                cols: 80,
                pixel_width: 640,
                pixel_height: 480,
            },
            recording: RecordingConfig::default(),
        }
    }

    fn test_parsed_playback_recording(
        backend: RecordingBackend,
        completeness: RecordingCompleteness,
    ) -> ParsedRecording {
        ParsedRecording {
            header: RecordingHeader {
                version: ASCIICAST_VERSION,
                width: 80,
                height: 24,
                timestamp: 1_700_000_000,
                navop: RecordingHeaderMetadata {
                    format_version: NAVOP_RECORDING_FORMAT_VERSION,
                    recording_id: "terminal-playback-test-recording".to_string(),
                    session_id: "terminal-playback-test-session".to_string(),
                    backend,
                    artifact_kind: RecordingArtifactKind::Recording,
                    application_version: "0.1.0-test".to_string(),
                    started_at_unix_ms: 1_700_000_000_123,
                    capture_input: true,
                    event_stream: NAVOP_EVENT_STREAM.to_string(),
                    session: None,
                },
            },
            events: vec![
                RecordingEvent {
                    elapsed: Duration::ZERO,
                    kind: RecordingEventKind::Output(b"one".to_vec()),
                },
                RecordingEvent {
                    elapsed: Duration::from_millis(1),
                    kind: RecordingEventKind::Resize(TerminalSize {
                        rows: 40,
                        cols: 100,
                        pixel_width: 0,
                        pixel_height: 0,
                    }),
                },
                RecordingEvent {
                    elapsed: Duration::from_millis(2),
                    kind: RecordingEventKind::Output(b"\r\ntwo".to_vec()),
                },
                RecordingEvent {
                    elapsed: Duration::from_millis(3),
                    kind: RecordingEventKind::Input(b"typed-command".to_vec()),
                },
                RecordingEvent {
                    elapsed: Duration::from_millis(4),
                    kind: RecordingEventKind::Marker("checkpoint".to_string()),
                },
            ],
            completeness,
        }
    }

    #[test]
    fn shell_escape_arg_handles_single_quote() {
        let escaped = shell_escape_arg("a'b");
        assert_eq!(escaped, "'a'\"'\"'b'");
    }

    #[test]
    fn reconnect_generation_only_counts_attempts_after_the_first() {
        assert!(!is_reconnect_generation(0));
        assert!(!is_reconnect_generation(1));
        assert!(is_reconnect_generation(2));
        assert!(is_reconnect_generation(u64::MAX));
    }

    #[test]
    fn terminal_recording_runtime_survives_reconnect_generation_change() {
        let directory = tempfile::tempdir().expect("create recording directory");
        let final_path = directory.path().join("session.cast");
        let runtime = RecordingRuntime::new(RecordingRuntimeConfig::default())
            .expect("create recording runtime");
        let mut terminal = test_terminal_with_recording_runtime(Ok(runtime));

        assert_eq!(
            RecordingState::Idle,
            terminal
                .recording_snapshot()
                .expect("read initial recording state")
                .state
        );
        assert_eq!(
            RecordingTransition::Changed,
            terminal
                .start_recording(test_recording_start_request(final_path.clone()))
                .expect("start terminal recording")
        );

        let first_tap = terminal.recording_tap().expect("first backend tap");
        assert_eq!(
            RecordingTapOutcome::Accepted,
            first_tap.record_output(b"before reconnect\r\n")
        );
        assert_eq!(
            RecordingTransition::Changed,
            terminal
                .pause_recording()
                .expect("pause terminal recording")
        );
        assert_eq!(
            RecordingTapOutcome::Inactive,
            first_tap.record_output(b"paused output")
        );
        assert_eq!(
            RecordingTransition::Changed,
            terminal
                .resume_recording()
                .expect("resume terminal recording")
        );
        let generation = terminal.next_connection_generation();
        assert_eq!(2, generation);
        terminal.record_connection_generation_marker(generation);

        assert_eq!(
            RecordingState::Recording,
            terminal
                .recording_snapshot()
                .expect("recording state survives surface reset")
                .state
        );
        let replacement_tap = terminal.recording_tap().expect("replacement backend tap");
        assert_eq!(
            RecordingTapOutcome::Accepted,
            replacement_tap.record_output(b"after reconnect\r\n")
        );
        assert_eq!(
            RecordingTransition::Changed,
            terminal.stop_recording().expect("stop terminal recording")
        );

        let recording = read_recording(&final_path, RecordingFileLimits::default())
            .expect("read completed recording");
        let events = recording
            .events
            .into_iter()
            .map(|event| event.kind)
            .collect::<Vec<_>>();
        assert_eq!(
            vec![
                RecordingEventKind::Output(b"before reconnect\r\n".to_vec()),
                RecordingEventKind::Marker("connection_generation:2".to_string()),
                RecordingEventKind::Output(b"after reconnect\r\n".to_vec()),
            ],
            events
        );

        terminal.shutdown();
        terminal.shutdown();
    }

    #[test]
    fn output_recording_request_uses_safe_metadata_and_stable_session_identity() {
        let runtime = RecordingRuntime::new(RecordingRuntimeConfig::default())
            .expect("create recording runtime");
        let terminal = test_terminal_with_recording_runtime(Ok(runtime));

        let first = terminal
            .build_output_recording_start_request(PathBuf::from("first.cast"))
            .expect("build first output recording request");
        let second = terminal
            .build_output_recording_start_request(PathBuf::from("second.cast"))
            .expect("build second output recording request");

        assert!(!first.metadata.recording_id.is_empty());
        assert_ne!(
            first.metadata.recording_id, second.metadata.recording_id,
            "each recording file must receive a fresh identity"
        );
        assert_eq!(
            "terminal-runtime-test-session", first.metadata.session_id,
            "metadata must use the pane's privacy-preserving logical session ID"
        );
        assert_eq!(first.metadata.session_id, second.metadata.session_id);
        assert_eq!(RecordingBackend::Ssh, first.metadata.backend);
        assert!(!first.metadata.capture_input);
        assert!(!first.recording.capture_input);
        assert_eq!(
            Some(&RecordingSessionMetadata {
                connection_id: Some(1),
                connection_name: Some("test SSH".to_string()),
                remote_user: Some("tester".to_string()),
                remote_host: Some("example.test".to_string()),
                remote_port: Some(22),
                ..RecordingSessionMetadata::default()
            }),
            first.metadata.session.as_ref()
        );
        assert_eq!(
            TerminalSize {
                rows: 24,
                cols: 80,
                pixel_width: 640,
                pixel_height: 480,
            },
            first.initial_size
        );
        assert!(first.metadata.started_at_unix_ms > 0);
        assert!(!first.metadata.application_version.is_empty());

        terminal.shutdown();
        terminal.shutdown();
    }

    #[test]
    fn automatic_session_log_request_is_output_only_and_uses_dated_catalog_path() {
        let request = build_automatic_session_log_request(AutomaticSessionLogRequestInput {
            data_directory: PathBuf::from("/data"),
            backend: RecordingBackend::Serial,
            session_id: "logical-session".to_string(),
            initial_size: TerminalSize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            },
            session: RecordingSessionMetadata {
                connection_id: Some(7),
                connection_name: Some("console".to_string()),
                serial_port: Some("/dev/ttyUSB0".to_string()),
                ..RecordingSessionMetadata::default()
            },
            started_at_unix_ms: 1_723_651_441_123,
            recording_id: "recording-id".to_string(),
        })
        .expect("build automatic session log request");

        assert_eq!(
            PathBuf::from(
                "/data/session-logs/2024/08/20240814-160401-123-serial-recording-id.cast"
            ),
            request.final_path
        );
        assert!(!request.metadata.capture_input);
        assert!(!request.recording.capture_input);
        assert_eq!("logical-session", request.metadata.session_id);
        assert_eq!(
            Some("/dev/ttyUSB0"),
            request
                .metadata
                .session
                .as_ref()
                .and_then(|session| session.serial_port.as_deref())
        );
    }

    #[test]
    fn output_recording_can_restart_with_a_new_file_in_the_same_session() {
        let directory = tempfile::tempdir().expect("create recording directory");
        let first_path = directory.path().join("first.cast");
        let second_path = directory.path().join("second.cast");
        let runtime = RecordingRuntime::new(RecordingRuntimeConfig::default())
            .expect("create recording runtime");
        let terminal = test_terminal_with_recording_runtime(Ok(runtime));
        let tap = terminal.recording_tap().expect("backend recording tap");

        assert_eq!(
            RecordingTransition::Changed,
            terminal
                .start_output_recording(first_path.clone())
                .expect("start first output recording")
        );
        assert_eq!(
            RecordingTapOutcome::Accepted,
            tap.record_output(b"first recording\r\n")
        );
        assert_eq!(
            RecordingTransition::Changed,
            terminal
                .stop_recording()
                .expect("stop first output recording")
        );

        assert_eq!(
            RecordingTransition::Changed,
            terminal
                .start_output_recording(second_path.clone())
                .expect("start second output recording")
        );
        assert_eq!(
            RecordingTapOutcome::Accepted,
            tap.record_output(b"second recording\r\n")
        );
        assert_eq!(
            RecordingTransition::Changed,
            terminal
                .stop_recording()
                .expect("stop second output recording")
        );

        let first = read_recording(&first_path, RecordingFileLimits::default())
            .expect("read first recording");
        let second = read_recording(&second_path, RecordingFileLimits::default())
            .expect("read second recording");
        assert_ne!(
            first.header.navop.recording_id, second.header.navop.recording_id,
            "each completed recording must have its own recording ID"
        );
        assert_eq!(
            first.header.navop.session_id, second.header.navop.session_id,
            "successive recordings from one pane must remain associated"
        );
        assert_eq!(
            vec![RecordingEventKind::Output(b"first recording\r\n".to_vec())],
            first
                .events
                .into_iter()
                .map(|event| event.kind)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            vec![RecordingEventKind::Output(b"second recording\r\n".to_vec())],
            second
                .events
                .into_iter()
                .map(|event| event.kind)
                .collect::<Vec<_>>()
        );

        terminal.shutdown();
        terminal.shutdown();
    }

    #[test]
    fn terminal_resize_records_only_cell_changes_delivered_to_a_backend() {
        let directory = tempfile::tempdir().expect("create recording directory");
        let final_path = directory.path().join("resize.cast");
        let runtime = RecordingRuntime::new(RecordingRuntimeConfig::default())
            .expect("create recording runtime");
        let mut terminal = test_terminal_with_recording_runtime(Ok(runtime));
        let sizes = Arc::new(Mutex::new(Vec::new()));
        terminal.backend = Some(Box::new(ResizeProbe {
            sizes: sizes.clone(),
        }));

        assert_eq!(
            RecordingTransition::Changed,
            terminal
                .start_recording(test_recording_start_request(final_path.clone()))
                .expect("start terminal recording")
        );

        let recorded_size = TerminalSize {
            rows: 40,
            cols: 100,
            pixel_width: 1_000,
            pixel_height: 800,
        };
        terminal.resize(100, 40, 1_000, 800);

        // Pixel-only changes are cached for the next backend nudge, but do not
        // resize the grid or create another recording event.
        terminal.resize(100, 40, 1_200, 900);
        terminal.nudge_resize();

        // A grid change that cannot be delivered to an active backend is not
        // represented as if the remote/local session had accepted it.
        terminal.backend = None;
        terminal.resize(120, 50, 1_440, 1_000);

        assert_eq!(
            RecordingTransition::Changed,
            terminal.stop_recording().expect("stop terminal recording")
        );
        assert_eq!(
            vec![
                recorded_size,
                TerminalSize {
                    rows: 40,
                    cols: 100,
                    pixel_width: 1_200,
                    pixel_height: 900,
                },
            ],
            *sizes.lock().expect("resize probe should lock")
        );

        let recording = read_recording(&final_path, RecordingFileLimits::default())
            .expect("read completed recording");
        assert_eq!(
            vec![RecordingEventKind::Resize(TerminalSize {
                rows: recorded_size.rows,
                cols: recorded_size.cols,
                // Asciicast v2 resize events carry cell dimensions only.
                pixel_width: 0,
                pixel_height: 0,
            })],
            recording
                .events
                .into_iter()
                .map(|event| event.kind)
                .collect::<Vec<_>>()
        );

        terminal.shutdown();
        terminal.shutdown();
    }

    #[test]
    fn terminal_cancel_transfer_delegates_without_a_visible_progress_snapshot() {
        let runtime = RecordingRuntime::new(RecordingRuntimeConfig::default())
            .expect("create recording runtime");
        let mut terminal = test_terminal_with_recording_runtime(Ok(runtime));
        let requests = Arc::new(Mutex::new(0));
        terminal.backend = Some(Box::new(CancelTransferProbe {
            requests: requests.clone(),
        }));

        assert!(terminal.zmodem_transfer_progress().is_none());
        assert!(terminal.cancel_zmodem_transfer());
        assert_eq!(1, *requests.lock().expect("cancel requests should lock"));
    }

    #[test]
    fn terminal_external_input_uses_the_backend_external_input_route() {
        let runtime = RecordingRuntime::new(RecordingRuntimeConfig::default())
            .expect("create recording runtime");
        let mut terminal = test_terminal_with_recording_runtime(Ok(runtime));
        let direct_writes = Arc::new(Mutex::new(Vec::new()));
        let external_writes = Arc::new(Mutex::new(Vec::new()));
        terminal.backend = Some(Box::new(InputRouteProbe {
            direct_writes: direct_writes.clone(),
            external_writes: external_writes.clone(),
        }));

        terminal.write_external_input(b"public mcp input");
        assert!(
            direct_writes
                .lock()
                .expect("direct input probe should lock")
                .is_empty()
        );
        assert_eq!(
            vec![b"public mcp input".to_vec()],
            *external_writes
                .lock()
                .expect("external input probe should lock")
        );

        terminal.write(b"human input");
        assert_eq!(
            vec![b"human input".to_vec()],
            *direct_writes
                .lock()
                .expect("direct input probe should lock")
        );

        terminal.shutdown();
        terminal.shutdown();
    }

    #[test]
    fn locked_session_blocks_all_input_routes_and_unlock_requires_password_match() {
        let runtime = RecordingRuntime::new(RecordingRuntimeConfig::default())
            .expect("create recording runtime");
        let mut terminal = test_terminal_with_recording_runtime(Ok(runtime));
        let direct_writes = Arc::new(Mutex::new(Vec::new()));
        let external_writes = Arc::new(Mutex::new(Vec::new()));
        terminal.backend = Some(Box::new(InputRouteProbe {
            direct_writes: direct_writes.clone(),
            external_writes: external_writes.clone(),
        }));

        let hash = "correct-lock-hash";
        terminal.session_lock = Some(SessionLockState {
            password_hash: hash.to_string(),
            hide_output: true,
        });

        assert!(terminal.is_locked());
        assert!(terminal.hide_output());
        assert!(terminal.lock_password_matches(hash));

        terminal.write(b"human input");
        terminal.write_external_input(b"external input");
        assert!(
            direct_writes
                .lock()
                .expect("direct input probe should lock")
                .is_empty()
        );
        assert!(
            external_writes
                .lock()
                .expect("external input probe should lock")
                .is_empty()
        );
        assert!(terminal.external_input_handle().is_none());
        assert!(terminal.external_exec_handle().is_none());
        assert!(terminal.external_control_handle().is_none());

        assert!(!terminal.lock_password_matches("wrong-lock-hash"));

        terminal.session_lock = None;
        assert!(!terminal.is_locked());

        terminal.write(b"human input");
        assert_eq!(
            vec![b"human input".to_vec()],
            *direct_writes
                .lock()
                .expect("direct input probe should lock")
        );

        terminal.shutdown();
        terminal.shutdown();
    }

    #[test]
    fn connection_generation_saturates_instead_of_wrapping() {
        let runtime = RecordingRuntime::new(RecordingRuntimeConfig::default())
            .expect("create recording runtime");
        let mut terminal = test_terminal_with_recording_runtime(Ok(runtime));
        terminal.connection_generation = u64::MAX;

        assert_eq!(u64::MAX, terminal.next_connection_generation());

        terminal.shutdown();
    }

    #[test]
    fn recording_playback_mode_revokes_live_terminal_capabilities() {
        let runtime = RecordingRuntime::new(RecordingRuntimeConfig::default())
            .expect("create recording runtime");
        let mut terminal = test_terminal_with_recording_runtime(Ok(runtime));
        let direct_writes = Arc::new(Mutex::new(Vec::new()));
        let external_writes = Arc::new(Mutex::new(Vec::new()));
        terminal.backend = Some(Box::new(InputRouteProbe {
            direct_writes: direct_writes.clone(),
            external_writes: external_writes.clone(),
        }));
        terminal.session_mode = TerminalSessionMode::RecordingPlayback;

        assert_eq!(
            TerminalSessionMode::RecordingPlayback,
            terminal.session_mode()
        );
        assert!(terminal.is_recording_playback());
        assert!(terminal.is_read_only());
        assert_eq!(None, terminal.live_connection_kind());
        assert!(!terminal.can_reconnect());
        assert!(terminal.recording_tap().is_none());

        terminal.write(b"human input");
        terminal.write_external_input(b"public mcp input");
        assert!(
            direct_writes
                .lock()
                .expect("direct input probe should lock")
                .is_empty()
        );
        assert!(
            external_writes
                .lock()
                .expect("external input probe should lock")
                .is_empty()
        );
        assert!(terminal.external_input_handle().is_none());
        assert!(terminal.external_exec_handle().is_none());
        assert!(terminal.external_control_handle().is_none());

        let expected = RecordingRuntimeError::ReadOnlyPlayback;
        assert_eq!(Err(expected.clone()), terminal.recording_snapshot());
        assert_eq!(
            Some(expected.clone()),
            terminal
                .build_output_recording_start_request(PathBuf::from("unused-output.cast"))
                .err()
        );
        assert_eq!(Err(expected.clone()), terminal.pause_recording());
        assert_eq!(Err(expected.clone()), terminal.resume_recording());
        assert_eq!(Err(expected.clone()), terminal.stop_recording());
        assert_eq!(
            Err(expected),
            terminal.start_recording(test_recording_start_request(PathBuf::from("unused.cast")))
        );

        terminal.shutdown();
        terminal.shutdown();
    }

    #[test]
    fn recording_playback_constructor_integrates_read_only_surface_and_controls() {
        let completeness = RecordingCompleteness::Partial {
            discarded_bytes: 42,
        };
        let playback = RecordingPlayback::from_parsed(
            test_parsed_playback_recording(RecordingBackend::Ssh, completeness.clone()),
            RecordingPlaybackLimits::default(),
        )
        .expect("validate recording playback");
        let (mut terminal, _event_loop) = Terminal::build_recording_playback(playback, 100_000);

        assert_eq!(
            TerminalSessionMode::RecordingPlayback,
            terminal.session_mode()
        );
        assert!(terminal.is_read_only());
        assert_eq!(TerminalConnectionKind::Ssh, terminal.connection_kind());
        assert_eq!(None, terminal.live_connection_kind());
        assert_eq!(&ConnectionState::Connected, terminal.connection_state());
        assert!(terminal.backend.is_none());
        assert!(terminal.ssh_config().is_none());
        assert!(terminal.ssh_session_manager().is_none());
        assert!(terminal.external_input_handle().is_none());
        assert!(terminal.external_exec_handle().is_none());
        assert!(terminal.external_control_handle().is_none());
        assert!(!terminal.can_reconnect());
        assert_eq!(None, terminal.connection_id());
        assert_eq!(None, terminal.connection_name());
        assert_eq!(80, terminal.cols());
        assert_eq!(24, terminal.rows());
        assert_eq!(
            Some(RecordingPlaybackState::Paused),
            terminal.recording_playback_state()
        );
        assert_eq!(Some(Duration::ZERO), terminal.recording_playback_elapsed());
        assert_eq!(
            Some(Duration::from_millis(4)),
            terminal.recording_playback_duration()
        );
        assert_eq!(Some(1.0), terminal.recording_playback_speed());
        assert_eq!(
            Some(&completeness),
            terminal.recording_playback_completeness()
        );
        assert!(
            !terminal
                .recording_playback_search_index_status()
                .expect("playback search index status")
                .truncated
        );
        assert_eq!(
            Err(RecordingRuntimeError::ReadOnlyPlayback),
            terminal.recording_snapshot()
        );

        assert_eq!(
            RecordingPlaybackTransition::Changed,
            terminal
                .set_recording_playback_speed(2.0)
                .expect("set playback speed")
        );
        assert_eq!(
            RecordingPlaybackTransition::Changed,
            terminal
                .resume_recording_playback()
                .expect("resume playback")
        );
        terminal
            .advance_recording_playback(Duration::from_millis(1))
            .expect("advance playback");

        assert_eq!(
            Some(RecordingPlaybackState::Playing),
            terminal.recording_playback_state()
        );
        assert_eq!(
            Some(Duration::from_millis(2)),
            terminal.recording_playback_elapsed()
        );
        assert_eq!(Some(2.0), terminal.recording_playback_speed());
        assert_eq!(100, terminal.cols());
        assert_eq!(40, terminal.rows());
        let text = terminal.visible_text();
        assert!(text.contains("one"));
        assert!(text.contains("two"));

        // Viewport layout must not overwrite dimensions sourced from the
        // recording's Resize event.
        terminal.resize(200, 60, 1_600, 900);
        assert_eq!(100, terminal.cols());
        assert_eq!(40, terminal.rows());

        assert!(matches!(
            terminal.set_recording_playback_speed(5.0),
            Err(RecordingPlaybackError::InvalidSpeed(_))
        ));
        terminal
            .seek_recording_playback(Duration::ZERO)
            .expect("seek playback to start");
        assert_eq!(
            RecordingPlaybackTransition::Changed,
            terminal.pause_recording_playback().expect("pause playback")
        );
        terminal.set_scrollback_lines(2_000);

        assert_eq!(80, terminal.cols());
        assert_eq!(24, terminal.rows());
        assert_eq!(2_000, terminal.scrollback_lines());
        assert_eq!(
            Some(RecordingPlaybackState::Paused),
            terminal.recording_playback_state()
        );
        assert_eq!(Some(Duration::ZERO), terminal.recording_playback_elapsed());
        let text = terminal.visible_text();
        assert!(text.contains("one"));
        assert!(!text.contains("two"));

        let input_results = terminal
            .search_recording_playback("typed-command", 10)
            .expect("search display-only input");
        assert_eq!(1, input_results.matches.len());
        assert_eq!(
            RecordingPlaybackSearchKind::InputDisplayOnly,
            input_results.matches[0].kind
        );
        assert!(!input_results.index_status.truncated);

        let marker_results = terminal
            .search_recording_playback("checkpoint", 10)
            .expect("search display-only marker");
        assert_eq!(1, marker_results.matches.len());
        assert_eq!(
            RecordingPlaybackSearchKind::MarkerDisplayOnly,
            marker_results.matches[0].kind
        );
    }

    #[test]
    fn session_log_constructor_materializes_static_read_only_terminal_history() {
        let mut parsed =
            test_parsed_playback_recording(RecordingBackend::Ssh, RecordingCompleteness::Complete);
        parsed.header.navop.artifact_kind = RecordingArtifactKind::SessionLog;
        let playback = RecordingPlayback::from_parsed(parsed, RecordingPlaybackLimits::default())
            .expect("validate session log");
        let (mut terminal, _event_loop) = Terminal::build_session_log(playback, 100_000);

        assert_eq!(TerminalSessionMode::SessionLog, terminal.session_mode());
        assert!(terminal.is_session_log());
        assert!(terminal.is_read_only());
        assert!(!terminal.is_recording_playback());
        assert_eq!(TerminalConnectionKind::Ssh, terminal.connection_kind());
        assert_eq!(None, terminal.live_connection_kind());
        assert!(terminal.backend.is_none());
        assert!(terminal.external_input_handle().is_none());
        assert!(terminal.external_exec_handle().is_none());
        assert!(terminal.external_control_handle().is_none());
        assert!(!terminal.can_reconnect());

        assert_eq!(100, terminal.cols());
        assert_eq!(40, terminal.rows());
        let text = terminal.visible_text();
        assert!(text.contains("one"));
        assert!(text.contains("two"));
        assert!(!text.contains("typed-command"));
        assert!(!text.contains("checkpoint"));

        assert_eq!(None, terminal.recording_playback_state());
        assert_eq!(None, terminal.recording_playback_elapsed());
        assert_eq!(None, terminal.recording_playback_duration());
        assert_eq!(None, terminal.recording_playback_speed());
        assert!(matches!(
            terminal.resume_recording_playback(),
            Err(RecordingPlaybackError::NotPlaybackSession)
        ));
        assert!(matches!(
            terminal.pause_recording_playback(),
            Err(RecordingPlaybackError::NotPlaybackSession)
        ));
        assert!(matches!(
            terminal.set_recording_playback_speed(2.0),
            Err(RecordingPlaybackError::NotPlaybackSession)
        ));
        assert!(matches!(
            terminal.advance_recording_playback(Duration::from_millis(1)),
            Err(RecordingPlaybackError::NotPlaybackSession)
        ));
        assert!(matches!(
            terminal.seek_recording_playback(Duration::ZERO),
            Err(RecordingPlaybackError::NotPlaybackSession)
        ));
        assert!(matches!(
            terminal.search_recording_playback("one", 10),
            Err(RecordingPlaybackError::NotPlaybackSession)
        ));

        terminal.resize(200, 60, 1_600, 900);
        assert_eq!(100, terminal.cols());
        assert_eq!(40, terminal.rows());
    }

    #[test]
    fn recording_playback_source_kind_never_restores_live_capabilities() {
        for (backend, expected_kind) in [
            (RecordingBackend::Local, TerminalConnectionKind::Local),
            (RecordingBackend::Ssh, TerminalConnectionKind::Ssh),
            (RecordingBackend::Serial, TerminalConnectionKind::Serial),
        ] {
            let playback = RecordingPlayback::from_parsed(
                test_parsed_playback_recording(backend, RecordingCompleteness::Complete),
                RecordingPlaybackLimits::default(),
            )
            .expect("validate recording playback");
            let (terminal, _event_loop) = Terminal::build_recording_playback(playback, 10_000);

            assert_eq!(expected_kind, terminal.connection_kind());
            assert_eq!(None, terminal.live_connection_kind());
            assert!(terminal.backend.is_none());
            assert!(terminal.ssh_config.is_none());
            assert!(terminal.ssh_session_manager.is_none());
            assert!(terminal.serial_params.is_none());
            assert!(terminal.event_proxy.is_none());
            assert_eq!(None, terminal.connection_id());
            assert_eq!(None, terminal.connection_name());
            assert!(terminal.history_repository.is_none());
            assert!(terminal.history_scope.is_none());
            assert_eq!(0, terminal.connection_generation);
            assert!(!terminal.can_reconnect());
            assert!(terminal.external_input_handle().is_none());
            assert!(terminal.external_exec_handle().is_none());
            assert!(terminal.external_control_handle().is_none());
        }
    }

    #[test]
    fn live_terminal_rejects_recording_playback_operations() {
        let runtime = RecordingRuntime::new(RecordingRuntimeConfig::default())
            .expect("create recording runtime");
        let mut terminal = test_terminal_with_recording_runtime(Ok(runtime));

        assert_eq!(None, terminal.recording_playback_state());
        assert!(matches!(
            terminal.resume_recording_playback(),
            Err(RecordingPlaybackError::NotPlaybackSession)
        ));
        assert!(matches!(
            terminal.pause_recording_playback(),
            Err(RecordingPlaybackError::NotPlaybackSession)
        ));
        assert!(matches!(
            terminal.set_recording_playback_speed(2.0),
            Err(RecordingPlaybackError::NotPlaybackSession)
        ));
        assert!(matches!(
            terminal.advance_recording_playback(Duration::from_millis(1)),
            Err(RecordingPlaybackError::NotPlaybackSession)
        ));
        assert!(matches!(
            terminal.seek_recording_playback(Duration::ZERO),
            Err(RecordingPlaybackError::NotPlaybackSession)
        ));
        assert!(matches!(
            terminal.search_recording_playback("query", 10),
            Err(RecordingPlaybackError::NotPlaybackSession)
        ));

        terminal.shutdown();
        terminal.shutdown();
    }

    #[test]
    fn terminal_recording_api_preserves_initialization_error() {
        let expected = RecordingRuntimeError::InvalidConfig(
            "recording worker could not be initialized".to_string(),
        );
        let terminal = test_terminal_with_recording_runtime(Err(expected.clone()));
        let request = test_recording_start_request(PathBuf::from("unused.cast"));

        assert_eq!(Err(expected.clone()), terminal.recording_snapshot());
        assert_eq!(
            Some(expected.clone()),
            terminal
                .build_output_recording_start_request(PathBuf::from("unused-output.cast"))
                .err()
        );
        assert_eq!(
            Err(expected.clone()),
            terminal.start_output_recording(PathBuf::from("unused-output.cast"))
        );
        assert_eq!(Err(expected.clone()), terminal.start_recording(request));
        assert_eq!(Err(expected.clone()), terminal.pause_recording());
        assert_eq!(Err(expected.clone()), terminal.resume_recording());
        assert_eq!(Err(expected.clone()), terminal.stop_recording());

        terminal.shutdown();
        terminal.shutdown();
    }

    #[test]
    fn build_cd_command_escapes_injection_chars() {
        let cmd = build_cd_command("dir; rm -rf /");
        assert_eq!(cmd, "cd -- 'dir; rm -rf /'");
    }

    #[test]
    fn build_cd_command_escapes_newline() {
        let cmd = build_cd_command("a\nb");
        assert_eq!(cmd, "cd -- 'a\nb'");
    }

    #[test]
    fn resolved_ssh_connection_uses_latest_stored_parameters() {
        let mut connection = StoredConnection::new_ssh(
            "Latest SSH".to_string(),
            SshParams {
                sftp_account: None,
                host: "latest.example".to_string(),
                port: 2222,
                username: "latest-user".to_string(),
                auth_method: SshAuthMethod::Password {
                    password: "latest-password".to_string(),
                },
                credential_reference: None,
                prompt_username: None,
                prompt_password: None,
                keyboard_interactive: None,
                terminal_encoding: Default::default(),
                terminal_type: one_core::storage::StoredTerminalType::Xterm,
                connect_timeout: None,
                keepalive_interval: None,
                keepalive_max: None,
                default_directory: Some("/srv/default".to_string()),
                init_script: Some("echo ready".to_string()),
                disable_shell_integration: None,
                x11_forwarding: None,
                allow_legacy_algorithms: None,
                jump_server: None,
                proxy: None,
                os_id: None,
                icon: None,
                icon_file_path: None,
                account_expect: Default::default(),
            },
            None,
        );
        connection.id = Some(42);
        let (event_tx, _event_rx) = unbounded_channel();

        let resolved = resolve_ssh_connection(
            SshConnectionUpdate {
                connection,
                working_dir: Some("/srv/current".to_string()),
                sync_path_with_terminal: true,
            },
            event_tx,
        )
        .expect("最新 SSH 配置应可解析");

        assert_eq!("latest.example", resolved.config.ssh_config.host);
        assert_eq!(2222, resolved.config.ssh_config.port);
        assert_eq!("latest-user", resolved.config.ssh_config.username);
        assert!(matches!(
            resolved.config.ssh_config.auth,
            SshAuth::Password(ref password) if password == "latest-password"
        ));
        assert_eq!("xterm", resolved.config.pty_config.term);
        assert_eq!(Some(42), resolved.connection_id);
        assert_eq!("Latest SSH", resolved.connection_name);
        assert!(
            resolved
                .init_commands
                .as_deref()
                .is_some_and(|commands| commands.contains("/srv/current"))
        );
    }

    #[test]
    fn terminal_ssh_credentials_are_injected_without_mutating_base() {
        let connection = StoredConnection::new_ssh(
            "Prompted SSH".to_string(),
            SshParams {
                sftp_account: None,
                host: "prompted.example".to_string(),
                port: 22,
                username: "stored-user".to_string(),
                auth_method: SshAuthMethod::Password {
                    password: "stored-password".to_string(),
                },
                credential_reference: None,
                prompt_username: None,
                prompt_password: None,
                keyboard_interactive: None,
                terminal_encoding: Default::default(),
                terminal_type: Default::default(),
                connect_timeout: None,
                keepalive_interval: None,
                keepalive_max: None,
                default_directory: None,
                init_script: None,
                disable_shell_integration: None,
                x11_forwarding: None,
                allow_legacy_algorithms: None,
                jump_server: None,
                proxy: None,
                os_id: None,
                icon: None,
                icon_file_path: None,
                account_expect: Default::default(),
            },
            None,
        );
        let (resolve_event_tx, _resolve_event_rx) = unbounded_channel();
        let base = resolve_ssh_connection(
            SshConnectionUpdate {
                connection,
                working_dir: None,
                sync_path_with_terminal: false,
            },
            resolve_event_tx,
        )
        .expect("SSH 配置应可解析")
        .config;
        let (event_tx, _event_rx) = unbounded_channel();

        let (runtime, _responder) = ssh_config_with_runtime_credentials(
            &base,
            &TerminalSshCredentials {
                username: Some("runtime-user".to_string()),
                password: Some("runtime-password".to_string()),
            },
            event_tx,
            true,
        )
        .expect("临时凭据应可注入运行时配置");

        assert_eq!("runtime-user", runtime.ssh_config.username);
        assert!(matches!(
            runtime.ssh_config.auth,
            SshAuth::Password(ref password) if password == "runtime-password"
        ));
        assert!(runtime.ssh_config.keyboard_interactive_responder.is_some());
        assert_eq!("stored-user", base.ssh_config.username);
        assert!(matches!(
            base.ssh_config.auth,
            SshAuth::Password(ref password) if password == "stored-password"
        ));
        assert!(base.ssh_config.keyboard_interactive_responder.is_none());
    }

    #[test]
    fn terminal_ssh_credentials_disable_keyboard_interactive_responder() {
        let connection = StoredConnection::new_ssh(
            "No keyboard-interactive".to_string(),
            SshParams {
                sftp_account: None,
                host: "no-ki.example".to_string(),
                port: 22,
                username: "user".to_string(),
                auth_method: SshAuthMethod::Password {
                    password: "password".to_string(),
                },
                credential_reference: None,
                prompt_username: None,
                prompt_password: None,
                keyboard_interactive: Some(false),
                terminal_encoding: Default::default(),
                terminal_type: Default::default(),
                connect_timeout: None,
                keepalive_interval: None,
                keepalive_max: None,
                default_directory: None,
                init_script: None,
                disable_shell_integration: None,
                x11_forwarding: None,
                allow_legacy_algorithms: None,
                jump_server: None,
                proxy: None,
                os_id: None,
                icon: None,
                icon_file_path: None,
                account_expect: Default::default(),
            },
            None,
        );
        let (resolve_event_tx, _resolve_event_rx) = unbounded_channel();
        let base = resolve_ssh_connection(
            SshConnectionUpdate {
                connection,
                working_dir: None,
                sync_path_with_terminal: false,
            },
            resolve_event_tx,
        )
        .expect("SSH 配置应可解析")
        .config;
        let (event_tx, _event_rx) = unbounded_channel();

        let (runtime, _responder) = ssh_config_with_runtime_credentials(
            &base,
            &TerminalSshCredentials::default(),
            event_tx,
            false,
        )
        .expect("关闭 keyboard-interactive 时仍应生成 SSH 配置");

        assert!(runtime.ssh_config.keyboard_interactive_responder.is_none());
    }

    #[test]
    fn host_key_retry_preserves_runtime_credentials_without_mutating_base() {
        let connection = StoredConnection::new_ssh(
            "Host-key retry".to_string(),
            SshParams {
                sftp_account: None,
                host: "host-key.example".to_string(),
                port: 22,
                username: "stored-user".to_string(),
                auth_method: SshAuthMethod::Password {
                    password: "stored-password".to_string(),
                },
                credential_reference: None,
                prompt_username: None,
                prompt_password: None,
                keyboard_interactive: None,
                terminal_encoding: Default::default(),
                terminal_type: Default::default(),
                connect_timeout: None,
                keepalive_interval: None,
                keepalive_max: None,
                default_directory: None,
                init_script: None,
                disable_shell_integration: None,
                x11_forwarding: None,
                allow_legacy_algorithms: None,
                jump_server: None,
                proxy: None,
                os_id: None,
                icon: None,
                icon_file_path: None,
                account_expect: Default::default(),
            },
            None,
        );
        let (resolve_event_tx, _resolve_event_rx) = unbounded_channel();
        let base = resolve_ssh_connection(
            SshConnectionUpdate {
                connection,
                working_dir: None,
                sync_path_with_terminal: false,
            },
            resolve_event_tx,
        )
        .expect("SSH 配置应可解析")
        .config;
        let (event_tx, _event_rx) = unbounded_channel();
        let (runtime, _responder) = ssh_config_with_runtime_credentials(
            &base,
            &TerminalSshCredentials {
                username: Some("runtime-user".to_string()),
                password: Some("runtime-password".to_string()),
            },
            event_tx,
            true,
        )
        .expect("临时凭据应可注入运行时配置");
        let request = HostKeyVerificationRequest {
            identity: HostKeyIdentity::new("host-key.example", 22, HostKeyRoute::Direct),
            presented: HostKeyDetails {
                algorithm: "ssh-ed25519".to_string(),
                fingerprint: "SHA256:test".to_string(),
            },
            reason: HostKeyVerificationReason::Unknown,
        };

        let retry = ssh_config_with_confirmed_host_key(&runtime, &request, false);

        assert_eq!("runtime-user", retry.ssh_config.username);
        assert!(matches!(
            retry.ssh_config.auth,
            SshAuth::Password(ref password) if password == "runtime-password"
        ));
        assert_eq!("stored-user", base.ssh_config.username);
        assert!(matches!(
            base.ssh_config.auth,
            SshAuth::Password(ref password) if password == "stored-password"
        ));
    }

    #[test]
    fn clear_screen_requests_remote_prompt_redraw_for_ssh_only() {
        assert_eq!(
            Some(b"\x0c".as_slice()),
            clear_screen_remote_redraw_bytes(TerminalConnectionKind::Ssh)
        );
        assert_eq!(
            None,
            clear_screen_remote_redraw_bytes(TerminalConnectionKind::Local)
        );
        assert_eq!(
            None,
            clear_screen_remote_redraw_bytes(TerminalConnectionKind::Serial)
        );
    }

    #[test]
    fn command_record_gate_accepts_success_when_finish_precedes_record() {
        let mut gate = CommandRecordGate::default();

        gate.command_started();
        assert_eq!(None, gate.command_finished(0));

        assert_eq!(
            Some(("git status".to_string(), 0)),
            gate.command_recorded("git status".to_string())
        );
    }

    #[test]
    fn command_record_gate_accepts_success_when_record_precedes_finish() {
        let mut gate = CommandRecordGate::default();

        gate.command_started();
        assert_eq!(None, gate.command_recorded("cargo test".to_string()));

        assert_eq!(
            Some(("cargo test".to_string(), 0)),
            gate.command_finished(0)
        );
    }

    #[test]
    fn command_record_gate_discards_failed_commands() {
        let mut gate = CommandRecordGate::default();

        gate.command_started();
        gate.command_finished(127);

        assert_eq!(None, gate.command_recorded("missing-command".to_string()));
    }

    #[test]
    fn command_record_gate_discards_record_without_command_start() {
        let mut gate = CommandRecordGate::default();

        assert_eq!(None, gate.command_recorded("git status".to_string()));
        assert_eq!(None, gate.command_finished(0));
    }

    #[test]
    fn merge_history_matches_prefers_database_and_deduplicates_fallback() {
        let merged = merge_history_matches(
            vec!["git status".to_string(), "cargo test".to_string()],
            vec![
                "git status".to_string(),
                "git stash".to_string(),
                "cargo test".to_string(),
            ],
            4,
        );

        assert_eq!(
            vec![
                "git status".to_string(),
                "cargo test".to_string(),
                "git stash".to_string(),
            ],
            merged
        );
    }

    #[test]
    fn merge_history_matches_respects_limit() {
        let merged = merge_history_matches(
            vec!["first".to_string(), "second".to_string()],
            vec!["third".to_string()],
            2,
        );

        assert_eq!(vec!["first".to_string(), "second".to_string()], merged);
    }

    #[test]
    fn normalize_history_matches_strips_recorded_prefix_and_deduplicates() {
        let matches = normalize_history_matches(
            vec![
                "2026-07-05 08:07:16 root cd /data/app".to_string(),
                "cd /data/app".to_string(),
                "git status".to_string(),
            ],
            Some("root"),
            10,
        );

        assert_eq!(
            vec!["cd /data/app".to_string(), "git status".to_string()],
            matches
        );
    }

    #[test]
    fn resolve_local_working_dir_uses_home_when_unspecified() {
        assert_eq!(dirs::home_dir(), resolve_local_working_dir(None));
    }

    #[test]
    fn resolve_local_working_dir_keeps_explicit_directory() {
        assert_eq!(
            Some(std::path::PathBuf::from("/tmp/onetcli")),
            resolve_local_working_dir(Some("/tmp/onetcli".to_string()))
        );
    }

    #[test]
    fn resolve_local_working_dir_treats_blank_as_unspecified() {
        assert_eq!(
            dirs::home_dir(),
            resolve_local_working_dir(Some("  ".to_string()))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn with_local_terminal_default_env_adds_homebrew_paths_to_existing_path() {
        let env = with_local_terminal_default_env(vec![(
            "PATH".to_string(),
            "/usr/bin:/bin".to_string(),
        )]);
        let path = env
            .iter()
            .find_map(|(key, value)| (key == "PATH").then_some(value.as_str()))
            .expect("local terminal env should include PATH");

        assert!(path.contains("/opt/homebrew/bin"));
        assert!(path.contains("/opt/homebrew/sbin"));
        assert!(path.contains("/usr/local/bin"));
        assert!(path.ends_with("/usr/bin:/bin"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn with_local_terminal_default_env_adds_utf8_locale_when_missing() {
        let env = with_local_terminal_default_env(vec![]);

        assert_eq!(Some("en_US.UTF-8"), env_value(&env, "LANG"));
        assert_eq!(Some("UTF-8"), env_value(&env, "LC_CTYPE"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn with_local_terminal_default_env_keeps_explicit_locale() {
        let env = with_local_terminal_default_env(vec![
            ("LANG".to_string(), "zh_CN.UTF-8".to_string()),
            ("LC_CTYPE".to_string(), "zh_CN.UTF-8".to_string()),
        ]);

        assert_eq!(Some("zh_CN.UTF-8"), env_value(&env, "LANG"));
        assert_eq!(Some("zh_CN.UTF-8"), env_value(&env, "LC_CTYPE"));
    }

    #[cfg(target_os = "macos")]
    fn env_value<'a>(env: &'a [(String, String)], key: &str) -> Option<&'a str> {
        env.iter()
            .find_map(|(name, value)| (name == key).then_some(value.as_str()))
    }

    #[test]
    fn build_ssh_init_commands_ignores_sync_path_switch_for_script_integration() {
        let enabled = build_ssh_init_commands(None, Some("/tmp"), Some("echo ready"), true)
            .expect("启用路径同步时应保留基础初始化命令");

        let disabled = build_ssh_init_commands(None, Some("/tmp"), Some("echo ready"), false)
            .expect("禁用路径同步时仍应保留其它初始化命令");
        assert_eq!(enabled, disabled);
        assert!(disabled.contains("echo ready"));
    }

    #[test]
    fn build_ssh_base_init_commands_prioritizes_explicit_working_dir() {
        let commands =
            build_ssh_base_init_commands(Some("/workspace"), Some("/default"), Some("echo ready"))
                .expect("显式工作目录应生成初始化命令");

        assert!(commands.contains("cd -- '/workspace'"));
        assert!(!commands.contains("/default"));
        assert!(!commands.contains("echo ready"));
    }

    #[test]
    fn compose_ssh_init_commands_supports_base_commands_only() {
        let commands =
            compose_ssh_init_commands(Some("echo ready"), true).expect("应保留基础初始化命令");
        assert_eq!(commands, "echo ready");

        assert!(
            compose_ssh_init_commands(None, false).is_none(),
            "无基础命令时不应生成初始化命令"
        );
        assert!(
            compose_ssh_init_commands(None, true).is_none(),
            "同步开关不应单独生成额外初始化命令"
        );
    }

    #[test]
    fn resolve_default_windows_shell_prefers_pwsh_from_path() {
        let temp_dir =
            std::env::temp_dir().join(format!("onetcli-terminal-test-{}", std::process::id()));
        fs::create_dir_all(&temp_dir).expect("应创建临时目录");

        let pwsh = temp_dir.join("pwsh.exe");
        let cmd = temp_dir.join("cmd.exe");
        fs::write(&pwsh, b"").expect("应创建 pwsh 占位文件");
        fs::write(&cmd, b"").expect("应创建 cmd 占位文件");

        let path_env = std::ffi::OsString::from(temp_dir.as_os_str());
        let resolved = resolve_default_windows_shell_from_env(
            Some(path_env.as_os_str()),
            None,
            Some(cmd.as_os_str()),
        );

        assert_eq!(resolved, pwsh.to_string_lossy());
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn resolve_default_windows_shell_falls_back_to_comspec() {
        let temp_dir = std::env::temp_dir().join(format!(
            "onetcli-terminal-test-comspec-{}",
            std::process::id()
        ));
        fs::create_dir_all(&temp_dir).expect("应创建临时目录");

        let cmd = temp_dir.join("cmd.exe");
        fs::write(&cmd, b"").expect("应创建 cmd 占位文件");

        let resolved = resolve_default_windows_shell_from_env(None, None, Some(cmd.as_os_str()));

        assert_eq!(resolved, cmd.to_string_lossy());
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn prepare_shell_integration_writes_lf_only_script() {
        let session_dir = std::env::temp_dir().join(format!("onetcli-{}", std::process::id()));
        let integration_path = session_dir.join("shell_integration.sh");
        let _ = fs::remove_dir_all(&session_dir);

        let (_env, _args) = super::prepare_shell_integration(Some("/bin/bash"));

        let script = fs::read_to_string(&integration_path).expect("应写入本地 integration 脚本");
        assert!(
            !script.contains('\r'),
            "本地 shell integration 脚本应统一写成 LF，避免 Windows 工件污染"
        );

        let _ = fs::remove_dir_all(&session_dir);
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn zsh_shell_integration_sources_generated_zshrc() {
        let zsh = std::path::Path::new("/bin/zsh");
        if !zsh.exists() {
            return;
        }

        let session_dir = std::env::temp_dir().join(format!("onetcli-{}", std::process::id()));
        let home_dir =
            std::env::temp_dir().join(format!("onetcli-zsh-home-{}", std::process::id()));
        let _ = fs::remove_dir_all(&session_dir);
        let _ = fs::remove_dir_all(&home_dir);
        fs::create_dir_all(&home_dir).expect("应创建临时 HOME");

        let (env_pairs, args) = super::prepare_shell_integration(Some("/bin/zsh"));
        assert!(args.is_empty(), "zsh 注入不应依赖额外启动参数");

        let mut command = Command::new(zsh);
        command
            .arg("-i")
            .arg("-c")
            .arg("print -r -- ${_ONETCLI_SHELL_INTEGRATED:-missing}")
            .env("HOME", &home_dir);
        for (key, value) in env_pairs {
            command.env(key, value);
        }
        command.env("_ONETCLI_ORIG_ZDOTDIR", &home_dir);

        let output = command.output().expect("应能启动 zsh");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stdout.lines().any(|line| line == "1"),
            "zsh 应加载生成的 .zshrc 并 source shell integration，stdout: {stdout:?}"
        );
        assert!(
            !stderr.contains("recursion limit") && !stderr.contains("job table full"),
            "zsh integration 不应递归 source 临时 .zshrc，stderr: {stderr:?}"
        );

        let _ = fs::remove_dir_all(&session_dir);
        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn normalize_history_command_trims_and_rejects_blank_input() {
        assert_eq!(
            normalize_history_command("  git status  "),
            Some("git status".to_string())
        );
        assert_eq!(normalize_history_command("   "), None);
        assert_eq!(normalize_history_command("\n\t"), None);
    }

    #[test]
    fn parse_shell_history_supports_zsh_extended_format() {
        let commands = parse_shell_history(
            ": 1710000000:0;git status\n: 1710000001:0;cargo test\n",
            ShellHistoryFormat::Zsh,
        );

        assert_eq!(commands, vec!["git status", "cargo test"]);
    }

    #[test]
    fn push_history_entry_dedupes_adjacent_duplicates() {
        let mut entries = VecDeque::new();
        push_history_entry(&mut entries, "git status", 5);
        push_history_entry(&mut entries, "git status", 5);
        push_history_entry(&mut entries, "cargo test", 5);

        let commands: Vec<_> = entries.iter().map(|e| e.command.as_str()).collect();
        assert_eq!(commands, vec!["git status", "cargo test"]);
    }

    #[test]
    fn collect_history_suggestions_prioritizes_session_history() {
        let session: VecDeque<HistoryEntry> = ["git status", "git stash", "cargo test"]
            .iter()
            .map(|c| HistoryEntry::new(c.to_string()))
            .collect();
        let persisted = vec![
            "git status".to_string(),
            "git switch main".to_string(),
            "git commit".to_string(),
        ];

        let matches = collect_history_suggestions(&session, &persisted, "git s", 4);

        // session 中的结果优先（frecency 更高），且去重
        assert!(matches.contains(&"git stash".to_string()));
        assert!(matches.contains(&"git status".to_string()));
        assert!(matches.contains(&"git switch main".to_string()));
    }

    #[test]
    fn collect_history_suggestions_skips_empty_prefix() {
        let session: VecDeque<HistoryEntry> = [HistoryEntry::new("git status".to_string())].into();
        let persisted = vec!["git switch".to_string()];

        let matches = collect_history_suggestions(&session, &persisted, "   ", 5);

        assert!(matches.is_empty());
    }

    #[tokio::test]
    async fn terminal_mfa_responder_waits_for_submitted_response() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let responder = TerminalMfaResponder::new(event_tx, Some("jump123".to_string()), None);
        let pending = responder.clone();
        let task = tokio::spawn(async move {
            pending
                .respond(KeyboardInteractiveRequest {
                    target: ssh::KeyboardInteractiveTarget::JumpServer,
                    name: "MFA".to_string(),
                    instructions: "Enter code".to_string(),
                    prompts: vec![ssh::KeyboardInteractivePrompt {
                        prompt: "Verification code:".to_string(),
                        echo: false,
                    }],
                })
                .await
        });

        assert!(matches!(
            event_rx.recv().await,
            Some(TerminalEvent::SshMfaChanged)
        ));
        assert_eq!(
            Some(TerminalMfaRequest {
                name: "MFA".to_string(),
                instructions: "Enter code".to_string(),
                prompts: vec![TerminalMfaPrompt {
                    prompt: "Verification code:".to_string(),
                    echo: false,
                }],
            }),
            responder.pending_request()
        );
        assert!(responder.submit(vec!["123456".to_string()]));
        assert!(matches!(
            event_rx.recv().await,
            Some(TerminalEvent::SshMfaChanged)
        ));
        assert_eq!(vec!["123456".to_string()], task.await.unwrap().unwrap());
    }

    #[tokio::test]
    async fn terminal_mfa_responder_prompts_for_one_time_password() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let responder = TerminalMfaResponder::new(event_tx, Some("login-secret".to_string()), None);
        let pending = responder.clone();
        let task = tokio::spawn(async move {
            pending
                .respond(KeyboardInteractiveRequest {
                    target: ssh::KeyboardInteractiveTarget::JumpServer,
                    name: "MFA".to_string(),
                    instructions: "Enter your one-time password".to_string(),
                    prompts: vec![ssh::KeyboardInteractivePrompt {
                        prompt: "One-time password:".to_string(),
                        echo: false,
                    }],
                })
                .await
        });

        assert!(matches!(
            tokio::time::timeout(std::time::Duration::from_secs(1), event_rx.recv()).await,
            Ok(Some(TerminalEvent::SshMfaChanged))
        ));
        assert_eq!(
            Some(TerminalMfaRequest {
                name: "MFA".to_string(),
                instructions: "Enter your one-time password".to_string(),
                prompts: vec![TerminalMfaPrompt {
                    prompt: "One-time password:".to_string(),
                    echo: false,
                }],
            }),
            responder.pending_request()
        );
        assert!(responder.submit(vec!["123456".to_string()]));
        assert!(matches!(
            event_rx.recv().await,
            Some(TerminalEvent::SshMfaChanged)
        ));
        assert_eq!(vec!["123456".to_string()], task.await.unwrap().unwrap());
    }

    #[tokio::test]
    async fn terminal_mfa_responder_cancel_clears_pending_request() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let responder = TerminalMfaResponder::new(event_tx, None, None);
        let pending = responder.clone();
        let task = tokio::spawn(async move {
            pending
                .respond(KeyboardInteractiveRequest {
                    target: ssh::KeyboardInteractiveTarget::TargetServer,
                    name: "MFA".to_string(),
                    instructions: "Enter code".to_string(),
                    prompts: vec![ssh::KeyboardInteractivePrompt {
                        prompt: "Verification code:".to_string(),
                        echo: false,
                    }],
                })
                .await
        });

        assert!(matches!(
            event_rx.recv().await,
            Some(TerminalEvent::SshMfaChanged)
        ));
        assert!(responder.pending_request().is_some());
        assert!(responder.cancel());
        assert!(matches!(
            event_rx.recv().await,
            Some(TerminalEvent::SshMfaChanged)
        ));
        assert!(responder.pending_request().is_none());
        assert!(task.await.unwrap().is_err());
        assert!(!responder.cancel());
    }

    #[test]
    fn inject_system_text_writes_into_the_grid_and_scrolls_to_bottom() {
        let runtime = RecordingRuntime::new(RecordingRuntimeConfig::default())
            .expect("create recording runtime");
        let terminal = test_terminal_with_recording_runtime(Ok(runtime));

        // 预写一段输出并向上滚动，模拟用户正在回看历史
        {
            let mut term = terminal.term.lock();
            let mut processor: Processor<StdSyncHandler> = Processor::new();
            processor.advance(&mut *term, b"remote output\r\n");
            term.scroll_display(alacritty_terminal::grid::Scroll::Top);
        }

        assert!(terminal.inject_system_text(
            "\r\n\x1b[33m[connection lost]\x1b[0m\r\n\x1b[2mpress enter to reconnect\x1b[0m\r\n"
        ));

        let snapshot = recent_text_from_term(&terminal.term, 10);
        assert!(snapshot.text.contains("[connection lost]"));
        assert!(snapshot.text.contains("press enter to reconnect"));
        assert!(snapshot.text.contains("remote output"));
        assert_eq!(0, terminal.term.lock().grid().display_offset());
        assert!(!terminal.inject_system_text(""));
    }

    #[test]
    fn inject_system_text_ignores_read_only_surfaces() {
        let runtime = RecordingRuntime::new(RecordingRuntimeConfig::default())
            .expect("create recording runtime");
        let mut terminal = test_terminal_with_recording_runtime(Ok(runtime));
        terminal.session_mode = TerminalSessionMode::RecordingPlayback;

        assert!(!terminal.inject_system_text("should not appear\r\n"));
        let snapshot = recent_text_from_term(&terminal.term, 5);
        assert!(!snapshot.text.contains("should not appear"));
    }

    #[tokio::test]
    async fn cancel_ssh_mfa_reports_whether_a_request_was_pending() {
        let runtime = RecordingRuntime::new(RecordingRuntimeConfig::default())
            .expect("create recording runtime");
        let mut terminal = test_terminal_with_recording_runtime(Ok(runtime));
        assert!(!terminal.cancel_ssh_mfa());

        let (event_tx, mut event_rx) = unbounded_channel();
        let responder = TerminalMfaResponder::new(event_tx, None, None);
        terminal.ssh_mfa_responder = Some(responder);
        let pending = terminal.ssh_mfa_responder.as_ref().unwrap().clone();
        let task = tokio::spawn(async move {
            pending
                .respond(KeyboardInteractiveRequest {
                    target: ssh::KeyboardInteractiveTarget::TargetServer,
                    name: "MFA".to_string(),
                    instructions: String::new(),
                    prompts: vec![ssh::KeyboardInteractivePrompt {
                        prompt: "Verification code:".to_string(),
                        echo: false,
                    }],
                })
                .await
        });
        assert!(matches!(
            event_rx.recv().await,
            Some(TerminalEvent::SshMfaChanged)
        ));

        assert!(terminal.cancel_ssh_mfa());
        assert!(task.await.unwrap().is_err());
        assert!(!terminal.cancel_ssh_mfa());
    }

    #[test]
    fn terminal_mfa_answers_jump_mfa_and_both_password_rounds() {
        let jump_password = keyboard_interactive_answers_for_terminal(
            &KeyboardInteractiveRequest {
                target: ssh::KeyboardInteractiveTarget::JumpServer,
                name: String::new(),
                instructions: String::new(),
                prompts: vec![ssh::KeyboardInteractivePrompt {
                    prompt: "Password:".to_string(),
                    echo: false,
                }],
            },
            &[],
            Some("jump123"),
            Some("target123"),
        )
        .unwrap();
        assert_eq!(vec!["jump123".to_string()], jump_password);

        let jump_mfa = keyboard_interactive_answers_for_terminal(
            &KeyboardInteractiveRequest {
                target: ssh::KeyboardInteractiveTarget::JumpServer,
                name: String::new(),
                instructions: String::new(),
                prompts: vec![ssh::KeyboardInteractivePrompt {
                    prompt: "Verification code:".to_string(),
                    echo: false,
                }],
            },
            &["123456".to_string()],
            Some("jump123"),
            Some("target123"),
        )
        .unwrap();
        assert_eq!(vec!["123456".to_string()], jump_mfa);

        let target_password = keyboard_interactive_answers_for_terminal(
            &KeyboardInteractiveRequest {
                target: ssh::KeyboardInteractiveTarget::TargetServer,
                name: String::new(),
                instructions: String::new(),
                prompts: vec![ssh::KeyboardInteractivePrompt {
                    prompt: "root@10.2.4.56's password:".to_string(),
                    echo: false,
                }],
            },
            &[],
            Some("jump123"),
            Some("target123"),
        )
        .unwrap();
        assert_eq!(vec!["target123".to_string()], target_password);
    }

    #[test]
    fn terminal_mfa_does_not_replace_one_time_password_with_login_password() {
        let answers = keyboard_interactive_answers_for_terminal(
            &KeyboardInteractiveRequest {
                target: ssh::KeyboardInteractiveTarget::TargetServer,
                name: "MFA".to_string(),
                instructions: "Enter your one-time password".to_string(),
                prompts: vec![ssh::KeyboardInteractivePrompt {
                    prompt: "One-time password:".to_string(),
                    echo: false,
                }],
            },
            &["123456".to_string()],
            None,
            Some("login-secret"),
        )
        .unwrap();

        assert_eq!(vec!["123456".to_string()], answers);
    }

    #[test]
    fn terminal_mfa_recognizes_common_login_password_prompts_without_misclassifying_codes() {
        for prompt in [
            "Password:",
            "root@host's password:",
            "Password for navop:",
            "Enter password:",
        ] {
            assert!(is_ssh_password_prompt(prompt), "{prompt}");
        }

        for prompt in [
            "One-time password:",
            "OTP:",
            "Verification code:",
            "Security code:",
            "Authentication code:",
            "Passcode:",
            "Token:",
        ] {
            assert!(!is_ssh_password_prompt(prompt), "{prompt}");
        }
    }

    #[test]
    fn format_connection_error_keeps_anyhow_context_chain() {
        let err = anyhow!("channel open failed")
            .context("shell setup channel failed")
            .context("SSH connect failed");

        let message = format_connection_error(&err);

        assert!(
            message.contains("SSH connect failed"),
            "格式化结果应保留顶层上下文，实际: {message}"
        );
        assert!(
            message.contains("shell setup channel failed"),
            "格式化结果应保留中间上下文，实际: {message}"
        );
        assert!(
            message.contains("channel open failed"),
            "格式化结果应保留底层错误，实际: {message}"
        );
    }

    #[test]
    fn unknown_host_key_error_becomes_verification_request() {
        let identity = HostKeyIdentity::new("host.example", 22, HostKeyRoute::Direct);
        let presented = HostKeyDetails {
            algorithm: "ssh-ed25519".to_string(),
            fingerprint: "SHA256:test".to_string(),
        };
        let error = anyhow::Error::new(HostKeyRejection::Unknown {
            identity: identity.clone(),
            presented: presented.clone(),
        })
        .context("SSH connect failed");

        let request =
            host_key_verification_request(&error).expect("unknown key should require confirmation");

        assert_eq!(request.identity, identity);
        assert_eq!(request.presented, presented);
        assert_eq!(request.reason, HostKeyVerificationReason::Unknown);
    }

    #[test]
    fn changed_host_key_error_becomes_verification_request() {
        let identity = HostKeyIdentity::new("host.example", 22, HostKeyRoute::Direct);
        let presented = HostKeyDetails {
            algorithm: "ssh-ed25519".to_string(),
            fingerprint: "SHA256:new".to_string(),
        };
        let expected = vec![HostKeyDetails {
            algorithm: "ssh-ed25519".to_string(),
            fingerprint: "SHA256:old".to_string(),
        }];
        let error = anyhow::Error::new(HostKeyRejection::Changed {
            identity: identity.clone(),
            presented: presented.clone(),
            expected: expected.clone(),
        });

        let request =
            host_key_verification_request(&error).expect("changed key should require confirmation");

        assert_eq!(request.identity, identity);
        assert_eq!(request.presented, presented);
        assert_eq!(
            request.reason,
            HostKeyVerificationReason::Changed { expected }
        );
    }

    #[test]
    fn create_term_shares_performance_metrics_with_event_proxy() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let metrics = Arc::new(TerminalPerformanceMetrics::enabled());
        let (_term, event_proxy, _colors, metrics) =
            Terminal::create_term_with_metrics(80, 24, 10_000, event_tx, metrics);

        assert!(Arc::ptr_eq(&metrics, &event_proxy.performance_metrics()));

        event_proxy.send_event(AlacTermEvent::Wakeup);
        assert!(matches!(event_rx.try_recv(), Ok(TerminalEvent::Wakeup)));

        let snapshot = metrics.snapshot();
        assert_eq!(1, snapshot.wakeup_requests);
        assert_eq!(1, snapshot.wakeup_queued);
    }

    #[test]
    fn wakeup_gate_stays_closed_until_gpui_acknowledges_forwarded_event() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let (_term, event_proxy, _colors, _metrics) =
            Terminal::create_term(80, 24, 10_000, event_tx.clone());
        let wakeup_pending = event_proxy.wakeup_pending_handle();
        let (render_tx, mut render_rx) = futures::channel::mpsc::unbounded();

        event_proxy.send_event(AlacTermEvent::Wakeup);
        let mut pending_events = Vec::new();
        let mut pending_wakeup = false;
        match event_rx.try_recv().expect("first Wakeup should be queued") {
            TerminalEvent::Wakeup => pending_wakeup = true,
            event => pending_events.push(event),
        }

        assert!(flush_pending_terminal_events(
            &render_tx,
            &mut pending_events,
            &mut pending_wakeup,
        ));
        assert!(
            wakeup_pending.load(Ordering::Acquire),
            "forwarding into the GPUI queue must not reopen the end-to-end Wakeup gate"
        );

        event_proxy.send_event(AlacTermEvent::Wakeup);
        assert!(
            event_rx.try_recv().is_err(),
            "new Wakeups must still coalesce while GPUI has not consumed the forwarded event"
        );
        send_coalesced_wakeup(&event_tx, &wakeup_pending);
        assert!(
            event_rx.try_recv().is_err(),
            "non-proxy invalidations must share the same end-to-end Wakeup gate"
        );

        let forwarded = futures::executor::block_on(receive_terminal_event_for_gpui(
            &mut render_rx,
            &wakeup_pending,
        ))
        .expect("GPUI should dequeue one forwarded Wakeup");
        assert!(matches!(forwarded, TerminalEvent::Wakeup));

        // The gate reopens as soon as GPUI dequeues the invalidation, before
        // handling it. Output produced concurrently with the handler therefore
        // queues the next Wakeup instead of being lost by a later reset.
        send_coalesced_wakeup(&event_tx, &wakeup_pending);
        assert!(matches!(event_rx.try_recv(), Ok(TerminalEvent::Wakeup)));
        event_proxy.send_event(AlacTermEvent::Wakeup);
        assert!(
            event_rx.try_recv().is_err(),
            "proxy Wakeups must coalesce behind a direct invalidation queued during handling"
        );
    }

    #[test]
    fn terminal_event_flush_preserves_semantic_order_and_finishes_with_wakeup() {
        let (render_tx, mut render_rx) = futures::channel::mpsc::unbounded();
        let mut pending_events = vec![
            TerminalEvent::TitleChanged("shell".to_string()),
            TerminalEvent::Bell,
            TerminalEvent::ChildExit(7),
        ];
        let mut pending_wakeup = true;

        assert!(flush_pending_terminal_events(
            &render_tx,
            &mut pending_events,
            &mut pending_wakeup,
        ));
        assert!(pending_events.is_empty());
        assert!(!pending_wakeup);

        assert!(matches!(
            render_rx.try_recv(),
            Ok(TerminalEvent::TitleChanged(title)) if title == "shell"
        ));
        assert!(matches!(render_rx.try_recv(), Ok(TerminalEvent::Bell)));
        assert!(matches!(
            render_rx.try_recv(),
            Ok(TerminalEvent::ChildExit(7))
        ));
        assert!(matches!(render_rx.try_recv(), Ok(TerminalEvent::Wakeup)));
        assert!(render_rx.try_recv().is_err());
    }

    #[test]
    fn reconnect_preparation_preserves_buffer_and_clears_stale_connection_metadata() {
        let (event_tx, _event_rx) = unbounded_channel();
        let (term, event_proxy, _colors, performance_metrics) = Terminal::create_term_with_metrics(
            80,
            24,
            10_000,
            event_tx.clone(),
            Arc::new(TerminalPerformanceMetrics::enabled()),
        );
        let wakeup_pending = event_proxy.wakeup_pending_handle();
        let shared_metrics = performance_metrics.clone();
        let original_term = term.clone();
        let mut terminal = Terminal {
            term,
            session_mode: TerminalSessionMode::Live,
            performance_metrics,
            backend: None,
            recording_runtime: Terminal::create_recording_runtime(
                event_tx.clone(),
                wakeup_pending.clone(),
            ),
            session_log_runtime: None,
            playback_runtime: None,
            recording_session_id: "surface-reset-test-session".to_string(),
            recording_session_metadata: RecordingSessionMetadata::default(),
            title: "old title".to_string(),
            current_working_dir: Some("/tmp/project".to_string()),
            child_exited: Some(255),
            connection_state: ConnectionState::Connected,
            session_lock: None,
            cols: 80,
            rows: 24,
            pixel_width: 0,
            pixel_height: 0,
            ssh_config: None,
            ssh_base_config: None,
            ssh_session_manager: None,
            ssh_credential_prompt_policy: SshCredentialPromptPolicy::default(),
            ssh_credential_request: None,
            ssh_keyboard_interactive_enabled: false,
            ssh_mfa_responder: None,
            zmodem_responder: None,
            pending_host_key_verification: None,
            serial_params: None,
            telnet_params: None,
            telnet_base_params: None,
            telnet_credential_request: None,
            wakeup_pending,
            event_tx: Some(event_tx),
            event_proxy: None,
            connection_id: Some(1),
            connection_name: Some("SSH".to_string()),
            init_commands: None,
            session_history: VecDeque::new(),
            persisted_history: Vec::new(),
            history_repository: None,
            history_scope: None,
            command_record_gate: CommandRecordGate::default(),
            connection_generation: 1,
            connection_kind: TerminalConnectionKind::Ssh,
            scrollback_lines: 10_000,
        };

        let mut processor: Processor<StdSyncHandler> = Processor::new();
        processor.advance(&mut *terminal.term.lock(), b"hello");

        assert_eq!(terminal.term.lock().grid()[Line(0)][Column(0)].c, 'h');
        processor.advance(&mut *terminal.term.lock(), b"\x1b[?1049hvim");
        {
            let term = terminal.term.lock();
            assert!(term.mode().contains(TermMode::ALT_SCREEN));
        }
        let previous = terminal.performance_snapshot();
        terminal
            .performance_metrics()
            .record_render(std::time::Duration::from_millis(4), true);
        let window = terminal.performance_window(&previous, std::time::Duration::from_secs(2));
        assert_eq!(1, window.render_samples);
        assert_eq!(4_000_000.0, window.average_render_ns);

        terminal.prepare_surface_for_reconnect();
        assert!(Arc::ptr_eq(
            &shared_metrics,
            &terminal.performance_metrics()
        ));
        assert!(Arc::ptr_eq(&original_term, &terminal.term));

        processor.advance(&mut *terminal.term.lock(), b" world");
        let term = terminal.term.lock();
        assert!(!term.mode().contains(TermMode::ALT_SCREEN));
        assert_eq!(term.grid()[Line(0)][Column(0)].c, 'h');
        assert_eq!(term.grid()[Line(0)][Column(5)].c, ' ');
        assert_eq!(term.grid()[Line(0)][Column(6)].c, 'w');
        assert_eq!(term.columns(), 80);
        assert_eq!(term.screen_lines(), 24);
        drop(term);
        assert_eq!(terminal.title(), "old title");
        assert_eq!(terminal.current_working_dir(), None);
        assert_eq!(terminal.child_exited(), None);
    }

    #[test]
    fn recent_text_reads_tail_from_scrollback_without_viewport_offset() {
        let (event_tx, _event_rx) = unbounded_channel();
        let (term, _event_proxy, _colors, _performance_metrics) =
            Terminal::create_term(20, 3, 10_000, event_tx);
        let mut processor: Processor<StdSyncHandler> = Processor::new();
        processor.advance(&mut *term.lock(), b"one\r\ntwo\r\nthree\r\nfour");

        let snapshot = recent_text_from_term(&term, 2);

        assert_eq!("three\nfour", snapshot.text);
        assert_eq!(2, snapshot.returned_lines);
        assert!(snapshot.available_lines >= 4);
        assert!(snapshot.history_size >= 1);
    }

    #[test]
    fn long_output_soft_wraps_without_losing_trailing_fields() {
        let (event_tx, _event_rx) = unbounded_channel();
        let (term, _event_proxy, _colors, _performance_metrics) =
            Terminal::create_term(10, 4, 10_000, event_tx);
        let mut processor: Processor<StdSyncHandler> = Processor::new();

        processor.advance(&mut *term.lock(), b"0123456789ABCDEFGHIJabcdefghij");

        let term = term.lock();
        let screen_line = |line: i32| -> String {
            term.grid()[Line(line)][..]
                .iter()
                .map(|cell| cell.c)
                .collect()
        };

        assert_eq!("0123456789", screen_line(0));
        assert_eq!("ABCDEFGHIJ", screen_line(1));
        assert_eq!("abcdefghij", screen_line(2));
        assert!(
            term.grid()[Line(0)][Column(9)]
                .flags
                .contains(Flags::WRAPLINE)
        );
        assert!(
            term.grid()[Line(1)][Column(9)]
                .flags
                .contains(Flags::WRAPLINE)
        );
    }

    #[test]
    fn resize_reflows_wrapped_output_without_losing_trailing_fields() {
        let (event_tx, _event_rx) = unbounded_channel();
        let (term, _event_proxy, _colors, _performance_metrics) =
            Terminal::create_term(20, 4, 10_000, event_tx);
        let mut processor: Processor<StdSyncHandler> = Processor::new();

        processor.advance(&mut *term.lock(), b"0123456789ABCDEFGHIJabcdefghij");

        {
            let mut term = term.lock();
            term.reset_damage();
            term.resize(TermDimensions { cols: 10, rows: 4 });
            assert!(matches!(term.damage(), TermDamage::Full));
        }

        {
            let term = term.lock();
            let line_text = |line: i32| -> String {
                term.grid()[Line(line)][..]
                    .iter()
                    .map(|cell| cell.c)
                    .collect::<String>()
                    .trim_end_matches(|character: char| character == ' ' || character == '\0')
                    .to_string()
            };
            let logical_lines = (-(term.history_size() as i32)..term.screen_lines() as i32)
                .map(line_text)
                .collect::<Vec<_>>();

            assert!(
                logical_lines
                    .windows(3)
                    .any(|lines| lines == ["0123456789", "ABCDEFGHIJ", "abcdefghij"]),
                "reflowed output not found: {logical_lines:?}"
            );
        }

        {
            let mut term = term.lock();
            term.reset_damage();
            term.resize(TermDimensions { cols: 40, rows: 4 });
            assert!(matches!(term.damage(), TermDamage::Full));
        }

        let term = term.lock();
        let matching_line =
            (-(term.history_size() as i32)..term.screen_lines() as i32).find(|line| {
                term.grid()[Line(*line)][..]
                    .iter()
                    .map(|cell| cell.c)
                    .collect::<String>()
                    .starts_with("0123456789ABCDEFGHIJabcdefghij")
            });
        let matching_line = matching_line.expect("expanded output was not found in the grid");
        assert!(
            !term.grid()[Line(matching_line)][Column(39)]
                .flags
                .contains(Flags::WRAPLINE)
        );
    }

    #[test]
    fn terminal_scroll_proxy_try_operations_never_wait_for_parser() {
        let (event_tx, _event_rx) = unbounded_channel();
        let (term, event_proxy, _colors, _performance_metrics) =
            Terminal::create_term(20, 3, 10_000, event_tx.clone());
        let proxy = TerminalScrollProxy {
            term: term.clone(),
            event_tx: Some(event_tx),
            wakeup_pending: event_proxy.wakeup_pending_handle(),
        };
        let parser_guard = term.lock();

        assert_eq!(None, proxy.try_snapshot());
        assert!(!proxy.try_set_display_offset(1));
        assert!(!proxy.try_scroll_display_delta(1));

        drop(parser_guard);
        assert_eq!(Some(proxy.snapshot()), proxy.try_snapshot());
    }

    #[test]
    fn terminal_scroll_proxy_try_scroll_updates_offset_and_coalesces_wakeup() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let (term, event_proxy, _colors, _performance_metrics) =
            Terminal::create_term(20, 3, 10_000, event_tx.clone());
        let wakeup_pending = event_proxy.wakeup_pending_handle();
        let proxy = TerminalScrollProxy {
            term: term.clone(),
            event_tx: Some(event_tx),
            wakeup_pending: wakeup_pending.clone(),
        };
        let mut processor: Processor<StdSyncHandler> = Processor::new();
        processor.advance(&mut *term.lock(), b"one\r\ntwo\r\nthree\r\nfour");
        while event_rx.try_recv().is_ok() {}
        wakeup_pending.store(false, Ordering::Release);

        assert!(proxy.try_scroll_display_delta(1));
        assert_eq!(1, proxy.try_snapshot().unwrap().display_offset);
        assert!(matches!(event_rx.try_recv(), Ok(TerminalEvent::Wakeup)));

        assert!(proxy.try_set_display_offset(0));
        assert_eq!(0, proxy.try_snapshot().unwrap().display_offset);
        assert!(
            event_rx.try_recv().is_err(),
            "scroll invalidations must share the end-to-end Wakeup gate"
        );
    }

    #[test]
    fn terminal_try_selection_operations_never_wait_for_parser() {
        let runtime = RecordingRuntime::new(RecordingRuntimeConfig::default())
            .expect("create recording runtime");
        let mut terminal = test_terminal_with_recording_runtime(Ok(runtime));
        let term = terminal.term.clone();
        let start = Point::new(Line(0), Column(0));
        let end = Point::new(Line(0), Column(2));
        let parser_guard = term.lock();

        assert!(!terminal.try_clear_selection());
        assert!(!terminal.try_start_selection(SelectionType::Simple, start, Side::Left));
        assert!(!terminal.try_update_selection(end, Side::Right));

        drop(parser_guard);
        assert!(terminal.try_start_selection(SelectionType::Simple, start, Side::Left));
        assert!(terminal.try_update_selection(end, Side::Right));
        {
            let term = term.lock();
            let selection = term.selection.as_ref().expect("selection should exist");
            assert_eq!(SelectionType::Simple, selection.ty);
            let range = selection
                .to_range(&term)
                .expect("selection should resolve to a grid range");
            assert_eq!(start, range.start);
            assert_eq!(end, range.end);
        }
        assert!(terminal.try_clear_selection());
        assert!(term.lock().selection.is_none());
    }

    #[test]
    fn terminal_scrollback_limit_can_be_updated_without_replacing_surface() {
        let (event_tx, _event_rx) = unbounded_channel();
        let (term, event_proxy, _colors, performance_metrics) =
            Terminal::create_term(80, 24, 10_000, event_tx.clone());
        let wakeup_pending = event_proxy.wakeup_pending_handle();
        let mut terminal = Terminal {
            term,
            session_mode: TerminalSessionMode::Live,
            performance_metrics,
            backend: None,
            recording_runtime: Terminal::create_recording_runtime(
                event_tx.clone(),
                wakeup_pending.clone(),
            ),
            session_log_runtime: None,
            playback_runtime: None,
            recording_session_id: "scrollback-test-session".to_string(),
            recording_session_metadata: RecordingSessionMetadata::default(),
            title: String::new(),
            current_working_dir: None,
            child_exited: None,
            connection_state: ConnectionState::Connected,
            session_lock: None,
            cols: 80,
            rows: 24,
            pixel_width: 0,
            pixel_height: 0,
            ssh_config: None,
            ssh_base_config: None,
            ssh_session_manager: None,
            ssh_credential_prompt_policy: SshCredentialPromptPolicy::default(),
            ssh_credential_request: None,
            ssh_keyboard_interactive_enabled: false,
            ssh_mfa_responder: None,
            zmodem_responder: None,
            pending_host_key_verification: None,
            serial_params: None,
            telnet_params: None,
            telnet_base_params: None,
            telnet_credential_request: None,
            wakeup_pending,
            event_tx: Some(event_tx),
            event_proxy: Some(event_proxy),
            connection_id: Some(1),
            connection_name: Some("SSH".to_string()),
            init_commands: None,
            session_history: VecDeque::new(),
            persisted_history: Vec::new(),
            history_repository: None,
            history_scope: None,
            command_record_gate: CommandRecordGate::default(),
            connection_generation: 1,
            connection_kind: TerminalConnectionKind::Ssh,
            scrollback_lines: 10_000,
        };

        terminal.set_scrollback_lines(250_000);
        assert_eq!(250_000, terminal.scrollback_lines());
    }

    #[test]
    fn telnet_params_parse_error_is_reported_without_panicking() {
        let mut conn = StoredConnection::new_telnet(
            "Broken Telnet".to_string(),
            one_core::storage::models::TelnetParams::default(),
            None,
        );
        conn.params = r#"[]"#.to_string();

        let error = parse_stored_telnet_params(&conn).expect_err("非法 JSON 应解析失败");
        assert!(error.contains("Telnet 连接参数损坏或不兼容"));
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct TermDimensions {
    cols: usize,
    rows: usize,
}

impl Dimensions for TermDimensions {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.cols
    }
}
