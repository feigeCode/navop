//! Bounded Serial output ingress and its synchronous parser worker.
//!
//! Serial reads happen on a dedicated OS thread. That thread may retain one
//! fixed-size source buffer while waiting for queue capacity, but it does not
//! perform another port read until the current payload has been accepted.

use std::future::{Future, poll_fn};
use std::io::{ErrorKind, Read};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Term;
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;

use crate::TerminalPerformanceMetrics;
use crate::ingress_queue::{
    BoundedTerminalSender, ReservedTerminalIngressItem, TerminalDataSendError,
    TerminalIngressBudget, bounded_terminal_queue,
};
use crate::pty_backend::GpuiEventProxy;
use crate::recording::RecordingTap;

pub(crate) const SERIAL_READ_BUFFER_BYTES: usize = 4 * 1024;
pub(crate) const SERIAL_PENDING_BYTES: u64 = 64 * 1024;
const SERIAL_PENDING_CHUNKS: usize = 16;
const SERIAL_PENDING_CONTROLS: usize = 1;
const SERIAL_COMPLETION_RUNNING: u8 = 0;
const SERIAL_COMPLETION_DRAINED: u8 = 1;
const SERIAL_COMPLETION_ABORTED: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SerialReaderExit {
    Shutdown,
    Disconnected,
}

#[derive(Clone, Debug)]
enum SerialIngressControl {
    SourceClosed {
        on_drained: Option<UnboundedSender<()>>,
    },
}

#[derive(Clone)]
pub(crate) struct SerialIngressProducer {
    sender: BoundedTerminalSender<SerialIngressControl>,
    source_closed: Arc<AtomicBool>,
    metrics: Arc<TerminalPerformanceMetrics>,
    on_source_closed: Option<UnboundedSender<()>>,
}

pub(crate) struct SerialParserIngress {
    sender: BoundedTerminalSender<SerialIngressControl>,
    source_closed: Arc<AtomicBool>,
    task: Option<JoinHandle<()>>,
    metrics: Arc<TerminalPerformanceMetrics>,
    on_source_closed: Option<UnboundedSender<()>>,
    completion_state: Arc<AtomicU8>,
}

impl SerialParserIngress {
    pub(crate) fn spawn(
        term: Arc<FairMutex<Term<GpuiEventProxy>>>,
        event_proxy: GpuiEventProxy,
        metrics: Arc<TerminalPerformanceMetrics>,
        on_source_closed: Option<UnboundedSender<()>>,
    ) -> std::io::Result<Self> {
        Self::spawn_with_recording(term, event_proxy, metrics, on_source_closed, None)
    }

    pub(crate) fn spawn_with_recording(
        term: Arc<FairMutex<Term<GpuiEventProxy>>>,
        event_proxy: GpuiEventProxy,
        metrics: Arc<TerminalPerformanceMetrics>,
        on_source_closed: Option<UnboundedSender<()>>,
        recording_tap: Option<RecordingTap>,
    ) -> std::io::Result<Self> {
        let budget = TerminalIngressBudget::new(
            SERIAL_PENDING_BYTES,
            SERIAL_PENDING_CHUNKS,
            SERIAL_PENDING_CONTROLS,
        )
        .expect("static Serial ingress budget should be valid");
        Self::spawn_with_budget_callback_and_recording(
            term,
            event_proxy,
            metrics,
            budget,
            on_source_closed,
            recording_tap,
        )
    }

    #[cfg(test)]
    pub(crate) fn spawn_with_budget(
        term: Arc<FairMutex<Term<GpuiEventProxy>>>,
        event_proxy: GpuiEventProxy,
        metrics: Arc<TerminalPerformanceMetrics>,
        budget: TerminalIngressBudget,
    ) -> std::io::Result<Self> {
        Self::spawn_with_budget_and_callback(term, event_proxy, metrics, budget, None)
    }

    pub(crate) fn spawn_with_budget_and_callback(
        term: Arc<FairMutex<Term<GpuiEventProxy>>>,
        event_proxy: GpuiEventProxy,
        metrics: Arc<TerminalPerformanceMetrics>,
        budget: TerminalIngressBudget,
        on_source_closed: Option<UnboundedSender<()>>,
    ) -> std::io::Result<Self> {
        Self::spawn_with_budget_callback_and_recording(
            term,
            event_proxy,
            metrics,
            budget,
            on_source_closed,
            None,
        )
    }

    pub(crate) fn spawn_with_budget_callback_and_recording(
        term: Arc<FairMutex<Term<GpuiEventProxy>>>,
        event_proxy: GpuiEventProxy,
        metrics: Arc<TerminalPerformanceMetrics>,
        budget: TerminalIngressBudget,
        on_source_closed: Option<UnboundedSender<()>>,
        recording_tap: Option<RecordingTap>,
    ) -> std::io::Result<Self> {
        let (sender, mut receiver) = bounded_terminal_queue::<SerialIngressControl>(budget);
        let source_closed = Arc::new(AtomicBool::new(false));
        let completion_state = Arc::new(AtomicU8::new(SERIAL_COMPLETION_RUNNING));
        let task_completion_state = completion_state.clone();
        let task_metrics = metrics.clone();
        let task = thread::Builder::new()
            .name("serial-parser".into())
            .spawn(move || {
                let mut processor = Processor::<StdSyncHandler>::new();
                futures::executor::block_on(async move {
                    let mut closing = false;
                    let mut on_drained = None;
                    let mut drained = false;
                    loop {
                        if closing && receiver.pending_bytes() == 0 {
                            drained = true;
                            break;
                        }

                        let Some(item) = receiver.recv_reserved().await else {
                            break;
                        };
                        match item {
                            ReservedTerminalIngressItem::Data(data) => {
                                advance_serial_term(
                                    &term,
                                    &mut processor,
                                    data.as_slice(),
                                    &task_metrics,
                                    recording_tap.as_ref(),
                                );
                                // Keep the byte reservation aligned with the actual
                                // synchronous parser boundary, not merely dequeue.
                                drop(data);
                                task_metrics.record_ingress_backlog(
                                    receiver.pending_bytes(),
                                    receiver.peak_pending_bytes(),
                                );
                                event_proxy.queue_wakeup();
                            }
                            ReservedTerminalIngressItem::Control(
                                SerialIngressControl::SourceClosed {
                                    on_drained: callback,
                                },
                            ) => {
                                closing = true;
                                on_drained = callback;
                            }
                        }
                    }
                    if drained
                        && task_completion_state
                            .compare_exchange(
                                SERIAL_COMPLETION_RUNNING,
                                SERIAL_COMPLETION_DRAINED,
                                Ordering::AcqRel,
                                Ordering::Acquire,
                            )
                            .is_ok()
                    {
                        if let Some(callback) = on_drained {
                            let _ = callback.send(());
                        }
                    }
                    task_metrics.record_ingress_backlog(0, receiver.peak_pending_bytes());
                });
            })?;

        Ok(Self {
            sender,
            source_closed,
            task: Some(task),
            metrics,
            on_source_closed,
            completion_state,
        })
    }

    pub(crate) fn producer(&self) -> SerialIngressProducer {
        SerialIngressProducer {
            sender: self.sender.clone(),
            source_closed: self.source_closed.clone(),
            metrics: self.metrics.clone(),
            on_source_closed: self.on_source_closed.clone(),
        }
    }

    pub(crate) fn abort(&self) {
        let _ = self.completion_state.compare_exchange(
            SERIAL_COMPLETION_RUNNING,
            SERIAL_COMPLETION_ABORTED,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        self.sender.abort();
    }

    #[cfg(test)]
    pub(crate) fn finish(mut self) -> thread::Result<()> {
        self.producer().close_source();
        self.task
            .take()
            .expect("Serial parser task should exist")
            .join()
    }
}

impl Drop for SerialParserIngress {
    fn drop(&mut self) {
        self.abort();
        // Dropping a JoinHandle detaches the OS thread. The worker observes the
        // aborted queue and exits without making the GPUI thread wait
        // synchronously for an unbounded join.
        self.task.take();
    }
}

impl SerialIngressProducer {
    pub(crate) fn send_data(&self, data: Vec<u8>) -> Result<(), TerminalDataSendError> {
        if self.source_closed.load(Ordering::Acquire) {
            return Err(TerminalDataSendError::Closed(data));
        }

        let mut send = Box::pin(self.sender.send_data(data));
        let result = futures::executor::block_on(poll_fn(|cx| {
            let result = Future::poll(send.as_mut(), cx);
            self.metrics.record_ingress_backlog(
                self.sender.pending_bytes(),
                self.sender.peak_pending_bytes(),
            );
            result
        }));
        self.metrics.record_ingress_backlog(
            self.sender.pending_bytes(),
            self.sender.peak_pending_bytes(),
        );
        result
    }

    pub(crate) fn close_source(&self) {
        if self.source_closed.swap(true, Ordering::AcqRel) {
            return;
        }
        let _ = futures::executor::block_on(self.sender.send_control(
            SerialIngressControl::SourceClosed {
                on_drained: self.on_source_closed.clone(),
            },
        ));
    }

    #[cfg(test)]
    pub(crate) fn pending_bytes(&self) -> usize {
        self.sender.pending_bytes()
    }
}

pub(crate) fn run_serial_reader(
    reader: &mut (impl Read + ?Sized),
    producer: &SerialIngressProducer,
    shutdown: &CancellationToken,
) -> SerialReaderExit {
    let mut buffer = [0; SERIAL_READ_BUFFER_BYTES];
    let exit = loop {
        if shutdown.is_cancelled() {
            break SerialReaderExit::Shutdown;
        }

        match reader.read(&mut buffer) {
            Ok(size) if size > 0 => {
                if shutdown.is_cancelled() {
                    break SerialReaderExit::Shutdown;
                }
                if let Err(error) = producer.send_data(buffer[..size].to_vec()) {
                    if !shutdown.is_cancelled() {
                        tracing::warn!(
                            error = %error,
                            "Serial terminal ingress rejected or closed"
                        );
                    }
                    break if shutdown.is_cancelled() {
                        SerialReaderExit::Shutdown
                    } else {
                        SerialReaderExit::Disconnected
                    };
                }
            }
            Ok(_) => thread::yield_now(),
            Err(error) if error.kind() == ErrorKind::TimedOut => {}
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(_) if shutdown.is_cancelled() => break SerialReaderExit::Shutdown,
            Err(_) => break SerialReaderExit::Disconnected,
        }
    };

    producer.close_source();
    exit
}

pub(crate) fn advance_serial_term(
    term: &Arc<FairMutex<Term<GpuiEventProxy>>>,
    processor: &mut Processor<StdSyncHandler>,
    data: &[u8],
    performance_metrics: &TerminalPerformanceMetrics,
    recording_tap: Option<&RecordingTap>,
) {
    if let Some(recording_tap) = recording_tap {
        let _ = recording_tap.record_output(data);
    }
    performance_metrics.record_parser_chunk(data.len());

    let wait_started = Instant::now();
    let mut term = term.lock();
    let wait = wait_started.elapsed();
    let hold_started = Instant::now();
    processor.advance(&mut *term, data);
    let hold = hold_started.elapsed();
    drop(term);

    performance_metrics.record_term_lock(wait, hold);
}
