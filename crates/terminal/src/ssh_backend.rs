use anyhow::Context as _;
use async_trait::async_trait;
use std::collections::{HashMap, VecDeque};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;
use tokio::time::Sleep;
use tokio_util::sync::CancellationToken;

use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Term;
use one_core::storage::SshAccountExpect;

use ssh::{ChannelEvent, PtyConfig, SshChannel, SshClient, SshSessionManager};

use crate::encoding::{TerminalEncoding, TerminalOutputDecoder, encode_terminal_input};
use crate::exec_supervisor::{ExecEffect, ExecPhase, ExecSupervisor, TerminalInputSource};
#[cfg(test)]
use crate::osc::extract_osc_events;
use crate::osc::{OscEvent, OscStreamParser};
use crate::pty_backend::{GpuiEventProxy, TerminalEvent};
use crate::recording::RecordingTap;
use crate::ssh_expect::SshLoginExpect;
use crate::ssh_ingress::{SshActorInput, SshParserIngress, next_ssh_actor_input};
use crate::ssh_shell_integration::{
    FilteredShellOutput, RuntimeShellIntegration, ShellIntegrationReady,
};
use crate::zmodem::{
    DetectedZmodem, ZmodemDetector, ZmodemResponder, is_channel_closed, run_transfer,
};
use crate::{
    TerminalBackend, TerminalControlAction, TerminalControlError, TerminalControlHandle,
    TerminalControlOutput, TerminalControlRequest, TerminalExecError, TerminalExecHandle,
    TerminalExecOutput, TerminalExecRequest, TerminalInputHandle, TerminalInputMetricSource,
    TerminalPerformanceMetrics, TerminalSize, TerminalTransferCancelHandle,
};

/// Shell 类型探测只用于确认运行时注入是否安全，不会写入远端文件。
const SHELL_INTEGRATION_PROBE_TIMEOUT: Duration = Duration::from_secs(1);
/// 运行时注入必须快速完成；超时后中断内部命令并降级为裸终端。
const SHELL_INTEGRATION_RUNTIME_TIMEOUT: Duration = Duration::from_secs(5);
/// 显式卸载属于用户主动操作，允许比自动探测更长的完成时间。
const SHELL_INTEGRATION_UNINSTALL_TIMEOUT: Duration = Duration::from_secs(10);
/// 没有 OSC prompt 信号时，多行初始化命令之间留出设备处理和显示交互提示的时间。
const PLAIN_INIT_COMMAND_DELAY: Duration = Duration::from_millis(250);
/// ZMODEM header prefixes overlap ordinary echoed `*` input. Keep a short
/// cross-chunk probe window, then release a retained `*`/`**` suffix.
const ZMODEM_PROBE_FLUSH_DELAY: Duration = Duration::from_millis(20);

type ZmodemProbeFlush = Pin<Box<Sleep>>;
type ShellIntegrationTimeout = Pin<Box<Sleep>>;

fn sync_zmodem_probe_flush(
    detector: &ZmodemDetector,
    pending_flush: &mut Option<ZmodemProbeFlush>,
) {
    if detector.has_plain_asterisk_prefix() {
        *pending_flush = Some(Box::pin(tokio::time::sleep(ZMODEM_PROBE_FLUSH_DELAY)));
    } else {
        pending_flush.take();
    }
}

async fn wait_for_zmodem_probe_flush(pending_flush: &mut Option<ZmodemProbeFlush>) {
    pending_flush
        .as_mut()
        .expect("pending ZMODEM probe flush should exist when polled")
        .as_mut()
        .await;
}

async fn wait_for_shell_integration_timeout(timeout: &mut Option<ShellIntegrationTimeout>) {
    timeout
        .as_mut()
        .expect("shell integration timeout should exist when polled")
        .as_mut()
        .await;
}

fn should_poll_zmodem_probe_flush(
    pending_flush: &Option<ZmodemProbeFlush>,
    has_pending_ingress: bool,
) -> bool {
    pending_flush.is_some() && !has_pending_ingress
}

fn shell_single_quote(input: &str) -> String {
    format!("'{}'", input.replace('\'', "'\"'\"'"))
}

fn is_channel_open_failure(err: &anyhow::Error) -> bool {
    let msg = format!("{err:#}").to_ascii_lowercase();
    msg.contains("channel open")
        || msg.contains("open channel")
        || msg.contains("maxsessions")
        || msg.contains("administrativelyprohibited")
        || msg.contains("administratively prohibited")
}

fn is_timeout_failure(err: &anyhow::Error) -> bool {
    let msg = format!("{err:#}").to_ascii_lowercase();
    msg.contains("timed out")
        || msg.contains("timeout")
        || msg.contains("deadline has elapsed")
        || msg.contains("i/o timeout")
}

fn add_connect_error_context(err: anyhow::Error) -> anyhow::Error {
    if is_channel_open_failure(&err) {
        return err.context(
            "the server refused to open an SSH session channel; check the account's \
             interactive CLI/EXEC permission, the device SSH service-type, and the VTY \
             / concurrent session limit",
        );
    }

    if is_timeout_failure(&err) {
        return err.context("connection timed out; check network/proxy/jump-host reachability");
    }

    err
}

#[async_trait]
trait SshSessionAccess: Send + Sync {
    type Client: SshClient;

    async fn client(&self) -> anyhow::Result<Arc<tokio::sync::Mutex<Self::Client>>>;
    async fn invalidate_client(&self, client: &Arc<tokio::sync::Mutex<Self::Client>>) -> bool;
}

#[async_trait]
impl SshSessionAccess for SshSessionManager {
    type Client = ssh::RusshClient;

    async fn client(&self) -> anyhow::Result<Arc<tokio::sync::Mutex<Self::Client>>> {
        SshSessionManager::client(self).await
    }

    async fn invalidate_client(&self, client: &Arc<tokio::sync::Mutex<Self::Client>>) -> bool {
        SshSessionManager::invalidate_client(self, client).await
    }
}

fn build_shell_integration_uninstall_script(success_marker: &str, home_marker: &str) -> String {
    let success_marker = shell_single_quote(success_marker);
    let home_marker = shell_single_quote(home_marker);

    format!(
        concat!(
            "set -e\n",
            "config_dir=\"$HOME/.config/onetcli\"\n",
            "remove_onetcli_block() {{\n",
            "    rc_file=\"$1\"\n",
            "    [ -f \"$rc_file\" ] || return 0\n",
            "    tmp_file=\"$rc_file.onetcli.$$\"\n",
            "    awk '\n",
            "        $0 == \"# BEGIN ONETCLI SHELL INTEGRATION\" {{ skip = 1; next }}\n",
            "        $0 == \"# END ONETCLI SHELL INTEGRATION\" {{ skip = 0; next }}\n",
            "        skip != 1 {{ print }}\n",
            "    ' \"$rc_file\" > \"$tmp_file\"\n",
            "    cat \"$tmp_file\" > \"$rc_file\"\n",
            "    rm -f \"$tmp_file\"\n",
            "}}\n",
            "remove_onetcli_block \"$HOME/.bashrc\"\n",
            "remove_onetcli_block \"$HOME/.bash_profile\"\n",
            "remove_onetcli_block \"$HOME/.bash_login\"\n",
            "remove_onetcli_block \"$HOME/.profile\"\n",
            "remove_onetcli_block \"$HOME/.zshrc\"\n",
            "rm -rf \"$config_dir/shell_integration.sh\"\n",
            "rm -rf \"$config_dir/sessions\"\n",
            "rmdir \"$config_dir\" 2>/dev/null || true\n",
            "printf '%s%s\\n' {home_marker} \"$HOME\"\n",
            "printf '%s\\n' {success_marker}\n"
        ),
        success_marker = success_marker,
        home_marker = home_marker,
    )
}

fn format_numbered_script(script: &str) -> String {
    script
        .lines()
        .enumerate()
        .map(|(index, line)| format!("{:>2} | {}", index + 1, line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_setup_failure_context(script: &str, stdout: &[u8], stderr: &[u8]) -> String {
    let stdout = String::from_utf8_lossy(stdout);
    let stderr = String::from_utf8_lossy(stderr);
    format!(
        "stderr: {}\nstdout: {}\nsetup script:\n{}",
        stderr.trim(),
        stdout.trim(),
        format_numbered_script(script)
    )
}

enum SshCommand {
    Write {
        source: TerminalInputSource,
        data: Vec<u8>,
    },
    InterruptForeground {
        request: TerminalControlRequest,
        cancellation: CancellationToken,
        result: oneshot::Sender<Result<TerminalControlOutput, TerminalControlError>>,
    },
    StartExec {
        id: u64,
        request: TerminalExecRequest,
        result: oneshot::Sender<Result<TerminalExecOutput, TerminalExecError>>,
    },
    CancelExec {
        id: u64,
    },
    ExecTimeout {
        id: u64,
        phase: ExecPhase,
    },
    Resize(TerminalSize),
    Shutdown,
}

enum SshRuntimeInput<Command> {
    Actor(SshActorInput<Command>),
    FlushZmodemProbe,
    FlushTerminalInput,
    ShellIntegrationTimeout,
}

enum DeferredSshActorInput {
    Command(SshCommand),
    TerminalResponse(Vec<u8>),
}

fn defer_zmodem_actor_command(
    deferred_inputs: &mut VecDeque<DeferredSshActorInput>,
    command: SshCommand,
) {
    match command {
        SshCommand::CancelExec { id } => {
            if let Some(result) = take_deferred_exec(deferred_inputs, id) {
                let _ = result.send(Err(TerminalExecError::CancelledBeforeSubmit));
            } else {
                deferred_inputs.push_back(DeferredSshActorInput::Command(SshCommand::CancelExec {
                    id,
                }));
            }
        }
        SshCommand::ExecTimeout { id, phase } => {
            if let Some(result) = take_deferred_exec(deferred_inputs, id) {
                let error = match phase {
                    ExecPhase::WaitingForReady | ExecPhase::Observing => {
                        TerminalExecError::ReadyTimeout
                    }
                    ExecPhase::ClearingInput => TerminalExecError::ClearInputTimeout,
                };
                let _ = result.send(Err(error));
            } else {
                deferred_inputs.push_back(DeferredSshActorInput::Command(
                    SshCommand::ExecTimeout { id, phase },
                ));
            }
        }
        command => {
            deferred_inputs.push_back(DeferredSshActorInput::Command(command));
        }
    }
}

fn take_deferred_exec(
    deferred_inputs: &mut VecDeque<DeferredSshActorInput>,
    id: u64,
) -> Option<ExecResultSender> {
    let index = deferred_inputs.iter().position(|input| {
        matches!(
            input,
            DeferredSshActorInput::Command(SshCommand::StartExec {
                id: deferred_id,
                ..
            }) if *deferred_id == id
        )
    })?;
    let DeferredSshActorInput::Command(SshCommand::StartExec { result, .. }) =
        deferred_inputs.remove(index)?
    else {
        return None;
    };
    Some(result)
}

const SSH_TERMINAL_INPUT_CHUNK_BYTES: usize = 4 * 1024;

#[derive(Default)]
struct PendingTerminalInput {
    chunks: VecDeque<(TerminalInputSource, Vec<u8>)>,
}

impl PendingTerminalInput {
    fn push(&mut self, source: TerminalInputSource, data: Vec<u8>) {
        self.chunks.extend(
            data.chunks(SSH_TERMINAL_INPUT_CHUNK_BYTES)
                .map(|chunk| (source, chunk.to_vec())),
        );
    }

    fn pop(&mut self) -> Option<(TerminalInputSource, Vec<u8>)> {
        self.chunks.pop_front()
    }

    fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }
}

struct ZmodemActorTransferResult {
    result: anyhow::Result<Vec<u8>>,
    shutdown_requested: bool,
}

#[derive(Clone, Default)]
struct ActiveTransferCancellation {
    current: Arc<StdMutex<Option<Arc<CancellationToken>>>>,
}

struct ActiveTransferGuard {
    owner: ActiveTransferCancellation,
    token: Arc<CancellationToken>,
}

impl ActiveTransferCancellation {
    fn current(&self) -> std::sync::MutexGuard<'_, Option<Arc<CancellationToken>>> {
        self.current
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn begin(&self) -> ActiveTransferGuard {
        let token = Arc::new(CancellationToken::new());
        if let Some(previous) = self.current().replace(token.clone()) {
            previous.cancel();
        }
        ActiveTransferGuard {
            owner: self.clone(),
            token,
        }
    }

    fn cancel(&self) -> bool {
        let token = self.current().clone();
        if let Some(token) = token {
            token.cancel();
            true
        } else {
            false
        }
    }

    fn cancel_handle(&self) -> Option<TerminalTransferCancelHandle> {
        let token = self.current().clone()?;
        let owner = self.clone();
        Some(TerminalTransferCancelHandle::new(move || {
            let current = owner.current();
            if !current
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &token))
            {
                return false;
            }
            token.cancel();
            true
        }))
    }
}

impl ActiveTransferGuard {
    fn token(&self) -> &CancellationToken {
        self.token.as_ref()
    }
}

impl Drop for ActiveTransferGuard {
    fn drop(&mut self) {
        let mut current = self.owner.current();
        if current
            .as_ref()
            .is_some_and(|token| Arc::ptr_eq(token, &self.token))
        {
            current.take();
        }
    }
}

async fn run_zmodem_transfer_while_servicing_actor<C: SshChannel>(
    channel: &mut C,
    detected: DetectedZmodem,
    responder: &ZmodemResponder,
    cancellation: &CancellationToken,
    command_rx: &mut UnboundedReceiver<SshCommand>,
    terminal_response_rx: &mut UnboundedReceiver<Vec<u8>>,
    deferred_inputs: &mut VecDeque<DeferredSshActorInput>,
) -> ZmodemActorTransferResult {
    let transfer = run_transfer(channel, detected, responder, cancellation);
    tokio::pin!(transfer);
    let mut shutdown_requested = false;

    loop {
        tokio::select! {
            biased;
            result = &mut transfer => {
                return ZmodemActorTransferResult {
                    result,
                    shutdown_requested,
                };
            }
            Some(command) = command_rx.recv() => {
                if matches!(command, SshCommand::Shutdown) {
                    shutdown_requested = true;
                    cancellation.cancel();
                } else {
                    // ZMODEM owns the SSH channel until its protocol session
                    // has ended. Keep the actor responsive without injecting
                    // terminal input, resize, or exec bytes into that session.
                    defer_zmodem_actor_command(deferred_inputs, command);
                }
            }
            Some(data) = terminal_response_rx.recv() => {
                deferred_inputs.push_back(DeferredSshActorInput::TerminalResponse(data));
            }
        }
    }
}

pub struct SshBackend {
    command_tx: UnboundedSender<SshCommand>,
    exec_ids: Arc<AtomicU64>,
    performance_metrics: Arc<TerminalPerformanceMetrics>,
    transfer_cancellation: ActiveTransferCancellation,
}

pub struct SshBackendConnect {
    pub session_manager: Arc<SshSessionManager>,
    pub pty_config: PtyConfig,
    pub terminal_encoding: TerminalEncoding,
    pub connection_id: Option<i64>,
    pub term: Arc<FairMutex<Term<GpuiEventProxy>>>,
    pub event_proxy: GpuiEventProxy,
    pub event_tx: UnboundedSender<TerminalEvent>,
    pub on_disconnect: Option<UnboundedSender<Option<String>>>,
    pub init_commands: Option<String>,
    pub account_expect: SshAccountExpect,
    pub expect_username: String,
    pub expect_password: Option<String>,
    pub disable_shell_integration: bool,
}

type ExecResultSender = oneshot::Sender<Result<TerminalExecOutput, TerminalExecError>>;

fn build_terminal_exec_handle(
    command_tx: UnboundedSender<SshCommand>,
    exec_ids: Arc<AtomicU64>,
) -> TerminalExecHandle {
    TerminalExecHandle::new(move |request, cancellation| {
        let command_tx = command_tx.clone();
        let exec_ids = exec_ids.clone();
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(TerminalExecError::CancelledBeforeSubmit);
            }
            let id = exec_ids.fetch_add(1, Ordering::Relaxed);
            let (result_tx, result_rx) = oneshot::channel();
            command_tx
                .send(SshCommand::StartExec {
                    id,
                    request,
                    result: result_tx,
                })
                .map_err(|_| TerminalExecError::Disconnected)?;
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => {
                    let _ = command_tx.send(SshCommand::CancelExec { id });
                    Err(TerminalExecError::Cancelled)
                }
                result = result_rx => result.unwrap_or(Err(TerminalExecError::Disconnected)),
            }
        })
    })
}

fn build_terminal_control_handle(command_tx: UnboundedSender<SshCommand>) -> TerminalControlHandle {
    TerminalControlHandle::new(move |request, cancellation| {
        let command_tx = command_tx.clone();
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(TerminalControlError::Cancelled);
            }
            let (result_tx, result_rx) = oneshot::channel();
            command_tx
                .send(SshCommand::InterruptForeground {
                    request,
                    cancellation,
                    result: result_tx,
                })
                .map_err(|_| TerminalControlError::Disconnected)?;
            result_rx
                .await
                .unwrap_or(Err(TerminalControlError::Disconnected))
        })
    })
}

async fn send_terminal_data<C: SshChannel + ?Sized>(
    channel: &mut C,
    data: &[u8],
) -> anyhow::Result<()> {
    match tokio::time::timeout(Duration::from_secs(30), channel.send_data(data)).await {
        Ok(result) => result.context("SSH channel data send failed"),
        Err(error) => {
            Err(anyhow::Error::new(error)
                .context("SSH channel data send timed out after 30 seconds"))
        }
    }
}

fn append_terminal_data(terminal_data: &mut Vec<u8>, chunk: Vec<u8>) {
    if terminal_data.is_empty() {
        *terminal_data = chunk;
    } else {
        terminal_data.extend(chunk);
    }
}

fn record_terminal_input(source: TerminalInputSource, data: &[u8], tap: Option<&RecordingTap>) {
    if source.is_recordable_user_input() {
        if let Some(tap) = tap {
            let _ = tap.record_input(data);
        }
    }
}

fn encode_exec_effects(
    terminal_encoding: TerminalEncoding,
    effects: Vec<ExecEffect>,
) -> Vec<ExecEffect> {
    effects
        .into_iter()
        .map(|effect| match effect {
            ExecEffect::Write { source, data } => ExecEffect::Write {
                source,
                data: encode_terminal_input(terminal_encoding, source, &data).into_owned(),
            },
            effect => effect,
        })
        .collect()
}

async fn apply_exec_effects<C: SshChannel + ?Sized>(
    effects: Vec<ExecEffect>,
    channel: &mut C,
    command_tx: &UnboundedSender<SshCommand>,
    results: &mut HashMap<u64, ExecResultSender>,
) -> anyhow::Result<()> {
    for effect in effects {
        match effect {
            ExecEffect::Write { data, .. } => {
                send_terminal_data(channel, &data)
                    .await
                    .context("failed to send exec supervisor data over SSH")?;
            }
            ExecEffect::Complete { id, output } => {
                if let Some(sender) = results.remove(&id) {
                    let _ = sender.send(Ok(output));
                }
            }
            ExecEffect::Fail { id, error } => {
                if let Some(sender) = results.remove(&id) {
                    let _ = sender.send(Err(error));
                }
            }
            ExecEffect::ArmTimeout {
                id,
                phase,
                duration,
            } => {
                let tx = command_tx.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(duration).await;
                    let _ = tx.send(SshCommand::ExecTimeout { id, phase });
                });
            }
        }
    }
    Ok(())
}

async fn send_init_commands<C: SshChannel + ?Sized>(
    channel: &mut C,
    terminal_encoding: TerminalEncoding,
    commands: &str,
    inter_command_delay: Option<Duration>,
    exec_supervisor: &mut ExecSupervisor,
    command_tx: &UnboundedSender<SshCommand>,
    exec_results: &mut HashMap<u64, ExecResultSender>,
) -> anyhow::Result<()> {
    let lines = commands
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let mut command = line.as_bytes().to_vec();
            // 终端 Enter 应发送 CR。Linux PTY 会通过 ICRNL 转成换行，网络设备 CLI 也普遍只认 CR。
            command.push(b'\r');
            command
        })
        .collect::<Vec<_>>();
    let last_index = lines.len().saturating_sub(1);
    for (index, cmd_data) in lines.into_iter().enumerate() {
        let effects = encode_exec_effects(
            terminal_encoding,
            exec_supervisor.on_input(TerminalInputSource::InitCommand, &cmd_data),
        );
        let encoded = encode_terminal_input(
            terminal_encoding,
            TerminalInputSource::InitCommand,
            &cmd_data,
        );
        apply_exec_effects(effects, channel, command_tx, exec_results)
            .await
            .context("failed to apply initialization command effects over SSH")?;
        send_terminal_data(channel, encoded.as_ref())
            .await
            .context("failed to send initialization command over SSH")?;

        if index < last_index {
            if let Some(delay) = inter_command_delay {
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }
    Ok(())
}

impl SshBackend {
    /// 用户显式触发的一次性远端清理操作。
    ///
    /// 卸载需要单独打开 exec channel，因此不能隐式塞进禁用 Shell 集成的连接流程；
    /// 否则只允许一个 session channel 的交换机会在随后打开交互终端时拒绝请求。
    pub async fn uninstall_shell_integration(
        session_manager: Arc<SshSessionManager>,
    ) -> anyhow::Result<()> {
        let client = session_manager
            .client()
            .await
            .map_err(add_connect_error_context)?;
        let result = {
            let mut guard = client.lock().await;
            Self::uninstall_shell_integration_for_client(&mut *guard).await
        };
        if result.is_ok() {
            session_manager.invalidate_client(&client).await;
        }
        result.map_err(add_connect_error_context)
    }

    /// 真机集成测试入口：探测 + 建立裸交互 channel，返回是否请求运行时注入。
    pub async fn establish_channel_for_test(
        session_manager: &Arc<SshSessionManager>,
        pty_config: &PtyConfig,
        connection_id: Option<i64>,
        disable_shell_integration: bool,
    ) -> anyhow::Result<(
        Arc<tokio::sync::Mutex<ssh::RusshClient>>,
        ssh::RusshChannel,
        bool,
    )> {
        Self::establish_channel(
            session_manager,
            pty_config,
            connection_id,
            disable_shell_integration,
        )
        .await
    }

    pub async fn connect(request: SshBackendConnect) -> anyhow::Result<Self> {
        let responder = ZmodemResponder::new(request.event_tx.clone());
        Self::connect_with_recording(request, None, responder).await
    }

    pub(crate) async fn connect_with_recording(
        request: SshBackendConnect,
        recording_tap: Option<RecordingTap>,
        zmodem_responder: ZmodemResponder,
    ) -> anyhow::Result<Self> {
        let SshBackendConnect {
            session_manager,
            pty_config,
            terminal_encoding,
            connection_id,
            term,
            event_proxy,
            event_tx,
            on_disconnect,
            init_commands,
            account_expect,
            expect_username,
            expect_password,
            disable_shell_integration,
        } = request;
        let login_expect = SshLoginExpect::new(
            &account_expect,
            &expect_username,
            expect_password.as_deref(),
        )
        .context("invalid SSH account expect configuration")?;
        let (client, mut channel, shell_integration_requested) = Self::establish_channel(
            &session_manager,
            &pty_config,
            connection_id,
            disable_shell_integration,
        )
        .await
        .map_err(add_connect_error_context)?;
        // Keep the exact transport generation with the actor so any late
        // failure report cannot invalidate a replacement published by another
        // shared consumer.
        let transport_client = client;

        // 运行时 Shell Integration 会在首段有效输出后注入；裸终端可立即处理初始化命令。
        let pending_init = init_commands;

        let (command_tx, mut command_rx) = unbounded_channel::<SshCommand>();
        let exec_ids = Arc::new(AtomicU64::new(1));
        let transfer_cancellation = ActiveTransferCancellation::default();
        let task_command_tx = command_tx.clone();
        let task_transfer_cancellation = transfer_cancellation.clone();

        // 创建 PtyWrite 回写通道
        let (pty_write_tx, mut pty_write_rx) = unbounded_channel::<Vec<u8>>();
        event_proxy.set_ssh_write_back(pty_write_tx);
        let performance_metrics = event_proxy.performance_metrics();
        let task_metrics = performance_metrics.clone();
        let parser_ingress = SshParserIngress::spawn_with_recording(
            term,
            event_proxy.clone(),
            task_metrics.clone(),
            recording_tap.clone(),
        );

        tokio::spawn(async move {
            let mut shutdown = false;
            let mut graceful_ingress_close = false;
            let mut disconnect_error: Option<anyhow::Error> = None;
            let mut pending_ingress = None;
            let mut exec_supervisor = ExecSupervisor::new();
            let mut osc_parser = OscStreamParser::default();
            let mut zmodem_detector = ZmodemDetector::default();
            let mut zmodem_probe_flush = None;
            let mut shell_integration = RuntimeShellIntegration::new(shell_integration_requested);
            let mut shell_integration_timeout = None;
            let mut output_decoder = TerminalOutputDecoder::new(terminal_encoding);
            let mut exec_results = HashMap::new();
            let mut shell_ready = shell_integration.accepts_terminal_input();
            let mut init_sent = false;
            let mut login_expect = login_expect;
            let mut deferred_actor_inputs = VecDeque::new();
            let mut pending_terminal_input = PendingTerminalInput::default();

            'actor: loop {
                // Do not let the timer-produced bytes replace a source chunk
                // that is still waiting for bounded parser-ingress capacity.
                let can_flush_zmodem_probe =
                    should_poll_zmodem_probe_flush(&zmodem_probe_flush, pending_ingress.is_some());
                let runtime_input = if shell_integration.accepts_terminal_input()
                    && deferred_actor_inputs.front().is_some()
                {
                    let input = deferred_actor_inputs
                        .pop_front()
                        .expect("checked deferred SSH actor input");
                    SshRuntimeInput::Actor(match input {
                        DeferredSshActorInput::Command(command) => SshActorInput::Command(command),
                        DeferredSshActorInput::TerminalResponse(data) => {
                            SshActorInput::TerminalResponse(data)
                        }
                    })
                } else {
                    // A large paste must yield between chunks so echoed output can reach the
                    // parser. Otherwise the remote PTY can block on stdout and stop consuming
                    // stdin, leaving russh waiting forever for more channel window capacity.
                    tokio::select! {
                        input = next_ssh_actor_input(
                            &mut channel,
                            &mut command_rx,
                            &mut pty_write_rx,
                            &mut pending_ingress,
                        ) => SshRuntimeInput::Actor(input),
                        _ = wait_for_zmodem_probe_flush(&mut zmodem_probe_flush),
                            if can_flush_zmodem_probe =>
                        {
                            SshRuntimeInput::FlushZmodemProbe
                        }
                        _ = std::future::ready(()),
                            if pending_ingress.is_none() && !pending_terminal_input.is_empty() =>
                        {
                            SshRuntimeInput::FlushTerminalInput
                        }
                        _ = wait_for_shell_integration_timeout(&mut shell_integration_timeout),
                            if shell_integration_timeout.is_some() =>
                        {
                            SshRuntimeInput::ShellIntegrationTimeout
                        }
                    }
                };
                let mut raw_terminal_data = None;

                match runtime_input {
                    SshRuntimeInput::FlushTerminalInput => {
                        let (source, data) = pending_terminal_input
                            .pop()
                            .expect("pending terminal input should contain a chunk");
                        if let Err(error) = send_terminal_data(&mut channel, &data)
                            .await
                            .context("failed to send terminal input over SSH")
                        {
                            disconnect_error = Some(error);
                            break;
                        }
                        record_terminal_input(source, &data, recording_tap.as_ref());
                    }
                    SshRuntimeInput::FlushZmodemProbe => {
                        zmodem_probe_flush.take();
                        raw_terminal_data = Some(zmodem_detector.flush_plain_asterisk_prefix());
                    }
                    SshRuntimeInput::ShellIntegrationTimeout => {
                        shell_integration_timeout.take();
                        if shell_integration.on_timeout() {
                            tracing::warn!(
                                target: "terminal.ssh.setup",
                                connection_id,
                                "运行时 Shell Integration 注入超时，发送 Ctrl+C 并降级为裸终端"
                            );
                            if let Err(error) = send_terminal_data(&mut channel, &[0x03])
                                .await
                                .context("failed to interrupt timed-out shell integration")
                            {
                                disconnect_error = Some(error);
                                break;
                            }
                        }
                    }
                    SshRuntimeInput::Actor(input) => match input {
                        SshActorInput::Command(cmd) => match cmd {
                            SshCommand::Write { source, data } => {
                                if !shell_integration.accepts_terminal_input() {
                                    deferred_actor_inputs.push_back(
                                        DeferredSshActorInput::Command(SshCommand::Write {
                                            source,
                                            data,
                                        }),
                                    );
                                    continue;
                                }
                                let effects = encode_exec_effects(
                                    terminal_encoding,
                                    exec_supervisor.on_input(source, &data),
                                );
                                if let Err(error) = apply_exec_effects(
                                    effects,
                                    &mut channel,
                                    &task_command_tx,
                                    &mut exec_results,
                                )
                                .await
                                {
                                    disconnect_error = Some(
                                        error.context("failed to apply SSH terminal input effects"),
                                    );
                                    break;
                                }
                                let encoded =
                                    encode_terminal_input(terminal_encoding, source, &data);
                                pending_terminal_input.push(source, encoded.into_owned());
                            }
                            SshCommand::InterruptForeground {
                                request,
                                cancellation,
                                result,
                            } => {
                                if cancellation.is_cancelled() {
                                    let _ = result.send(Err(TerminalControlError::Cancelled));
                                    continue;
                                }
                                let readiness = match request.action {
                                    TerminalControlAction::Interrupt => {
                                        exec_supervisor.interrupt_foreground()
                                    }
                                };
                                match readiness {
                                    Ok(readiness_before) => {
                                        match send_terminal_data(&mut channel, &[0x03])
                                            .await
                                            .context("failed to send Ctrl-C over SSH")
                                        {
                                            Ok(()) => {
                                                let _ = result.send(Ok(TerminalControlOutput {
                                                    action: request.action,
                                                    sent: true,
                                                    readiness_before,
                                                }));
                                            }
                                            Err(error) => {
                                                let _ = result
                                                    .send(Err(TerminalControlError::Disconnected));
                                                disconnect_error = Some(error);
                                                break;
                                            }
                                        }
                                    }
                                    Err(error) => {
                                        let _ = result.send(Err(error));
                                    }
                                }
                            }
                            SshCommand::StartExec {
                                id,
                                request,
                                result,
                            } => {
                                exec_results.insert(id, result);
                                let effects = encode_exec_effects(
                                    terminal_encoding,
                                    exec_supervisor.start(id, request),
                                );
                                if let Err(error) = apply_exec_effects(
                                    effects,
                                    &mut channel,
                                    &task_command_tx,
                                    &mut exec_results,
                                )
                                .await
                                {
                                    disconnect_error = Some(
                                        error.context("failed to start SSH terminal exec request"),
                                    );
                                    break;
                                }
                            }
                            SshCommand::CancelExec { id } => {
                                exec_results.remove(&id);
                                let effects = encode_exec_effects(
                                    terminal_encoding,
                                    exec_supervisor.cancel(id),
                                );
                                if let Err(error) = apply_exec_effects(
                                    effects,
                                    &mut channel,
                                    &task_command_tx,
                                    &mut exec_results,
                                )
                                .await
                                {
                                    disconnect_error = Some(
                                        error.context("failed to cancel SSH terminal exec request"),
                                    );
                                    break;
                                }
                            }
                            SshCommand::ExecTimeout { id, phase } => {
                                let effects = encode_exec_effects(
                                    terminal_encoding,
                                    exec_supervisor.timeout(id, phase),
                                );
                                if let Err(error) = apply_exec_effects(
                                    effects,
                                    &mut channel,
                                    &task_command_tx,
                                    &mut exec_results,
                                )
                                .await
                                {
                                    disconnect_error =
                                        Some(error.context(
                                            "failed to time out SSH terminal exec request",
                                        ));
                                    break;
                                }
                            }
                            SshCommand::Resize(size) => {
                                let _ =
                                    channel.resize_pty(size.cols as u32, size.rows as u32).await;
                            }
                            SshCommand::Shutdown => {
                                shutdown = true;
                                task_transfer_cancellation.cancel();
                                parser_ingress.abort();
                                let _ = channel.close().await;
                                break;
                            }
                        },
                        SshActorInput::TerminalResponse(data) => {
                            if !shell_integration.accepts_terminal_input() {
                                deferred_actor_inputs
                                    .push_back(DeferredSshActorInput::TerminalResponse(data));
                                continue;
                            }
                            let _ = exec_supervisor
                                .on_input(TerminalInputSource::TerminalResponse, &data);
                            if let Err(error) = send_terminal_data(&mut channel, &data)
                                .await
                                .context("failed to send terminal response over SSH")
                            {
                                disconnect_error = Some(error);
                                break;
                            }
                        }
                        SshActorInput::Ingress(Ok(())) => {}
                        SshActorInput::Ingress(Err(error)) => {
                            disconnect_error = Some(
                                anyhow::Error::new(error)
                                    .context("SSH terminal parser ingress rejected or closed"),
                            );
                            break;
                        }
                        SshActorInput::Channel(event) => {
                            match event {
                                Some(ChannelEvent::Data(data))
                                | Some(ChannelEvent::ExtendedData { data, .. }) => {
                                    // A zero-length SSH event carries no terminal
                                    // bytes and must not be turned into a queue
                                    // error that disconnects an otherwise valid
                                    // session.
                                    if data.is_empty() {
                                        continue;
                                    }
                                    let routed = zmodem_detector.push(&data);
                                    sync_zmodem_probe_flush(
                                        &zmodem_detector,
                                        &mut zmodem_probe_flush,
                                    );
                                    let mut routed_terminal_data = routed.terminal;
                                    if let Some(detected) = routed.transfer {
                                        let active_transfer = task_transfer_cancellation.begin();
                                        let transfer = run_zmodem_transfer_while_servicing_actor(
                                            &mut channel,
                                            detected,
                                            &zmodem_responder,
                                            active_transfer.token(),
                                            &mut command_rx,
                                            &mut pty_write_rx,
                                            &mut deferred_actor_inputs,
                                        )
                                        .await;
                                        if transfer.shutdown_requested {
                                            shutdown = true;
                                            parser_ingress.abort();
                                            let _ = channel.close().await;
                                            break 'actor;
                                        }
                                        match transfer.result {
                                            Ok(trailing) if !trailing.is_empty() => {
                                                append_terminal_data(
                                                    &mut routed_terminal_data,
                                                    trailing,
                                                );
                                            }
                                            Ok(_) => {}
                                            Err(error) => {
                                                let channel_closed = is_channel_closed(&error);
                                                tracing::warn!(
                                                    target: "terminal.ssh.runtime",
                                                    error = %format!("{error:#}"),
                                                    error_debug = ?error,
                                                    "SSH ZMODEM transfer failed"
                                                );
                                                if channel_closed {
                                                    disconnect_error = Some(error.context(
                                                    "SSH ZMODEM transfer stopped because the channel closed",
                                                ));
                                                    graceful_ingress_close = true;
                                                    break 'actor;
                                                }
                                            }
                                        }
                                    }
                                    raw_terminal_data = Some(routed_terminal_data);
                                }
                                Some(ChannelEvent::Eof) | Some(ChannelEvent::Close) | None => {
                                    graceful_ingress_close = true;
                                    let effects = encode_exec_effects(
                                        terminal_encoding,
                                        exec_supervisor.disconnect(),
                                    );
                                    let _ = apply_exec_effects(
                                        effects,
                                        &mut channel,
                                        &task_command_tx,
                                        &mut exec_results,
                                    )
                                    .await;
                                    break;
                                }
                                _ => {}
                            }
                        }
                    },
                }

                let Some(raw_terminal_data) = raw_terminal_data else {
                    continue;
                };
                if raw_terminal_data.is_empty() {
                    continue;
                }
                let data = output_decoder.decode(&raw_terminal_data);
                if data.is_empty() {
                    continue;
                }
                let expect_sends = login_expect.advance(&data);
                let expect_responded = !expect_sends.is_empty();
                for send in expect_sends {
                    if let Err(error) = send_terminal_data(&mut channel, &send)
                        .await
                        .context("failed to send SSH expect response")
                    {
                        disconnect_error = Some(error);
                        break 'actor;
                    }
                }
                let was_injecting = shell_integration.is_injecting();
                let (data, integration_ready) = match shell_integration.filter_output(data) {
                    FilteredShellOutput::Suppressed => continue,
                    FilteredShellOutput::Forward { data, ready } => (data, ready),
                };
                if was_injecting && !shell_integration.is_injecting() {
                    shell_integration_timeout.take();
                }
                if integration_ready == ShellIntegrationReady::Plain {
                    shell_ready = true;
                }
                if data.is_empty() {
                    continue;
                }
                // 解析所有 OSC 事件
                let osc_events = osc_parser.push(&data);
                let effects = encode_exec_effects(
                    terminal_encoding,
                    exec_supervisor.on_terminal_chunk(&data, &osc_events),
                );
                tracing::trace!(
                    readiness = ?exec_supervisor.readiness(),
                    "SSH terminal exec readiness updated"
                );
                if let Err(error) =
                    apply_exec_effects(effects, &mut channel, &task_command_tx, &mut exec_results)
                        .await
                {
                    disconnect_error =
                        Some(error.context("failed to apply SSH terminal output effects"));
                    break 'actor;
                }
                for osc_event in &osc_events {
                    match osc_event {
                        OscEvent::WorkingDirChanged(path) => {
                            let _ = event_tx.send(TerminalEvent::WorkingDirChanged(path.clone()));
                        }
                        OscEvent::PromptStart => {
                            let _ = event_tx.send(TerminalEvent::PromptStart);
                        }
                        OscEvent::InputStart => {
                            let _ = event_tx.send(TerminalEvent::InputStart);
                            // 133;B: prompt 渲染完，用户可以输入了
                            // 第一次收到时发送 init_commands
                            shell_integration.on_input_start();
                            if !shell_ready {
                                shell_ready = true;
                            }
                        }
                        OscEvent::CommandStart => {
                            let _ = event_tx.send(TerminalEvent::CommandStart);
                        }
                        OscEvent::CommandFinished { exit_code } => {
                            // 133;D: 命令执行完毕
                            let _ = event_tx.send(TerminalEvent::CommandFinished {
                                exit_code: *exit_code,
                            });
                        }
                        OscEvent::CommandRecorded(command) => {
                            let _ = event_tx.send(TerminalEvent::CommandRecorded(command.clone()));
                        }
                    }
                }

                if shell_integration.should_inject(
                    &data,
                    login_expect.is_complete(),
                    expect_responded,
                ) {
                    if let Err(error) =
                        send_terminal_data(&mut channel, shell_integration.injection_command())
                            .await
                            .context("failed to inject runtime shell integration")
                    {
                        disconnect_error = Some(error);
                        break 'actor;
                    }
                    shell_integration.begin_injection();
                    shell_integration_timeout = Some(Box::pin(tokio::time::sleep(
                        SHELL_INTEGRATION_RUNTIME_TIMEOUT,
                    )));
                }

                // 自动登录完成且本轮没有刚发送应答时，再发送 init_commands。
                // 避免用户名/密码应答和初始化命令落在同一轮输出中，被设备误当成登录输入。
                if shell_ready && login_expect.is_complete() && !expect_responded && !init_sent {
                    init_sent = true;
                    if let Some(ref commands) = pending_init {
                        let inter_command_delay = (!shell_integration.is_integrated())
                            .then_some(PLAIN_INIT_COMMAND_DELAY);
                        if let Err(error) = send_init_commands(
                            &mut channel,
                            terminal_encoding,
                            commands,
                            inter_command_delay,
                            &mut exec_supervisor,
                            &task_command_tx,
                            &mut exec_results,
                        )
                        .await
                        {
                            disconnect_error = Some(error);
                            break 'actor;
                        }
                    }
                }

                pending_ingress = Some(parser_ingress.pending(data));
            }

            if !graceful_ingress_close {
                parser_ingress.abort();
            } else {
                let mut trailing = output_decoder.decode(&zmodem_detector.flush_pending());
                append_terminal_data(&mut trailing, output_decoder.finish());
                if !trailing.is_empty() {
                    let mut trailing_ingress = parser_ingress.pending(trailing);
                    if let Err(error) = trailing_ingress.wait().await {
                        tracing::warn!(
                            target: "terminal.ssh.runtime",
                            error = %error,
                            error_debug = ?error,
                            "SSH terminal ingress rejected decoder trailing bytes"
                        );
                        if disconnect_error.is_none() {
                            disconnect_error = Some(anyhow::Error::new(error).context(
                                "SSH terminal parser ingress rejected decoder trailing bytes",
                            ));
                        }
                        parser_ingress.abort();
                    }
                }
            }
            // The pending future owns a sender clone. It must be dropped before
            // waiting for the parser worker, otherwise a graceful worker drain
            // can wait forever for the queue to close.
            drop(pending_ingress.take());
            if let Err(error) = parser_ingress.finish().await {
                tracing::warn!(
                    target: "terminal.ssh.runtime",
                    error = %error,
                    error_debug = ?error,
                    "SSH terminal parser worker failed"
                );
                if disconnect_error.is_none() {
                    disconnect_error = Some(
                        anyhow::Error::new(error).context("SSH terminal parser worker failed"),
                    );
                }
            }

            let effects = encode_exec_effects(terminal_encoding, exec_supervisor.disconnect());
            let _ = apply_exec_effects(effects, &mut channel, &task_command_tx, &mut exec_results)
                .await;

            if !shutdown && session_manager.invalidate_client(&transport_client).await {
                task_metrics.record_ssh_invalidation();
            }
            let disconnect_detail = disconnect_error.as_ref().map(|error| format!("{error:#}"));
            if let (Some(error), Some(detail)) =
                (disconnect_error.as_ref(), disconnect_detail.as_ref())
            {
                tracing::error!(
                    target: "terminal.ssh.runtime",
                    error = %detail,
                    error_debug = ?error,
                    "SSH terminal runtime failed"
                );
            }
            if let Some(tx) = on_disconnect {
                let _ = tx.send(disconnect_detail);
            }
        });

        Ok(Self {
            command_tx,
            exec_ids,
            performance_metrics,
            transfer_cancellation,
        })
    }

    /// 获取一个 interactive channel，封装了"channel open 失败时失效当前 transport generation
    /// 并重试一次"的重连逻辑。
    /// Shell integration 通过运行时注入生效，这里只返回"是否请求了集成"。
    async fn establish_channel(
        session_manager: &Arc<SshSessionManager>,
        pty_config: &PtyConfig,
        connection_id: Option<i64>,
        disable_shell_integration: bool,
    ) -> anyhow::Result<(
        Arc<tokio::sync::Mutex<ssh::RusshClient>>,
        ssh::RusshChannel,
        bool,
    )> {
        Self::establish_channel_with_manager(
            session_manager.as_ref(),
            pty_config,
            connection_id,
            disable_shell_integration,
        )
        .await
    }

    async fn establish_channel_with_manager<M: SshSessionAccess>(
        session_manager: &M,
        pty_config: &PtyConfig,
        connection_id: Option<i64>,
        disable_shell_integration: bool,
    ) -> anyhow::Result<(
        Arc<tokio::sync::Mutex<M::Client>>,
        <M::Client as SshClient>::Channel,
        bool,
    )> {
        let mut attempt = 0usize;
        let mut plain_channel_only = disable_shell_integration;
        loop {
            let client = session_manager.client().await?;

            let result = {
                let mut guard = client.lock().await;
                let shell_integration_requested = if plain_channel_only {
                    false
                } else {
                    Self::probe_shell_integration_support(&mut *guard, connection_id).await
                };
                Self::prepare_plain_ssh_channel(&mut *guard, pty_config)
                    .await
                    .map(|channel| (channel, shell_integration_requested))
            };

            match result {
                Ok((channel, shell_integration_requested)) => {
                    return Ok((client, channel, shell_integration_requested));
                }
                Err(err) if attempt == 0 && is_channel_open_failure(&err) => {
                    tracing::warn!(
                        target: "terminal.ssh.connect",
                        error = %err,
                        "SSH session channel 被拒绝，重建连接并降级为单个裸交互 channel"
                    );
                    let invalidated = session_manager.invalidate_client(&client).await;
                    tracing::debug!(
                        target: "terminal.ssh.connect",
                        invalidated,
                        "已报告被拒绝 channel 所属的 SSH transport generation"
                    );
                    attempt += 1;
                    plain_channel_only = true;
                    continue;
                }
                Err(err) => return Err(err),
            }
        }
    }

    async fn prepare_plain_ssh_channel<C: SshClient>(
        client: &mut C,
        pty_config: &PtyConfig,
    ) -> anyhow::Result<C::Channel> {
        let mut channel = client.open_channel().await?;
        Self::start_interactive_shell(client, &mut channel, pty_config).await?;
        Ok(channel)
    }

    /// 通过独立 exec channel 只读探测登录 shell 是否支持运行时注入。
    /// 任何失败（open 失败 / 探测出错 / 超时）都只记 warn 日志并返回 `false`，
    /// 不阻断 SSH 连接，也不向远端写入任何文件。
    async fn probe_shell_integration_support<C: SshClient>(
        client: &mut C,
        connection_id: Option<i64>,
    ) -> bool {
        Self::probe_shell_integration_support_with_timeout(
            client,
            connection_id,
            SHELL_INTEGRATION_PROBE_TIMEOUT,
        )
        .await
    }

    async fn probe_shell_integration_support_with_timeout<C: SshClient>(
        client: &mut C,
        connection_id: Option<i64>,
        timeout: Duration,
    ) -> bool {
        let mut probe_channel = match client.open_channel().await {
            Ok(ch) => ch,
            Err(err) => {
                tracing::warn!(
                    target: "terminal.ssh.setup",
                    connection_id,
                    error = %err,
                    "打开 shell integration 探测通道失败，降级为无 integration 模式"
                );
                return false;
            }
        };

        let probe_future = Self::run_shell_integration_probe(&mut probe_channel);
        let result = match tokio::time::timeout(timeout, probe_future).await {
            Ok(r) => r,
            Err(_) => {
                tracing::warn!(
                    target: "terminal.ssh.setup",
                    connection_id,
                    timeout_secs = timeout.as_secs(),
                    "shell integration 探测超时，降级为无 integration 模式"
                );
                let _ = probe_channel.close().await;
                return false;
            }
        };
        let _ = probe_channel.close().await;

        match result {
            Ok(supported) => supported,
            Err(err) => {
                tracing::warn!(
                    target: "terminal.ssh.setup",
                    connection_id,
                    error = %err,
                    "shell integration 探测失败，降级为无 integration 模式（终端仍可使用，\
                     但无 prompt hook / 命令记录）"
                );
                false
            }
        }
    }

    async fn uninstall_shell_integration_for_client<C: SshClient>(
        client: &mut C,
    ) -> anyhow::Result<()> {
        let mut channel = client.open_channel().await?;
        let result = tokio::time::timeout(
            SHELL_INTEGRATION_UNINSTALL_TIMEOUT,
            Self::run_shell_integration_uninstall(&mut channel),
        )
        .await;
        let _ = channel.close().await;

        match result {
            Ok(result) => result,
            Err(_) => anyhow::bail!(
                "shell integration uninstall timed out after {}s",
                SHELL_INTEGRATION_UNINSTALL_TIMEOUT.as_secs()
            ),
        }
    }

    async fn run_shell_integration_uninstall(channel: &mut dyn SshChannel) -> anyhow::Result<()> {
        const SUCCESS_MARKER: &str = "__ONETCLI_UNINSTALL_OK__";
        const HOME_MARKER: &str = "__ONETCLI_UNINSTALL_HOME__=";
        let uninstall_script =
            build_shell_integration_uninstall_script(SUCCESS_MARKER, HOME_MARKER);
        let cmd = format!("sh -c {}", shell_single_quote(&uninstall_script));

        channel.exec(&cmd).await?;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        loop {
            match channel.recv().await {
                Some(ChannelEvent::Data(data)) => stdout.extend(data),
                Some(ChannelEvent::ExtendedData { data, .. }) => stderr.extend(data),
                Some(ChannelEvent::ExitStatus(code)) => {
                    let context = format_setup_failure_context(&uninstall_script, &stdout, &stderr);
                    anyhow::ensure!(
                        code == 0,
                        "shell integration uninstall failed with exit code {code}: {context}",
                    );
                }
                Some(ChannelEvent::Eof) | Some(ChannelEvent::Close) | None => {
                    let output = String::from_utf8_lossy(&stdout);
                    let context = format_setup_failure_context(&uninstall_script, &stdout, &stderr);
                    anyhow::ensure!(
                        output.contains(SUCCESS_MARKER),
                        "shell integration uninstall ended before confirming completion: {context}",
                    );
                    return Ok(());
                }
                _ => {}
            }
        }
    }

    /// 只读探测登录 shell：bash/zsh 才支持运行时注入，其余（ash/fish/受限 CLI）直接跳过。
    /// 完整排空探测 channel（包括对端 Close），避免急切关闭与后续交互 channel 竞争复用。
    async fn run_shell_integration_probe(channel: &mut dyn SshChannel) -> anyhow::Result<bool> {
        const SUPPORTED_MARKER: &str = "__ONETCLI_SHELL_SUPPORTED__=1";
        let cmd = "case \"${SHELL:-}\" in *bash*|*zsh*) printf '%s\\n' '__ONETCLI_SHELL_SUPPORTED__=1';; esac";
        channel.exec(cmd).await?;

        let mut output = String::new();
        loop {
            match channel.recv().await {
                Some(ChannelEvent::Data(data)) => {
                    if output.len() < 256 {
                        output.push_str(&String::from_utf8_lossy(&data));
                    }
                }
                Some(ChannelEvent::Eof) | Some(ChannelEvent::Close) | None => {
                    return Ok(output.contains(SUPPORTED_MARKER));
                }
                _ => {}
            }
        }
    }

    async fn start_interactive_shell<C: SshClient>(
        client: &C,
        channel: &mut C::Channel,
        pty_config: &PtyConfig,
    ) -> anyhow::Result<()> {
        channel.request_pty(pty_config).await?;
        Self::maybe_request_x11_forwarding(client, channel).await;
        channel.request_shell().await?;
        Ok(())
    }

    /// 连接启用了 X11 转发时在 pty 之后、shell 之前发送 x11-req。
    /// 服务端拒绝（如 sshd 未开 X11Forwarding）只告警降级，不影响终端使用。
    async fn maybe_request_x11_forwarding<C: SshClient>(client: &C, channel: &mut C::Channel) {
        let Some(proxy) = client.x11_forwarding() else {
            return;
        };
        let request = proxy.issue_request(false);
        if let Err(error) = channel.request_x11_forwarding(&request).await {
            proxy.retract_request(&request);
            tracing::warn!(
                target: "terminal.ssh.x11",
                error = %error,
                "服务端拒绝 X11 转发请求，本会话停用 X11 转发"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::osc::parse_osc_payload;
    use crate::{TerminalControlReadiness, TerminalExecCompletion};
    use anyhow::{Result, anyhow};
    use async_trait::async_trait;
    use ssh::SshConnectConfig;
    use std::collections::VecDeque;
    use std::sync::atomic::AtomicU64;
    use std::sync::{Arc, Mutex};
    use tokio::time::sleep;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn zmodem_probe_flush_arms_only_for_plain_asterisk_prefixes() {
        let mut detector = ZmodemDetector::default();
        let mut pending_flush = None;

        let routed = detector.push(b"*");
        assert!(routed.terminal.is_empty());
        sync_zmodem_probe_flush(&detector, &mut pending_flush);
        assert!(pending_flush.is_some());
        assert!(should_poll_zmodem_probe_flush(&pending_flush, false));
        assert!(!should_poll_zmodem_probe_flush(&pending_flush, true));

        let routed = detector.push(b"\x18A");
        assert!(routed.terminal.is_empty());
        assert!(routed.transfer.is_none());
        sync_zmodem_probe_flush(&detector, &mut pending_flush);
        assert!(pending_flush.is_none());
        assert!(detector.flush_plain_asterisk_prefix().is_empty());
    }

    #[tokio::test]
    async fn active_zmodem_transfer_keeps_actor_shutdown_responsive() {
        let (mut channel, state) =
            MockChannel::new_with_delay([], false, Some(Duration::from_secs(30)));
        let (event_tx, _event_rx) = unbounded_channel();
        let responder = ZmodemResponder::new(event_tx);
        let cancellation = CancellationToken::new();
        let (command_tx, mut command_rx) = unbounded_channel();
        let (_terminal_response_tx, mut terminal_response_rx) = unbounded_channel();
        let mut deferred_inputs = VecDeque::new();

        command_tx
            .send(SshCommand::Write {
                source: TerminalInputSource::User,
                data: b"defer-until-transfer-finishes".to_vec(),
            })
            .unwrap();
        command_tx.send(SshCommand::Shutdown).unwrap();

        let transfer = tokio::time::timeout(
            Duration::from_millis(250),
            run_zmodem_transfer_while_servicing_actor(
                &mut channel,
                DetectedZmodem {
                    direction: crate::zmodem::ZmodemDirection::Upload,
                    wire: Vec::new(),
                },
                &responder,
                &cancellation,
                &mut command_rx,
                &mut terminal_response_rx,
                &mut deferred_inputs,
            ),
        )
        .await
        .expect("shutdown should cancel an active transfer without waiting for channel input");

        assert!(transfer.shutdown_requested);
        assert!(transfer.result.is_err());
        assert!(cancellation.is_cancelled());
        assert!(matches!(
            deferred_inputs.pop_front(),
            Some(DeferredSshActorInput::Command(SshCommand::Write {
                source: TerminalInputSource::User,
                data,
            })) if data == b"defer-until-transfer-finishes"
        ));
        assert!(deferred_inputs.is_empty());
        assert!(
            state
                .lock()
                .unwrap()
                .ops
                .iter()
                .any(|op| matches!(op, ChannelOp::SendData(data) if data.as_slice() == crate::zmodem::ZCAN)),
            "cancelled transfer should still notify the remote rz/sz process with ZCAN"
        );
    }

    #[tokio::test]
    async fn cancelled_exec_is_removed_from_zmodem_deferred_inputs() {
        let mut deferred_inputs = VecDeque::new();
        let (result_tx, result_rx) = oneshot::channel();
        defer_zmodem_actor_command(
            &mut deferred_inputs,
            SshCommand::StartExec {
                id: 42,
                request: request("must-not-run"),
                result: result_tx,
            },
        );

        defer_zmodem_actor_command(&mut deferred_inputs, SshCommand::CancelExec { id: 42 });

        assert!(deferred_inputs.is_empty());
        assert_eq!(
            Err(TerminalExecError::CancelledBeforeSubmit),
            result_rx.await.expect("deferred result")
        );
    }

    #[test]
    fn ssh_backend_records_direct_and_handle_input_without_double_counting() {
        let (command_tx, mut command_rx) = unbounded_channel();
        let metrics = Arc::new(TerminalPerformanceMetrics::enabled());
        let backend = SshBackend {
            command_tx,
            exec_ids: Arc::new(AtomicU64::new(1)),
            performance_metrics: metrics.clone(),
            transfer_cancellation: ActiveTransferCancellation::default(),
        };

        TerminalBackend::write(&backend, b"direct".to_vec());
        TerminalBackend::input_handle(&backend)
            .expect("SSH input handle")
            .write(b"handle".to_vec());

        assert!(matches!(
            command_rx.try_recv(),
            Ok(SshCommand::Write {
                source: TerminalInputSource::User,
                data,
            }) if data == b"direct"
        ));
        assert!(matches!(
            command_rx.try_recv(),
            Ok(SshCommand::Write {
                source: TerminalInputSource::ExternalInput,
                data,
            }) if data == b"handle"
        ));
        assert!(command_rx.try_recv().is_err());
        assert_eq!(
            (b"direct".len() + b"handle".len()) as u64,
            metrics.snapshot().user_input_bytes
        );
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum ChannelOp {
        Exec,
        SetEnv(String, String),
        SendData(Vec<u8>),
        RequestPty,
        RequestShell,
        Close,
    }

    #[derive(Default)]
    struct MockChannelState {
        ops: Vec<ChannelOp>,
        events: VecDeque<ChannelEvent>,
        exec_consumes_session: bool,
        recv_delay: Option<Duration>,
        send_data_error: bool,
    }

    struct MockChannel {
        state: Arc<Mutex<MockChannelState>>,
    }

    impl MockChannel {
        fn new(
            events: impl IntoIterator<Item = ChannelEvent>,
            exec_consumes_session: bool,
        ) -> (Self, Arc<Mutex<MockChannelState>>) {
            Self::new_with_delay(events, exec_consumes_session, None)
        }

        fn new_with_delay(
            events: impl IntoIterator<Item = ChannelEvent>,
            exec_consumes_session: bool,
            recv_delay: Option<Duration>,
        ) -> (Self, Arc<Mutex<MockChannelState>>) {
            let state = Arc::new(Mutex::new(MockChannelState {
                ops: Vec::new(),
                events: events.into_iter().collect(),
                exec_consumes_session,
                recv_delay,
                send_data_error: false,
            }));
            (
                Self {
                    state: Arc::clone(&state),
                },
                state,
            )
        }
    }

    #[async_trait]
    impl SshChannel for MockChannel {
        async fn request_pty(&mut self, _config: &PtyConfig) -> Result<()> {
            let mut state = self.state.lock().expect("mock channel state should lock");
            state.ops.push(ChannelOp::RequestPty);
            if state.exec_consumes_session {
                return Err(anyhow!("cannot request pty after exec on the same session"));
            }
            Ok(())
        }

        async fn exec(&mut self, _command: &str) -> Result<()> {
            let mut state = self.state.lock().expect("mock channel state should lock");
            state.ops.push(ChannelOp::Exec);
            Ok(())
        }

        async fn request_shell(&mut self) -> Result<()> {
            let mut state = self.state.lock().expect("mock channel state should lock");
            state.ops.push(ChannelOp::RequestShell);
            if state.exec_consumes_session {
                return Err(anyhow!(
                    "cannot request shell after exec on the same session"
                ));
            }
            Ok(())
        }

        async fn set_env(&mut self, _name: &str, _value: &str) -> Result<()> {
            self.state
                .lock()
                .expect("mock channel state should lock")
                .ops
                .push(ChannelOp::SetEnv(_name.to_string(), _value.to_string()));
            Ok(())
        }

        async fn send_data(&mut self, data: &[u8]) -> Result<()> {
            let mut state = self.state.lock().expect("mock channel state should lock");
            state.ops.push(ChannelOp::SendData(data.to_vec()));
            if state.send_data_error {
                Err(anyhow!("mock send failure"))
            } else {
                Ok(())
            }
        }

        async fn resize_pty(&mut self, _width: u32, _height: u32) -> Result<()> {
            Ok(())
        }

        async fn recv(&mut self) -> Option<ChannelEvent> {
            let delay = {
                self.state
                    .lock()
                    .expect("mock channel state should lock")
                    .recv_delay
            };
            if let Some(delay) = delay {
                sleep(delay).await;
            }
            self.state
                .lock()
                .expect("mock channel state should lock")
                .events
                .pop_front()
        }

        async fn eof(&mut self) -> Result<()> {
            Ok(())
        }

        async fn close(&mut self) -> Result<()> {
            self.state
                .lock()
                .expect("mock channel state should lock")
                .ops
                .push(ChannelOp::Close);
            Ok(())
        }
    }

    struct MockClient {
        channels: VecDeque<MockChannel>,
        open_error: Option<&'static str>,
    }

    impl MockClient {
        fn new(channels: impl IntoIterator<Item = MockChannel>) -> Self {
            Self {
                channels: channels.into_iter().collect(),
                open_error: None,
            }
        }

        fn new_with_open_error(
            channels: impl IntoIterator<Item = MockChannel>,
            open_error: &'static str,
        ) -> Self {
            Self {
                channels: channels.into_iter().collect(),
                open_error: Some(open_error),
            }
        }
    }

    #[async_trait]
    impl SshClient for MockClient {
        type Channel = MockChannel;

        async fn connect(_config: SshConnectConfig) -> Result<Self>
        where
            Self: Sized,
        {
            unreachable!("mock client connect is not used in this test")
        }

        async fn open_channel(&mut self) -> Result<Self::Channel> {
            if let Some(channel) = self.channels.pop_front() {
                return Ok(channel);
            }
            if let Some(error) = self.open_error.take() {
                return Err(anyhow!(error));
            }
            Err(anyhow!("no more mock channels"))
        }

        async fn disconnect(&mut self) -> Result<()> {
            Ok(())
        }

        fn is_connected(&self) -> bool {
            true
        }
    }

    struct MockSessionManager {
        clients: tokio::sync::Mutex<VecDeque<Arc<tokio::sync::Mutex<MockClient>>>>,
        invalidations: AtomicU64,
    }

    impl MockSessionManager {
        fn new(clients: impl IntoIterator<Item = MockClient>) -> Self {
            Self {
                clients: tokio::sync::Mutex::new(
                    clients
                        .into_iter()
                        .map(|client| Arc::new(tokio::sync::Mutex::new(client)))
                        .collect(),
                ),
                invalidations: AtomicU64::new(0),
            }
        }

        fn invalidation_count(&self) -> u64 {
            self.invalidations.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl SshSessionAccess for MockSessionManager {
        type Client = MockClient;

        async fn client(&self) -> Result<Arc<tokio::sync::Mutex<Self::Client>>> {
            self.clients
                .lock()
                .await
                .pop_front()
                .ok_or_else(|| anyhow!("no more mock clients"))
        }

        async fn invalidate_client(&self, _client: &Arc<tokio::sync::Mutex<Self::Client>>) -> bool {
            self.invalidations
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            true
        }
    }

    fn recorded_ops(state: &Arc<Mutex<MockChannelState>>) -> Vec<ChannelOp> {
        state
            .lock()
            .expect("mock channel state should lock")
            .ops
            .clone()
    }

    #[tokio::test]
    async fn send_terminal_data_preserves_channel_error_without_logging_payload() {
        let (mut channel, state) = MockChannel::new([], false);
        state
            .lock()
            .expect("mock channel state should lock")
            .send_data_error = true;

        let error = send_terminal_data(&mut channel, b"secret terminal input")
            .await
            .expect_err("SSH channel send failure should be returned");
        let message = format!("{error:#}");

        assert!(
            message.contains("SSH channel data send failed"),
            "错误链应包含发送操作上下文，实际: {message}"
        );
        assert!(
            message.contains("mock send failure"),
            "错误链应保留底层 channel 错误，实际: {message}"
        );
        assert!(
            !message.contains("secret terminal input"),
            "错误消息不能包含终端输入内容，实际: {message}"
        );
    }

    #[test]
    fn pending_terminal_input_splits_large_paste_without_reordering_bytes() {
        let data = (0..SSH_TERMINAL_INPUT_CHUNK_BYTES * 3 + 17)
            .map(|index| (index % 251) as u8)
            .collect::<Vec<_>>();
        let mut pending = PendingTerminalInput::default();

        pending.push(TerminalInputSource::User, data.clone());

        let mut restored = Vec::new();
        let mut chunk_sizes = Vec::new();
        while let Some((source, chunk)) = pending.pop() {
            assert_eq!(TerminalInputSource::User, source);
            chunk_sizes.push(chunk.len());
            restored.extend(chunk);
        }
        assert_eq!(data, restored);
        assert_eq!(
            vec![
                SSH_TERMINAL_INPUT_CHUNK_BYTES,
                SSH_TERMINAL_INPUT_CHUNK_BYTES,
                SSH_TERMINAL_INPUT_CHUNK_BYTES,
                17,
            ],
            chunk_sizes
        );
        assert!(pending.is_empty());
    }

    #[tokio::test]
    async fn ssh_input_recording_captures_only_successfully_sent_user_bytes() {
        let recording = crate::recording::test_support::TestRecording::start(
            crate::recording::RecordingBackend::Ssh,
            true,
        );
        let tap = recording.tap();
        let (mut channel, state) = MockChannel::new([], false);

        let user = encode_terminal_input(
            crate::encoding::TerminalEncoding::EucJp,
            TerminalInputSource::User,
            "あ".as_bytes(),
        );
        send_terminal_data(&mut channel, user.as_ref())
            .await
            .unwrap();
        record_terminal_input(TerminalInputSource::User, user.as_ref(), Some(&tap));
        let external = encode_terminal_input(
            crate::encoding::TerminalEncoding::EucJp,
            TerminalInputSource::ExternalInput,
            "い".as_bytes(),
        );
        send_terminal_data(&mut channel, external.as_ref())
            .await
            .unwrap();
        record_terminal_input(
            TerminalInputSource::ExternalInput,
            external.as_ref(),
            Some(&tap),
        );
        state
            .lock()
            .expect("mock channel state should lock")
            .send_data_error = true;
        assert!(
            send_terminal_data(&mut channel, b"failed input")
                .await
                .is_err()
        );

        drop(tap);
        let parsed = recording.finish();
        assert_eq!(1, parsed.events.len());
        assert!(matches!(
            &parsed.events[0].kind,
            crate::recording::RecordingEventKind::Input(data) if data == &[0xA4, 0xA2]
        ));
        assert_eq!(
            recorded_ops(&state),
            vec![
                ChannelOp::SendData(vec![0xA4, 0xA2]),
                ChannelOp::SendData(vec![0xA4, 0xA4]),
                ChannelOp::SendData(b"failed input".to_vec()),
            ],
        );
    }

    #[tokio::test]
    async fn exec_effects_encode_agent_commands_but_preserve_preflight_bytes() {
        let (mut channel, state) = MockChannel::new([], false);
        let (command_tx, _command_rx) = unbounded_channel();
        let mut exec_results = HashMap::new();
        let effects = vec![
            ExecEffect::Write {
                source: TerminalInputSource::AgentCommand,
                data: "あ\r".as_bytes().to_vec(),
            },
            ExecEffect::Write {
                source: TerminalInputSource::AgentPreflight,
                data: vec![0x03],
            },
        ];

        apply_exec_effects(
            encode_exec_effects(TerminalEncoding::EucJp, effects),
            &mut channel,
            &command_tx,
            &mut exec_results,
        )
        .await
        .expect("exec effects should be sent");
        assert_eq!(
            recorded_ops(&state),
            vec![
                ChannelOp::SendData(vec![0xA4, 0xA2, b'\r']),
                ChannelOp::SendData(vec![0x03]),
            ],
        );
    }

    #[test]
    fn zmodem_terminal_prefix_and_trailing_are_combined_before_ingress() {
        let mut terminal_data = b"before-transfer".to_vec();
        append_terminal_data(&mut terminal_data, b"after-transfer".to_vec());

        assert_eq!(terminal_data, b"before-transferafter-transfer");
    }

    #[tokio::test]
    async fn plain_init_commands_send_enable_and_password_as_separate_lines() {
        let (mut channel, state) = MockChannel::new([], false);
        let (command_tx, _command_rx) = unbounded_channel();
        let mut exec_supervisor = ExecSupervisor::new();
        let mut exec_results = HashMap::new();

        let sent = send_init_commands(
            &mut channel,
            crate::encoding::TerminalEncoding::Utf8,
            "enable\n\npassword\n",
            Some(Duration::ZERO),
            &mut exec_supervisor,
            &command_tx,
            &mut exec_results,
        )
        .await;

        assert!(sent.is_ok(), "裸终端初始化脚本应成功发送");
        assert_eq!(
            recorded_ops(&state),
            vec![
                ChannelOp::SendData(b"enable\r".to_vec()),
                ChannelOp::SendData(b"password\r".to_vec()),
            ],
            "空行应跳过，enable 和密码必须模拟终端 Enter，以 CR 分行发送"
        );
    }

    #[tokio::test]
    async fn init_commands_use_selected_terminal_encoding() {
        let (mut channel, state) = MockChannel::new([], false);
        let (command_tx, _command_rx) = unbounded_channel();
        let mut exec_supervisor = ExecSupervisor::new();
        let mut exec_results = HashMap::new();

        let sent = send_init_commands(
            &mut channel,
            crate::encoding::TerminalEncoding::EucJp,
            "あ\n",
            Some(Duration::ZERO),
            &mut exec_supervisor,
            &command_tx,
            &mut exec_results,
        )
        .await;

        assert!(sent.is_ok(), "初始化命令应使用连接选择的终端字符集");
        assert_eq!(
            recorded_ops(&state),
            vec![ChannelOp::SendData(vec![0xA4, 0xA2, b'\r'])],
        );
    }

    #[tokio::test]
    async fn establish_channel_reconnects_with_one_plain_channel_after_russh_open_failure() {
        let (probe_channel, probe_state) = MockChannel::new(
            [
                ChannelEvent::Data(b"__ONETCLI_SHELL_SUPPORTED__=1\n".to_vec()),
                ChannelEvent::Close,
            ],
            false,
        );
        let first_client = MockClient::new_with_open_error(
            [probe_channel],
            "Failed to open channel (AdministrativelyProhibited)",
        );

        let (fallback_channel, fallback_state) = MockChannel::new([], false);
        let second_client = MockClient::new([fallback_channel]);
        let manager = MockSessionManager::new([first_client, second_client]);

        let (_client, _channel, shell_integration_requested) =
            SshBackend::establish_channel_with_manager(
                &manager,
                &PtyConfig::default(),
                Some(42),
                false,
            )
            .await
            .expect("russh open channel 错误后应以单通道模式重连");

        assert!(
            !shell_integration_requested,
            "单通道降级后不能继续等待 OSC Shell Integration 信号"
        );
        assert_eq!(
            manager.invalidation_count(),
            1,
            "首次失败后只应失效发生错误的 transport generation"
        );
        assert_eq!(
            recorded_ops(&probe_state),
            vec![ChannelOp::Exec, ChannelOp::Close],
            "首次连接仍按默认配置尝试 integration 探测"
        );
        assert_eq!(
            recorded_ops(&fallback_state),
            vec![ChannelOp::RequestPty, ChannelOp::RequestShell],
            "重连后必须把唯一 channel 直接用于交互 shell"
        );
    }

    #[tokio::test]
    async fn establish_channel_reports_shell_integration_requested_after_supported_probe() {
        let (probe_channel, probe_state) = MockChannel::new(
            [
                ChannelEvent::Data(b"__ONETCLI_SHELL_SUPPORTED__=1\n".to_vec()),
                ChannelEvent::Close,
            ],
            false,
        );
        let (interactive_channel, interactive_state) = MockChannel::new([], false);
        let manager =
            MockSessionManager::new([MockClient::new([probe_channel, interactive_channel])]);

        let (_client, _channel, shell_integration_requested) =
            SshBackend::establish_channel_with_manager(
                &manager,
                &PtyConfig::default(),
                Some(42),
                false,
            )
            .await
            .expect("支持的 shell 应通过只读探测后建立交互通道");

        assert!(
            shell_integration_requested,
            "探测到 bash/zsh 后应请求运行时注入"
        );
        assert_eq!(
            recorded_ops(&probe_state),
            vec![ChannelOp::Exec, ChannelOp::Close],
            "探测通道必须只读：exec 探测命令 + close，不写任何远端文件"
        );
        assert_eq!(
            recorded_ops(&interactive_state),
            vec![ChannelOp::RequestPty, ChannelOp::RequestShell]
        );
        assert_eq!(manager.invalidation_count(), 0);
    }

    #[tokio::test]
    async fn establish_channel_skips_runtime_injection_when_probe_reports_unsupported_shell() {
        let (probe_channel, probe_state) = MockChannel::new(
            [ChannelEvent::Data(b"\n".to_vec()), ChannelEvent::Close],
            false,
        );
        let (interactive_channel, interactive_state) = MockChannel::new([], false);
        let manager =
            MockSessionManager::new([MockClient::new([probe_channel, interactive_channel])]);

        let (_client, _channel, shell_integration_requested) =
            SshBackend::establish_channel_with_manager(
                &manager,
                &PtyConfig::default(),
                Some(42),
                false,
            )
            .await
            .expect("不支持的 shell 仍应建立交互通道");

        assert!(
            !shell_integration_requested,
            "ash/fish/受限 CLI 不能注入 bash/zsh 语法脚本"
        );
        assert_eq!(
            recorded_ops(&probe_state),
            vec![ChannelOp::Exec, ChannelOp::Close]
        );
        assert_eq!(
            recorded_ops(&interactive_state),
            vec![ChannelOp::RequestPty, ChannelOp::RequestShell]
        );
    }

    #[tokio::test]
    async fn probe_shell_integration_support_respects_configured_timeout() {
        // 测试里用短 timeout 验证逻辑；生产探测 1s。
        let (probe_channel, _) = MockChannel::new_with_delay(
            [ChannelEvent::Data(b"pending...".to_vec())],
            false,
            Some(Duration::from_millis(20)),
        );
        let mut client = MockClient::new([probe_channel]);

        let res = SshBackend::probe_shell_integration_support_with_timeout(
            &mut client,
            Some(42),
            Duration::from_millis(1),
        )
        .await;
        assert!(!res, "探测超时后应降级为不注入");
    }

    #[test]
    fn add_connect_error_context_wraps_channel_open_failures() {
        let err = anyhow!("channel open failed: administratively prohibited");
        let message = add_connect_error_context(err).to_string();

        assert!(
            message.contains("the server refused to open an SSH session channel"),
            "channel open 错误应补充设备权限和会话限制提示，实际: {message}"
        );
    }

    #[test]
    fn channel_open_failure_recognizes_russh_administratively_prohibited_text() {
        let err = anyhow!("Failed to open channel (AdministrativelyProhibited)");

        assert!(
            is_channel_open_failure(&err),
            "应识别截图中的 russh 原始错误文本并触发单通道重连"
        );
    }

    #[test]
    fn add_connect_error_context_wraps_timeout_failures() {
        let err = anyhow!("dial tcp 10.0.0.8:22: i/o timeout");
        let message = add_connect_error_context(err).to_string();

        assert!(
            message.contains("connection timed out"),
            "timeout 错误应补充网络/代理排查提示，实际: {message}"
        );
    }

    #[test]
    fn parse_osc_payload_decodes_recorded_command() {
        let payload = "1337;Command=Z2l0IHN0YXR1cw==";

        let event = parse_osc_payload(payload);

        match event {
            Some(OscEvent::CommandRecorded(command)) => {
                assert_eq!(command, "git status");
            }
            other => panic!("expected recorded command event, got {other:?}"),
        }
    }

    #[test]
    fn extract_osc_events_keeps_command_recording_between_prompt_events() {
        let events = extract_osc_events(
            b"\x1b]133;A\x07\x1b]1337;Command=Z2l0IHN0YXR1cw==\x07\x1b]133;D;0\x07",
        );

        assert!(matches!(events.first(), Some(OscEvent::PromptStart)));
        assert!(
            matches!(events.get(1), Some(OscEvent::CommandRecorded(cmd)) if cmd == "git status")
        );
        assert!(matches!(
            events.get(2),
            Some(OscEvent::CommandFinished { exit_code: 0 })
        ));
    }

    #[test]
    fn ssh_backend_cancel_transfer_is_immediate_and_reusable() {
        let (command_tx, mut command_rx) = unbounded_channel();
        let transfer_cancellation = ActiveTransferCancellation::default();
        let backend = SshBackend {
            command_tx,
            exec_ids: Arc::new(AtomicU64::new(1)),
            performance_metrics: Arc::new(TerminalPerformanceMetrics::enabled()),
            transfer_cancellation: transfer_cancellation.clone(),
        };

        let first = transfer_cancellation.begin();
        assert!(TerminalBackend::cancel_transfer(&backend));
        assert!(first.token().is_cancelled());
        assert!(
            command_rx.try_recv().is_err(),
            "transfer cancellation must not wait for the SSH actor command loop"
        );
        drop(first);

        assert!(!TerminalBackend::cancel_transfer(&backend));
        let stale = transfer_cancellation.begin();
        let stale_handle =
            TerminalBackend::transfer_cancel_handle(&backend).expect("active transfer handle");
        let current = transfer_cancellation.begin();
        assert!(stale.token().is_cancelled());
        assert!(!current.token().is_cancelled());
        assert!(
            !stale_handle.cancel(),
            "a handle captured for an older transfer must reject cancellation"
        );
        assert!(
            !current.token().is_cancelled(),
            "a stale task callback must not cancel the replacement transfer"
        );

        drop(stale);
        assert!(TerminalBackend::cancel_transfer(&backend));
        assert!(
            current.token().is_cancelled(),
            "dropping a stale guard must not clear the newer transfer token"
        );
    }

    #[tokio::test]
    async fn terminal_exec_handle_cancels_waiter_without_shutdown() {
        let (command_tx, mut command_rx) = unbounded_channel();
        let handle = build_terminal_exec_handle(command_tx, Arc::new(AtomicU64::new(1)));
        let cancellation = CancellationToken::new();
        let task = tokio::spawn({
            let cancellation = cancellation.clone();
            async move { handle.exec(request("sleep 300"), cancellation).await }
        });

        let id = match command_rx.recv().await {
            Some(SshCommand::StartExec { id, .. }) => id,
            _ => panic!("expected terminal exec start command"),
        };
        cancellation.cancel();
        assert!(matches!(
            command_rx.recv().await,
            Some(SshCommand::CancelExec { id: cancelled }) if cancelled == id
        ));
        assert_eq!(
            TerminalExecError::Cancelled,
            task.await.unwrap().unwrap_err()
        );
    }

    #[tokio::test]
    async fn pre_cancelled_terminal_exec_never_enqueues_start() {
        let (command_tx, mut command_rx) = unbounded_channel();
        let handle = build_terminal_exec_handle(command_tx, Arc::new(AtomicU64::new(1)));
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let error = handle
            .exec(request("pwd"), cancellation)
            .await
            .expect_err("pre-cancelled terminal exec should not start");

        assert_eq!(TerminalExecError::CancelledBeforeSubmit, error);
        assert!(command_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn terminal_exec_handle_returns_supervisor_result() {
        let (command_tx, mut command_rx) = unbounded_channel();
        let handle = build_terminal_exec_handle(command_tx, Arc::new(AtomicU64::new(1)));
        let task =
            tokio::spawn(
                async move { handle.exec(request("pwd"), CancellationToken::new()).await },
            );

        let result = TerminalExecOutput {
            completion: TerminalExecCompletion::ShellIntegrationExit,
            exit_code: Some(0),
            output: "/tmp".to_string(),
            truncated: false,
            captured_bytes: 4,
            discarded_bytes: 0,
            duration_ms: 4,
        };
        match command_rx.recv().await {
            Some(SshCommand::StartExec { result: sender, .. }) => {
                sender.send(Ok(result.clone())).unwrap();
            }
            _ => panic!("expected terminal exec start command"),
        }

        assert_eq!(result, task.await.unwrap().unwrap());
    }

    #[tokio::test]
    async fn terminal_control_handle_returns_actor_result() {
        let (command_tx, mut command_rx) = unbounded_channel();
        let handle = build_terminal_control_handle(command_tx);
        let task = tokio::spawn(async move {
            handle
                .control(
                    TerminalControlRequest {
                        action: TerminalControlAction::Interrupt,
                    },
                    CancellationToken::new(),
                )
                .await
        });

        let output = TerminalControlOutput {
            action: TerminalControlAction::Interrupt,
            sent: true,
            readiness_before: TerminalControlReadiness::CommandRunning,
        };
        match command_rx.recv().await {
            Some(SshCommand::InterruptForeground { result, .. }) => {
                result.send(Ok(output.clone())).unwrap();
            }
            _ => panic!("expected terminal control command"),
        }

        assert_eq!(output, task.await.unwrap().unwrap());
    }

    #[tokio::test]
    async fn pre_cancelled_terminal_control_never_enqueues_command() {
        let (command_tx, mut command_rx) = unbounded_channel();
        let handle = build_terminal_control_handle(command_tx);
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let error = handle
            .control(
                TerminalControlRequest {
                    action: TerminalControlAction::Interrupt,
                },
                cancellation,
            )
            .await
            .expect_err("pre-cancelled control should not start");

        assert_eq!(TerminalControlError::Cancelled, error);
        assert!(command_rx.try_recv().is_err());
    }

    fn request(command: &str) -> TerminalExecRequest {
        TerminalExecRequest {
            command: command.to_string(),
            submit: true,
            wait_for_output: true,
            ready_timeout: Duration::ZERO,
            timeout: Duration::from_secs(30),
            observer: None,
        }
    }
}

impl TerminalBackend for SshBackend {
    fn write(&self, data: Vec<u8>) {
        self.performance_metrics
            .record_input(TerminalInputMetricSource::User, data.len());
        let _ = self.command_tx.send(SshCommand::Write {
            source: TerminalInputSource::User,
            data,
        });
    }

    fn input_handle(&self) -> Option<TerminalInputHandle> {
        let tx = self.command_tx.clone();
        Some(TerminalInputHandle::with_metrics(
            self.performance_metrics.clone(),
            move |data| {
                let _ = tx.send(SshCommand::Write {
                    source: TerminalInputSource::ExternalInput,
                    data,
                });
            },
        ))
    }

    fn exec_handle(&self) -> Option<TerminalExecHandle> {
        Some(build_terminal_exec_handle(
            self.command_tx.clone(),
            self.exec_ids.clone(),
        ))
    }

    fn control_handle(&self) -> Option<TerminalControlHandle> {
        Some(build_terminal_control_handle(self.command_tx.clone()))
    }

    fn transfer_cancel_handle(&self) -> Option<TerminalTransferCancelHandle> {
        self.transfer_cancellation.cancel_handle()
    }

    fn cancel_transfer(&self) -> bool {
        self.transfer_cancellation.cancel()
    }

    fn resize(&self, size: TerminalSize) {
        tracing::info!(
            "SshBackend::resize: 发送 resize 命令到远程 PTY: {}x{}",
            size.cols,
            size.rows
        );
        let _ = self.command_tx.send(SshCommand::Resize(size));
    }

    fn shutdown(&self) {
        self.transfer_cancellation.cancel();
        let _ = self.command_tx.send(SshCommand::Shutdown);
    }
}

impl Drop for SshBackend {
    fn drop(&mut self) {
        self.transfer_cancellation.cancel();
    }
}
