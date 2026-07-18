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
use crate::osc::{OscEvent, extract_osc_events};
use crate::{
    TerminalBackend, TerminalControlError, TerminalControlHandle, TerminalControlOutput,
    TerminalControlRequest, TerminalExecError, TerminalExecHandle, TerminalExecOutput,
    TerminalExecRequest, TerminalInputHandle, TerminalSize,
};

/// 终端事件类型
#[derive(Debug, Clone)]
pub enum TerminalEvent {
    /// 终端内容已更新，需要重新渲染
    Wakeup,
    /// SSH keyboard-interactive/MFA 请求状态变化
    SshMfaChanged,
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
    /// 终端程序请求存储到剪贴板
    ClipboardStore(ClipboardType, String),
    /// 终端程序请求从剪贴板加载
    ClipboardLoad(ClipboardType),
    /// 远程工作目录变更（OSC 7）
    WorkingDirChanged(String),
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
    TerminalChunk(Vec<u8>),
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
        OscEvent::WorkingDirChanged(path) => TerminalEvent::WorkingDirChanged(path),
        OscEvent::CommandRecorded(command) => TerminalEvent::CommandRecorded(command),
    }
}

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
    ) -> Self {
        let mut inner = Box::new(inner);
        let reader = OscTrackingReader {
            inner: inner.as_mut() as *mut T,
            event_tx,
            command_tx,
            capture_output,
        };
        Self { inner, reader }
    }
}

impl<T: EventedPty> Read for OscTrackingReader<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let bytes_read = unsafe { (&mut *self.inner).reader().read(buf) }?;
        if bytes_read > 0 {
            let data = &buf[..bytes_read];
            let terminal_events = terminal_events_from_osc_chunk(data);
            if self.capture_output.load(Ordering::Acquire) || !terminal_events.is_empty() {
                let _ = self
                    .command_tx
                    .send(LocalPtyCommand::TerminalChunk(data.to_vec()));
            }
            for event in terminal_events {
                let _ = self.event_tx.send(event);
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
    _event_loop_handle: JoinHandle<()>,
    _supervisor_handle: JoinHandle<()>,
}

impl LocalPtyBackend {
    pub fn new(
        term: Arc<FairMutex<Term<GpuiEventProxy>>>,
        event_proxy: GpuiEventProxy,
        pty_options: PtyOptions,
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
        let pty = tty::new(&pty_options, window_size, 0)?;
        let pty = OscTrackingPty::new(
            pty,
            event_proxy.event_tx.clone(),
            command_tx.clone(),
            capture_output.clone(),
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
            );
        });
        let event_loop_handle = thread::spawn(move || {
            let _ = event_loop.spawn().join();
        });

        Ok(Self {
            event_loop_sender,
            command_tx,
            exec_ids: Arc::new(AtomicU64::new(1)),
            event_proxy,
            _event_loop_handle: event_loop_handle,
            _supervisor_handle: supervisor_handle,
        })
    }

    pub fn write(&self, data: Vec<u8>) {
        let _ = self.command_tx.send(LocalPtyCommand::Write {
            source: TerminalInputSource::User,
            data,
        });
    }

    pub fn resize(&self, size: TerminalSize) {
        let window_size = WindowSize {
            num_lines: size.rows,
            num_cols: size.cols,
            cell_width: if size.cols > 0 {
                size.pixel_width / size.cols
            } else {
                8
            },
            cell_height: if size.rows > 0 {
                size.pixel_height / size.rows
            } else {
                18
            },
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
        let _ = self.command_tx.send(LocalPtyCommand::Shutdown);
    }
}

impl TerminalBackend for LocalPtyBackend {
    fn write(&self, data: Vec<u8>) {
        LocalPtyBackend::write(self, data);
    }

    fn input_handle(&self) -> Option<TerminalInputHandle> {
        let sender = self.command_tx.clone();
        Some(TerminalInputHandle::new(move |data| {
            let _ = sender.send(LocalPtyCommand::Write {
                source: TerminalInputSource::User,
                data,
            });
        }))
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
) {
    let mut supervisor = ExecSupervisor::new();
    let mut exec_results = HashMap::<u64, ExecResultSender>::new();
    while let Some(command) = command_rx.blocking_recv() {
        let keep_running = match command {
            LocalPtyCommand::Write { source, data } => {
                apply_local_exec_effects(
                    supervisor.on_input(source, &data),
                    &event_loop_sender,
                    &command_tx,
                    &mut exec_results,
                ) && event_loop_sender.send(Msg::Input(Cow::Owned(data))).is_ok()
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
            LocalPtyCommand::TerminalChunk(data) => {
                let events = extract_osc_events(&data);
                apply_local_exec_effects(
                    supervisor.on_terminal_chunk(&data, &events),
                    &event_loop_sender,
                    &command_tx,
                    &mut exec_results,
                )
            }
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
#[derive(Clone)]
pub struct GpuiEventProxy {
    event_tx: UnboundedSender<TerminalEvent>,
    /// PtyWrite 回写通道（在后端创建后设置）
    write_back: Arc<Mutex<Option<PtyWriteBack>>>,
    /// 共享窗口尺寸，供 TextAreaSizeRequest 真实回复使用
    window_size: Arc<Mutex<WindowSize>>,
    /// Wakeup 去重标记：true 表示已有未消费的 Wakeup 在事件队列里
    wakeup_pending: Arc<AtomicBool>,
}

impl GpuiEventProxy {
    pub fn new(event_tx: UnboundedSender<TerminalEvent>) -> Self {
        Self {
            event_tx,
            write_back: Arc::new(Mutex::new(None)),
            window_size: Arc::new(Mutex::new(WindowSize {
                num_lines: 24,
                num_cols: 80,
                cell_width: 8,
                cell_height: 18,
            })),
            wakeup_pending: Arc::new(AtomicBool::new(false)),
        }
    }

    /// 设置回写通道
    fn set_write_back(&self, wb: PtyWriteBack) {
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
    pub fn reset_wakeup_pending(&self) {
        self.wakeup_pending.store(false, Ordering::Release);
    }

    /// 返回 Wakeup 去重标记的句柄，便于事件聚合任务在转发 Wakeup 后立即 reset，
    /// 让下一次 PTY 输出能继续触发 Wakeup
    pub fn wakeup_pending_handle(&self) -> Arc<AtomicBool> {
        self.wakeup_pending.clone()
    }

    fn current_window_size(&self) -> WindowSize {
        *self.window_size.lock().unwrap()
    }

    fn write_back(&self, data: Vec<u8>) {
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

impl EventListener for GpuiEventProxy {
    fn send_event(&self, event: AlacTermEvent) {
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
                // 去重：已有未消费 Wakeup 时直接丢弃，避免高速输出下事件堆积
                if self.wakeup_pending.swap(true, Ordering::AcqRel) {
                    return;
                }
                TerminalEvent::Wakeup
            }
            AlacTermEvent::Title(title) => TerminalEvent::TitleChanged(title),
            AlacTermEvent::Bell => TerminalEvent::Bell,
            AlacTermEvent::ClipboardStore(ty, data) => TerminalEvent::ClipboardStore(ty, data),
            AlacTermEvent::ClipboardLoad(ty, _) => TerminalEvent::ClipboardLoad(ty),
            AlacTermEvent::Exit => {
                self.disconnect_local_backend();
                TerminalEvent::ChildExit(0)
            }
            _ => return,
        };
        let _ = self.event_tx.send(terminal_event);
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
    use base64::Engine;
    use std::sync::Arc;
    use tokio::sync::mpsc::unbounded_channel;
    use tokio_util::sync::CancellationToken;

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
        let proxy = GpuiEventProxy::new(tx);

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
    fn local_pty_osc_chunk_maps_working_dir_and_recorded_command() {
        let encoded = base64::engine::general_purpose::STANDARD.encode("git status");
        let chunk = format!("\x1b]7;file://host/tmp/project\x07\x1b]1337;Command={encoded}\x07");

        let events = terminal_events_from_osc_chunk(chunk.as_bytes());

        assert!(matches!(
            events.first(),
            Some(TerminalEvent::WorkingDirChanged(path)) if path == "/tmp/project"
        ));
        assert!(matches!(
            events.get(1),
            Some(TerminalEvent::CommandRecorded(command)) if command == "git status"
        ));
    }
}
