use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config as TermConfig, Term};
use anyhow::Result;
use async_trait::async_trait;
use ssh::{ChannelEvent, PtyConfig, SshChannel};
use tokio::sync::mpsc::unbounded_channel;
use tokio::time::timeout;

use crate::TerminalPerformanceMetrics;
use crate::ingress_queue::{
    TerminalDataSendError, TerminalIngressBudget, TerminalIngressItem, bounded_terminal_queue,
};
use crate::pty_backend::{GpuiEventProxy, TerminalEvent};
use crate::ssh_ingress::{
    SSH_PENDING_BYTES, SshActorInput, SshParserIngress, SshPendingIngress, next_ssh_actor_input,
};

struct TestDimensions;

impl Dimensions for TestDimensions {
    fn total_lines(&self) -> usize {
        24
    }

    fn screen_lines(&self) -> usize {
        24
    }

    fn columns(&self) -> usize {
        80
    }
}

struct CountingChannel {
    recv_count: Arc<AtomicUsize>,
    events: VecDeque<ChannelEvent>,
}

impl CountingChannel {
    fn new(events: impl IntoIterator<Item = ChannelEvent>) -> (Self, Arc<AtomicUsize>) {
        let recv_count = Arc::new(AtomicUsize::new(0));
        (
            Self {
                recv_count: recv_count.clone(),
                events: events.into_iter().collect(),
            },
            recv_count,
        )
    }
}

#[async_trait]
impl SshChannel for CountingChannel {
    async fn request_pty(&mut self, _config: &PtyConfig) -> Result<()> {
        Ok(())
    }

    async fn exec(&mut self, _command: &str) -> Result<()> {
        Ok(())
    }

    async fn request_shell(&mut self) -> Result<()> {
        Ok(())
    }

    async fn set_env(&mut self, _name: &str, _value: &str) -> Result<()> {
        Ok(())
    }

    async fn send_data(&mut self, _data: &[u8]) -> Result<()> {
        Ok(())
    }

    async fn resize_pty(&mut self, _width: u32, _height: u32) -> Result<()> {
        Ok(())
    }

    async fn recv(&mut self) -> Option<ChannelEvent> {
        self.recv_count.fetch_add(1, Ordering::SeqCst);
        self.events.pop_front()
    }

    async fn eof(&mut self) -> Result<()> {
        Ok(())
    }

    async fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

fn one_byte_budget() -> TerminalIngressBudget {
    TerminalIngressBudget::new(1, 1, 1).expect("valid test budget")
}

#[tokio::test]
async fn full_ingress_keeps_actor_commands_responsive_and_pauses_transport_reads() {
    let (sender, mut receiver) = bounded_terminal_queue::<()>(one_byte_budget());
    sender.send_data(vec![1]).await.expect("fill queue");
    let mut pending = Some(SshPendingIngress::new(sender.clone(), vec![2]));
    let (mut channel, recv_count) = CountingChannel::new([ChannelEvent::Data(vec![3])]);
    let (command_tx, mut command_rx) = unbounded_channel();
    let (_response_tx, mut response_rx) = unbounded_channel();
    command_tx.send("control").expect("queue actor command");

    let input = next_ssh_actor_input(
        &mut channel,
        &mut command_rx,
        &mut response_rx,
        &mut pending,
    )
    .await;

    assert!(matches!(input, SshActorInput::Command("control")));
    assert_eq!(recv_count.load(Ordering::SeqCst), 0);
    assert_eq!(sender.pending_bytes(), 1);
    assert!(pending.is_some());

    assert_eq!(
        receiver.recv().await,
        Some(TerminalIngressItem::Data(vec![1]))
    );
    let input = next_ssh_actor_input(
        &mut channel,
        &mut command_rx,
        &mut response_rx,
        &mut pending,
    )
    .await;
    assert!(matches!(input, SshActorInput::Ingress(Ok(()))));
    assert!(pending.is_none());
    assert_eq!(recv_count.load(Ordering::SeqCst), 0);

    let input = next_ssh_actor_input(
        &mut channel,
        &mut command_rx,
        &mut response_rx,
        &mut pending,
    )
    .await;
    assert!(
        matches!(input, SshActorInput::Channel(Some(ChannelEvent::Data(data))) if data == vec![3])
    );
    assert_eq!(recv_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn full_ingress_keeps_terminal_responses_responsive_and_pauses_transport_reads() {
    let (sender, mut receiver) = bounded_terminal_queue::<()>(one_byte_budget());
    sender.send_data(vec![1]).await.expect("fill queue");
    let mut pending = Some(SshPendingIngress::new(sender.clone(), vec![2]));
    let (mut channel, recv_count) = CountingChannel::new([ChannelEvent::Data(vec![3])]);
    let (_command_tx, mut command_rx) = unbounded_channel::<()>();
    let (response_tx, mut response_rx) = unbounded_channel();
    response_tx
        .send(b"terminal response".to_vec())
        .expect("queue terminal response");

    let input = next_ssh_actor_input(
        &mut channel,
        &mut command_rx,
        &mut response_rx,
        &mut pending,
    )
    .await;

    assert!(matches!(input, SshActorInput::TerminalResponse(data) if data == b"terminal response"));
    assert_eq!(recv_count.load(Ordering::SeqCst), 0);
    assert_eq!(sender.pending_bytes(), 1);
    assert!(pending.is_some());

    assert_eq!(
        receiver.recv().await,
        Some(TerminalIngressItem::Data(vec![1]))
    );
    let input = next_ssh_actor_input(
        &mut channel,
        &mut command_rx,
        &mut response_rx,
        &mut pending,
    )
    .await;
    assert!(matches!(input, SshActorInput::Ingress(Ok(()))));
    assert!(pending.is_none());
    assert_eq!(recv_count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn abort_releases_an_actor_send_waiting_on_full_ingress() {
    let (sender, mut receiver) = bounded_terminal_queue::<()>(one_byte_budget());
    sender.send_data(vec![1]).await.expect("fill queue");
    let mut pending = Some(SshPendingIngress::new(sender.clone(), vec![2]));
    let (mut channel, recv_count) = CountingChannel::new([ChannelEvent::Data(vec![3])]);
    let (_command_tx, mut command_rx) = unbounded_channel::<()>();
    let (_response_tx, mut response_rx) = unbounded_channel();

    sender.abort();
    let input = timeout(
        Duration::from_millis(100),
        next_ssh_actor_input(
            &mut channel,
            &mut command_rx,
            &mut response_rx,
            &mut pending,
        ),
    )
    .await
    .expect("abort should wake blocked ingress");

    assert!(matches!(
        input,
        SshActorInput::Ingress(Err(TerminalDataSendError::Closed(data))) if data == vec![2]
    ));
    assert!(pending.is_none());
    assert_eq!(recv_count.load(Ordering::SeqCst), 0);
    assert_eq!(receiver.recv().await, None);
    assert_eq!(sender.pending_bytes(), 0);
}

#[tokio::test]
async fn oversized_source_chunk_is_rejected_without_queueing() {
    const SECRET: &[u8] = b"ssh-oversized-secret";

    let budget =
        TerminalIngressBudget::new(SSH_PENDING_BYTES, 1, 1).expect("valid SSH ingress budget");
    let (sender, _receiver) = bounded_terminal_queue::<()>(budget);
    let mut oversized = vec![0; SSH_PENDING_BYTES as usize + 1];
    oversized[..SECRET.len()].copy_from_slice(SECRET);
    let mut pending = Some(SshPendingIngress::new(sender.clone(), oversized));
    let (mut channel, recv_count) = CountingChannel::new([ChannelEvent::Data(vec![3])]);
    let (_command_tx, mut command_rx) = unbounded_channel::<()>();
    let (_response_tx, mut response_rx) = unbounded_channel();

    let input = timeout(
        Duration::from_millis(100),
        next_ssh_actor_input(
            &mut channel,
            &mut command_rx,
            &mut response_rx,
            &mut pending,
        ),
    )
    .await
    .expect("oversized source chunk must fail immediately");

    let error = match input {
        SshActorInput::Ingress(Err(error)) => error,
        _ => panic!("oversized source chunk should be rejected by the ingress queue"),
    };
    let error_debug = format!("{error:?}");
    let error_display = error.to_string();
    assert!(matches!(
        error,
        TerminalDataSendError::Oversized { ref data, max_bytes }
            if data.len() == SSH_PENDING_BYTES as usize + 1
                && max_bytes == SSH_PENDING_BYTES as usize
    ));
    assert!(pending.is_none());
    assert_eq!(sender.pending_bytes(), 0);
    assert_eq!(sender.peak_pending_bytes(), 0);
    assert_eq!(recv_count.load(Ordering::SeqCst), 0);
    assert!(
        !error_debug.contains("ssh-oversized-secret")
            && !error_display.contains("ssh-oversized-secret"),
        "ingress errors must not expose payload contents"
    );
}

#[tokio::test]
async fn sustained_transport_holds_only_one_source_chunk_and_never_exceeds_the_budget() {
    const BUDGET_BYTES: usize = 4;
    const SOURCE_CHUNKS: usize = 32;

    let budget =
        TerminalIngressBudget::new(BUDGET_BYTES as u64, 1, 1).expect("valid bounded test budget");
    let (sender, mut receiver) = bounded_terminal_queue::<()>(budget);
    sender
        .send_data(vec![0; BUDGET_BYTES])
        .await
        .expect("fill queue");
    let events =
        (0..SOURCE_CHUNKS).map(|index| ChannelEvent::Data(vec![(index + 1) as u8; BUDGET_BYTES]));
    let (mut channel, recv_count) = CountingChannel::new(events);
    let (_command_tx, mut command_rx) = unbounded_channel::<()>();
    let (_response_tx, mut response_rx) = unbounded_channel();
    let mut pending = None;

    for index in 0..SOURCE_CHUNKS {
        let input = next_ssh_actor_input(
            &mut channel,
            &mut command_rx,
            &mut response_rx,
            &mut pending,
        )
        .await;
        let data = match input {
            SshActorInput::Channel(Some(ChannelEvent::Data(data))) => data,
            _ => panic!("expected source chunk {index}"),
        };
        assert_eq!(data, vec![(index + 1) as u8; BUDGET_BYTES]);
        assert_eq!(recv_count.load(Ordering::SeqCst), index + 1);
        pending = Some(SshPendingIngress::new(sender.clone(), data));

        assert!(
            timeout(
                Duration::from_millis(10),
                next_ssh_actor_input(
                    &mut channel,
                    &mut command_rx,
                    &mut response_rx,
                    &mut pending,
                ),
            )
            .await
            .is_err(),
            "a full ingress queue must pause source reads"
        );
        assert!(pending.is_some(), "exactly one source chunk is held");
        assert_eq!(recv_count.load(Ordering::SeqCst), index + 1);
        assert!(sender.pending_bytes() <= BUDGET_BYTES);
        assert!(sender.peak_pending_bytes() <= BUDGET_BYTES);

        let accepted = receiver.recv().await.expect("drain accepted queue item");
        assert!(matches!(accepted, TerminalIngressItem::Data(_)));
        let input = timeout(
            Duration::from_millis(100),
            next_ssh_actor_input(
                &mut channel,
                &mut command_rx,
                &mut response_rx,
                &mut pending,
            ),
        )
        .await
        .expect("released queue must accept held source chunk");
        assert!(matches!(input, SshActorInput::Ingress(Ok(()))));
        assert!(pending.is_none());
        assert_eq!(recv_count.load(Ordering::SeqCst), index + 1);
        assert!(sender.pending_bytes() <= BUDGET_BYTES);
        assert!(sender.peak_pending_bytes() <= BUDGET_BYTES);
    }

    assert_eq!(recv_count.load(Ordering::SeqCst), SOURCE_CHUNKS);
    assert!(matches!(
        receiver.recv().await,
        Some(TerminalIngressItem::Data(_))
    ));
    assert_eq!(sender.pending_bytes(), 0);
    assert_eq!(sender.peak_pending_bytes(), BUDGET_BYTES);
}

#[tokio::test]
async fn parser_worker_preserves_order_drains_and_coalesces_wakeups() {
    let (event_tx, mut event_rx) = unbounded_channel();
    let metrics = Arc::new(TerminalPerformanceMetrics::default());
    let proxy = GpuiEventProxy::with_metrics(event_tx, metrics.clone());
    let term = Arc::new(FairMutex::new(Term::new(
        TermConfig::default(),
        &TestDimensions,
        proxy.clone(),
    )));
    let ingress = SshParserIngress::spawn_with_budget(
        term.clone(),
        proxy,
        metrics.clone(),
        TerminalIngressBudget::new(4, 2, 1).expect("valid worker budget"),
    );
    let sender = ingress.sender();
    sender
        .send_data(b"ab".to_vec())
        .await
        .expect("queue first chunk");
    sender
        .send_data(b"c".to_vec())
        .await
        .expect("queue second chunk");
    drop(sender);

    timeout(Duration::from_secs(1), ingress.finish())
        .await
        .expect("worker should gracefully drain")
        .expect("worker should not panic");

    let term = term.lock();
    assert_eq!(term.grid()[Line(0)][Column(0)].c, 'a');
    assert_eq!(term.grid()[Line(0)][Column(1)].c, 'b');
    assert_eq!(term.grid()[Line(0)][Column(2)].c, 'c');
    drop(term);
    assert!(matches!(event_rx.try_recv(), Ok(TerminalEvent::Wakeup)));
    assert!(event_rx.try_recv().is_err());
    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.ingress_bytes, 3);
    assert_eq!(snapshot.parser_chunks, 2);
    assert_eq!(snapshot.parser_chunk_bytes, 3);
    assert_eq!(snapshot.term_lock_samples, 2);
    assert_eq!(snapshot.ingress_pending_bytes, 0);
    assert!((2..=4).contains(&snapshot.ingress_pending_bytes_max));
    assert_eq!(snapshot.wakeup_requests, 2);
    assert_eq!(snapshot.wakeup_queued, 1);
    assert_eq!(snapshot.wakeup_coalesced, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parser_worker_holds_byte_reservation_through_term_lock_and_parser_consumption() {
    let (event_tx, _event_rx) = unbounded_channel();
    let metrics = Arc::new(TerminalPerformanceMetrics::default());
    let proxy = GpuiEventProxy::with_metrics(event_tx, metrics.clone());
    let term = Arc::new(FairMutex::new(Term::new(
        TermConfig::default(),
        &TestDimensions,
        proxy.clone(),
    )));

    // Reserve the fair lock before starting the worker. The parser can receive
    // the first payload and enter advance_terminal(), but it cannot finish
    // Processor::advance until this lease is released.
    let term_lease = term.lease();
    let ingress = SshParserIngress::spawn_with_budget(
        term.clone(),
        proxy,
        metrics.clone(),
        TerminalIngressBudget::new(4, 2, 1).expect("valid worker budget"),
    );
    let sender = ingress.sender();
    sender
        .send_data(b"abc".to_vec())
        .await
        .expect("queue first chunk");

    timeout(Duration::from_secs(1), async {
        while metrics.snapshot().parser_chunks == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("worker should receive the first chunk and wait for the term lock");

    assert_eq!(
        sender.pending_bytes(),
        3,
        "the dequeued payload must remain reserved while Processor::advance is blocked"
    );
    sender
        .send_data(b"d".to_vec())
        .await
        .expect("the remaining byte budget should be usable");
    assert_eq!(sender.pending_bytes(), 4);

    let blocked_sender = sender.clone();
    let mut blocked_send =
        tokio::spawn(async move { blocked_sender.send_data(b"e".to_vec()).await });
    assert!(
        timeout(Duration::from_millis(25), &mut blocked_send)
            .await
            .is_err(),
        "a third send must wait until the parser releases the first reservation"
    );

    drop(term_lease);
    timeout(Duration::from_secs(1), &mut blocked_send)
        .await
        .expect("parser consumption should release byte capacity")
        .expect("blocked sender task should not panic")
        .expect("blocked payload should be accepted");
    drop(sender);

    timeout(Duration::from_secs(1), ingress.finish())
        .await
        .expect("worker should drain after the lock is released")
        .expect("worker should not panic");

    let term = term.lock();
    assert_eq!(term.grid()[Line(0)][Column(0)].c, 'a');
    assert_eq!(term.grid()[Line(0)][Column(1)].c, 'b');
    assert_eq!(term.grid()[Line(0)][Column(2)].c, 'c');
    assert_eq!(term.grid()[Line(0)][Column(3)].c, 'd');
    assert_eq!(term.grid()[Line(0)][Column(4)].c, 'e');
    drop(term);
    assert_eq!(metrics.snapshot().ingress_pending_bytes, 0);
}

#[tokio::test(flavor = "current_thread")]
async fn parser_worker_abort_discards_unconsumed_backlog_without_payload_metrics_or_wakeup() {
    const SECRET: &[u8] = b"never-retain-this-payload";

    let (event_tx, mut event_rx) = unbounded_channel();
    let metrics = Arc::new(TerminalPerformanceMetrics::default());
    let proxy = GpuiEventProxy::with_metrics(event_tx, metrics.clone());
    let term = Arc::new(FairMutex::new(Term::new(
        TermConfig::default(),
        &TestDimensions,
        proxy.clone(),
    )));
    let term_guard = term.lock();
    let ingress = SshParserIngress::spawn_with_budget(
        term.clone(),
        proxy,
        metrics.clone(),
        TerminalIngressBudget::new(64, 2, 1).expect("valid worker budget"),
    );
    let sender = ingress.sender();
    sender
        .send_data(SECRET.to_vec())
        .await
        .expect("queue secret chunk");
    sender
        .send_data(b"second".to_vec())
        .await
        .expect("queue second chunk");

    ingress.abort();
    drop(sender);
    drop(term_guard);
    timeout(Duration::from_secs(1), ingress.finish())
        .await
        .expect("abort should promptly stop worker")
        .expect("worker should not panic");

    let term = term.lock();
    assert_eq!(term.grid()[Line(0)][Column(0)].c, ' ');
    drop(term);
    assert!(event_rx.try_recv().is_err());
    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.ingress_bytes, 0);
    assert_eq!(snapshot.parser_chunks, 0);
    assert_eq!(snapshot.term_lock_samples, 0);
    assert_eq!(snapshot.ingress_pending_bytes, 0);
    assert_eq!(
        snapshot.ingress_pending_bytes_max,
        (SECRET.len() + b"second".len()) as u64
    );
    assert_eq!(snapshot.wakeup_requests, 0);
    assert!(
        !format!("{snapshot:?}").contains("never-retain-this-payload"),
        "metrics must remain numeric-only"
    );
}
