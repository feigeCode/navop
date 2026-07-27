//! Bounded SSH output ingress and the parser worker that consumes it.
//!
//! The SSH actor deliberately keeps at most one source chunk outside the
//! bounded queue.  Once that chunk is waiting for queue capacity, transport
//! reads are disabled while actor commands and terminal responses remain
//! selectable.

use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Term;
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
use ssh::{ChannelEvent, SshChannel};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::{JoinError, JoinHandle};

use crate::TerminalPerformanceMetrics;
use crate::ingress_queue::{
    BoundedTerminalSender, ReservedTerminalIngressItem, TerminalDataSendError,
    TerminalIngressBudget, bounded_terminal_queue,
};
use crate::pty_backend::GpuiEventProxy;
use crate::recording::RecordingTap;

/// Maximum accepted SSH output bytes waiting for the parser worker.
///
/// The SSH channel adapter is expected to deliver source chunks no larger than
/// this limit. An oversized source chunk is rejected instead of copied or
/// split in the actor, so the one-source-chunk memory allowance remains
/// explicit and bounded by the transport contract.
pub(crate) const SSH_PENDING_BYTES: u64 = 512 * 1024;
const SSH_PENDING_CHUNKS: usize = 16;
const SSH_PENDING_CONTROLS: usize = 8;

type PendingSend =
    Pin<Box<dyn Future<Output = Result<(), TerminalDataSendError>> + Send + 'static>>;

/// A single source chunk whose enqueue operation may still be waiting for
/// bounded ingress capacity.
pub(crate) struct SshPendingIngress {
    send: PendingSend,
}

/// Queue and parser task owned by one SSH connection.
pub(crate) struct SshParserIngress {
    sender: BoundedTerminalSender<()>,
    metrics: Arc<TerminalPerformanceMetrics>,
    task: JoinHandle<()>,
}

impl SshParserIngress {
    pub(crate) fn spawn(
        term: Arc<FairMutex<Term<GpuiEventProxy>>>,
        event_proxy: GpuiEventProxy,
        metrics: Arc<TerminalPerformanceMetrics>,
    ) -> Self {
        Self::spawn_with_recording(term, event_proxy, metrics, None)
    }

    pub(crate) fn spawn_with_recording(
        term: Arc<FairMutex<Term<GpuiEventProxy>>>,
        event_proxy: GpuiEventProxy,
        metrics: Arc<TerminalPerformanceMetrics>,
        recording_tap: Option<RecordingTap>,
    ) -> Self {
        let budget =
            TerminalIngressBudget::new(SSH_PENDING_BYTES, SSH_PENDING_CHUNKS, SSH_PENDING_CONTROLS)
                .expect("static SSH ingress budget should be valid");
        Self::spawn_with_budget_and_recording(term, event_proxy, metrics, budget, recording_tap)
    }

    pub(crate) fn spawn_with_budget(
        term: Arc<FairMutex<Term<GpuiEventProxy>>>,
        event_proxy: GpuiEventProxy,
        metrics: Arc<TerminalPerformanceMetrics>,
        budget: TerminalIngressBudget,
    ) -> Self {
        Self::spawn_with_budget_and_recording(term, event_proxy, metrics, budget, None)
    }

    pub(crate) fn spawn_with_budget_and_recording(
        term: Arc<FairMutex<Term<GpuiEventProxy>>>,
        event_proxy: GpuiEventProxy,
        metrics: Arc<TerminalPerformanceMetrics>,
        budget: TerminalIngressBudget,
        recording_tap: Option<RecordingTap>,
    ) -> Self {
        let (sender, mut receiver) = bounded_terminal_queue::<()>(budget);
        let task_metrics = metrics.clone();
        let task = tokio::spawn(async move {
            let mut processor = Processor::<StdSyncHandler>::new();
            while let Some(item) = receiver.recv_reserved().await {
                let ReservedTerminalIngressItem::Data(data) = item else {
                    continue;
                };
                advance_terminal(
                    &term,
                    &mut processor,
                    data.as_slice(),
                    &task_metrics,
                    recording_tap.as_ref(),
                );
                // Keep the byte reservation until the synchronous parser
                // consumption boundary has completed.
                drop(data);
                task_metrics.record_ingress_backlog(
                    receiver.pending_bytes(),
                    receiver.peak_pending_bytes(),
                );
                event_proxy.queue_wakeup();
            }
            task_metrics.record_ingress_backlog(0, receiver.peak_pending_bytes());
        });
        Self {
            sender,
            metrics,
            task,
        }
    }

    #[cfg(test)]
    pub(crate) fn sender(&self) -> BoundedTerminalSender<()> {
        self.sender.clone()
    }

    pub(crate) fn pending(&self, data: Vec<u8>) -> SshPendingIngress {
        let sender = self.sender.clone();
        let metrics = self.metrics.clone();
        SshPendingIngress::from_future(async move {
            // Keep metrics current even when the send has reserved bytes but
            // is waiting for a bounded chunk slot.
            let mut send = Box::pin(sender.send_data(data));
            let result = poll_fn(|cx| {
                let result = std::future::Future::poll(send.as_mut(), cx);
                metrics.record_ingress_backlog(sender.pending_bytes(), sender.peak_pending_bytes());
                result
            })
            .await;
            metrics.record_ingress_backlog(sender.pending_bytes(), sender.peak_pending_bytes());
            result
        })
    }

    pub(crate) fn abort(&self) {
        self.sender.abort();
    }

    pub(crate) async fn finish(self) -> Result<(), JoinError> {
        let Self { sender, task, .. } = self;
        drop(sender);
        task.await
    }
}

impl SshPendingIngress {
    #[cfg(test)]
    pub(crate) fn new(sender: BoundedTerminalSender<()>, data: Vec<u8>) -> Self {
        Self::from_future(async move { sender.send_data(data).await })
    }

    fn from_future(
        send: impl Future<Output = Result<(), TerminalDataSendError>> + Send + 'static,
    ) -> Self {
        Self {
            send: Box::pin(send),
        }
    }

    async fn wait(&mut self) -> Result<(), TerminalDataSendError> {
        self.send.as_mut().await
    }
}

/// Inputs selected by the SSH actor's fair, bounded scheduling gate.
pub(crate) enum SshActorInput<Command> {
    Command(Command),
    TerminalResponse(Vec<u8>),
    Ingress(Result<(), TerminalDataSendError>),
    Channel(Option<ChannelEvent>),
}

/// Select the next actor input.
///
/// Commands and terminal responses always bypass a full ingress queue.
/// Transport reads are disabled while one source chunk is pending enqueue.
pub(crate) async fn next_ssh_actor_input<C, Command>(
    channel: &mut C,
    command_rx: &mut UnboundedReceiver<Command>,
    response_rx: &mut UnboundedReceiver<Vec<u8>>,
    pending: &mut Option<SshPendingIngress>,
) -> SshActorInput<Command>
where
    C: SshChannel,
{
    let has_pending = pending.is_some();
    tokio::select! {
        biased;
        Some(command) = command_rx.recv() => SshActorInput::Command(command),
        Some(data) = response_rx.recv() => SshActorInput::TerminalResponse(data),
        result = wait_for_pending(pending), if has_pending => {
            pending.take();
            SshActorInput::Ingress(result)
        }
        event = channel.recv(), if !has_pending => SshActorInput::Channel(event),
    }
}

async fn wait_for_pending(
    pending: &mut Option<SshPendingIngress>,
) -> Result<(), TerminalDataSendError> {
    pending
        .as_mut()
        .expect("pending ingress should exist when polled")
        .wait()
        .await
}

fn advance_terminal(
    term: &Arc<FairMutex<Term<GpuiEventProxy>>>,
    processor: &mut Processor<StdSyncHandler>,
    data: &[u8],
    metrics: &TerminalPerformanceMetrics,
    recording_tap: Option<&RecordingTap>,
) {
    if let Some(recording_tap) = recording_tap {
        let _ = recording_tap.record_output(data);
    }
    metrics.record_parser_chunk(data.len());
    let wait_started = Instant::now();
    let mut term = term.lock();
    let wait = wait_started.elapsed();
    let hold_started = Instant::now();
    processor.advance(&mut *term, data);
    let hold = hold_started.elapsed();
    drop(term);
    metrics.record_term_lock(wait, hold);
}
