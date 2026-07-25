use crate::{
    TerminalActivity, TerminalInputMetricSource, TerminalPerformanceMetrics,
    TerminalPerformanceSnapshot,
};
use std::sync::Arc;
use std::time::Duration;

#[test]
fn metrics_aggregate_counts_backlog_and_maxima_without_payloads() {
    let metrics = TerminalPerformanceMetrics::default();

    metrics.record_parser_chunk(4_096);
    metrics.record_parser_chunk(1_024);
    metrics.record_ingress_backlog(512, 4_096);
    metrics.record_ingress_backlog(128, 2_048);
    metrics.record_input(TerminalInputMetricSource::User, 7);
    metrics.record_input(TerminalInputMetricSource::TerminalResponse, 11);
    metrics.record_wakeup_request();
    metrics.record_wakeup_request();
    metrics.record_wakeup_queued();
    metrics.record_wakeup_coalesced();
    metrics.record_ssh_connect(false);
    metrics.record_ssh_connect(true);
    metrics.record_ssh_invalidation();

    let snapshot = metrics.snapshot();
    assert_eq!(5_120, snapshot.ingress_bytes);
    assert_eq!(128, snapshot.ingress_pending_bytes);
    assert_eq!(4_096, snapshot.ingress_pending_bytes_max);
    assert_eq!(2, snapshot.parser_chunks);
    assert_eq!(5_120, snapshot.parser_chunk_bytes);
    assert_eq!(4_096, snapshot.parser_chunk_max_bytes);
    assert_eq!(7, snapshot.user_input_bytes);
    assert_eq!(11, snapshot.terminal_response_bytes);
    assert_eq!(2, snapshot.wakeup_requests);
    assert_eq!(1, snapshot.wakeup_queued);
    assert_eq!(1, snapshot.wakeup_coalesced);
    assert_eq!(2, snapshot.ssh_connects);
    assert_eq!(1, snapshot.ssh_reconnects);
    assert_eq!(1, snapshot.ssh_invalidations);
}

#[test]
fn metrics_record_lock_render_and_activity_state() {
    let metrics = TerminalPerformanceMetrics::default();
    assert_eq!(TerminalActivity::Background, metrics.snapshot().activity());

    metrics.record_term_lock(Duration::from_nanos(10), Duration::from_nanos(30));
    metrics.record_term_lock(Duration::from_nanos(20), Duration::from_nanos(40));
    metrics.record_render_at(Duration::from_nanos(50), true, Duration::from_secs(2));

    let snapshot = metrics.snapshot();
    assert_eq!(2, snapshot.term_lock_samples);
    assert_eq!(30, snapshot.term_lock_wait_ns);
    assert_eq!(20, snapshot.term_lock_wait_max_ns);
    assert_eq!(70, snapshot.term_lock_hold_ns);
    assert_eq!(40, snapshot.term_lock_hold_max_ns);
    assert_eq!(1, snapshot.render_samples);
    assert_eq!(50, snapshot.render_ns);
    assert_eq!(50, snapshot.render_max_ns);
    assert_eq!(TerminalActivity::Focused, snapshot.activity());

    metrics.record_render_at(Duration::from_nanos(25), false, Duration::from_secs(3));
    assert_eq!(TerminalActivity::Visible, metrics.snapshot().activity());
    metrics.set_view_visible(false);
    assert_eq!(TerminalActivity::Background, metrics.snapshot().activity());
}

#[test]
fn metrics_saturate_duration_totals_instead_of_wrapping() {
    let metrics = TerminalPerformanceMetrics::default();

    metrics.record_term_lock(Duration::MAX, Duration::MAX);
    metrics.record_term_lock(Duration::from_nanos(1), Duration::from_nanos(1));
    metrics.record_render_at(Duration::MAX, false, Duration::MAX);
    metrics.record_render_at(Duration::from_nanos(1), false, Duration::from_nanos(1));

    let snapshot = metrics.snapshot();
    assert_eq!(u64::MAX, snapshot.term_lock_wait_ns);
    assert_eq!(u64::MAX, snapshot.term_lock_wait_max_ns);
    assert_eq!(u64::MAX, snapshot.term_lock_hold_ns);
    assert_eq!(u64::MAX, snapshot.term_lock_hold_max_ns);
    assert_eq!(u64::MAX, snapshot.render_ns);
    assert_eq!(u64::MAX, snapshot.render_max_ns);
}

#[test]
fn metrics_window_uses_saturating_deltas_and_handles_zero_elapsed_time() {
    let previous = TerminalPerformanceSnapshot {
        ingress_bytes: 100,
        parser_chunks: 10,
        parser_chunk_bytes: 1_000,
        wakeup_requests: 8,
        ..TerminalPerformanceSnapshot::default()
    };
    let current = TerminalPerformanceSnapshot {
        ingress_bytes: 300,
        ingress_pending_bytes: 64,
        ingress_pending_bytes_max: 512,
        parser_chunks: 14,
        parser_chunk_bytes: 1_800,
        wakeup_requests: 6,
        ..TerminalPerformanceSnapshot::default()
    };

    let window = current.delta_since(&previous, Duration::from_secs(2));
    assert_eq!(200, window.ingress_bytes);
    assert_eq!(100.0, window.ingress_bytes_per_second);
    assert_eq!(64, window.ingress_pending_bytes);
    assert_eq!(512, window.ingress_pending_bytes_max);
    assert_eq!(4, window.parser_chunks);
    assert_eq!(200.0, window.average_parser_chunk_bytes);
    assert_eq!(0, window.wakeup_requests);

    let zero_window = current.delta_since(&previous, Duration::ZERO);
    assert_eq!(0.0, zero_window.ingress_bytes_per_second);
}

#[test]
fn metrics_support_concurrent_atomic_updates() {
    let metrics = Arc::new(TerminalPerformanceMetrics::default());
    let workers = (0..4)
        .map(|_| {
            let metrics = metrics.clone();
            std::thread::spawn(move || {
                for _ in 0..1_000 {
                    metrics.record_parser_chunk(16);
                    metrics.record_wakeup_request();
                }
            })
        })
        .collect::<Vec<_>>();

    for worker in workers {
        worker.join().expect("metrics worker should finish");
    }

    let snapshot = metrics.snapshot();
    assert_eq!(4_000, snapshot.parser_chunks);
    assert_eq!(64_000, snapshot.ingress_bytes);
    assert_eq!(4_000, snapshot.wakeup_requests);
}
