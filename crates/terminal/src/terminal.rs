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
use alacritty_terminal::vte::ansi::Color;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use futures::StreamExt;
use gpui::*;
use one_core::gpui_tokio::Tokio;
use one_core::settings::AppSettings;
use one_core::storage::models::{
    ActiveConnections, ProxyType as StorageProxyType, SerialParams, SshAuthMethod, StoredConnection,
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
use std::env;
#[cfg(any(test, target_os = "windows"))]
use std::ffi::OsStr;
#[cfg(any(test, target_os = "windows"))]
use std::path::Path;

use crate::history::{
    HistoryEntry, PERSISTED_HISTORY_LIMIT, SESSION_HISTORY_LIMIT, ShellHistoryFormat,
    collect_history_search_results, collect_history_suggestions_with_cwd,
    normalize_recorded_command, parse_shell_history, push_rich_history_entry,
};
use crate::pty_backend::{GpuiEventProxy, LocalPtyBackend};
use crate::recording::{
    RecordingBackend, RecordingCompleteness, RecordingConfig, RecordingMetadata, RecordingPlayback,
    RecordingPlaybackError, RecordingPlaybackSearchIndexStatus, RecordingPlaybackSearchResults,
    RecordingPlaybackState, RecordingPlaybackTransition, RecordingRuntime, RecordingRuntimeConfig,
    RecordingRuntimeError, RecordingSnapshot, RecordingStartRequest, RecordingTap,
    RecordingTransition, TerminalPlaybackRuntime,
};
#[cfg(not(target_os = "windows"))]
#[cfg(not(target_os = "windows"))]
use crate::shell_integration::embedded_shell_integration_script;

use crate::{
    LocalConfig, SerialBackend, SshBackend, TerminalBackend, TerminalControlHandle, TerminalEvent,
    TerminalExecHandle, TerminalInputHandle, TerminalPerformanceMetrics,
    TerminalPerformanceSnapshot, TerminalPerformanceWindow, TerminalSize,
};
use ssh::{
    ChannelEvent, HostKeyVerifier, KeyboardInteractiveRequest, KeyboardInteractiveResponder,
    KeyboardInteractiveTarget, SshChannel, SshSessionManager,
};
pub use ssh::{
    JumpServerConnectConfig, ProxyConnectConfig, ProxyType, PtyConfig, SshAuth, SshConnectConfig,
};

/// Terminal 发出的事件，供 TerminalView 订阅
#[derive(Debug, Clone)]
pub enum TerminalModelEvent {
    /// 终端内容已更新，需要重新渲染
    Wakeup,
    /// SSH keyboard-interactive/MFA 请求状态变化
    SshMfaChanged,
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
}

/// 终端连接状态
#[derive(Clone, PartialEq, Debug)]
pub enum ConnectionState {
    Connected,
    Connecting,
    Disconnected { error: Option<String> },
}

/// 终端连接类型
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TerminalConnectionKind {
    Local,
    Ssh,
    Serial,
}

/// Capability mode for a terminal surface.
///
/// A recording may describe output originally produced by SSH, but replaying
/// that output must never recreate the source session's live capabilities.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TerminalSessionMode {
    Live,
    RecordingPlayback,
}

const SSH_CLEAR_SCREEN_REDRAW_BYTES: &[u8] = b"\x0c";

fn clear_screen_remote_redraw_bytes(kind: TerminalConnectionKind) -> Option<&'static [u8]> {
    match kind {
        TerminalConnectionKind::Ssh => Some(SSH_CLEAR_SCREEN_REDRAW_BYTES),
        TerminalConnectionKind::Local | TerminalConnectionKind::Serial => None,
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
    /// 关闭 shell integration 注入:走裸 request_shell,失去 OSC 集成。
    pub disable_shell_integration: bool,
}

pub struct SshConnectionUpdate {
    pub connection: StoredConnection,
    pub working_dir: Option<String>,
    pub sync_path_with_terminal: bool,
}

struct ResolvedSshConnection {
    config: SshTerminalConfig,
    responder: TerminalMfaResponder,
    init_commands: Option<String>,
    connection_id: Option<i64>,
    connection_name: String,
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
        SshAuthMethod::AutoPublicKey => SshAuth::AutoPublicKey,
    }
}

fn password_from_storage_auth(auth: &SshAuthMethod) -> Option<String> {
    match auth {
        SshAuthMethod::Password { password } => Some(password.clone()),
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
            event_tx: Some(event_tx),
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
    prompt
        .trim()
        .trim_end_matches(':')
        .to_ascii_lowercase()
        .ends_with("password")
}

fn resolve_ssh_connection(
    update: SshConnectionUpdate,
    event_tx: UnboundedSender<TerminalEvent>,
) -> Result<ResolvedSshConnection> {
    let params = update.connection.to_ssh_params()?;
    let target_password = password_from_storage_auth(&params.auth_method);
    let jump_password = params
        .jump_server
        .as_ref()
        .and_then(|jump| password_from_storage_auth(&jump.auth_method));
    let init_commands = build_ssh_init_commands(
        update.working_dir.as_deref(),
        params.default_directory.as_deref(),
        params.init_script.as_deref(),
        update.sync_path_with_terminal,
    );
    let responder = TerminalMfaResponder::new(event_tx, jump_password, target_password);
    let mut ssh_config = SshConnectConfig {
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
    };
    ssh_config.keyboard_interactive_responder = Some(Arc::new(responder.clone()));
    Ok(ResolvedSshConnection {
        config: SshTerminalConfig {
            ssh_config,
            pty_config: PtyConfig::default(),
            disable_shell_integration: params.disable_shell_integration.unwrap_or(false),
        },
        responder,
        init_commands,
        connection_id: update.connection.id,
        connection_name: update.connection.name,
    })
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
    env::split_paths(path_env)
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
fn resolve_default_windows_shell() -> String {
    // Shell discovery can touch every entry in PATH. Resolve it once for the
    // process so opening another terminal does not repeat potentially slow
    // network/antivirus-backed filesystem checks.
    static DEFAULT_SHELL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    DEFAULT_SHELL
        .get_or_init(|| {
            resolve_default_windows_shell_from_env(
                env::var_os("PATH").as_deref(),
                env::var_os("SystemRoot")
                    .or_else(|| env::var_os("SYSTEMROOT"))
                    .as_deref(),
                env::var_os("COMSPEC").as_deref(),
            )
        })
        .clone()
}

#[cfg(target_os = "windows")]
fn build_local_shell(shell: Option<String>, extra_args: Vec<String>) -> Option<tty::Shell> {
    let program = shell.unwrap_or_else(resolve_default_windows_shell);
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
    let program = shell
        .map(str::to_string)
        .unwrap_or_else(resolve_default_windows_shell);
    crate::windows_shell_integration::prepare(&program)
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
    /// read-only recording.
    session_mode: TerminalSessionMode,
    /// 当前终端实例的共享性能指标。
    performance_metrics: Arc<TerminalPerformanceMetrics>,
    /// PTY/SSH 后端
    backend: Option<Box<dyn TerminalBackend>>,
    /// 与终端实例同生命周期的录制运行时；重连只会克隆新的 tap，不会替换时间线。
    recording_runtime: std::result::Result<RecordingRuntime, RecordingRuntimeError>,
    /// Playback owns a separate fail-closed parser and grid. Live terminals
    /// never populate this field.
    playback_runtime: Option<TerminalPlaybackRuntime>,
    /// 只用于录制文件关联的随机逻辑会话 ID；不包含连接名称、地址或凭据。
    recording_session_id: String,

    /// 终端标题
    title: String,
    /// 当前工作目录（由 OSC 7 更新，仅 SSH 终端）
    current_working_dir: Option<String>,
    /// 子进程退出码
    child_exited: Option<i32>,
    /// 连接状态
    connection_state: ConnectionState,

    /// 终端尺寸
    cols: usize,
    rows: usize,
    /// 最近一次同步给 PTY 的像素尺寸,用于 nudge_resize 重发 SIGWINCH
    pixel_width: u16,
    pixel_height: u16,

    /// SSH 配置（用于重连）
    ssh_config: Option<SshTerminalConfig>,
    /// SSH 会话管理器（同一 SSH tab 共享底层连接）
    ssh_session_manager: Option<Arc<SshSessionManager>>,
    /// SSH keyboard-interactive/MFA 输入响应器
    ssh_mfa_responder: Option<TerminalMfaResponder>,
    /// 串口参数（用于重连）
    serial_params: Option<SerialParams>,
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
}

/// Snapshot of terminal scroll state, captured in a single lock acquisition
/// to ensure consistency.
#[derive(Clone, Debug)]
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
    /// Snapshot all scroll-related state in a single lock acquisition
    /// to avoid inconsistency from multiple separate locks.
    pub fn snapshot(&self) -> TerminalScrollSnapshot {
        let term = self.term.lock();
        TerminalScrollSnapshot {
            display_offset: term.grid().display_offset(),
            history_size: term.history_size(),
            screen_lines: term.screen_lines(),
            columns: term.columns(),
        }
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
            let _ = tx.send(TerminalEvent::Wakeup);
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

impl Terminal {
    fn new_recording_session_id() -> String {
        Uuid::new_v4().to_string()
    }

    fn create_recording_runtime(
        event_tx: UnboundedSender<TerminalEvent>,
    ) -> std::result::Result<RecordingRuntime, RecordingRuntimeError> {
        RecordingRuntime::with_observer(RecordingRuntimeConfig::default(), move |_| {
            // Recording control transitions and asynchronous failures must
            // invalidate the pane, but they must never block the recording
            // worker when the terminal has already gone away.
            let _ = event_tx.send(TerminalEvent::Wakeup);
        })
    }

    fn recording_tap(&self) -> Option<RecordingTap> {
        if self.is_read_only() {
            return None;
        }
        self.recording_runtime
            .as_ref()
            .ok()
            .map(RecordingRuntime::tap)
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
        let recording_runtime = Self::create_recording_runtime(event_tx.clone());
        let scrollback_lines = AppSettings::current(cx).terminal_scrollback_lines;
        let (term, event_proxy, _colors, performance_metrics) = Self::create_term(
            DEFAULT_COLS,
            DEFAULT_ROWS,
            scrollback_lines,
            event_tx.clone(),
        );

        Self::spawn_event_loop(event_rx, event_proxy.wakeup_pending_handle(), cx);

        Self {
            term,
            session_mode: TerminalSessionMode::Live,
            performance_metrics,
            backend: None,
            recording_runtime,
            playback_runtime: None,
            recording_session_id: Self::new_recording_session_id(),
            title: String::new(),
            current_working_dir: None,
            child_exited: None,
            connection_state: ConnectionState::Disconnected { error: Some(error) },
            cols: DEFAULT_COLS,
            rows: DEFAULT_ROWS,
            pixel_width: 0,
            pixel_height: 0,
            ssh_config: None,
            ssh_session_manager: None,
            ssh_mfa_responder: None,
            serial_params: None,
            event_tx: Some(event_tx),
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
        let recording_runtime = Self::create_recording_runtime(event_tx.clone());
        let recording_tap = recording_runtime.as_ref().ok().map(RecordingRuntime::tap);
        let scrollback_lines = AppSettings::current(cx).terminal_scrollback_lines;
        let (term, event_proxy, _colors, performance_metrics) = Self::create_term(
            DEFAULT_COLS,
            DEFAULT_ROWS,
            scrollback_lines,
            event_tx.clone(),
        );
        let LocalConfig {
            shell,
            args,
            working_dir,
            env,
        } = config;
        #[cfg(target_os = "windows")]
        let shell = shell.or_else(|| Some(resolve_default_windows_shell()));
        let history_shell = shell.clone();
        let working_directory = resolve_local_working_dir(working_dir);

        // 准备 Shell Integration 环境（写入集成脚本、生成 wrapper 配置）
        let (integration_env, integration_args) = prepare_shell_integration(shell.as_deref());
        let mut shell_args = args;
        shell_args.extend(integration_args);

        // 合并用户环境变量与 Shell Integration 环境变量
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

        Self::spawn_event_loop(event_rx, event_proxy.wakeup_pending_handle(), cx);
        Self::spawn_local_history_loader(history_shell.as_deref(), cx);
        let history_repository = Self::history_repository(cx);

        Ok(Self {
            term,
            session_mode: TerminalSessionMode::Live,
            performance_metrics,
            backend: Some(Box::new(local_backend)),
            recording_runtime,
            playback_runtime: None,
            recording_session_id: Self::new_recording_session_id(),
            title: String::new(),
            current_working_dir: None,
            child_exited: None,
            connection_state: ConnectionState::Connected,
            cols: DEFAULT_COLS,
            rows: DEFAULT_ROWS,
            pixel_width: 0,
            pixel_height: 0,
            ssh_config: None,
            ssh_session_manager: None,
            ssh_mfa_responder: None,
            serial_params: None,
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

    /// 创建 SSH 终端
    pub fn new_ssh(
        conn: StoredConnection,
        cx: &mut Context<Self>,
        working_dir: Option<&str>,
        sync_path_with_terminal: bool,
    ) -> Self {
        let (event_tx, event_rx) = unbounded_channel::<TerminalEvent>();
        let recording_runtime = Self::create_recording_runtime(event_tx.clone());
        let recording_tap = recording_runtime.as_ref().ok().map(RecordingRuntime::tap);
        let resolved = resolve_ssh_connection(
            SshConnectionUpdate {
                connection: conn,
                working_dir: working_dir.map(str::to_string),
                sync_path_with_terminal,
            },
            event_tx.clone(),
        )
        .expect("StoredConnection should contain valid SSH params");
        let config = resolved.config;
        let ssh_session_manager = Arc::new(SshSessionManager::new(config.ssh_config.clone()));

        let cols = config.pty_config.width as usize;
        let rows = config.pty_config.height as usize;

        let scrollback_lines = AppSettings::current(cx).terminal_scrollback_lines;
        let (term, event_proxy, _colors, performance_metrics) =
            Self::create_term(cols, rows, scrollback_lines, event_tx.clone());
        let (disconnect_tx, disconnect_rx) = oneshot::channel::<()>();
        let connection_generation = 1;

        Self::spawn_disconnect_handler(disconnect_rx, connection_generation, cx);
        Self::spawn_event_loop(event_rx, event_proxy.wakeup_pending_handle(), cx);
        Self::spawn_ssh_connect(
            ssh_session_manager.clone(),
            config.clone(),
            term.clone(),
            event_proxy.clone(),
            event_tx.clone(),
            resolved.connection_id,
            Some(disconnect_tx),
            resolved.init_commands.clone(),
            recording_tap,
            connection_generation,
            cx,
        );
        Self::spawn_ssh_history_loader(ssh_session_manager.clone(), cx);
        let history_repository = Self::history_repository(cx);
        let history_scope = resolved.connection_id.map(TerminalHistoryScope::ssh);

        Self {
            term,
            session_mode: TerminalSessionMode::Live,
            performance_metrics,
            backend: None,
            recording_runtime,
            playback_runtime: None,
            recording_session_id: Self::new_recording_session_id(),
            title: String::new(),
            current_working_dir: None,
            child_exited: None,
            connection_state: ConnectionState::Connecting,
            cols,
            rows,
            pixel_width: 0,
            pixel_height: 0,
            ssh_config: Some(config),
            ssh_session_manager: Some(ssh_session_manager),
            ssh_mfa_responder: Some(resolved.responder),
            serial_params: None,
            event_tx: Some(event_tx),
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
        }
    }

    /// 创建串口终端
    pub fn new_serial(conn: StoredConnection, cx: &mut Context<Self>) -> Self {
        let serial_params = conn
            .to_serial_params()
            .expect("StoredConnection 应包含有效的 SerialParams");

        let (event_tx, event_rx) = unbounded_channel::<TerminalEvent>();
        let recording_runtime = Self::create_recording_runtime(event_tx.clone());
        let recording_tap = recording_runtime.as_ref().ok().map(RecordingRuntime::tap);
        let scrollback_lines = AppSettings::current(cx).terminal_scrollback_lines;
        let (term, event_proxy, _colors, performance_metrics) = Self::create_term(
            DEFAULT_COLS,
            DEFAULT_ROWS,
            scrollback_lines,
            event_tx.clone(),
        );
        let (disconnect_tx, disconnect_rx) = tokio::sync::oneshot::channel::<()>();
        let connection_generation = 1;

        Self::spawn_disconnect_handler(disconnect_rx, connection_generation, cx);
        Self::spawn_event_loop(event_rx, event_proxy.wakeup_pending_handle(), cx);
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
            playback_runtime: None,
            recording_session_id: Self::new_recording_session_id(),
            title: String::new(),
            current_working_dir: None,
            child_exited: None,
            connection_state: ConnectionState::Connecting,
            cols: DEFAULT_COLS,
            rows: DEFAULT_ROWS,
            pixel_width: 0,
            pixel_height: 0,
            ssh_config: None,
            ssh_session_manager: None,
            ssh_mfa_responder: None,
            serial_params: Some(serial_params),
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

    /// Creates a terminal surface that renders an untrusted recording without
    /// recreating any live PTY, SSH, serial, input, exec, or control capability.
    pub fn new_recording_playback(playback: RecordingPlayback, cx: &mut Context<Self>) -> Self {
        let scrollback_lines = AppSettings::current(cx).terminal_scrollback_lines;
        let (terminal, event_loop) = Self::build_recording_playback(playback, scrollback_lines);
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
        let source_backend = playback.recording().header.navop.backend;
        let connection_kind = match source_backend {
            RecordingBackend::Local => TerminalConnectionKind::Local,
            RecordingBackend::Ssh => TerminalConnectionKind::Ssh,
            RecordingBackend::Serial => TerminalConnectionKind::Serial,
        };
        let scrollback_lines = AppSettings::normalize_terminal_scrollback_lines(scrollback_lines);
        let (event_tx, event_rx) = unbounded_channel::<TerminalEvent>();
        let performance_metrics = Arc::new(TerminalPerformanceMetrics::default());
        let playback_runtime = TerminalPlaybackRuntime::new(
            playback,
            scrollback_lines,
            event_tx.clone(),
            performance_metrics.clone(),
        );
        let initial_size = playback_runtime.initial_size();
        let term = playback_runtime.term().clone();
        let wakeup_pending = playback_runtime.wakeup_pending_handle();

        (
            Self {
                term,
                session_mode: TerminalSessionMode::RecordingPlayback,
                performance_metrics,
                backend: None,
                recording_runtime: Err(RecordingRuntimeError::ReadOnlyPlayback),
                playback_runtime: Some(playback_runtime),
                recording_session_id: Self::new_recording_session_id(),
                title: String::new(),
                current_working_dir: None,
                child_exited: None,
                // Connected suppresses the live reconnect overlay. Capability
                // checks still fail closed through `session_mode`.
                connection_state: ConnectionState::Connected,
                cols: usize::from(initial_size.cols),
                rows: usize::from(initial_size.rows),
                pixel_width: 0,
                pixel_height: 0,
                ssh_config: None,
                ssh_session_manager: None,
                ssh_mfa_responder: None,
                serial_params: None,
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
        self.connection_generation = self.connection_generation.wrapping_add(1).max(1);
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
        let term_config = TermConfig {
            scrolling_history: scrollback_lines,
            ..Default::default()
        };
        let performance_metrics = Arc::new(TerminalPerformanceMetrics::default());
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
        let _entity = cx.entity().downgrade();
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
                            None => break,
                            Some(event) => {
                                match &event {
                                    TerminalEvent::Wakeup => pending_wakeup = true,
                                    _ => pending_events.push(event),
                                }
                            }
                        }
                    }
                    _ = render_interval.tick() => {
                        // 先发送非 Wakeup 事件
                        for event in pending_events.drain(..) {
                            if render_tx.unbounded_send(event).is_err() {
                                return;
                            }
                        }
                        // 最后发送 Wakeup
                        if pending_wakeup {
                            pending_wakeup = false;
                            // 转发完毕后允许 alacritty 线程的下一次 Wakeup 重新入队，
                            // 避免高速输出时被 GpuiEventProxy 的去重永久吞掉
                            wakeup_pending.store(false, Ordering::Release);
                            if render_tx.unbounded_send(TerminalEvent::Wakeup).is_err() {
                                return;
                            }
                        }
                    }
                }
            }
        })
        .detach();

        // GPUI 线程事件处理
        cx.spawn(async move |this, cx| {
            while let Some(event) = render_rx.next().await {
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

    fn spawn_ssh_connect(
        session_manager: Arc<SshSessionManager>,
        config: SshTerminalConfig,
        term: Arc<FairMutex<Term<GpuiEventProxy>>>,
        event_proxy: GpuiEventProxy,
        event_tx: UnboundedSender<TerminalEvent>,
        connection_id: Option<i64>,
        on_disconnect: Option<tokio::sync::oneshot::Sender<()>>,
        init_commands: Option<String>,
        recording_tap: Option<RecordingTap>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        let task = Tokio::spawn(cx, async move {
            let disconnect_tx = on_disconnect.map(|tx| {
                let (sender, mut receiver) = unbounded_channel::<()>();
                tokio::spawn(async move {
                    if receiver.recv().await.is_some() {
                        let _ = tx.send(());
                    }
                });
                sender
            });
            SshBackend::connect_with_recording(
                session_manager,
                config.pty_config,
                connection_id,
                term,
                event_proxy,
                event_tx,
                disconnect_tx,
                init_commands,
                config.disable_shell_integration,
                recording_tap,
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
                self.connection_state = ConnectionState::Disconnected {
                    error: Some(format_connection_error(&e)),
                };
                self.set_connection_active(false, cx);
            }
            Err(e) => {
                if let Some(responder) = &self.ssh_mfa_responder {
                    responder.cancel();
                }
                self.connection_state = ConnectionState::Disconnected {
                    error: Some(e.to_string()),
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
            TerminalConnectionKind::Serial => None,
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
        let session_manager = self
            .ssh_session_manager
            .as_ref()
            .ok_or_else(|| anyhow!("SSH session manager is unavailable"))?;

        if let Some(responder) = &self.ssh_mfa_responder {
            responder.cancel();
        }
        session_manager.replace_config(resolved.config.ssh_config.clone());
        self.ssh_config = Some(resolved.config);
        self.ssh_mfa_responder = Some(resolved.responder);
        self.connection_id = resolved.connection_id;
        self.connection_name = Some(resolved.connection_name);
        self.init_commands = resolved.init_commands;
        self.history_scope = resolved.connection_id.map(TerminalHistoryScope::ssh);
        Ok(())
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

    pub fn is_read_only(&self) -> bool {
        self.is_recording_playback()
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
        };

        Ok(RecordingStartRequest {
            final_path,
            metadata: RecordingMetadata {
                recording_id: Uuid::new_v4().to_string(),
                session_id: self.recording_session_id.clone(),
                backend,
                application_version: option_env!("NAVOP_APPLICATION_VERSION")
                    .unwrap_or(env!("CARGO_PKG_VERSION"))
                    .to_string(),
                started_at_unix_ms,
                capture_input: false,
            },
            initial_size: TerminalSize {
                rows,
                cols,
                pixel_width: self.pixel_width,
                pixel_height: self.pixel_height,
            },
            recording: RecordingConfig::default(),
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
        !self.is_read_only() && (self.ssh_config.is_some() || self.serial_params.is_some())
    }

    /// 写入数据到终端
    pub fn write(&self, data: &[u8]) {
        if self.is_read_only() {
            return;
        }
        if let Some(ref backend) = self.backend {
            backend.write(data.to_vec());
        }
    }

    /// 写入来自外部集成的输入，例如 Public MCP。
    pub fn write_external_input(&self, data: &[u8]) {
        if let Some(handle) = self.external_input_handle() {
            handle.write(data.to_vec());
        }
    }

    pub fn external_input_handle(&self) -> Option<TerminalInputHandle> {
        if self.is_read_only() {
            return None;
        }
        self.backend
            .as_ref()
            .and_then(|backend| backend.input_handle())
    }

    pub fn external_exec_handle(&self) -> Option<TerminalExecHandle> {
        if self.is_read_only() {
            return None;
        }
        self.backend
            .as_ref()
            .and_then(|backend| backend.exec_handle())
    }

    pub fn external_control_handle(&self) -> Option<TerminalControlHandle> {
        if self.is_read_only() {
            return None;
        }
        self.backend
            .as_ref()
            .and_then(|backend| backend.control_handle())
    }

    /// 调整终端大小
    pub fn resize(&mut self, cols: usize, rows: usize, pixel_width: u16, pixel_height: u16) {
        if self.is_recording_playback() {
            // The recording header and Resize events are authoritative for the
            // playback grid. Canvas layout may update only cached pixel size.
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
    pub fn reconnect(&mut self, cx: &mut Context<Self>) {
        if self.is_read_only() {
            return;
        }
        if let Some(config) = self.ssh_config.clone() {
            let Some(session_manager) = self.ssh_session_manager.clone() else {
                return;
            };
            let Some(event_tx) = self.event_tx.clone() else {
                return;
            };
            let Some(event_proxy) = self.event_proxy.clone() else {
                return;
            };

            self.connection_state = ConnectionState::Connecting;
            self.set_connection_active(false, cx);
            if let Some(backend) = self.backend.take() {
                backend.shutdown();
            }
            self.reset_terminal_surface();

            let generation = self.next_connection_generation();
            self.record_connection_generation_marker(generation);
            if let Some(responder) = &self.ssh_mfa_responder {
                responder.cancel();
            }
            let term = self.term.clone();
            let connection_id = self.connection_id;
            let init_commands = self.init_commands.clone();
            let recording_tap = self.recording_tap();
            let entity = cx.entity().downgrade();
            cx.spawn(async move |_, cx| {
                let _ = session_manager.disconnect().await;
                let _ = entity.update(cx, |terminal, cx| {
                    if !terminal.is_current_connection_generation(generation) {
                        return;
                    }

                    let (disconnect_tx, disconnect_rx) = tokio::sync::oneshot::channel::<()>();
                    Self::spawn_disconnect_handler(disconnect_rx, generation, cx);
                    Self::spawn_ssh_connect(
                        session_manager.clone(),
                        config.clone(),
                        term.clone(),
                        event_proxy.clone(),
                        event_tx.clone(),
                        connection_id,
                        Some(disconnect_tx),
                        init_commands.clone(),
                        recording_tap.clone(),
                        generation,
                        cx,
                    );
                    Self::spawn_ssh_history_loader(session_manager.clone(), cx);
                });
            })
            .detach();
        } else if let Some(params) = self.serial_params.clone() {
            let Some(event_proxy) = self.event_proxy.clone() else {
                return;
            };

            self.connection_state = ConnectionState::Connecting;
            self.set_connection_active(false, cx);
            if let Some(backend) = self.backend.take() {
                backend.shutdown();
            }
            self.reset_terminal_surface();
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
        } else {
            return;
        }

        cx.emit(TerminalModelEvent::Wakeup);
    }

    fn reset_terminal_surface(&mut self) {
        if self.is_recording_playback() {
            return;
        }
        let Some(event_tx) = self.event_tx.clone() else {
            return;
        };

        let event_proxy = self.event_proxy_for_surface_reset(event_tx);
        let term_config = TermConfig {
            scrolling_history: self.scrollback_lines,
            ..Default::default()
        };
        let new_term = Term::new(
            term_config,
            &TermDimensions {
                cols: self.cols,
                rows: self.rows,
            },
            event_proxy,
        );

        *self.term.lock() = new_term;
        self.title.clear();
        self.current_working_dir = None;
        self.child_exited = None;
    }

    fn event_proxy_for_surface_reset(
        &self,
        event_tx: UnboundedSender<TerminalEvent>,
    ) -> GpuiEventProxy {
        self.event_proxy.clone().unwrap_or_else(|| {
            GpuiEventProxy::with_metrics(event_tx, self.performance_metrics.clone())
        })
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
        if let Some(ref backend) = self.backend {
            backend.shutdown();
        }
        if let Ok(recording_runtime) = &self.recording_runtime {
            if let Err(error) = recording_runtime.shutdown() {
                tracing::warn!(%error, "failed to shut down terminal recording runtime");
            }
        }
    }

    // ========== 选择操作 ==========

    /// 获取选中的文本
    pub fn selection_text(&self) -> Option<String> {
        self.term.lock().selection_to_string()
    }

    /// 清除选择
    pub fn clear_selection(&mut self) {
        self.term.lock().selection = None;
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

fn format_connection_error(err: &anyhow::Error) -> String {
    format!("{err:#}")
}

impl EventEmitter<TerminalModelEvent> for Terminal {}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    use super::with_local_terminal_default_env;
    use super::{
        CommandRecordGate, ConnectionState, SshConnectionUpdate, Terminal, TerminalConnectionKind,
        TerminalMfaPrompt, TerminalMfaRequest, TerminalMfaResponder, TerminalSessionMode,
        build_cd_command, build_ssh_base_init_commands, build_ssh_init_commands,
        clear_screen_remote_redraw_bytes, compose_ssh_init_commands, format_connection_error,
        is_reconnect_generation, keyboard_interactive_answers_for_terminal, merge_history_matches,
        normalize_history_matches, recent_text_from_term, resolve_default_windows_shell_from_env,
        resolve_local_working_dir, resolve_ssh_connection, shell_escape_arg,
    };
    use crate::history::{
        HistoryEntry, ShellHistoryFormat, collect_history_suggestions, normalize_history_command,
        parse_shell_history, push_history_entry,
    };
    use crate::recording::{
        ASCIICAST_VERSION, NAVOP_EVENT_STREAM, NAVOP_RECORDING_FORMAT_VERSION, ParsedRecording,
        RecordingBackend, RecordingCompleteness, RecordingConfig, RecordingEvent,
        RecordingEventKind, RecordingFileLimits, RecordingHeader, RecordingHeaderMetadata,
        RecordingMetadata, RecordingPlayback, RecordingPlaybackError, RecordingPlaybackLimits,
        RecordingPlaybackSearchKind, RecordingPlaybackState, RecordingPlaybackTransition,
        RecordingRuntime, RecordingRuntimeConfig, RecordingRuntimeError, RecordingStartRequest,
        RecordingState, RecordingTapOutcome, RecordingTransition, read_recording,
    };
    use crate::{
        TerminalBackend, TerminalControlHandle, TerminalEvent, TerminalExecHandle,
        TerminalInputHandle, TerminalSize,
    };
    use alacritty_terminal::event::{Event as AlacTermEvent, EventListener};
    use alacritty_terminal::grid::Dimensions;
    use alacritty_terminal::index::{Column, Line};
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
    use anyhow::anyhow;
    use one_core::storage::models::{SshAuthMethod, SshParams, StoredConnection};
    use ssh::{KeyboardInteractiveRequest, KeyboardInteractiveResponder, SshAuth};
    use std::collections::VecDeque;
    use std::fs;
    use std::path::PathBuf;
    #[cfg(not(target_os = "windows"))]
    use std::process::Command;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio::sync::mpsc::unbounded_channel;

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

        Terminal {
            term,
            session_mode: TerminalSessionMode::Live,
            performance_metrics,
            backend: None,
            recording_runtime,
            playback_runtime: None,
            recording_session_id: "terminal-runtime-test-session".to_string(),
            title: String::new(),
            current_working_dir: None,
            child_exited: None,
            connection_state: ConnectionState::Connected,
            cols: 80,
            rows: 24,
            pixel_width: 640,
            pixel_height: 480,
            ssh_config: None,
            ssh_session_manager: None,
            ssh_mfa_responder: None,
            serial_params: None,
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
                application_version: "0.1.0-test".to_string(),
                started_at_unix_ms: 1_700_000_000_123,
                capture_input: false,
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
                    application_version: "0.1.0-test".to_string(),
                    started_at_unix_ms: 1_700_000_000_123,
                    capture_input: true,
                    event_stream: NAVOP_EVENT_STREAM.to_string(),
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
    fn terminal_recording_runtime_survives_surface_reset_and_generation_change() {
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
        terminal.reset_terminal_surface();

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
                host: "latest.example".to_string(),
                port: 2222,
                username: "latest-user".to_string(),
                auth_method: SshAuthMethod::Password {
                    password: "latest-password".to_string(),
                },
                connect_timeout: None,
                keepalive_interval: None,
                keepalive_max: None,
                default_directory: Some("/srv/default".to_string()),
                init_script: Some("echo ready".to_string()),
                disable_shell_integration: None,
                x11_forwarding: None,
                jump_server: None,
                proxy: None,
                os_id: None,
                icon: None,
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
    fn create_term_shares_performance_metrics_with_event_proxy() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let (_term, event_proxy, _colors, metrics) =
            Terminal::create_term(80, 24, 10_000, event_tx);

        assert!(Arc::ptr_eq(&metrics, &event_proxy.performance_metrics()));

        event_proxy.send_event(AlacTermEvent::Wakeup);
        assert!(matches!(event_rx.try_recv(), Ok(TerminalEvent::Wakeup)));

        let snapshot = metrics.snapshot();
        assert_eq!(1, snapshot.wakeup_requests);
        assert_eq!(1, snapshot.wakeup_queued);
    }

    #[test]
    fn reset_terminal_surface_clears_buffer_and_stale_connection_metadata() {
        let (event_tx, _event_rx) = unbounded_channel();
        let (term, _event_proxy, _colors, performance_metrics) =
            Terminal::create_term(80, 24, 10_000, event_tx.clone());
        let shared_metrics = performance_metrics.clone();
        let mut terminal = Terminal {
            term,
            session_mode: TerminalSessionMode::Live,
            performance_metrics,
            backend: None,
            recording_runtime: Terminal::create_recording_runtime(event_tx.clone()),
            playback_runtime: None,
            recording_session_id: "surface-reset-test-session".to_string(),
            title: "old title".to_string(),
            current_working_dir: Some("/tmp/project".to_string()),
            child_exited: Some(255),
            connection_state: ConnectionState::Connected,
            cols: 80,
            rows: 24,
            pixel_width: 0,
            pixel_height: 0,
            ssh_config: None,
            ssh_session_manager: None,
            ssh_mfa_responder: None,
            serial_params: None,
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
        let reset_proxy = terminal.event_proxy_for_surface_reset(
            terminal
                .event_tx
                .clone()
                .expect("event sender should exist"),
        );
        assert!(Arc::ptr_eq(
            &shared_metrics,
            &reset_proxy.performance_metrics()
        ));

        let previous = terminal.performance_snapshot();
        terminal
            .performance_metrics()
            .record_render(std::time::Duration::from_millis(4), true);
        let window = terminal.performance_window(&previous, std::time::Duration::from_secs(2));
        assert_eq!(1, window.render_samples);
        assert_eq!(4_000_000.0, window.average_render_ns);

        terminal.reset_terminal_surface();
        assert!(Arc::ptr_eq(
            &shared_metrics,
            &terminal.performance_metrics()
        ));

        let term = terminal.term.lock();
        assert_eq!(term.grid()[Line(0)][Column(0)].c, ' ');
        assert_eq!(term.columns(), 80);
        assert_eq!(term.screen_lines(), 24);
        drop(term);
        assert_eq!(terminal.title(), "");
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
    fn terminal_scrollback_limit_can_be_updated_and_survives_surface_reset() {
        let (event_tx, _event_rx) = unbounded_channel();
        let (term, event_proxy, _colors, performance_metrics) =
            Terminal::create_term(80, 24, 10_000, event_tx.clone());
        let mut terminal = Terminal {
            term,
            session_mode: TerminalSessionMode::Live,
            performance_metrics,
            backend: None,
            recording_runtime: Terminal::create_recording_runtime(event_tx.clone()),
            playback_runtime: None,
            recording_session_id: "scrollback-test-session".to_string(),
            title: String::new(),
            current_working_dir: None,
            child_exited: None,
            connection_state: ConnectionState::Connected,
            cols: 80,
            rows: 24,
            pixel_width: 0,
            pixel_height: 0,
            ssh_config: None,
            ssh_session_manager: None,
            ssh_mfa_responder: None,
            serial_params: None,
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

        terminal.reset_terminal_surface();

        assert_eq!(250_000, terminal.scrollback_lines());
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
