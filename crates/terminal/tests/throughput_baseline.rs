use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config as TermConfig, Term};
use alacritty_terminal::tty::{Options as PtyOptions, Shell};
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
use terminal::pty_backend::{GpuiEventProxy, LocalPtyBackend, TerminalEvent};
use terminal::{TerminalActivity, TerminalPerformanceMetrics};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

struct BaselineDimensions;

impl Dimensions for BaselineDimensions {
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

type SharedTerm = Arc<FairMutex<Term<GpuiEventProxy>>>;

fn test_term() -> (
    SharedTerm,
    GpuiEventProxy,
    Arc<TerminalPerformanceMetrics>,
    UnboundedReceiver<TerminalEvent>,
) {
    let (event_tx, event_rx) = unbounded_channel();
    let metrics = Arc::new(TerminalPerformanceMetrics::default());
    let proxy = GpuiEventProxy::with_metrics(event_tx, metrics.clone());
    let term = Term::new(TermConfig::default(), &BaselineDimensions, proxy.clone());
    (Arc::new(FairMutex::new(term)), proxy, metrics, event_rx)
}

fn parse_chunks(term: &SharedTerm, metrics: &TerminalPerformanceMetrics, chunks: usize) {
    let mut processor = Processor::<StdSyncHandler>::new();
    let chunk = baseline_chunk();
    for _ in 0..chunks {
        metrics.record_parser_chunk(chunk.len());
        let wait_started = Instant::now();
        let mut term = term.lock();
        let wait = wait_started.elapsed();
        let hold_started = Instant::now();
        processor.advance(&mut *term, &chunk);
        let hold = hold_started.elapsed();
        drop(term);
        metrics.record_term_lock(wait, hold);
    }
}

fn parse_chunks_without_metrics(term: &SharedTerm, chunks: usize) {
    let mut processor = Processor::<StdSyncHandler>::new();
    let chunk = baseline_chunk();
    for _ in 0..chunks {
        processor.advance(&mut *term.lock(), &chunk);
    }
}

fn baseline_chunk() -> Vec<u8> {
    vec![b'x'; 4094].into_iter().chain([b'\r', b'\n']).collect()
}

fn measure_parser(instrumented: bool, chunks: usize) -> Duration {
    let (term, _proxy, metrics, _events) = test_term();
    let started = Instant::now();
    if instrumented {
        parse_chunks(&term, &metrics, chunks);
    } else {
        parse_chunks_without_metrics(&term, chunks);
    }
    started.elapsed()
}

#[test]
#[ignore = "performance baseline"]
fn parser_and_metrics_high_output_baseline() {
    let (term, _proxy, metrics, _events) = test_term();
    let chunks = 4096;
    let started = Instant::now();

    parse_chunks(&term, &metrics, chunks);

    let elapsed = started.elapsed();
    let snapshot = metrics.snapshot();
    let mib = snapshot.ingress_bytes as f64 / (1024.0 * 1024.0);
    println!(
        "parser_high_output mib={mib:.2} elapsed_ms={:.3} mib_per_sec={:.2} lock_wait_avg_ns={:.1} lock_hold_avg_ns={:.1}",
        elapsed.as_secs_f64() * 1000.0,
        mib / elapsed.as_secs_f64(),
        snapshot.term_lock_wait_ns as f64 / snapshot.term_lock_samples as f64,
        snapshot.term_lock_hold_ns as f64 / snapshot.term_lock_samples as f64,
    );
    assert_eq!(snapshot.ingress_bytes, chunks as u64 * 4096);
    assert_eq!(snapshot.parser_chunks, chunks as u64);
}

#[test]
#[ignore = "performance baseline"]
fn parser_metrics_overhead_baseline() {
    let chunks = 2048;
    let mut uninstrumented = measure_parser(false, chunks);
    let mut instrumented = measure_parser(true, chunks);
    instrumented += measure_parser(true, chunks);
    uninstrumented += measure_parser(false, chunks);
    let ratio = instrumented.as_secs_f64() / uninstrumented.as_secs_f64();

    println!(
        "parser_metrics_overhead baseline_ms={:.3} instrumented_ms={:.3} ratio={ratio:.3}",
        uninstrumented.as_secs_f64() * 1000.0,
        instrumented.as_secs_f64() * 1000.0,
    );
    assert!(uninstrumented > Duration::ZERO);
    assert!(instrumented > Duration::ZERO);
}

#[test]
#[ignore = "performance baseline"]
fn concurrent_terminal_parser_baseline() {
    let started = Instant::now();
    let workers = (0..4)
        .map(|_| {
            std::thread::spawn(|| {
                let (term, _proxy, metrics, _events) = test_term();
                parse_chunks(&term, &metrics, 1024);
                metrics.snapshot()
            })
        })
        .collect::<Vec<_>>();
    let snapshots = workers
        .into_iter()
        .map(|worker| worker.join().expect("baseline worker should finish"))
        .collect::<Vec<_>>();
    let elapsed = started.elapsed();
    let bytes = snapshots
        .iter()
        .map(|snapshot| snapshot.ingress_bytes)
        .sum::<u64>();

    println!(
        "concurrent_terminals terminals=4 bytes={bytes} elapsed_ms={:.3}",
        elapsed.as_secs_f64() * 1000.0
    );
    assert_eq!(bytes, 16 * 1024 * 1024);
}

#[test]
#[ignore = "performance baseline"]
fn background_terminal_output_remains_observable() {
    let (term, _proxy, metrics, _events) = test_term();
    metrics.set_view_visible(false);

    parse_chunks(&term, &metrics, 256);

    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.ingress_bytes, 1024 * 1024);
    assert_eq!(snapshot.activity(), TerminalActivity::Background);
    println!(
        "background_terminal bytes={} chunks={} activity={:?}",
        snapshot.ingress_bytes,
        snapshot.parser_chunks,
        snapshot.activity()
    );
}

#[cfg(unix)]
#[test]
#[ignore = "performance baseline"]
fn local_pty_close_during_high_output_baseline() {
    let (term, proxy, metrics, mut events) = test_term();
    let options = PtyOptions {
        shell: Some(Shell::new(
            "/bin/sh".to_string(),
            vec!["-c".to_string(), "yes navop-baseline".to_string()],
        )),
        working_directory: None,
        drain_on_exit: false,
        env: HashMap::new(),
    };
    let backend = LocalPtyBackend::new(term, proxy, options).expect("local PTY should start");
    let output_started = wait_for_ingress(&metrics, &mut events, Duration::from_secs(2));
    let close_started = Instant::now();
    backend.shutdown();
    let (output_stopped, child_exit) =
        wait_for_output_quiet(&metrics, &mut events, Duration::from_secs(5));
    let close_elapsed = close_started.elapsed();
    let snapshot = metrics.snapshot();

    println!(
        "local_close bytes={} close_ms={:.3} output_stopped={output_stopped} child_exit={child_exit}",
        snapshot.ingress_bytes,
        close_elapsed.as_secs_f64() * 1000.0
    );
    assert!(
        output_started && snapshot.ingress_bytes > 0,
        "local PTY should produce observable output before shutdown"
    );
    assert!(
        output_stopped,
        "local PTY output should stop after shutdown"
    );
}

fn wait_for_ingress(
    metrics: &TerminalPerformanceMetrics,
    events: &mut UnboundedReceiver<TerminalEvent>,
    timeout: Duration,
) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        while events.try_recv().is_ok() {}
        if metrics.snapshot().ingress_bytes > 0 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn wait_for_output_quiet(
    metrics: &TerminalPerformanceMetrics,
    events: &mut UnboundedReceiver<TerminalEvent>,
    timeout: Duration,
) -> (bool, bool) {
    let deadline = Instant::now() + timeout;
    let mut previous_bytes = metrics.snapshot().ingress_bytes;
    let mut quiet_samples = 0;
    let mut child_exit = false;
    while Instant::now() < deadline {
        while let Ok(event) = events.try_recv() {
            child_exit |= matches!(event, TerminalEvent::ChildExit(_));
        }
        std::thread::sleep(Duration::from_millis(20));
        let current_bytes = metrics.snapshot().ingress_bytes;
        if current_bytes == previous_bytes {
            quiet_samples += 1;
            if quiet_samples >= 5 {
                return (true, child_exit);
            }
        } else {
            quiet_samples = 0;
            previous_bytes = current_bytes;
        }
    }
    (false, child_exit)
}
