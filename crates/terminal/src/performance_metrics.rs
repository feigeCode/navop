//! Payload-free, lock-free counters for terminal performance observability.
//!
//! Snapshots are best-effort observations assembled from independent atomics.
//! They are not transactionally consistent across fields and must not be used
//! to drive correctness-sensitive terminal behavior.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalActivity {
    Focused,
    Visible,
    Background,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalInputMetricSource {
    User,
    TerminalResponse,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TerminalPerformanceSnapshot {
    pub ingress_bytes: u64,
    pub ingress_pending_bytes: u64,
    pub ingress_pending_bytes_max: u64,
    pub parser_chunks: u64,
    pub parser_chunk_bytes: u64,
    pub parser_chunk_max_bytes: u64,
    pub user_input_bytes: u64,
    pub terminal_response_bytes: u64,
    pub term_lock_samples: u64,
    pub term_lock_wait_ns: u64,
    pub term_lock_wait_max_ns: u64,
    pub term_lock_hold_ns: u64,
    pub term_lock_hold_max_ns: u64,
    pub wakeup_requests: u64,
    pub wakeup_queued: u64,
    pub wakeup_coalesced: u64,
    pub render_samples: u64,
    pub render_ns: u64,
    pub render_max_ns: u64,
    pub ssh_connects: u64,
    pub ssh_reconnects: u64,
    pub ssh_invalidations: u64,
    pub last_render_tick_ns: u64,
    pub last_render_focused: bool,
    pub view_visible: bool,
}

impl TerminalPerformanceSnapshot {
    /// Builds a window from two best-effort snapshots.
    ///
    /// Counter deltas saturate at zero when the supplied snapshots are out of
    /// order. `ingress_pending_bytes_max` remains the lifetime peak observed by
    /// the metrics instance rather than a peak scoped to this window.
    pub fn delta_since(&self, previous: &Self, elapsed: Duration) -> TerminalPerformanceWindow {
        let ingress_bytes = delta(self.ingress_bytes, previous.ingress_bytes);
        let parser_chunks = delta(self.parser_chunks, previous.parser_chunks);
        let parser_bytes = delta(self.parser_chunk_bytes, previous.parser_chunk_bytes);
        let lock_samples = delta(self.term_lock_samples, previous.term_lock_samples);
        let lock_wait = delta(self.term_lock_wait_ns, previous.term_lock_wait_ns);
        let lock_hold = delta(self.term_lock_hold_ns, previous.term_lock_hold_ns);
        let render_samples = delta(self.render_samples, previous.render_samples);
        let render_ns = delta(self.render_ns, previous.render_ns);

        TerminalPerformanceWindow {
            elapsed,
            ingress_bytes,
            ingress_bytes_per_second: rate(ingress_bytes, elapsed),
            ingress_pending_bytes: self.ingress_pending_bytes,
            ingress_pending_bytes_max: self.ingress_pending_bytes_max,
            parser_chunks,
            average_parser_chunk_bytes: average(parser_bytes, parser_chunks),
            user_input_bytes: delta(self.user_input_bytes, previous.user_input_bytes),
            terminal_response_bytes: delta(
                self.terminal_response_bytes,
                previous.terminal_response_bytes,
            ),
            term_lock_samples: lock_samples,
            average_term_lock_wait_ns: average(lock_wait, lock_samples),
            average_term_lock_hold_ns: average(lock_hold, lock_samples),
            wakeup_requests: delta(self.wakeup_requests, previous.wakeup_requests),
            wakeup_queued: delta(self.wakeup_queued, previous.wakeup_queued),
            wakeup_coalesced: delta(self.wakeup_coalesced, previous.wakeup_coalesced),
            render_samples,
            average_render_ns: average(render_ns, render_samples),
            ssh_connects: delta(self.ssh_connects, previous.ssh_connects),
            ssh_reconnects: delta(self.ssh_reconnects, previous.ssh_reconnects),
            ssh_invalidations: delta(self.ssh_invalidations, previous.ssh_invalidations),
        }
    }

    pub fn activity(&self) -> TerminalActivity {
        if !self.view_visible {
            TerminalActivity::Background
        } else if self.last_render_focused {
            TerminalActivity::Focused
        } else {
            TerminalActivity::Visible
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TerminalPerformanceWindow {
    pub elapsed: Duration,
    pub ingress_bytes: u64,
    pub ingress_bytes_per_second: f64,
    pub ingress_pending_bytes: u64,
    pub ingress_pending_bytes_max: u64,
    pub parser_chunks: u64,
    pub average_parser_chunk_bytes: f64,
    pub user_input_bytes: u64,
    pub terminal_response_bytes: u64,
    pub term_lock_samples: u64,
    pub average_term_lock_wait_ns: f64,
    pub average_term_lock_hold_ns: f64,
    pub wakeup_requests: u64,
    pub wakeup_queued: u64,
    pub wakeup_coalesced: u64,
    pub render_samples: u64,
    pub average_render_ns: f64,
    pub ssh_connects: u64,
    pub ssh_reconnects: u64,
    pub ssh_invalidations: u64,
}

#[derive(Default)]
pub struct TerminalPerformanceMetrics {
    ingress_bytes: AtomicU64,
    ingress_pending_bytes: AtomicU64,
    ingress_pending_bytes_max: AtomicU64,
    parser_chunks: AtomicU64,
    parser_chunk_bytes: AtomicU64,
    parser_chunk_max_bytes: AtomicU64,
    user_input_bytes: AtomicU64,
    terminal_response_bytes: AtomicU64,
    term_lock_samples: AtomicU64,
    term_lock_wait_ns: AtomicU64,
    term_lock_wait_max_ns: AtomicU64,
    term_lock_hold_ns: AtomicU64,
    term_lock_hold_max_ns: AtomicU64,
    wakeup_requests: AtomicU64,
    wakeup_queued: AtomicU64,
    wakeup_coalesced: AtomicU64,
    render_samples: AtomicU64,
    render_ns: AtomicU64,
    render_max_ns: AtomicU64,
    ssh_connects: AtomicU64,
    ssh_reconnects: AtomicU64,
    ssh_invalidations: AtomicU64,
    last_render_tick_ns: AtomicU64,
    last_render_focused: AtomicBool,
    view_visible: AtomicBool,
}

impl TerminalPerformanceMetrics {
    pub fn record_parser_chunk(&self, bytes: usize) {
        let bytes = usize_to_u64(bytes);
        add(&self.ingress_bytes, bytes);
        add(&self.parser_chunks, 1);
        add(&self.parser_chunk_bytes, bytes);
        self.parser_chunk_max_bytes
            .fetch_max(bytes, Ordering::Relaxed);
    }

    pub fn record_ingress_backlog(&self, current_bytes: usize, peak_bytes: usize) {
        let current_bytes = usize_to_u64(current_bytes);
        let peak_bytes = usize_to_u64(peak_bytes).max(current_bytes);
        self.ingress_pending_bytes
            .store(current_bytes, Ordering::Relaxed);
        self.ingress_pending_bytes_max
            .fetch_max(peak_bytes, Ordering::Relaxed);
    }

    pub fn record_input(&self, source: TerminalInputMetricSource, bytes: usize) {
        let target = match source {
            TerminalInputMetricSource::User => &self.user_input_bytes,
            TerminalInputMetricSource::TerminalResponse => &self.terminal_response_bytes,
        };
        add(target, usize_to_u64(bytes));
    }

    pub fn record_term_lock(&self, wait: Duration, hold: Duration) {
        add(&self.term_lock_samples, 1);
        record_duration(&self.term_lock_wait_ns, &self.term_lock_wait_max_ns, wait);
        record_duration(&self.term_lock_hold_ns, &self.term_lock_hold_max_ns, hold);
    }

    pub fn record_wakeup_request(&self) {
        add(&self.wakeup_requests, 1);
    }

    pub fn record_wakeup_queued(&self) {
        add(&self.wakeup_queued, 1);
    }

    pub fn record_wakeup_coalesced(&self) {
        add(&self.wakeup_coalesced, 1);
    }

    pub fn record_render(&self, duration: Duration, focused: bool) {
        self.record_render_at(duration, focused, monotonic_now());
    }

    pub fn record_render_at(&self, duration: Duration, focused: bool, rendered_at: Duration) {
        add(&self.render_samples, 1);
        record_duration(&self.render_ns, &self.render_max_ns, duration);
        self.last_render_focused.store(focused, Ordering::Release);
        self.view_visible.store(true, Ordering::Release);
        self.last_render_tick_ns
            .store(encode_tick(rendered_at), Ordering::Release);
    }

    pub fn set_view_visible(&self, visible: bool) {
        self.view_visible.store(visible, Ordering::Release);
    }

    pub fn record_ssh_connect(&self, reconnect: bool) {
        add(&self.ssh_connects, 1);
        if reconnect {
            add(&self.ssh_reconnects, 1);
        }
    }

    pub fn record_ssh_invalidation(&self) {
        add(&self.ssh_invalidations, 1);
    }

    pub fn snapshot(&self) -> TerminalPerformanceSnapshot {
        // Each field is intentionally loaded independently. Metrics consumers
        // must treat this as observability data, not a coherent state snapshot.
        TerminalPerformanceSnapshot {
            ingress_bytes: load(&self.ingress_bytes),
            ingress_pending_bytes: load(&self.ingress_pending_bytes),
            ingress_pending_bytes_max: load(&self.ingress_pending_bytes_max),
            parser_chunks: load(&self.parser_chunks),
            parser_chunk_bytes: load(&self.parser_chunk_bytes),
            parser_chunk_max_bytes: load(&self.parser_chunk_max_bytes),
            user_input_bytes: load(&self.user_input_bytes),
            terminal_response_bytes: load(&self.terminal_response_bytes),
            term_lock_samples: load(&self.term_lock_samples),
            term_lock_wait_ns: load(&self.term_lock_wait_ns),
            term_lock_wait_max_ns: load(&self.term_lock_wait_max_ns),
            term_lock_hold_ns: load(&self.term_lock_hold_ns),
            term_lock_hold_max_ns: load(&self.term_lock_hold_max_ns),
            wakeup_requests: load(&self.wakeup_requests),
            wakeup_queued: load(&self.wakeup_queued),
            wakeup_coalesced: load(&self.wakeup_coalesced),
            render_samples: load(&self.render_samples),
            render_ns: load(&self.render_ns),
            render_max_ns: load(&self.render_max_ns),
            ssh_connects: load(&self.ssh_connects),
            ssh_reconnects: load(&self.ssh_reconnects),
            ssh_invalidations: load(&self.ssh_invalidations),
            last_render_tick_ns: load(&self.last_render_tick_ns),
            last_render_focused: self.last_render_focused.load(Ordering::Acquire),
            view_visible: self.view_visible.load(Ordering::Acquire),
        }
    }
}

fn add(target: &AtomicU64, value: u64) {
    let _ = target.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
        Some(current.saturating_add(value))
    });
}

fn record_duration(total: &AtomicU64, maximum: &AtomicU64, duration: Duration) {
    let nanos = duration_ns(duration);
    add(total, nanos);
    maximum.fetch_max(nanos, Ordering::Relaxed);
}

fn load(value: &AtomicU64) -> u64 {
    value.load(Ordering::Relaxed)
}

fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn duration_ns(duration: Duration) -> u64 {
    duration.as_nanos().min(u128::from(u64::MAX)) as u64
}

fn delta(current: u64, previous: u64) -> u64 {
    current.saturating_sub(previous)
}

fn rate(value: u64, elapsed: Duration) -> f64 {
    let seconds = elapsed.as_secs_f64();
    if seconds == 0.0 {
        0.0
    } else {
        value as f64 / seconds
    }
}

fn average(total: u64, samples: u64) -> f64 {
    if samples == 0 {
        0.0
    } else {
        total as f64 / samples as f64
    }
}

fn monotonic_now() -> Duration {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed()
}

fn encode_tick(value: Duration) -> u64 {
    duration_ns(value).saturating_add(1)
}
