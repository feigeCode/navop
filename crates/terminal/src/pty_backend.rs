use alacritty_terminal::event::{Event as AlacTermEvent, EventListener, OnResize, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{ClipboardType, Term};
use alacritty_terminal::tty::{self, EventedPty, EventedReadWrite, Options as PtyOptions};
use alacritty_terminal::vte::ansi::{NamedColor, Rgb};
use std::borrow::Cow;
use std::collections::HashMap;
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::thread::JoinHandle;
use tokio::sync::{
    mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
    oneshot,
};
use tokio_util::sync::CancellationToken;

use crate::exec_supervisor::{ExecEffect, ExecPhase, ExecSupervisor, TerminalInputSource};
#[cfg(test)]
use crate::osc::extract_osc_events;
use crate::osc::{OscEvent, OscStreamParser};
use crate::recording::RecordingTap;
use crate::zmodem::{ZmodemTransferId, ZmodemTransferOutcome, ZmodemTransferProgress};
use crate::{
    TerminalBackend, TerminalControlError, TerminalControlHandle, TerminalControlOutput,
    TerminalControlRequest, TerminalExecError, TerminalExecHandle, TerminalExecOutput,
    TerminalExecRequest, TerminalInputHandle, TerminalInputMetricSource,
    TerminalPerformanceMetrics, TerminalSize,
};

/// 终端事件类型
#[derive(Debug, Clone)]
pub enum TerminalEvent {
    /// 终端内容已更新，需要重新渲染
    Wakeup,
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
    /// shell prompt 已渲染完成，进入可输入状态（OSC 133;B）
    InputStart,
    /// shell 命令开始执行（OSC 133;C）
    CommandStart,
    /// 终端标题已更改
    TitleChanged(String),
    /// 终端响铃
    Bell,
    /// 子进程已退出
    ChildExit(i32),
    /// 本地 PTY 后端在未被请求关闭、且子进程未退出的情况下停止工作
    ///
    /// 触发点是 alacritty 事件循环线程异常终止——读取线程 panic，或因 I/O / 轮询错误
    /// 提前退出。终止后 PTY 既不再读取输出也不再接受输入，而进程仍然存活，用户看到的
    /// 就是「终端静默卡死」。这个事件把这种不可恢复状态显式暴露给模型层，让会话结束而
    /// 不是无限期地停在「看起来还在运行」。
    ///
    /// 已知可触发读取线程 panic 的路径之一：`Conpty::on_resize` 用
    /// `assert_eq!(result, S_OK)` 断言 `ResizePseudoConsole` 成功，而该调用在句柄已失效
    /// （`E_HANDLE`）时会返回失败，例如会话拆除后仍到达一次 resize。
    ///
    /// 模型层必须显式结束会话，而不是让面板停留在「看起来还在运行」的状态。
    BackendStopped,
    /// 终端程序请求存储到剪贴板
    ClipboardStore(ClipboardType, String),
    /// 终端程序请求从剪贴板加载
    ClipboardLoad(ClipboardType),
    /// 远程工作目录变更（OSC 7），带上报主机名
    WorkingDirChanged(crate::osc::ReportedWorkingDir),
    /// 命令执行完毕（OSC 133;D）
    CommandFinished { exit_code: i32 },
    /// 记录 shell 实际执行过的命令
    CommandRecorded(String),
}

enum LocalPtyCommand {
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
    TerminalChunk {
        data: Vec<u8>,
        events: Vec<OscEvent>,
    },
    Disconnect,
    Shutdown,
}

type ExecResultSender = oneshot::Sender<Result<TerminalExecOutput, TerminalExecError>>;

fn build_local_terminal_exec_handle(
    command_tx: UnboundedSender<LocalPtyCommand>,
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
                .send(LocalPtyCommand::StartExec {
                    id,
                    request,
                    result: result_tx,
                })
                .map_err(|_| TerminalExecError::Disconnected)?;
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => {
                    let _ = command_tx.send(LocalPtyCommand::CancelExec { id });
                    Err(TerminalExecError::Cancelled)
                }
                result = result_rx => result.unwrap_or(Err(TerminalExecError::Disconnected)),
            }
        })
    })
}

fn build_local_terminal_control_handle(
    command_tx: UnboundedSender<LocalPtyCommand>,
) -> TerminalControlHandle {
    TerminalControlHandle::new(move |request, cancellation| {
        let command_tx = command_tx.clone();
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(TerminalControlError::Cancelled);
            }
            let (result_tx, result_rx) = oneshot::channel();
            command_tx
                .send(LocalPtyCommand::InterruptForeground {
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

fn terminal_event_from_osc_event(event: OscEvent) -> TerminalEvent {
    match event {
        OscEvent::PromptStart => TerminalEvent::PromptStart,
        OscEvent::InputStart => TerminalEvent::InputStart,
        OscEvent::CommandStart => TerminalEvent::CommandStart,
        OscEvent::CommandFinished { exit_code } => TerminalEvent::CommandFinished { exit_code },
        OscEvent::WorkingDirChanged(reported) => TerminalEvent::WorkingDirChanged(reported),
        OscEvent::CommandRecorded(command) => TerminalEvent::CommandRecorded(command),
    }
}

#[cfg(test)]
fn terminal_events_from_osc_chunk(data: &[u8]) -> Vec<TerminalEvent> {
    extract_osc_events(data)
        .into_iter()
        .map(terminal_event_from_osc_event)
        .collect()
}

struct OscTrackingPty<T: EventedPty> {
    inner: Box<T>,
    reader: OscTrackingReader<T>,
}

struct OscTrackingReader<T: EventedPty> {
    inner: *mut T,
    event_tx: UnboundedSender<TerminalEvent>,
    command_tx: UnboundedSender<LocalPtyCommand>,
    capture_output: Arc<AtomicBool>,
    metrics: Arc<TerminalPerformanceMetrics>,
    recording_tap: Option<RecordingTap>,
    osc_parser: OscStreamParser,
}

// EventLoop owns the wrapper on one thread; the reader pointer targets the boxed
// PTY allocation and is only dereferenced while EventLoop holds `&mut reader`.
unsafe impl<T: EventedPty + Send> Send for OscTrackingReader<T> {}

impl<T: EventedPty> OscTrackingPty<T> {
    fn new(
        inner: T,
        event_tx: UnboundedSender<TerminalEvent>,
        command_tx: UnboundedSender<LocalPtyCommand>,
        capture_output: Arc<AtomicBool>,
        metrics: Arc<TerminalPerformanceMetrics>,
        recording_tap: Option<RecordingTap>,
    ) -> Self {
        let mut inner = Box::new(inner);
        let reader = OscTrackingReader {
            inner: inner.as_mut() as *mut T,
            event_tx,
            command_tx,
            capture_output,
            metrics,
            recording_tap,
            osc_parser: OscStreamParser::default(),
        };
        Self { inner, reader }
    }
}

impl<T: EventedPty> Read for OscTrackingReader<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let bytes_read = unsafe { (&mut *self.inner).reader().read(buf) }?;
        if bytes_read > 0 {
            let data = &buf[..bytes_read];
            if let Some(recording_tap) = &self.recording_tap {
                let _ = recording_tap.record_output(data);
            }
            self.metrics.record_parser_chunk(bytes_read);
            let osc_events = self.osc_parser.push(data);
            if self.capture_output.load(Ordering::Acquire) || !osc_events.is_empty() {
                let _ = self.command_tx.send(LocalPtyCommand::TerminalChunk {
                    data: data.to_vec(),
                    events: osc_events.clone(),
                });
            }
            for event in osc_events {
                let _ = self.event_tx.send(terminal_event_from_osc_event(event));
            }
        }
        Ok(bytes_read)
    }
}

impl<T: EventedPty> EventedReadWrite for OscTrackingPty<T> {
    type Reader = OscTrackingReader<T>;
    type Writer = T::Writer;

    unsafe fn register(
        &mut self,
        poller: &Arc<polling::Poller>,
        event: polling::Event,
        mode: polling::PollMode,
    ) -> io::Result<()> {
        unsafe { self.inner.register(poller, event, mode) }
    }

    fn reregister(
        &mut self,
        poller: &Arc<polling::Poller>,
        event: polling::Event,
        mode: polling::PollMode,
    ) -> io::Result<()> {
        self.inner.reregister(poller, event, mode)
    }

    fn deregister(&mut self, poller: &Arc<polling::Poller>) -> io::Result<()> {
        self.inner.deregister(poller)
    }

    fn reader(&mut self) -> &mut Self::Reader {
        &mut self.reader
    }

    fn writer(&mut self) -> &mut Self::Writer {
        self.inner.writer()
    }
}

impl<T: EventedPty> EventedPty for OscTrackingPty<T> {
    fn next_child_event(&mut self) -> Option<tty::ChildEvent> {
        self.inner.next_child_event()
    }
}

impl<T> OnResize for OscTrackingPty<T>
where
    T: EventedPty + OnResize,
{
    fn on_resize(&mut self, window_size: WindowSize) {
        self.inner.on_resize(window_size);
    }
}

/// 用于将数据写回 PTY/SSH 通道的回写通道
///
/// 当 alacritty_terminal 处理 DA 查询等序列时，会生成 PtyWrite 事件，
/// 需要通过此通道将响应写回终端。
#[derive(Clone)]
enum PtyWriteBack {
    /// 本地 PTY：先经过 exec supervisor，再写回 EventLoop
    Local(UnboundedSender<LocalPtyCommand>),
    /// SSH：通过 UnboundedSender 写回
    Ssh(UnboundedSender<Vec<u8>>),
}

impl PtyWriteBack {
    fn write(&self, data: Vec<u8>) {
        match self {
            PtyWriteBack::Local(sender) => {
                let _ = sender.send(LocalPtyCommand::Write {
                    source: TerminalInputSource::TerminalResponse,
                    data,
                });
            }
            PtyWriteBack::Ssh(sender) => {
                let _ = sender.send(data);
            }
        }
    }

    fn disconnect(&self) {
        if let PtyWriteBack::Local(sender) = self {
            let _ = sender.send(LocalPtyCommand::Disconnect);
        }
    }
}

/// 判断事件循环的停止是否需要上报给模型层。
///
/// - 读取线程 panic：一定上报，终端已不可恢复；
/// - 主动 `shutdown()` 或已经上报过子进程退出：属于正常结束，不上报；
/// - 其余情况（事件循环因 I/O 或轮询错误提前退出）：上报。
fn should_report_backend_stopped(panicked: bool, expected_stop: bool) -> bool {
    panicked || !expected_stop
}

/// 把 UI 尺寸换算成可以提交给 ConPTY 的窗口尺寸。
///
/// 风险背景：`ResizePseudoConsole` 接受 `COORD`（i16 对），而 alacritty 的
/// `Conpty::on_resize` 用 `assert_eq!(result, S_OK)` 断言调用成功——一旦返回失败
/// HRESULT，整个 PTY 读取线程会被 panic 掉，此后终端不再产出输出、不再接受输入，
/// 界面却仍显示为存活，表现为永久卡死。
///
/// 实测结论（Windows 10 19045 / 系统内置 ConPTY，本机无第三方 `conpty.dll`）：
/// `0x0`、`-1`、`i16::MIN` 这类零/越界尺寸都会返回 `S_OK`，**不会**触发该断言；
/// 目前唯一实测到的失败来源是**句柄已失效**（`ClosePseudoConsole` 之后）返回
/// `E_HANDLE(0x80070006)`，即「会话拆除后又到达一次 resize」的竞态。
///
/// 所以本函数是**防御性**的：保证绝不把 0 或超出 `i16` 的尺寸写进 `COORD`，避免依赖
/// 平台对越界值的行为，并顺手挡掉无意义的 0 尺寸。它是加固，不是已证实的故障根因。
/// 无法安全提交时返回 `None`，由调用方保留旧尺寸。
fn conpty_safe_window_size(size: TerminalSize) -> Option<WindowSize> {
    const MAX_COORD: u16 = i16::MAX as u16;
    if size.rows == 0 || size.cols == 0 || size.rows > MAX_COORD || size.cols > MAX_COORD {
        return None;
    }
    Some(WindowSize {
        num_lines: size.rows,
        num_cols: size.cols,
        cell_width: size.pixel_width / size.cols,
        cell_height: size.pixel_height / size.rows,
    })
}

/// Local PTY backend using alacritty_terminal's EventLoop
///
/// EventLoop runs in background thread:
/// 1. Reads data from local PTY
/// 2. Parses ANSI sequences and updates Term grid
/// 3. Sends Wakeup event via EventListener
pub struct LocalPtyBackend {
    event_loop_sender: EventLoopSender,
    command_tx: UnboundedSender<LocalPtyCommand>,
    exec_ids: Arc<AtomicU64>,
    event_proxy: GpuiEventProxy,
    performance_metrics: Arc<TerminalPerformanceMetrics>,
    /// alacritty 事件循环线程已经终止；终止后任何写入/尺寸调整都不会再被处理
    stopped: Arc<AtomicBool>,
    /// 由 `shutdown()` 置位，用于把“主动关闭”与“异常停止”区分开
    shutdown_requested: Arc<AtomicBool>,
    _event_loop_handle: JoinHandle<()>,
    _supervisor_handle: JoinHandle<()>,
}

impl LocalPtyBackend {
    pub fn new(
        term: Arc<FairMutex<Term<GpuiEventProxy>>>,
        event_proxy: GpuiEventProxy,
        pty_options: PtyOptions,
    ) -> anyhow::Result<Self> {
        Self::new_with_recording(term, event_proxy, pty_options, None)
    }

    pub(crate) fn new_with_recording(
        term: Arc<FairMutex<Term<GpuiEventProxy>>>,
        event_proxy: GpuiEventProxy,
        pty_options: PtyOptions,
        recording_tap: Option<RecordingTap>,
    ) -> anyhow::Result<Self> {
        let window_size = WindowSize {
            num_lines: 24,
            num_cols: 80,
            cell_width: 8,
            cell_height: 18,
        };

        tracing::debug!(
            "LocalPtyBackend::new: 初始尺寸 {}x{}, cell={}x{}",
            window_size.num_cols,
            window_size.num_lines,
            window_size.cell_width,
            window_size.cell_height
        );

        let (command_tx, command_rx) = unbounded_channel();
        let capture_output = Arc::new(AtomicBool::new(false));
        let performance_metrics = event_proxy.performance_metrics();
        let stopped = Arc::new(AtomicBool::new(false));
        let shutdown_requested = Arc::new(AtomicBool::new(false));
        let child_exit_reported = event_proxy.child_exit_reported_handle();
        let pty = tty::new(&pty_options, window_size, 0)?;
        let pty = OscTrackingPty::new(
            pty,
            event_proxy.event_tx.clone(),
            command_tx.clone(),
            capture_output.clone(),
            performance_metrics.clone(),
            recording_tap.clone(),
        );
        let event_loop = EventLoop::new(term, event_proxy.clone(), pty, true, false)?;
        let event_loop_sender = event_loop.channel();

        // 设置 PtyWrite 回写通道，使 DA 等终端响应能写回 PTY
        event_proxy.set_write_back(PtyWriteBack::Local(command_tx.clone()));
        event_proxy.set_window_size(window_size);

        let supervisor_event_loop_sender = event_loop_sender.clone();
        let supervisor_command_tx = command_tx.clone();
        let supervisor_handle = thread::spawn(move || {
            run_local_exec_supervisor(
                command_rx,
                supervisor_command_tx,
                supervisor_event_loop_sender,
                capture_output,
                recording_tap,
            );
        });
        let event_loop_handle = {
            let stopped = stopped.clone();
            let shutdown_requested = shutdown_requested.clone();
            let event_tx = event_proxy.event_tx.clone();
            thread::spawn(move || {
                // alacritty 的 EventLoop 在内层读取线程结束时把自身和状态一起返回；
                // 内层 join 失败说明该线程 panic，此时终端已经不可恢复，
                // 必须让模型层结束会话而不是静默卡死。
                let panicked = event_loop.spawn().join().is_err();
                let expected_stop = shutdown_requested.load(Ordering::Acquire)
                    || child_exit_reported.load(Ordering::Acquire);
                stopped.store(true, Ordering::Release);
                if should_report_backend_stopped(panicked, expected_stop) {
                    tracing::warn!(
                        panicked,
                        "本地 PTY 事件循环已停止且无预期关闭来源，标记会话结束"
                    );
                    let _ = event_tx.send(TerminalEvent::BackendStopped);
                }
            })
        };

        Ok(Self {
            event_loop_sender,
            command_tx,
            exec_ids: Arc::new(AtomicU64::new(1)),
            event_proxy,
            performance_metrics,
            stopped,
            shutdown_requested,
            _event_loop_handle: event_loop_handle,
            _supervisor_handle: supervisor_handle,
        })
    }

    /// 事件循环是否已经终止；终止后写入不会再被处理。
    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }

    pub fn write(&self, data: Vec<u8>) {
        if self.is_stopped() {
            tracing::debug!("本地 PTY 已停止，丢弃 {} 字节输入", data.len());
            return;
        }
        self.performance_metrics
            .record_input(TerminalInputMetricSource::User, data.len());
        let _ = self.command_tx.send(LocalPtyCommand::Write {
            source: TerminalInputSource::User,
            data,
        });
    }

    pub fn resize(&self, size: TerminalSize) {
        if self.is_stopped() {
            tracing::debug!("本地 PTY 已停止，忽略尺寸调整 {}x{}", size.cols, size.rows);
            return;
        }
        let Some(window_size) = conpty_safe_window_size(size) else {
            // 防御性跳过：不把 0 / 越界尺寸提交给 ConPTY。注意本机实测表明这类尺寸
            // 目前返回 S_OK（并不会触发 alacritty 的 assert），所以这里是加固而非根因修复。
            // 保留旧尺寸，让后续合法尺寸仍有机会生效。
            tracing::warn!(
                cols = size.cols,
                rows = size.rows,
                "忽略无法提交给 ConPTY 的终端尺寸"
            );
            return;
        };
        tracing::debug!(
            "LocalPtyBackend::resize: {}x{}, cell={}x{}, pixel={}x{}",
            window_size.num_cols,
            window_size.num_lines,
            window_size.cell_width,
            window_size.cell_height,
            size.pixel_width,
            size.pixel_height
        );
        self.event_proxy.set_window_size(window_size);
        let _ = self.event_loop_sender.send(Msg::Resize(window_size));
    }

    pub fn shutdown(&self) {
        self.shutdown_requested.store(true, Ordering::Release);
        let _ = self.command_tx.send(LocalPtyCommand::Shutdown);
    }
}

impl TerminalBackend for LocalPtyBackend {
    fn write(&self, data: Vec<u8>) {
        LocalPtyBackend::write(self, data);
    }

    fn input_handle(&self) -> Option<TerminalInputHandle> {
        let sender = self.command_tx.clone();
        Some(TerminalInputHandle::with_metrics(
            self.performance_metrics.clone(),
            move |data| {
                let _ = sender.send(LocalPtyCommand::Write {
                    source: TerminalInputSource::ExternalInput,
                    data,
                });
            },
        ))
    }

    fn exec_handle(&self) -> Option<TerminalExecHandle> {
        Some(build_local_terminal_exec_handle(
            self.command_tx.clone(),
            self.exec_ids.clone(),
        ))
    }

    fn control_handle(&self) -> Option<TerminalControlHandle> {
        Some(build_local_terminal_control_handle(self.command_tx.clone()))
    }

    fn resize(&self, size: TerminalSize) {
        LocalPtyBackend::resize(self, size);
    }

    fn shutdown(&self) {
        LocalPtyBackend::shutdown(self);
    }
}

fn run_local_exec_supervisor(
    mut command_rx: UnboundedReceiver<LocalPtyCommand>,
    command_tx: UnboundedSender<LocalPtyCommand>,
    event_loop_sender: EventLoopSender,
    capture_output: Arc<AtomicBool>,
    recording_tap: Option<RecordingTap>,
) {
    let mut supervisor = ExecSupervisor::new();
    let mut exec_results = HashMap::<u64, ExecResultSender>::new();
    while let Some(command) = command_rx.blocking_recv() {
        let keep_running = match command {
            LocalPtyCommand::Write { source, data } => {
                let effects_applied = apply_local_exec_effects(
                    supervisor.on_input(source, &data),
                    &event_loop_sender,
                    &command_tx,
                    &mut exec_results,
                );
                effects_applied
                    && send_local_input(source, data, recording_tap.as_ref(), |data| {
                        event_loop_sender.send(Msg::Input(Cow::Owned(data))).is_ok()
                    })
            }
            LocalPtyCommand::InterruptForeground {
                request,
                cancellation,
                result,
            } => {
                if cancellation.is_cancelled() {
                    let _ = result.send(Err(TerminalControlError::Cancelled));
                    true
                } else {
                    let readiness = match request.action {
                        crate::TerminalControlAction::Interrupt => {
                            supervisor.interrupt_foreground()
                        }
                    };
                    match readiness {
                        Ok(readiness_before) => {
                            if event_loop_sender
                                .send(Msg::Input(Cow::Owned(vec![0x03])))
                                .is_ok()
                            {
                                let _ = result.send(Ok(TerminalControlOutput {
                                    action: request.action,
                                    sent: true,
                                    readiness_before,
                                }));
                                true
                            } else {
                                let _ = result.send(Err(TerminalControlError::Disconnected));
                                false
                            }
                        }
                        Err(error) => {
                            let _ = result.send(Err(error));
                            true
                        }
                    }
                }
            }
            LocalPtyCommand::StartExec {
                id,
                request,
                result,
            } => {
                exec_results.insert(id, result);
                apply_local_exec_effects(
                    supervisor.start(id, request),
                    &event_loop_sender,
                    &command_tx,
                    &mut exec_results,
                )
            }
            LocalPtyCommand::CancelExec { id } => {
                exec_results.remove(&id);
                apply_local_exec_effects(
                    supervisor.cancel(id),
                    &event_loop_sender,
                    &command_tx,
                    &mut exec_results,
                )
            }
            LocalPtyCommand::ExecTimeout { id, phase } => apply_local_exec_effects(
                supervisor.timeout(id, phase),
                &event_loop_sender,
                &command_tx,
                &mut exec_results,
            ),
            LocalPtyCommand::TerminalChunk { data, events } => apply_local_exec_effects(
                supervisor.on_terminal_chunk(&data, &events),
                &event_loop_sender,
                &command_tx,
                &mut exec_results,
            ),
            LocalPtyCommand::Disconnect => false,
            LocalPtyCommand::Shutdown => {
                let _ = event_loop_sender.send(Msg::Shutdown);
                false
            }
        };
        capture_output.store(supervisor.captures_terminal_output(), Ordering::Release);
        if !keep_running {
            break;
        }
    }

    let _ = apply_local_exec_effects(
        supervisor.disconnect(),
        &event_loop_sender,
        &command_tx,
        &mut exec_results,
    );
    for (_, result) in exec_results.drain() {
        let _ = result.send(Err(TerminalExecError::Disconnected));
    }
    capture_output.store(false, Ordering::Release);
}

fn apply_local_exec_effects(
    effects: Vec<ExecEffect>,
    event_loop_sender: &EventLoopSender,
    command_tx: &UnboundedSender<LocalPtyCommand>,
    results: &mut HashMap<u64, ExecResultSender>,
) -> bool {
    for effect in effects {
        match effect {
            ExecEffect::Write { data, .. } => {
                if event_loop_sender
                    .send(Msg::Input(Cow::Owned(data)))
                    .is_err()
                {
                    return false;
                }
            }
            ExecEffect::Complete { id, output } => {
                if let Some(result) = results.remove(&id) {
                    let _ = result.send(Ok(output));
                }
            }
            ExecEffect::Fail { id, error } => {
                if let Some(result) = results.remove(&id) {
                    let _ = result.send(Err(error));
                }
            }
            ExecEffect::ArmTimeout {
                id,
                phase,
                duration,
            } => {
                let command_tx = command_tx.clone();
                thread::spawn(move || {
                    thread::sleep(duration);
                    let _ = command_tx.send(LocalPtyCommand::ExecTimeout { id, phase });
                });
            }
        }
    }
    true
}

/// GPUI Event proxy for alacritty_terminal
/// 将 alacritty 事件转换为 TerminalEvent 并发送，
/// 同时处理 PtyWrite 等需要回写 PTY 的事件
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GpuiEventPolicy {
    Live,
    /// Fail-closed policy for untrusted recording playback.
    ///
    /// Only grid invalidation may leave the parser. Terminal responses,
    /// clipboard access, title changes, bells, and exit events are discarded.
    PlaybackSafe,
}

#[derive(Clone)]
pub struct GpuiEventProxy {
    event_tx: UnboundedSender<TerminalEvent>,
    policy: GpuiEventPolicy,
    /// PtyWrite 回写通道（在后端创建后设置）
    write_back: Arc<Mutex<Option<PtyWriteBack>>>,
    /// 共享窗口尺寸，供 TextAreaSizeRequest 真实回复使用
    window_size: Arc<Mutex<WindowSize>>,
    /// Wakeup 去重标记：true 表示完整事件链路中已有尚未被 GPUI 消费的 Wakeup
    wakeup_pending: Arc<AtomicBool>,
    /// 是否已上报过子进程退出；用于把“子进程退出”与“后端异常停止”区分开
    child_exit_reported: Arc<AtomicBool>,
    metrics: Arc<TerminalPerformanceMetrics>,
}

impl GpuiEventProxy {
    pub fn new(event_tx: UnboundedSender<TerminalEvent>) -> Self {
        Self::with_metrics(event_tx, Arc::new(TerminalPerformanceMetrics::default()))
    }

    pub fn with_metrics(
        event_tx: UnboundedSender<TerminalEvent>,
        metrics: Arc<TerminalPerformanceMetrics>,
    ) -> Self {
        Self::with_metrics_and_policy(event_tx, metrics, GpuiEventPolicy::Live)
    }

    pub(crate) fn playback_safe(
        event_tx: UnboundedSender<TerminalEvent>,
        metrics: Arc<TerminalPerformanceMetrics>,
    ) -> Self {
        Self::with_metrics_and_policy(event_tx, metrics, GpuiEventPolicy::PlaybackSafe)
    }

    fn with_metrics_and_policy(
        event_tx: UnboundedSender<TerminalEvent>,
        metrics: Arc<TerminalPerformanceMetrics>,
        policy: GpuiEventPolicy,
    ) -> Self {
        Self {
            event_tx,
            policy,
            write_back: Arc::new(Mutex::new(None)),
            window_size: Arc::new(Mutex::new(WindowSize {
                num_lines: 24,
                num_cols: 80,
                cell_width: 8,
                cell_height: 18,
            })),
            wakeup_pending: Arc::new(AtomicBool::new(false)),
            child_exit_reported: Arc::new(AtomicBool::new(false)),
            metrics,
        }
    }

    pub fn performance_metrics(&self) -> Arc<TerminalPerformanceMetrics> {
        self.metrics.clone()
    }

    /// 设置回写通道
    fn set_write_back(&self, wb: PtyWriteBack) {
        if self.policy == GpuiEventPolicy::PlaybackSafe {
            return;
        }
        *self.write_back.lock().unwrap() = Some(wb);
    }

    /// 设置 SSH 回写通道
    pub(crate) fn set_ssh_write_back(&self, sender: UnboundedSender<Vec<u8>>) {
        self.set_write_back(PtyWriteBack::Ssh(sender));
    }

    /// 同步当前真实窗口尺寸（含 cell 像素），后续 TextAreaSizeRequest 将以此回复
    pub(crate) fn set_window_size(&self, size: WindowSize) {
        *self.window_size.lock().unwrap() = size;
    }

    /// 当 UI 已经消费 Wakeup 后调用，允许下一次 Wakeup 入队
    #[cfg(test)]
    fn reset_wakeup_pending(&self) {
        self.wakeup_pending.store(false, Ordering::Release);
    }

    pub(crate) fn queue_wakeup(&self) {
        self.send_event(AlacTermEvent::Wakeup);
    }

    /// 返回端到端 Wakeup 去重标记的句柄。
    /// 只有 GPUI 消费对应 Wakeup 后才能 reset，防止前台繁忙时渲染队列无限积压。
    pub(crate) fn wakeup_pending_handle(&self) -> Arc<AtomicBool> {
        self.wakeup_pending.clone()
    }

    /// 返回“子进程退出已上报”标记的句柄。
    ///
    /// 本地 PTY 后端用它区分正常结束（子进程退出、主动关闭）与异常停止，
    /// 避免子进程正常退出后又补发一次后端停止。
    fn child_exit_reported_handle(&self) -> Arc<AtomicBool> {
        self.child_exit_reported.clone()
    }

    fn current_window_size(&self) -> WindowSize {
        *self.window_size.lock().unwrap()
    }

    fn write_back(&self, data: Vec<u8>) {
        self.metrics
            .record_input(TerminalInputMetricSource::TerminalResponse, data.len());
        if let Some(wb) = self.write_back.lock().unwrap().as_ref() {
            wb.write(data);
        }
    }

    fn disconnect_local_backend(&self) {
        if let Some(wb) = self.write_back.lock().unwrap().as_ref() {
            wb.disconnect();
        }
    }
}

fn send_local_input(
    source: TerminalInputSource,
    data: Vec<u8>,
    recording_tap: Option<&RecordingTap>,
    send: impl FnOnce(Vec<u8>) -> bool,
) -> bool {
    // The Alacritty event-loop sender consumes the Vec on success. Preserve a
    // copy only while disclosed input capture is active so inactive terminals
    // retain the zero-copy fast path.
    let recording_data = (source.is_recordable_user_input()
        && recording_tap.is_some_and(RecordingTap::is_input_capture_active))
    .then(|| data.clone());
    let sent = send(data);
    if sent {
        if let (Some(tap), Some(recording_data)) = (recording_tap, recording_data) {
            let _ = tap.record_input(&recording_data);
        }
    }
    sent
}

impl EventListener for GpuiEventProxy {
    fn send_event(&self, event: AlacTermEvent) {
        // Recording bytes are untrusted terminal input. Fail closed for every
        // current and future Alacritty event except the render invalidation
        // needed after harmless grid mutation.
        if self.policy == GpuiEventPolicy::PlaybackSafe && !matches!(&event, AlacTermEvent::Wakeup)
        {
            return;
        }

        let terminal_event = match event {
            AlacTermEvent::PtyWrite(text) => {
                self.write_back(text.into_bytes());
                return;
            }
            AlacTermEvent::ColorRequest(index, format_fn) => {
                let text = format_fn(default_color_for_index(index));
                self.write_back(text.into_bytes());
                return;
            }
            AlacTermEvent::TextAreaSizeRequest(format_fn) => {
                let text = format_fn(self.current_window_size());
                self.write_back(text.into_bytes());
                return;
            }
            AlacTermEvent::Wakeup => {
                self.metrics.record_wakeup_request();
                // 去重：已有未消费 Wakeup 时直接丢弃，避免高速输出下事件堆积
                if self.wakeup_pending.swap(true, Ordering::AcqRel) {
                    self.metrics.record_wakeup_coalesced();
                    return;
                }
                TerminalEvent::Wakeup
            }
            AlacTermEvent::Title(title) => TerminalEvent::TitleChanged(title),
            AlacTermEvent::Bell => TerminalEvent::Bell,
            AlacTermEvent::ClipboardStore(ty, data) => TerminalEvent::ClipboardStore(ty, data),
            AlacTermEvent::ClipboardLoad(ty, _) => TerminalEvent::ClipboardLoad(ty),
            AlacTermEvent::Exit => {
                // 子进程退出是正常结束：记录后，后端读取线程随之结束时不再补发
                // `BackendStopped`，以免把退出码覆盖成“无退出码”的哨兵值。
                self.child_exit_reported.store(true, Ordering::Release);
                self.disconnect_local_backend();
                TerminalEvent::ChildExit(0)
            }
            _ => return,
        };
        let is_wakeup = matches!(terminal_event, TerminalEvent::Wakeup);
        if self.event_tx.send(terminal_event).is_ok() && is_wakeup {
            self.metrics.record_wakeup_queued();
        }
    }
}

/// 为 OSC 4/10/11 等颜色查询提供合理的默认回复，避免一律返回黑色
fn default_color_for_index(index: usize) -> Rgb {
    match index {
        // OSC 10：默认前景色 -> 接近白色
        idx if idx == NamedColor::Foreground as usize => Rgb {
            r: 0xE4,
            g: 0xE4,
            b: 0xE4,
        },
        // OSC 11：默认背景色 -> 接近深灰
        idx if idx == NamedColor::Background as usize => Rgb {
            r: 0x1E,
            g: 0x1E,
            b: 0x1E,
        },
        // OSC 12：光标颜色
        idx if idx == NamedColor::Cursor as usize => Rgb {
            r: 0xFF,
            g: 0xFF,
            b: 0xFF,
        },
        _ => Rgb { r: 0, g: 0, b: 0 },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TerminalPerformanceMetrics;
    use base64::Engine;
    use std::io::Cursor;
    use std::sync::Arc;
    use tokio::sync::mpsc::unbounded_channel;
    use tokio_util::sync::CancellationToken;

    struct TestEventedPty {
        reader: Cursor<Vec<u8>>,
        writer: io::Sink,
    }

    impl TestEventedPty {
        fn new(data: impl Into<Vec<u8>>) -> Self {
            Self {
                reader: Cursor::new(data.into()),
                writer: io::sink(),
            }
        }
    }

    impl EventedReadWrite for TestEventedPty {
        type Reader = Cursor<Vec<u8>>;
        type Writer = io::Sink;

        unsafe fn register(
            &mut self,
            _poller: &Arc<polling::Poller>,
            _event: polling::Event,
            _mode: polling::PollMode,
        ) -> io::Result<()> {
            Ok(())
        }

        fn reregister(
            &mut self,
            _poller: &Arc<polling::Poller>,
            _event: polling::Event,
            _mode: polling::PollMode,
        ) -> io::Result<()> {
            Ok(())
        }

        fn deregister(&mut self, _poller: &Arc<polling::Poller>) -> io::Result<()> {
            Ok(())
        }

        fn reader(&mut self) -> &mut Self::Reader {
            &mut self.reader
        }

        fn writer(&mut self) -> &mut Self::Writer {
            &mut self.writer
        }
    }

    impl EventedPty for TestEventedPty {
        fn next_child_event(&mut self) -> Option<tty::ChildEvent> {
            None
        }
    }

    fn exec_request(command: &str) -> crate::TerminalExecRequest {
        crate::TerminalExecRequest {
            command: command.to_string(),
            submit: true,
            wait_for_output: false,
            ready_timeout: std::time::Duration::ZERO,
            timeout: std::time::Duration::from_secs(1),
            observer: None,
        }
    }

    #[test]
    fn osc_tracking_reader_records_each_non_empty_pty_read() {
        let (event_tx, _event_rx) = unbounded_channel();
        let (command_tx, mut command_rx) = unbounded_channel();
        let metrics = Arc::new(TerminalPerformanceMetrics::enabled());
        let mut pty = OscTrackingPty::new(
            TestEventedPty::new(b"local output".to_vec()),
            event_tx,
            command_tx,
            Arc::new(AtomicBool::new(false)),
            metrics.clone(),
            None,
        );
        let mut buffer = [0; 64];

        assert_eq!(12, pty.reader().read(&mut buffer).expect("PTY read"));
        assert_eq!(0, pty.reader().read(&mut buffer).expect("PTY EOF"));
        assert!(command_rx.try_recv().is_err());

        let snapshot = metrics.snapshot();
        assert_eq!(12, snapshot.ingress_bytes);
        assert_eq!(1, snapshot.parser_chunks);
        assert_eq!(12, snapshot.parser_chunk_bytes);
        assert_eq!(12, snapshot.parser_chunk_max_bytes);
    }

    #[test]
    fn osc_tracking_reader_records_raw_output_at_the_parser_boundary() {
        let recording = crate::recording::test_support::TestRecording::start(
            crate::recording::RecordingBackend::Local,
            false,
        );
        let (event_tx, _event_rx) = unbounded_channel();
        let (command_tx, _command_rx) = unbounded_channel();
        let metrics = Arc::new(TerminalPerformanceMetrics::enabled());
        let mut pty = OscTrackingPty::new(
            TestEventedPty::new(b"\xffraw\x1b]133;A\x07output".to_vec()),
            event_tx,
            command_tx,
            Arc::new(AtomicBool::new(false)),
            metrics,
            Some(recording.tap()),
        );
        let mut buffer = [0; 64];

        let bytes_read = pty.reader().read(&mut buffer).expect("PTY read");
        assert_eq!(b"\xffraw\x1b]133;A\x07output".len(), bytes_read);

        let parsed = recording.finish();
        assert_eq!(1, parsed.events.len());
        assert!(matches!(
            &parsed.events[0].kind,
            crate::recording::RecordingEventKind::Output(data)
                if data == b"\xffraw\x1b]133;A\x07output"
        ));
    }

    #[test]
    fn local_input_recording_captures_only_accepted_disclosed_user_bytes() {
        let recording = crate::recording::test_support::TestRecording::start(
            crate::recording::RecordingBackend::Local,
            true,
        );
        let tap = recording.tap();

        assert!(send_local_input(
            TerminalInputSource::User,
            b"user input".to_vec(),
            Some(&tap),
            |data| data == b"user input"
        ));
        for source in [
            TerminalInputSource::ExternalInput,
            TerminalInputSource::AgentPreflight,
            TerminalInputSource::AgentCommand,
            TerminalInputSource::TerminalResponse,
            TerminalInputSource::InitCommand,
        ] {
            assert!(send_local_input(
                source,
                b"excluded input".to_vec(),
                Some(&tap),
                |_| true
            ));
        }
        assert!(!send_local_input(
            TerminalInputSource::User,
            b"rejected input".to_vec(),
            Some(&tap),
            |_| false
        ));

        drop(tap);
        let parsed = recording.finish();
        assert_eq!(1, parsed.events.len());
        assert!(matches!(
            &parsed.events[0].kind,
            crate::recording::RecordingEventKind::Input(data) if data == b"user input"
        ));
    }

    #[test]
    fn event_proxy_records_wakeup_request_queue_and_coalescing() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let metrics = Arc::new(TerminalPerformanceMetrics::enabled());
        let proxy = GpuiEventProxy::with_metrics(event_tx, metrics.clone());

        proxy.send_event(AlacTermEvent::Wakeup);
        proxy.send_event(AlacTermEvent::Wakeup);
        proxy.send_event(AlacTermEvent::Wakeup);

        assert!(matches!(event_rx.try_recv(), Ok(TerminalEvent::Wakeup)));
        assert!(event_rx.try_recv().is_err());
        let snapshot = metrics.snapshot();
        assert_eq!(3, snapshot.wakeup_requests);
        assert_eq!(1, snapshot.wakeup_queued);
        assert_eq!(2, snapshot.wakeup_coalesced);

        proxy.reset_wakeup_pending();
        proxy.send_event(AlacTermEvent::Wakeup);
        assert!(matches!(event_rx.try_recv(), Ok(TerminalEvent::Wakeup)));
        let snapshot = proxy.performance_metrics().snapshot();
        assert_eq!(4, snapshot.wakeup_requests);
        assert_eq!(2, snapshot.wakeup_queued);
        assert_eq!(2, snapshot.wakeup_coalesced);
    }

    #[test]
    fn event_proxy_only_records_queued_wakeup_after_successful_send() {
        let (event_tx, event_rx) = unbounded_channel();
        let metrics = Arc::new(TerminalPerformanceMetrics::enabled());
        let proxy = GpuiEventProxy::with_metrics(event_tx, metrics.clone());
        drop(event_rx);

        proxy.send_event(AlacTermEvent::Wakeup);

        let snapshot = metrics.snapshot();
        assert_eq!(1, snapshot.wakeup_requests);
        assert_eq!(0, snapshot.wakeup_queued);
        assert_eq!(0, snapshot.wakeup_coalesced);
    }

    #[test]
    fn event_proxy_records_all_terminal_responses_without_a_write_back_sink() {
        let (event_tx, _event_rx) = unbounded_channel();
        let metrics = Arc::new(TerminalPerformanceMetrics::enabled());
        let proxy = GpuiEventProxy::with_metrics(event_tx, metrics.clone());

        proxy.send_event(AlacTermEvent::PtyWrite("pty".to_string()));
        proxy.send_event(AlacTermEvent::ColorRequest(
            NamedColor::Foreground as usize,
            Arc::new(|_| "color".to_string()),
        ));
        proxy.send_event(AlacTermEvent::TextAreaSizeRequest(Arc::new(|_| {
            "size".to_string()
        })));

        assert_eq!(
            ("pty".len() + "color".len() + "size".len()) as u64,
            metrics.snapshot().terminal_response_bytes
        );
    }

    #[test]
    fn playback_safe_event_proxy_only_allows_grid_wakeup() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let metrics = Arc::new(TerminalPerformanceMetrics::enabled());
        let proxy = GpuiEventProxy::playback_safe(event_tx, metrics.clone());
        let (write_tx, mut write_rx) = unbounded_channel::<Vec<u8>>();

        // Even an accidental attempt to attach a live response sink must not
        // grant a playback parser write-back capability.
        proxy.set_ssh_write_back(write_tx);
        proxy.send_event(AlacTermEvent::PtyWrite("response".to_string()));
        proxy.send_event(AlacTermEvent::ColorRequest(
            NamedColor::Foreground as usize,
            Arc::new(|_| "color".to_string()),
        ));
        proxy.send_event(AlacTermEvent::TextAreaSizeRequest(Arc::new(|_| {
            "size".to_string()
        })));
        proxy.send_event(AlacTermEvent::ClipboardStore(
            ClipboardType::Clipboard,
            "secret".to_string(),
        ));
        proxy.send_event(AlacTermEvent::ClipboardLoad(
            ClipboardType::Clipboard,
            Arc::new(|_| "clipboard contents".to_string()),
        ));
        proxy.send_event(AlacTermEvent::Title("recorded title".to_string()));
        proxy.send_event(AlacTermEvent::Bell);
        proxy.send_event(AlacTermEvent::Exit);
        proxy.send_event(AlacTermEvent::Wakeup);

        assert!(write_rx.try_recv().is_err());
        assert!(matches!(event_rx.try_recv(), Ok(TerminalEvent::Wakeup)));
        assert!(event_rx.try_recv().is_err());

        let snapshot = metrics.snapshot();
        assert_eq!(0, snapshot.terminal_response_bytes);
        assert_eq!(1, snapshot.wakeup_requests);
        assert_eq!(1, snapshot.wakeup_queued);
    }

    #[test]
    fn wakeup_dedup_collapses_repeated_wakeups_until_reset() {
        let (tx, mut rx) = unbounded_channel::<TerminalEvent>();
        let proxy = GpuiEventProxy::new(tx);

        proxy.send_event(AlacTermEvent::Wakeup);
        proxy.send_event(AlacTermEvent::Wakeup);
        proxy.send_event(AlacTermEvent::Wakeup);

        // 多次 Wakeup 只入队一次
        let first = rx.try_recv();
        assert!(matches!(first, Ok(TerminalEvent::Wakeup)));
        assert!(rx.try_recv().is_err());

        // reset 后允许新一轮 Wakeup 入队
        proxy.reset_wakeup_pending();
        proxy.send_event(AlacTermEvent::Wakeup);
        let next = rx.try_recv();
        assert!(matches!(next, Ok(TerminalEvent::Wakeup)));
    }

    #[test]
    fn non_wakeup_events_are_not_swallowed_by_dedup() {
        let (tx, mut rx) = unbounded_channel::<TerminalEvent>();
        let proxy = GpuiEventProxy::new(tx);

        // 先压一个 Wakeup 进去拉起去重标记
        proxy.send_event(AlacTermEvent::Wakeup);
        // 期间发生 Title/Bell/Exit 等事件，不应被去重逻辑吞掉
        proxy.send_event(AlacTermEvent::Title("shell".to_string()));
        proxy.send_event(AlacTermEvent::Bell);
        proxy.send_event(AlacTermEvent::Exit);

        let mut got = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            got.push(ev);
        }
        assert_eq!(got.len(), 4);
        assert!(matches!(got[0], TerminalEvent::Wakeup));
        assert!(matches!(got[1], TerminalEvent::TitleChanged(ref t) if t == "shell"));
        assert!(matches!(got[2], TerminalEvent::Bell));
        assert!(matches!(got[3], TerminalEvent::ChildExit(0)));
    }

    #[test]
    fn text_area_size_request_uses_current_window_size() {
        let (tx, _rx) = unbounded_channel::<TerminalEvent>();
        let metrics = Arc::new(TerminalPerformanceMetrics::enabled());
        let proxy = GpuiEventProxy::with_metrics(tx, metrics);

        // 注入一个回写通道收集 reply 字节
        let captured: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let (write_tx, mut write_rx) = unbounded_channel::<Vec<u8>>();
        proxy.set_ssh_write_back(write_tx);

        proxy.set_window_size(WindowSize {
            num_lines: 40,
            num_cols: 132,
            cell_width: 9,
            cell_height: 20,
        });

        proxy.send_event(AlacTermEvent::TextAreaSizeRequest(std::sync::Arc::new(
            |size| format!("{}x{}", size.num_cols, size.num_lines),
        )));

        if let Ok(bytes) = write_rx.try_recv() {
            captured.lock().unwrap().extend_from_slice(&bytes);
        }
        let reply = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
        assert_eq!(reply, "132x40");
        assert_eq!(
            reply.len() as u64,
            proxy
                .performance_metrics()
                .snapshot()
                .terminal_response_bytes
        );
    }

    #[test]
    fn color_request_returns_named_defaults_instead_of_black() {
        let fg = default_color_for_index(NamedColor::Foreground as usize);
        let bg = default_color_for_index(NamedColor::Background as usize);
        let cursor = default_color_for_index(NamedColor::Cursor as usize);
        let other = default_color_for_index(NamedColor::Red as usize);

        assert_ne!((fg.r, fg.g, fg.b), (0, 0, 0));
        assert_ne!((bg.r, bg.g, bg.b), (0, 0, 0));
        assert_eq!((cursor.r, cursor.g, cursor.b), (0xFF, 0xFF, 0xFF));
        assert_eq!((other.r, other.g, other.b), (0, 0, 0));
    }

    #[tokio::test]
    async fn local_exec_handle_submits_visible_command_to_the_pty() {
        let (command_tx, mut command_rx) = unbounded_channel();
        let handle = build_local_terminal_exec_handle(command_tx, Arc::new(AtomicU64::new(1)));
        let task = tokio::spawn(async move {
            handle
                .exec(exec_request("pwd"), CancellationToken::new())
                .await
        });

        let output = crate::TerminalExecOutput {
            completion: crate::TerminalExecCompletion::SubmittedOnly,
            exit_code: None,
            output: String::new(),
            truncated: false,
            captured_bytes: 0,
            discarded_bytes: 0,
            duration_ms: 1,
        };
        match command_rx.recv().await {
            Some(LocalPtyCommand::StartExec {
                id,
                request,
                result,
            }) => {
                assert_eq!(1, id);
                assert_eq!("pwd", request.command);
                result.send(Ok(output.clone())).unwrap();
            }
            _ => panic!("expected local terminal exec start command"),
        }

        assert_eq!(output, task.await.unwrap().unwrap());
    }

    #[tokio::test]
    async fn local_exec_handle_honors_cancellation_before_submit() {
        let (command_tx, mut command_rx) = unbounded_channel();
        let handle = build_local_terminal_exec_handle(command_tx, Arc::new(AtomicU64::new(1)));
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let error = handle
            .exec(exec_request("pwd"), cancellation)
            .await
            .expect_err("cancelled local exec should not submit");

        assert_eq!(crate::TerminalExecError::CancelledBeforeSubmit, error);
        assert!(command_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn local_control_handle_forwards_interrupt_requests() {
        let (command_tx, mut command_rx) = unbounded_channel();
        let handle = build_local_terminal_control_handle(command_tx);
        let task = tokio::spawn(async move {
            handle
                .control(
                    crate::TerminalControlRequest {
                        action: crate::TerminalControlAction::Interrupt,
                    },
                    CancellationToken::new(),
                )
                .await
        });

        match command_rx.recv().await {
            Some(LocalPtyCommand::InterruptForeground {
                request, result, ..
            }) => {
                assert_eq!(crate::TerminalControlAction::Interrupt, request.action);
                result
                    .send(Ok(crate::TerminalControlOutput {
                        action: request.action,
                        sent: true,
                        readiness_before: crate::TerminalControlReadiness::CommandRunning,
                    }))
                    .unwrap();
            }
            _ => panic!("expected local terminal interrupt command"),
        }

        assert!(task.await.unwrap().unwrap().sent);
    }

    #[test]
    fn local_pty_osc_chunk_maps_prompt_lifecycle_events() {
        let events = terminal_events_from_osc_chunk(
            b"\x1b]133;A\x07\x1b]133;B\x07\x1b]133;C\x07\x1b]133;D;7\x07",
        );

        assert!(matches!(events.first(), Some(TerminalEvent::PromptStart)));
        assert!(matches!(events.get(1), Some(TerminalEvent::InputStart)));
        assert!(matches!(events.get(2), Some(TerminalEvent::CommandStart)));
        assert!(matches!(
            events.get(3),
            Some(TerminalEvent::CommandFinished { exit_code: 7 })
        ));
    }

    #[test]
    fn conpty_window_size_accepts_coord_expressible_dimensions() {
        let window_size = conpty_safe_window_size(TerminalSize {
            rows: 40,
            cols: 132,
            pixel_width: 1_320,
            pixel_height: 800,
        })
        .expect("valid terminal size should be accepted");

        assert_eq!(40, window_size.num_lines);
        assert_eq!(132, window_size.num_cols);
        assert_eq!(10, window_size.cell_width);
        assert_eq!(20, window_size.cell_height);
    }

    #[test]
    fn conpty_window_size_rejects_sizes_conpty_cannot_express() {
        // ConPTY 的 COORD 只能表达 i16 内的正数。本机实测表明越界/零尺寸目前仍返回
        // S_OK（不会触发 alacritty 的 assert_eq!），所以这里是防御性约束：
        // 不把平台未定义行为的尺寸提交下去，并挡掉无意义的 0 尺寸。
        for size in [
            TerminalSize {
                rows: 0,
                cols: 132,
                ..TerminalSize::default()
            },
            TerminalSize {
                rows: 40,
                cols: 0,
                ..TerminalSize::default()
            },
            TerminalSize {
                rows: i16::MAX as u16 + 1,
                cols: 132,
                ..TerminalSize::default()
            },
            TerminalSize {
                rows: 40,
                cols: i16::MAX as u16 + 1,
                ..TerminalSize::default()
            },
            TerminalSize {
                rows: u16::MAX,
                cols: u16::MAX,
                ..TerminalSize::default()
            },
        ] {
            assert!(
                conpty_safe_window_size(size).is_none(),
                "size {size:?} must not reach ResizePseudoConsole"
            );
        }
    }

    #[test]
    fn backend_stop_is_reported_unless_it_was_expected() {
        // panic 一定上报：终端已经不可恢复。
        assert!(should_report_backend_stopped(true, false));
        assert!(should_report_backend_stopped(true, true));
        // 主动关闭或子进程退出后自然结束：不再补发后端停止。
        assert!(!should_report_backend_stopped(false, true));
        // 其它提前退出（I/O 或轮询错误）：必须上报，否则界面永久卡死。
        assert!(should_report_backend_stopped(false, false));
    }

    #[test]
    fn local_pty_osc_chunk_maps_working_dir_and_recorded_command() {
        let encoded = base64::engine::general_purpose::STANDARD.encode("git status");
        let chunk = format!("\x1b]7;file://host/tmp/project\x07\x1b]1337;Command={encoded}\x07");

        let events = terminal_events_from_osc_chunk(chunk.as_bytes());

        assert!(matches!(
            events.first(),
            Some(TerminalEvent::WorkingDirChanged(reported))
                if reported.path == "/tmp/project" && reported.host.as_deref() == Some("host")
        ));
        assert!(matches!(
            events.get(1),
            Some(TerminalEvent::CommandRecorded(command)) if command == "git status"
        ));
    }
}
