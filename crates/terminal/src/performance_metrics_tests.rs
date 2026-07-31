use crate::{
    TerminalActivity, TerminalInputMetricSource, TerminalPerformanceMetrics,
    TerminalPerformanceSnapshot,
};
use std::sync::Arc;
use std::time::Duration;

#[test]
fn metrics_aggregate_counts_backlog_and_maxima_without_payloads() {
    let metrics = TerminalPerformanceMetrics::enabled();
    let ingress_generation = metrics.begin_ingress_backlog();

    metrics.record_parser_chunk(4_096);
    metrics.record_parser_chunk(1_024);
    metrics.record_ingress_backlog(ingress_generation, 512, 512, 4_096);
    metrics.record_ingress_backlog(ingress_generation, 128, 128, 4_096);
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
    assert_eq!(512, snapshot.ingress_pending_bytes_window_max);
    assert_eq!(4_096, snapshot.ingress_pending_bytes_lifetime_max);
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
fn metrics_are_disabled_by_default_and_ignore_recording_calls() {
    let metrics = TerminalPerformanceMetrics::default();
    assert!(!metrics.is_enabled());
    let ingress_generation = metrics.begin_ingress_backlog();

    metrics.record_parser_chunk(4_096);
    metrics.record_ingress_backlog(ingress_generation, 512, 512, 4_096);
    metrics.record_input(TerminalInputMetricSource::User, 7);
    metrics.record_input(TerminalInputMetricSource::TerminalResponse, 11);
    metrics.record_term_lock(Duration::from_nanos(10), Duration::from_nanos(30));
    metrics.record_wakeup_request();
    metrics.record_wakeup_queued();
    metrics.record_wakeup_coalesced();
    metrics.record_render_at(Duration::from_nanos(50), true, Duration::from_secs(2));
    metrics.set_view_visible(true);
    metrics.set_tab_active(true);
    metrics.set_pane_active(true);
    metrics.record_ssh_connect(true);
    metrics.record_ssh_invalidation();

    assert_eq!(TerminalPerformanceSnapshot::default(), metrics.snapshot());
}

#[test]
fn metrics_record_lock_render_and_activity_state() {
    let metrics = TerminalPerformanceMetrics::enabled();
    assert_eq!(TerminalActivity::Background, metrics.snapshot().activity());

    metrics.set_tab_active(true);
    metrics.set_pane_active(true);
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
    assert_eq!(
        TerminalActivity::MountedHidden,
        metrics.snapshot().activity()
    );
    metrics.set_tab_active(false);
    assert_eq!(TerminalActivity::Background, metrics.snapshot().activity());
}

#[test]
fn activity_requires_the_active_pane_for_focused_state() {
    let metrics = TerminalPerformanceMetrics::enabled();
    metrics.set_tab_active(true);
    metrics.set_pane_active(false);
    metrics.record_render_at(Duration::from_nanos(10), true, Duration::from_secs(1));

    assert_eq!(TerminalActivity::Visible, metrics.snapshot().activity());

    metrics.set_pane_active(true);
    assert_eq!(TerminalActivity::Focused, metrics.snapshot().activity());
}

#[test]
fn backlog_window_peak_resets_without_resetting_lifetime_peak() {
    let metrics = TerminalPerformanceMetrics::enabled();
    let ingress_generation = metrics.begin_ingress_backlog();
    metrics.record_ingress_backlog(ingress_generation, 512, 512, 4_096);
    metrics.record_ingress_backlog(ingress_generation, 128, 128, 4_096);

    let first = metrics.snapshot_for_window();
    assert_eq!(512, first.ingress_pending_bytes_window_max);
    assert_eq!(4_096, first.ingress_pending_bytes_lifetime_max);

    metrics.record_ingress_backlog(ingress_generation, 256, 256, 4_096);
    let second = metrics.snapshot_for_window();
    assert_eq!(256, second.ingress_pending_bytes_window_max);
    assert_eq!(4_096, second.ingress_pending_bytes_lifetime_max);
}

#[test]
fn unchanged_lifetime_peak_is_not_replayed_into_a_new_window() {
    let metrics = TerminalPerformanceMetrics::enabled();
    let generation = metrics.begin_ingress_backlog();
    metrics.record_ingress_backlog(generation, 0, 4_096, 4_096);
    assert_eq!(
        4_096,
        metrics
            .snapshot_for_window()
            .ingress_pending_bytes_window_max
    );

    metrics.record_ingress_backlog(generation, 0, 0, 4_096);
    let next = metrics.snapshot_for_window();

    assert_eq!(0, next.ingress_pending_bytes_window_max);
    assert_eq!(4_096, next.ingress_pending_bytes_lifetime_max);
}

#[test]
fn stale_ingress_generation_cannot_overwrite_reconnected_queue_state() {
    let metrics = TerminalPerformanceMetrics::enabled();
    let old_generation = metrics.begin_ingress_backlog();
    metrics.record_ingress_backlog(old_generation, 512, 512, 512);

    let new_generation = metrics.begin_ingress_backlog();
    metrics.record_ingress_backlog(new_generation, 256, 256, 256);
    metrics.record_ingress_backlog(old_generation, 0, 1_024, 1_024);

    let snapshot = metrics.snapshot();
    assert_eq!(256, snapshot.ingress_pending_bytes);
    assert_eq!(512, snapshot.ingress_pending_bytes_window_max);
    assert_eq!(1_024, snapshot.ingress_pending_bytes_lifetime_max);
}

#[test]
fn metrics_saturate_duration_totals_instead_of_wrapping() {
    let metrics = TerminalPerformanceMetrics::enabled();

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
        ingress_pending_bytes_window_max: 512,
        ingress_pending_bytes_lifetime_max: 1_024,
        parser_chunks: 14,
        parser_chunk_bytes: 1_800,
        wakeup_requests: 6,
        ..TerminalPerformanceSnapshot::default()
    };

    let window = current.delta_since(&previous, Duration::from_secs(2));
    assert_eq!(200, window.ingress_bytes);
    assert_eq!(100.0, window.ingress_bytes_per_second);
    assert_eq!(64, window.ingress_pending_bytes);
    assert_eq!(512, window.ingress_pending_bytes_window_max);
    assert_eq!(1_024, window.ingress_pending_bytes_lifetime_max);
    assert_eq!(4, window.parser_chunks);
    assert_eq!(200.0, window.average_parser_chunk_bytes);
    assert_eq!(0, window.wakeup_requests);

    let zero_window = current.delta_since(&previous, Duration::ZERO);
    assert_eq!(0.0, zero_window.ingress_bytes_per_second);
}

#[test]
fn metrics_support_concurrent_atomic_updates() {
    let metrics = Arc::new(TerminalPerformanceMetrics::enabled());
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
