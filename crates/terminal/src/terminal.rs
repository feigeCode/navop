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
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;
use tokio::time::interval;

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
#[cfg(not(target_os = "windows"))]
use crate::shell_integration::embedded_shell_integration_script;

use crate::{
    LocalConfig, SerialBackend, SshBackend, TerminalBackend, TerminalEvent, TerminalExecHandle,
    TerminalInputHandle, TerminalSize,
};
use ssh::{
    ChannelEvent, KeyboardInteractiveRequest, KeyboardInteractiveResponder,
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
fn build_local_shell(shell: Option<String>, _extra_args: Vec<String>) -> Option<tty::Shell> {
    let program = shell.unwrap_or_else(|| {
        resolve_default_windows_shell_from_env(
            env::var_os("PATH").as_deref(),
            env::var_os("SystemRoot")
                .or_else(|| env::var_os("SYSTEMROOT"))
                .as_deref(),
            env::var_os("COMSPEC").as_deref(),
        )
    });
    Some(tty::Shell::new(program, vec![]))
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

        let script = integration_path.display();

        // .zshenv — 恢复原始 ZDOTDIR 并 source 用户的 .zshenv
        let zshenv = "ZDOTDIR=\"${_ONETCLI_ORIG_ZDOTDIR:-$HOME}\"\n\
                       [[ -f \"$ZDOTDIR/.zshenv\" ]] && source \"$ZDOTDIR/.zshenv\"\n";
        let _ = fs::write(zsh_dir.join(".zshenv"), zshenv);

        // .zshrc — 恢复 ZDOTDIR，source 用户 .zshrc，再 source 集成脚本
        let zshrc = format!(
            "ZDOTDIR=\"${{_ONETCLI_ORIG_ZDOTDIR:-$HOME}}\"\n\
             [[ -f \"$ZDOTDIR/.zshrc\" ]] && source \"$ZDOTDIR/.zshrc\"\n\
             source \"{script}\"\n"
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
fn prepare_shell_integration(_shell: Option<&str>) -> (Vec<(String, String)>, Vec<String>) {
    // Windows 暂不支持 Shell Integration
    (vec![], vec![])
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
    /// PTY/SSH 后端
    backend: Option<Box<dyn TerminalBackend>>,

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

impl Terminal {
    fn new_local_disconnected(error: String, cx: &mut Context<Self>) -> Self {
        let (event_tx, event_rx) = unbounded_channel::<TerminalEvent>();
        let (term, event_proxy, _colors) =
            Self::create_term(DEFAULT_COLS, DEFAULT_ROWS, event_tx.clone());

        Self::spawn_event_loop(event_rx, event_proxy.wakeup_pending_handle(), cx);

        Self {
            term,
            backend: None,
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
        let (term, event_proxy, _colors) =
            Self::create_term(DEFAULT_COLS, DEFAULT_ROWS, event_tx.clone());
        let LocalConfig {
            shell,
            working_dir,
            env,
        } = config;
        let history_shell = shell.clone();
        let working_directory = resolve_local_working_dir(working_dir);

        // 准备 Shell Integration 环境（写入集成脚本、生成 wrapper 配置）
        let (integration_env, shell_args) = prepare_shell_integration(shell.as_deref());

        // 合并用户环境变量与 Shell Integration 环境变量
        let mut env_pairs = env;
        env_pairs.extend(integration_env);

        let pty_options = PtyOptions {
            shell: build_local_shell(shell, shell_args),
            working_directory,
            env: env_pairs.into_iter().collect(),
            drain_on_exit: true,
            #[cfg(target_os = "windows")]
            escape_args: true,
        };
        let local_backend = LocalPtyBackend::new(term.clone(), event_proxy.clone(), pty_options)?;

        Self::spawn_event_loop(event_rx, event_proxy.wakeup_pending_handle(), cx);
        Self::spawn_local_history_loader(history_shell.as_deref(), cx);
        let history_repository = Self::history_repository(cx);

        Ok(Self {
            term,
            backend: Some(Box::new(local_backend)),
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
        })
    }

    pub fn clear_screen(&mut self, cx: &mut Context<Self>) {
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
        let ssh_params = conn
            .to_ssh_params()
            .expect("StoredConnection should contain valid SSH params");

        let target_password = match &ssh_params.auth_method {
            SshAuthMethod::Password { password } => Some(password.clone()),
            _ => None,
        };
        let jump_password =
            ssh_params
                .jump_server
                .as_ref()
                .and_then(|jump| match &jump.auth_method {
                    SshAuthMethod::Password { password } => Some(password.clone()),
                    _ => None,
                });

        let auth = match ssh_params.auth_method.clone() {
            SshAuthMethod::Password { password } => SshAuth::Password(password),
            SshAuthMethod::PrivateKey {
                key_path,
                passphrase,
            } => SshAuth::PrivateKey {
                key_path,
                passphrase,
                certificate_path: None,
            },
            SshAuthMethod::Agent => SshAuth::Agent,
            SshAuthMethod::AutoPublicKey => SshAuth::AutoPublicKey,
        };

        // 构建初始化命令
        let init_commands = build_ssh_init_commands(
            working_dir,
            ssh_params.default_directory.as_deref(),
            ssh_params.init_script.as_deref(),
            sync_path_with_terminal,
        );

        let mut ssh_config = SshConnectConfig {
            host: ssh_params.host,
            port: ssh_params.port,
            username: ssh_params.username,
            auth,
            timeout: ssh_params.connect_timeout.map(Duration::from_secs),
            keepalive_interval: ssh_params.keepalive_interval.map(Duration::from_secs),
            keepalive_max: ssh_params.keepalive_max,
            jump_server: ssh_params.jump_server.map(|jump| {
                let jump_auth = match jump.auth_method {
                    SshAuthMethod::Password { password } => SshAuth::Password(password),
                    SshAuthMethod::PrivateKey {
                        key_path,
                        passphrase,
                    } => SshAuth::PrivateKey {
                        key_path,
                        passphrase,
                        certificate_path: None,
                    },
                    SshAuthMethod::Agent => SshAuth::Agent,
                    SshAuthMethod::AutoPublicKey => SshAuth::AutoPublicKey,
                };
                JumpServerConnectConfig {
                    host: jump.host,
                    port: jump.port,
                    username: jump.username,
                    auth: jump_auth,
                }
            }),
            proxy: ssh_params.proxy.map(|p| {
                let proxy_type = match p.proxy_type {
                    StorageProxyType::Socks5 => ProxyType::Socks5,
                    StorageProxyType::Http => ProxyType::Http,
                };
                ProxyConnectConfig {
                    proxy_type,
                    host: p.host,
                    port: p.port,
                    username: p.username,
                    password: p.password,
                }
            }),
            keyboard_interactive_responder: None,
        };

        let pty_config = PtyConfig::default();
        let (event_tx, event_rx) = unbounded_channel::<TerminalEvent>();
        let ssh_mfa_responder =
            TerminalMfaResponder::new(event_tx.clone(), jump_password, target_password);
        ssh_config.keyboard_interactive_responder = Some(Arc::new(ssh_mfa_responder.clone()));
        let config = SshTerminalConfig {
            ssh_config,
            pty_config,
            disable_shell_integration: ssh_params.disable_shell_integration.unwrap_or(false),
        };
        let ssh_session_manager = Arc::new(SshSessionManager::new(config.ssh_config.clone()));

        let cols = config.pty_config.width as usize;
        let rows = config.pty_config.height as usize;

        let (term, event_proxy, _colors) = Self::create_term(cols, rows, event_tx.clone());
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
            conn.id,
            Some(disconnect_tx),
            init_commands.clone(),
            connection_generation,
            cx,
        );
        Self::spawn_ssh_history_loader(ssh_session_manager.clone(), cx);
        let history_repository = Self::history_repository(cx);
        let history_scope = conn.id.map(TerminalHistoryScope::ssh);

        Self {
            term,
            backend: None,
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
            ssh_mfa_responder: Some(ssh_mfa_responder),
            serial_params: None,
            event_tx: Some(event_tx),
            event_proxy: Some(event_proxy),
            connection_id: conn.id,
            connection_name: Some(conn.name),
            init_commands,
            session_history: VecDeque::new(),
            persisted_history: Vec::new(),
            history_repository,
            history_scope,
            command_record_gate: CommandRecordGate::default(),
            connection_generation,
            connection_kind: TerminalConnectionKind::Ssh,
        }
    }

    /// 创建串口终端
    pub fn new_serial(conn: StoredConnection, cx: &mut Context<Self>) -> Self {
        let serial_params = conn
            .to_serial_params()
            .expect("StoredConnection 应包含有效的 SerialParams");

        let (event_tx, event_rx) = unbounded_channel::<TerminalEvent>();
        let (term, event_proxy, _colors) =
            Self::create_term(DEFAULT_COLS, DEFAULT_ROWS, event_tx.clone());
        let (disconnect_tx, disconnect_rx) = tokio::sync::oneshot::channel::<()>();
        let connection_generation = 1;

        Self::spawn_disconnect_handler(disconnect_rx, connection_generation, cx);
        Self::spawn_event_loop(event_rx, event_proxy.wakeup_pending_handle(), cx);
        Self::spawn_serial_connect(
            serial_params.clone(),
            term.clone(),
            event_tx.clone(),
            Some(disconnect_tx),
            connection_generation,
            cx,
        );

        Self {
            term,
            backend: None,
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
            event_proxy: None,
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
        }
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
        event_tx: UnboundedSender<TerminalEvent>,
    ) -> (
        Arc<FairMutex<Term<GpuiEventProxy>>>,
        GpuiEventProxy,
        alacritty_terminal::term::color::Colors,
    ) {
        let term_config = TermConfig {
            scrolling_history: 10000,
            ..Default::default()
        };
        let event_proxy = GpuiEventProxy::new(event_tx);
        let term = Term::new(
            term_config,
            &TermDimensions { cols, rows },
            event_proxy.clone(),
        );
        let colors = term.colors().clone();
        (Arc::new(FairMutex::new(term)), event_proxy, colors)
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
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        // 创建 SSH 后端需要的通知通道（UnboundedSender<()>）
        let (notify_tx, mut notify_rx) = unbounded_channel::<()>();

        let task = Tokio::spawn(cx, async move {
            // 转发 SSH 通知到事件通道（必须在 tokio runtime 内部）
            let event_tx_clone = event_tx.clone();
            tokio::spawn(async move {
                while notify_rx.recv().await.is_some() {
                    let _ = event_tx_clone.send(TerminalEvent::Wakeup);
                }
            });

            let disconnect_tx = on_disconnect.map(|tx| {
                let (sender, mut receiver) = unbounded_channel::<()>();
                tokio::spawn(async move {
                    if receiver.recv().await.is_some() {
                        let _ = tx.send(());
                    }
                });
                sender
            });
            SshBackend::connect(
                session_manager,
                config.pty_config,
                connection_id,
                term,
                event_proxy,
                event_tx,
                notify_tx,
                disconnect_tx,
                init_commands,
                config.disable_shell_integration,
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
        event_tx: UnboundedSender<TerminalEvent>,
        on_disconnect: Option<tokio::sync::oneshot::Sender<()>>,
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

        let result = SerialBackend::connect(params, term, event_tx, disconnect_tx);

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
        if let (Some(repo), Some(scope)) = (&self.history_repository, &self.history_scope) {
            if let Err(error) =
                repo.record_success(scope, &command, cwd.as_deref(), Some(exit_code))
            {
                tracing::warn!(%error, "failed to record terminal command history");
            }
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
        tracing::debug!(
            target: "terminal.history_prompt.osc",
            event = ?event,
            "terminal model handling terminal event"
        );
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
        self.record_successful_history_entry(command, 0, cx);
    }

    /// 获取 SSH 连接配置（仅 SSH 终端）
    pub fn ssh_config(&self) -> Option<&SshTerminalConfig> {
        self.ssh_config.as_ref()
    }

    pub fn ssh_session_manager(&self) -> Option<&Arc<SshSessionManager>> {
        self.ssh_session_manager.as_ref()
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

    /// 是否可以重连
    pub fn can_reconnect(&self) -> bool {
        self.ssh_config.is_some() || self.serial_params.is_some()
    }

    /// 写入数据到终端
    pub fn write(&self, data: &[u8]) {
        if let Some(ref backend) = self.backend {
            backend.write(data.to_vec());
        }
    }

    /// 写入来自外部集成的输入，例如 Public MCP。
    pub fn write_external_input(&self, data: &[u8]) {
        self.write(data);
    }

    pub fn external_input_handle(&self) -> Option<TerminalInputHandle> {
        self.backend
            .as_ref()
            .and_then(|backend| backend.input_handle())
    }

    pub fn external_exec_handle(&self) -> Option<TerminalExecHandle> {
        self.backend
            .as_ref()
            .and_then(|backend| backend.exec_handle())
    }

    /// 调整终端大小
    pub fn resize(&mut self, cols: usize, rows: usize, pixel_width: u16, pixel_height: u16) {
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
            backend.resize(TerminalSize {
                rows: rows as u16,
                cols: cols as u16,
                pixel_width,
                pixel_height,
            });
        }
    }

    /// 重新向 PTY 后端发送当前尺寸,不修改 alacritty grid。
    ///
    /// 用于在 alt screen 切换等场景下触发 SIGWINCH,
    /// 让 TUI 应用(opencode/lazygit/vim 等)重新查询尺寸并刷新整屏画面,
    /// 避免出现底部残留旧画面的问题。
    pub fn nudge_resize(&self) {
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
            if let Some(responder) = &self.ssh_mfa_responder {
                responder.cancel();
            }
            let term = self.term.clone();
            let connection_id = self.connection_id;
            let init_commands = self.init_commands.clone();
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
                        generation,
                        cx,
                    );
                    Self::spawn_ssh_history_loader(session_manager.clone(), cx);
                });
            })
            .detach();
        } else if let Some(params) = self.serial_params.clone() {
            let Some(event_tx) = self.event_tx.clone() else {
                return;
            };

            self.connection_state = ConnectionState::Connecting;
            self.set_connection_active(false, cx);
            if let Some(backend) = self.backend.take() {
                backend.shutdown();
            }
            self.reset_terminal_surface();
            let generation = self.next_connection_generation();

            let (disconnect_tx, disconnect_rx) = tokio::sync::oneshot::channel::<()>();
            Self::spawn_disconnect_handler(disconnect_rx, generation, cx);
            Self::spawn_serial_connect(
                params,
                self.term.clone(),
                event_tx,
                Some(disconnect_tx),
                generation,
                cx,
            );
        } else {
            return;
        }

        cx.emit(TerminalModelEvent::Wakeup);
    }

    fn reset_terminal_surface(&mut self) {
        let Some(event_tx) = self.event_tx.clone() else {
            return;
        };

        let event_proxy = self
            .event_proxy
            .clone()
            .unwrap_or_else(|| GpuiEventProxy::new(event_tx.clone()));
        let term_config = TermConfig {
            scrolling_history: 10000,
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
    use super::{
        CommandRecordGate, ConnectionState, Terminal, TerminalConnectionKind, TerminalMfaPrompt,
        TerminalMfaRequest, TerminalMfaResponder, build_cd_command, build_ssh_base_init_commands,
        build_ssh_init_commands, clear_screen_remote_redraw_bytes, compose_ssh_init_commands,
        format_connection_error, keyboard_interactive_answers_for_terminal, merge_history_matches,
        normalize_history_matches, resolve_default_windows_shell_from_env,
        resolve_local_working_dir, shell_escape_arg,
    };
    use crate::TerminalEvent;
    use crate::history::{
        HistoryEntry, ShellHistoryFormat, collect_history_suggestions, normalize_history_command,
        parse_shell_history, push_history_entry,
    };
    use alacritty_terminal::grid::Dimensions;
    use alacritty_terminal::index::{Column, Line};
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
    use anyhow::anyhow;
    use ssh::{KeyboardInteractiveRequest, KeyboardInteractiveResponder};
    use std::collections::VecDeque;
    use std::fs;
    use tokio::sync::mpsc::unbounded_channel;

    #[test]
    fn shell_escape_arg_handles_single_quote() {
        let escaped = shell_escape_arg("a'b");
        assert_eq!(escaped, "'a'\"'\"'b'");
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
    fn reset_terminal_surface_clears_buffer_and_stale_connection_metadata() {
        let (event_tx, _event_rx) = unbounded_channel();
        let (term, event_proxy, _colors) = Terminal::create_term(80, 24, event_tx.clone());
        let mut terminal = Terminal {
            term,
            backend: None,
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
        };

        let mut processor: Processor<StdSyncHandler> = Processor::new();
        processor.advance(&mut *terminal.term.lock(), b"hello");

        assert_eq!(terminal.term.lock().grid()[Line(0)][Column(0)].c, 'h');

        terminal.reset_terminal_surface();

        let term = terminal.term.lock();
        assert_eq!(term.grid()[Line(0)][Column(0)].c, ' ');
        assert_eq!(term.columns(), 80);
        assert_eq!(term.screen_lines(), 24);
        drop(term);
        assert_eq!(terminal.title(), "");
        assert_eq!(terminal.current_working_dir(), None);
        assert_eq!(terminal.child_exited(), None);
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
