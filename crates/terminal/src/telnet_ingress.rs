//! Bounded Telnet output ingress and its synchronous parser worker.
//!
//! Telnet transport I/O runs on a dedicated Tokio task. That task may retain
//! one fixed-size source buffer while waiting for queue capacity, but it does
//! not perform another socket read until the current payload has been accepted
//! by the bounded queue. Parsing happens on the queue consumer, keeping the
//! transport decoupled from the terminal lock.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Instant;

use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Term;
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

use crate::TerminalPerformanceMetrics;
use crate::ingress_queue::{
    BoundedTerminalSender, ReservedTerminalIngressItem, TerminalDataSendError,
    TerminalIngressBudget, bounded_terminal_queue,
};
use crate::pty_backend::GpuiEventProxy;
use crate::recording::RecordingTap;

pub(crate) const TELNET_PENDING_BYTES: u64 = 128 * 1024;
const TELNET_PENDING_CHUNKS: usize = 16;
const TELNET_PENDING_CONTROLS: usize = 1;
const TELNET_COMPLETION_RUNNING: u8 = 0;
const TELNET_COMPLETION_DRAINED: u8 = 1;
const TELNET_COMPLETION_ABORTED: u8 = 2;

#[derive(Clone, Debug)]
enum TelnetIngressControl {
    SourceClosed {
        on_drained: Option<UnboundedSender<()>>,
    },
}

/// Producer handed to the transport worker. `send_data` awaits bounded queue
/// capacity, so it must be driven from an async context (the Telnet worker).
#[derive(Clone)]
pub(crate) struct TelnetIngressProducer {
    sender: BoundedTerminalSender<TelnetIngressControl>,
    source_closed: Arc<AtomicBool>,
    metrics: Arc<TerminalPerformanceMetrics>,
    ingress_generation: u64,
    on_source_closed: Option<UnboundedSender<()>>,
}

pub(crate) struct TelnetParserIngress {
    sender: BoundedTerminalSender<TelnetIngressControl>,
    source_closed: Arc<AtomicBool>,
    task: Option<JoinHandle<()>>,
    metrics: Arc<TerminalPerformanceMetrics>,
    ingress_generation: u64,
    on_source_closed: Option<UnboundedSender<()>>,
    completion_state: Arc<AtomicU8>,
}

impl TelnetParserIngress {
    pub(crate) fn spawn_with_recording(
        term: Arc<FairMutex<Term<GpuiEventProxy>>>,
        event_proxy: GpuiEventProxy,
        metrics: Arc<TerminalPerformanceMetrics>,
        on_source_closed: Option<UnboundedSender<()>>,
        recording_tap: Option<RecordingTap>,
    ) -> Self {
        let budget = TerminalIngressBudget::new(
            TELNET_PENDING_BYTES,
            TELNET_PENDING_CHUNKS,
            TELNET_PENDING_CONTROLS,
        )
        .expect("static Telnet ingress budget should be valid");
        Self::spawn_with_budget_callback_and_recording(
            term,
            event_proxy,
            metrics,
            budget,
            on_source_closed,
            recording_tap,
        )
    }

    pub(crate) fn spawn_with_budget_callback_and_recording(
        term: Arc<FairMutex<Term<GpuiEventProxy>>>,
        event_proxy: GpuiEventProxy,
        metrics: Arc<TerminalPerformanceMetrics>,
        budget: TerminalIngressBudget,
        on_source_closed: Option<UnboundedSender<()>>,
        recording_tap: Option<RecordingTap>,
    ) -> Self {
        let (sender, mut receiver) = bounded_terminal_queue::<TelnetIngressControl>(budget);
        let ingress_generation = metrics.begin_ingress_backlog();
        let source_closed = Arc::new(AtomicBool::new(false));
        let completion_state = Arc::new(AtomicU8::new(TELNET_COMPLETION_RUNNING));
        let task_completion_state = completion_state.clone();
        let task_metrics = metrics.clone();
        let task = tokio::spawn(async move {
            let mut processor = Processor::<StdSyncHandler>::new();
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
                        advance_telnet_term(
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
                            ingress_generation,
                            receiver.pending_bytes(),
                            receiver.take_interval_peak_pending_bytes(),
                            receiver.peak_pending_bytes(),
                        );
                        event_proxy.queue_wakeup();
                    }
                    ReservedTerminalIngressItem::Control(TelnetIngressControl::SourceClosed {
                        on_drained: callback,
                    }) => {
                        closing = true;
                        on_drained = callback;
                    }
                }
            }
            if drained
                && task_completion_state
                    .compare_exchange(
                        TELNET_COMPLETION_RUNNING,
                        TELNET_COMPLETION_DRAINED,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    )
                    .is_ok()
            {
                if let Some(callback) = on_drained {
                    let _ = callback.send(());
                }
            }
            task_metrics.record_ingress_backlog(
                ingress_generation,
                0,
                receiver.take_interval_peak_pending_bytes(),
                receiver.peak_pending_bytes(),
            );
        });

        Self {
            sender,
            source_closed,
            task: Some(task),
            metrics,
            ingress_generation,
            on_source_closed,
            completion_state,
        }
    }

    pub(crate) fn producer(&self) -> TelnetIngressProducer {
        TelnetIngressProducer {
            sender: self.sender.clone(),
            source_closed: self.source_closed.clone(),
            metrics: self.metrics.clone(),
            ingress_generation: self.ingress_generation,
            on_source_closed: self.on_source_closed.clone(),
        }
    }

    pub(crate) fn abort(&self) {
        let _ = self.completion_state.compare_exchange(
            TELNET_COMPLETION_RUNNING,
            TELNET_COMPLETION_ABORTED,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        self.sender.abort();
    }
}

impl Drop for TelnetParserIngress {
    fn drop(&mut self) {
        self.abort();
        // Detach the Tokio task; the aborted queue makes the worker exit.
        self.task.take();
    }
}

impl TelnetIngressProducer {
    /// Await bounded output capacity while retaining ownership of the payload.
    ///
    /// The Telnet worker stores this future as its pending-ingress branch, so
    /// capacity release wakes the worker without requiring an unrelated socket,
    /// input or resize event.
    pub(crate) async fn send_data(&self, data: Vec<u8>) -> Result<(), TerminalDataSendError> {
        if self.source_closed.load(Ordering::Acquire) {
            return Err(TerminalDataSendError::Closed(data));
        }
        let result = self.sender.send_data(data).await;
        self.metrics.record_ingress_backlog(
            self.ingress_generation,
            self.sender.pending_bytes(),
            self.sender.take_interval_peak_pending_bytes(),
            self.sender.peak_pending_bytes(),
        );
        result
    }

    /// Non-blocking output path used by the transport worker.
    ///
    /// On [`TerminalDataSendError::Full`] the caller keeps the payload and
    /// retries in a later loop iteration while still servicing user input,
    /// resize, terminal responses and shutdown.
    pub(crate) fn try_send_data(&self, data: Vec<u8>) -> Result<(), TerminalDataSendError> {
        if self.source_closed.load(Ordering::Acquire) {
            return Err(TerminalDataSendError::Closed(data));
        }
        let result = self.sender.try_send_data(data);
        self.metrics.record_ingress_backlog(
            self.ingress_generation,
            self.sender.pending_bytes(),
            self.sender.take_interval_peak_pending_bytes(),
            self.sender.peak_pending_bytes(),
        );
        result
    }

    pub(crate) fn close_source(&self) {
        if self.source_closed.swap(true, Ordering::AcqRel) {
            return;
        }
        let sender = self.sender.clone();
        let on_source_closed = self.on_source_closed.clone();
        tokio::spawn(async move {
            let _ = sender
                .send_control(TelnetIngressControl::SourceClosed {
                    on_drained: on_source_closed,
                })
                .await;
        });
    }
}

pub(crate) fn advance_telnet_term(
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

    if performance_metrics.is_enabled() {
        let wait_started = Instant::now();
        let mut term = term.lock();
        let wait = wait_started.elapsed();
        let hold_started = Instant::now();
        processor.advance(&mut *term, data);
        let hold = hold_started.elapsed();
        drop(term);
        performance_metrics.record_term_lock(wait, hold);
    } else {
        processor.advance(&mut *term.lock(), data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingress_queue::{ReservedTerminalIngressItem, bounded_terminal_queue};
    use std::time::Duration;

    #[tokio::test]
    async fn pending_send_wakes_when_parser_releases_capacity() {
        let budget =
            TerminalIngressBudget::new(4, 1, 1).expect("test ingress budget should be valid");
        let (sender, mut receiver) = bounded_terminal_queue::<TelnetIngressControl>(budget);
        let metrics = Arc::new(TerminalPerformanceMetrics::disabled());
        let ingress_generation = metrics.begin_ingress_backlog();
        let producer = TelnetIngressProducer {
            sender,
            source_closed: Arc::new(AtomicBool::new(false)),
            metrics,
            ingress_generation,
            on_source_closed: None,
        };

        producer
            .try_send_data(vec![1, 2, 3, 4])
            .expect("first chunk should fill the queue");

        let mut pending = Box::pin(producer.send_data(vec![5]));
        assert!(
            tokio::time::timeout(Duration::from_millis(20), pending.as_mut())
                .await
                .is_err(),
            "pending send should wait while byte capacity is exhausted"
        );

        let item = receiver
            .recv_reserved()
            .await
            .expect("queued data should be available");
        assert!(matches!(item, ReservedTerminalIngressItem::Data(_)));
        drop(item);

        tokio::time::timeout(Duration::from_millis(200), pending.as_mut())
            .await
            .expect("capacity release should wake the pending send")
            .expect("pending send should succeed after capacity release");
    }
}
