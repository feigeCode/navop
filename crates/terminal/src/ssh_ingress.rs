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

/// Maximum SSH output bytes queued at once for the parser worker.
///
/// A decoded source chunk can be larger than the transport chunk because
/// transcoding to UTF-8 may expand it. `SshParserIngress::pending` keeps that
/// source chunk as the actor's single pending item and splits only the queue
/// writes so the byte budget remains bounded.
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
    max_chunk_bytes: usize,
    metrics: Arc<TerminalPerformanceMetrics>,
    ingress_generation: u64,
    task: JoinHandle<()>,
}

impl SshParserIngress {
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

    #[cfg(test)]
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
        let max_chunk_bytes = budget.max_pending_bytes();
        let (sender, mut receiver) = bounded_terminal_queue::<()>(budget);
        let ingress_generation = metrics.begin_ingress_backlog();
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
                    ingress_generation,
                    receiver.pending_bytes(),
                    receiver.take_interval_peak_pending_bytes(),
                    receiver.peak_pending_bytes(),
                );
                event_proxy.queue_wakeup();
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
            max_chunk_bytes,
            metrics,
            ingress_generation,
            task,
        }
    }

    #[cfg(test)]
    pub(crate) fn sender(&self) -> BoundedTerminalSender<()> {
        self.sender.clone()
    }

    pub(crate) fn pending(&self, data: Vec<u8>) -> SshPendingIngress {
        let sender = self.sender.clone();
        let max_chunk_bytes = self.max_chunk_bytes;
        let metrics = self.metrics.clone();
        let ingress_generation = self.ingress_generation;
        SshPendingIngress::from_future(async move {
            // Keep metrics current even when the send has reserved bytes but
            // is waiting for a bounded chunk slot.
            let mut send = Box::pin(async {
                if data.len() <= max_chunk_bytes {
                    return sender.send_data(data).await;
                }
                for chunk in data.chunks(max_chunk_bytes) {
                    sender.send_data(chunk.to_vec()).await?;
                }
                Ok(())
            });
            let result = poll_fn(|cx| {
                let result = std::future::Future::poll(send.as_mut(), cx);
                metrics.record_ingress_backlog(
                    ingress_generation,
                    sender.pending_bytes(),
                    sender.take_interval_peak_pending_bytes(),
                    sender.peak_pending_bytes(),
                );
                result
            })
            .await;
            metrics.record_ingress_backlog(
                ingress_generation,
                sender.pending_bytes(),
                sender.take_interval_peak_pending_bytes(),
                sender.peak_pending_bytes(),
            );
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

    pub(crate) async fn wait(&mut self) -> Result<(), TerminalDataSendError> {
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
    if metrics.is_enabled() {
        let wait_started = Instant::now();
        let mut term = term.lock();
        let wait = wait_started.elapsed();
        let hold_started = Instant::now();
        processor.advance(&mut *term, data);
        let hold = hold_started.elapsed();
        drop(term);
        metrics.record_term_lock(wait, hold);
    } else {
        processor.advance(&mut *term.lock(), data);
    }
}
