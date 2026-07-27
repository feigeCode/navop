use std::collections::VecDeque;
use std::io::{self, Read};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config as TermConfig, Term};
use tokio::sync::mpsc::unbounded_channel;
use tokio_util::sync::CancellationToken;

use crate::TerminalPerformanceMetrics;
use crate::ingress_queue::{TerminalDataSendError, TerminalIngressBudget};
use crate::pty_backend::{GpuiEventProxy, TerminalEvent};
use crate::recording::{RecordingBackend, RecordingEventKind, test_support::TestRecording};
use crate::serial_ingress::{SerialParserIngress, SerialReaderExit, run_serial_reader};

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

struct ScriptedReader {
    chunks: VecDeque<Vec<u8>>,
    read_count: Arc<AtomicUsize>,
}

impl ScriptedReader {
    fn new(chunks: impl IntoIterator<Item = Vec<u8>>) -> (Self, Arc<AtomicUsize>) {
        let read_count = Arc::new(AtomicUsize::new(0));
        (
            Self {
                chunks: chunks.into_iter().collect(),
                read_count: read_count.clone(),
            },
            read_count,
        )
    }
}

impl Read for ScriptedReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.read_count.fetch_add(1, Ordering::SeqCst);
        let Some(chunk) = self.chunks.pop_front() else {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "scripted serial source closed",
            ));
        };
        assert!(
            chunk.len() <= buffer.len(),
            "scripted chunk must fit the Serial source buffer"
        );
        buffer[..chunk.len()].copy_from_slice(&chunk);
        Ok(chunk.len())
    }
}

struct PanicReader;

impl Read for PanicReader {
    fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
        panic!("a cancelled Serial reader must not touch the port");
    }
}

fn terminal_harness() -> (
    Arc<FairMutex<Term<GpuiEventProxy>>>,
    GpuiEventProxy,
    Arc<TerminalPerformanceMetrics>,
    tokio::sync::mpsc::UnboundedReceiver<TerminalEvent>,
) {
    let (event_tx, event_rx) = unbounded_channel();
    let metrics = Arc::new(TerminalPerformanceMetrics::default());
    let event_proxy = GpuiEventProxy::with_metrics(event_tx, metrics.clone());
    let term = Arc::new(FairMutex::new(Term::new(
        TermConfig::default(),
        &TestDimensions,
        event_proxy.clone(),
    )));
    (term, event_proxy, metrics, event_rx)
}

fn test_budget(pending_bytes: u64, pending_chunks: usize) -> TerminalIngressBudget {
    TerminalIngressBudget::new(pending_bytes, pending_chunks, 1).expect("valid Serial test budget")
}

fn wait_until(description: &str, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(1);
    while !predicate() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {description}"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn reader_and_parser_preserve_order_drain_and_coalesce_wakeups() {
    let (term, event_proxy, metrics, mut event_rx) = terminal_harness();
    let ingress = SerialParserIngress::spawn_with_budget(
        term.clone(),
        event_proxy,
        metrics.clone(),
        test_budget(8, 2),
    )
    .expect("spawn Serial parser worker");
    let producer = ingress.producer();
    let (mut reader, _read_count) = ScriptedReader::new([b"ab".to_vec(), b"cd".to_vec()]);

    let exit = run_serial_reader(&mut reader, &producer, &CancellationToken::new());
    assert_eq!(exit, SerialReaderExit::Disconnected);
    ingress
        .finish()
        .expect("Serial parser worker should drain without panicking");

    let term = term.lock();
    assert_eq!(term.grid()[Line(0)][Column(0)].c, 'a');
    assert_eq!(term.grid()[Line(0)][Column(1)].c, 'b');
    assert_eq!(term.grid()[Line(0)][Column(2)].c, 'c');
    assert_eq!(term.grid()[Line(0)][Column(3)].c, 'd');
    drop(term);

    assert!(matches!(event_rx.try_recv(), Ok(TerminalEvent::Wakeup)));
    assert!(event_rx.try_recv().is_err());
    assert_eq!(producer.pending_bytes(), 0);
    assert!(matches!(
        producer.send_data(b"after-close".to_vec()),
        Err(TerminalDataSendError::Closed(_))
    ));

    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.ingress_bytes, 4);
    assert_eq!(snapshot.parser_chunks, 2);
    assert_eq!(snapshot.parser_chunk_bytes, 4);
    assert_eq!(snapshot.term_lock_samples, 2);
    assert_eq!(snapshot.ingress_pending_bytes, 0);
    assert!((2..=4).contains(&snapshot.ingress_pending_bytes_max));
    assert_eq!(snapshot.wakeup_requests, 2);
    assert_eq!(snapshot.wakeup_queued, 1);
    assert_eq!(snapshot.wakeup_coalesced, 1);
}

#[test]
fn natural_disconnect_is_reported_only_after_accepted_data_is_drained() {
    let (term, event_proxy, metrics, _event_rx) = terminal_harness();
    let term_lease = term.lease();
    let (disconnect_tx, mut disconnect_rx) = unbounded_channel();
    let ingress = SerialParserIngress::spawn_with_budget_and_callback(
        term.clone(),
        event_proxy,
        metrics.clone(),
        test_budget(4, 2),
        Some(disconnect_tx),
    )
    .expect("spawn Serial parser worker");
    let producer = ingress.producer();
    let (mut reader, _read_count) = ScriptedReader::new([b"abc".to_vec()]);

    assert_eq!(
        run_serial_reader(&mut reader, &producer, &CancellationToken::new()),
        SerialReaderExit::Disconnected
    );
    wait_until("the parser to reach the terminal lock", || {
        metrics.snapshot().parser_chunks > 0
    });
    assert!(
        disconnect_rx.try_recv().is_err(),
        "natural disconnect must not be reported while accepted bytes are still parsing"
    );

    drop(term_lease);
    ingress
        .finish()
        .expect("Serial parser worker should drain without panicking");
    assert!(
        matches!(disconnect_rx.try_recv(), Ok(())),
        "natural disconnect should be reported after the parser drains accepted bytes"
    );
    assert!(disconnect_rx.try_recv().is_err());
    assert_eq!(producer.pending_bytes(), 0);
}

#[test]
fn parser_worker_holds_byte_reservation_through_term_lock_and_parser_consumption() {
    let (term, event_proxy, metrics, _event_rx) = terminal_harness();
    let term_lease = term.lease();
    let ingress = SerialParserIngress::spawn_with_budget(
        term.clone(),
        event_proxy,
        metrics.clone(),
        test_budget(4, 2),
    )
    .expect("spawn Serial parser worker");
    let producer = ingress.producer();

    producer
        .send_data(b"abc".to_vec())
        .expect("queue first Serial payload");
    wait_until("the parser to reach the terminal lock", || {
        metrics.snapshot().parser_chunks > 0
    });
    assert_eq!(
        producer.pending_bytes(),
        3,
        "a dequeued Serial payload must retain its reservation until parsing finishes"
    );

    producer
        .send_data(b"d".to_vec())
        .expect("use the remaining byte of ingress capacity");
    assert_eq!(producer.pending_bytes(), 4);

    let blocked_producer = producer.clone();
    let (send_result_tx, send_result_rx) = mpsc::channel();
    let blocked_send = thread::spawn(move || {
        let result = blocked_producer.send_data(b"e".to_vec());
        let _ = send_result_tx.send(result);
    });
    assert!(
        matches!(
            send_result_rx.recv_timeout(Duration::from_millis(25)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ),
        "the full byte budget must stop a third Serial payload"
    );

    drop(term_lease);
    send_result_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("parser completion should release ingress capacity")
        .expect("blocked Serial payload should be accepted");
    blocked_send
        .join()
        .expect("blocked Serial producer should not panic");
    ingress
        .finish()
        .expect("Serial parser worker should drain without panicking");

    let term = term.lock();
    assert_eq!(term.grid()[Line(0)][Column(0)].c, 'a');
    assert_eq!(term.grid()[Line(0)][Column(1)].c, 'b');
    assert_eq!(term.grid()[Line(0)][Column(2)].c, 'c');
    assert_eq!(term.grid()[Line(0)][Column(3)].c, 'd');
    assert_eq!(term.grid()[Line(0)][Column(4)].c, 'e');
    drop(term);
    assert_eq!(producer.pending_bytes(), 0);
    assert_eq!(metrics.snapshot().ingress_pending_bytes, 0);
}

#[test]
fn full_ingress_holds_one_source_chunk_and_abort_releases_the_reader() {
    let (term, event_proxy, metrics, _event_rx) = terminal_harness();
    let term_lease = term.lease();
    let (disconnect_tx, mut disconnect_rx) = unbounded_channel();
    let ingress = SerialParserIngress::spawn_with_budget_and_callback(
        term.clone(),
        event_proxy,
        metrics.clone(),
        test_budget(1, 1),
        Some(disconnect_tx),
    )
    .expect("spawn Serial parser worker");
    let producer = ingress.producer();
    let shutdown = CancellationToken::new();
    let reader_shutdown = shutdown.clone();
    let reader_producer = producer.clone();
    let (mut reader, read_count) =
        ScriptedReader::new([b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]);
    let (reader_exit_tx, reader_exit_rx) = mpsc::channel();
    let reader_task = thread::spawn(move || {
        let exit = run_serial_reader(&mut reader, &reader_producer, &reader_shutdown);
        let _ = reader_exit_tx.send(exit);
    });

    wait_until("the parser to hold the first byte reservation", || {
        metrics.snapshot().parser_chunks > 0
    });
    wait_until("the reader to retain its second source chunk", || {
        read_count.load(Ordering::SeqCst) == 2
    });
    thread::sleep(Duration::from_millis(25));
    assert_eq!(
        read_count.load(Ordering::SeqCst),
        2,
        "a blocked send must prevent reading a third source chunk"
    );
    assert_eq!(producer.pending_bytes(), 1);

    shutdown.cancel();
    ingress.abort();
    assert_eq!(
        reader_exit_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("abort should wake the blocked Serial reader"),
        SerialReaderExit::Shutdown
    );
    reader_task
        .join()
        .expect("Serial reader thread should not panic");
    assert_eq!(read_count.load(Ordering::SeqCst), 2);
    assert!(
        disconnect_rx.try_recv().is_err(),
        "abortive shutdown must not report a natural Serial disconnect"
    );

    drop(term_lease);
    ingress
        .finish()
        .expect("aborted Serial parser worker should stop without panicking");
    assert!(
        disconnect_rx.try_recv().is_err(),
        "aborted parser completion must not emit a delayed disconnect notification"
    );
    assert_eq!(producer.pending_bytes(), 0);
    assert_eq!(metrics.snapshot().ingress_pending_bytes, 0);
}

#[test]
fn source_close_bypasses_full_data_budget_and_rejects_later_payloads() {
    let (term, event_proxy, metrics, _event_rx) = terminal_harness();
    let term_lease = term.lease();
    let ingress = SerialParserIngress::spawn_with_budget(
        term.clone(),
        event_proxy,
        metrics.clone(),
        test_budget(1, 1),
    )
    .expect("spawn Serial parser worker");
    let producer = ingress.producer();
    producer
        .send_data(b"a".to_vec())
        .expect("queue the payload that fills the data budget");
    wait_until("the parser to retain the full data budget", || {
        metrics.snapshot().parser_chunks > 0
    });
    assert_eq!(producer.pending_bytes(), 1);

    let closing_producer = producer.clone();
    let (closed_tx, closed_rx) = mpsc::channel();
    let close_task = thread::spawn(move || {
        closing_producer.close_source();
        let _ = closed_tx.send(());
    });
    closed_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("SourceClosed control must bypass full data capacity");
    close_task
        .join()
        .expect("SourceClosed sender should not panic");
    assert!(matches!(
        producer.send_data(b"b".to_vec()),
        Err(TerminalDataSendError::Closed(data)) if data == b"b"
    ));

    drop(term_lease);
    ingress
        .finish()
        .expect("Serial parser worker should drain accepted data after SourceClosed");
    assert_eq!(producer.pending_bytes(), 0);
}

#[test]
fn cancelled_reader_exits_without_performing_a_port_read() {
    let (term, event_proxy, metrics, _event_rx) = terminal_harness();
    let ingress =
        SerialParserIngress::spawn_with_budget(term, event_proxy, metrics, test_budget(1, 1))
            .expect("spawn Serial parser worker");
    let producer = ingress.producer();
    let shutdown = CancellationToken::new();
    shutdown.cancel();

    assert_eq!(
        run_serial_reader(&mut PanicReader, &producer, &shutdown),
        SerialReaderExit::Shutdown
    );
    ingress
        .finish()
        .expect("cancelled Serial parser worker should stop without panicking");
    assert_eq!(producer.pending_bytes(), 0);
}

#[test]
fn parser_worker_records_accepted_raw_output_at_the_parser_boundary() {
    let recording = TestRecording::start(RecordingBackend::Serial, false);
    let payload = b"\xffserial\x1b]133;A\x07output".to_vec();
    let (term, event_proxy, metrics, _event_rx) = terminal_harness();
    let ingress = SerialParserIngress::spawn_with_budget_callback_and_recording(
        term,
        event_proxy,
        metrics,
        test_budget(payload.len() as u64, 1),
        None,
        Some(recording.tap()),
    )
    .expect("spawn Serial recording parser worker");
    let producer = ingress.producer();

    producer
        .send_data(payload.clone())
        .expect("queue Serial output");
    producer.close_source();
    ingress
        .finish()
        .expect("Serial parser worker should drain without panicking");

    let parsed = recording.finish();
    assert_eq!(1, parsed.events.len());
    assert!(matches!(
        &parsed.events[0].kind,
        RecordingEventKind::Output(data) if data == &payload
    ));
}
