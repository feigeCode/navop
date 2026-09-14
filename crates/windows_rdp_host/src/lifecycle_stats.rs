//! Process-lifetime lifecycle counters for the Windows native RDP host.
//!
//! These counters exist because the Rust facade's `Drop` is **not** evidence
//! that the native resource was released. Several teardown paths deliberately
//! quarantine a native object to keep pending COM callbacks away from freed
//! memory:
//!
//! * the close/drain deadline leaks or retires a whole adapter,
//! * `WindowsRdpHost::drop` on the wrong thread keeps the native object,
//! * the Navop overlay is abandoned when its host parent must stay alive.
//!
//! Counting *live*, *destroyed* and *quarantined* objects separately is what
//! makes that difference observable, so a memory-growth report can distinguish
//! "the tab closed" from "the native resource actually went away".
//!
//! The counters are plain atomics with no Windows dependency, so they also
//! compile and are unit-testable on every platform.

use std::sync::atomic::{AtomicU64, Ordering};

/// A point-in-time copy of the process lifetime counters.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WindowsRdpLifecycleSnapshot {
    /// Native RDP host handles allocated by [`crate::WindowsRdpHost::create`].
    pub hosts_created: u64,
    /// Native RDP host handles whose native destroy call returned success.
    pub hosts_destroyed: u64,
    /// Native host close/destroy attempts that returned an error.
    pub host_close_failures: u64,
    /// Native host objects kept alive because `Drop` ran on a foreign thread.
    pub wrong_thread_drops: u64,
    /// Navop overlay windows created.
    pub overlays_created: u64,
    /// Navop overlay windows confirmed destroyed.
    pub overlays_destroyed: u64,
    /// Overlay destroy attempts that failed (the window may still be alive).
    pub overlay_destroy_failures: u64,
    /// Overlay windows deliberately leaked to preserve a live host parent.
    pub overlays_abandoned: u64,
    /// Adapters handed to the owner-thread retirement queue.
    pub adapters_retired: u64,
    /// Retired adapters that the retirement queue later destroyed.
    pub retired_destroyed: u64,
    /// Retired adapters finally leaked when the bounded retirement budget ran out.
    pub retired_leaked_at_exit: u64,
}

impl WindowsRdpLifecycleSnapshot {
    /// Native hosts that have not reported a confirmed destroy yet.
    ///
    /// Quarantined (retired) adapters still own a live host, so they stay
    /// counted here on purpose.
    pub const fn live_hosts(&self) -> u64 {
        self.hosts_created.saturating_sub(self.hosts_destroyed)
    }

    /// Overlay windows that are neither destroyed nor knowingly abandoned.
    pub const fn live_overlays(&self) -> u64 {
        self.overlays_created
            .saturating_sub(self.overlays_destroyed)
            .saturating_sub(self.overlays_abandoned)
    }

    /// Retired adapters the queue is still responsible for destroying.
    pub const fn quarantined_adapters(&self) -> u64 {
        self.adapters_retired
            .saturating_sub(self.retired_destroyed)
            .saturating_sub(self.retired_leaked_at_exit)
    }

    /// Whether any native resource is knowingly still alive.
    pub const fn has_quarantined_resources(&self) -> bool {
        self.quarantined_adapters() != 0
            || self.wrong_thread_drops != 0
            || self.overlays_abandoned != 0
    }
}

/// Mutable counter set. The production build uses [`global`]; tests build local
/// instances so parallel tests cannot observe each other's arithmetic.
#[derive(Debug, Default)]
pub struct WindowsRdpLifecycleCounters {
    hosts_created: AtomicU64,
    hosts_destroyed: AtomicU64,
    host_close_failures: AtomicU64,
    wrong_thread_drops: AtomicU64,
    overlays_created: AtomicU64,
    overlays_destroyed: AtomicU64,
    overlay_destroy_failures: AtomicU64,
    overlays_abandoned: AtomicU64,
    adapters_retired: AtomicU64,
    retired_destroyed: AtomicU64,
    retired_leaked_at_exit: AtomicU64,
}

impl WindowsRdpLifecycleCounters {
    /// Creates a zeroed counter set.
    pub const fn new() -> Self {
        Self {
            hosts_created: AtomicU64::new(0),
            hosts_destroyed: AtomicU64::new(0),
            host_close_failures: AtomicU64::new(0),
            wrong_thread_drops: AtomicU64::new(0),
            overlays_created: AtomicU64::new(0),
            overlays_destroyed: AtomicU64::new(0),
            overlay_destroy_failures: AtomicU64::new(0),
            overlays_abandoned: AtomicU64::new(0),
            adapters_retired: AtomicU64::new(0),
            retired_destroyed: AtomicU64::new(0),
            retired_leaked_at_exit: AtomicU64::new(0),
        }
    }

    fn bump(counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }

    /// Records one native host handle allocation.
    pub fn record_host_created(&self) {
        Self::bump(&self.hosts_created);
    }

    /// Records one confirmed native host destroy.
    pub fn record_host_destroyed(&self) {
        Self::bump(&self.hosts_destroyed);
    }

    /// Records one failed native host close/destroy attempt.
    pub fn record_host_close_failed(&self) {
        Self::bump(&self.host_close_failures);
    }

    /// Records one native host kept alive because it was dropped off-owner-thread.
    pub fn record_wrong_thread_drop(&self) {
        Self::bump(&self.wrong_thread_drops);
    }

    /// Records one Navop overlay window creation.
    pub fn record_overlay_created(&self) {
        Self::bump(&self.overlays_created);
    }

    /// Records one confirmed Navop overlay window destroy.
    pub fn record_overlay_destroyed(&self) {
        Self::bump(&self.overlays_destroyed);
    }

    /// Records one failed Navop overlay window destroy attempt.
    pub fn record_overlay_destroy_failed(&self) {
        Self::bump(&self.overlay_destroy_failures);
    }

    /// Records one Navop overlay window deliberately leaked.
    pub fn record_overlay_abandoned(&self) {
        Self::bump(&self.overlays_abandoned);
    }

    /// Records one adapter handed to the owner-thread retirement queue.
    pub fn record_adapter_retired(&self) {
        Self::bump(&self.adapters_retired);
    }

    /// Records one retired adapter that the queue eventually destroyed.
    pub fn record_retired_destroyed(&self) {
        Self::bump(&self.retired_destroyed);
    }

    /// Records one retired adapter finally leaked after the bounded budget ran out.
    pub fn record_retired_leaked_at_exit(&self) {
        Self::bump(&self.retired_leaked_at_exit);
    }

    /// Returns a consistent-enough snapshot for diagnostics.
    ///
    /// The counters are only read for reporting, so `Relaxed` ordering is
    /// intentional: a slightly stale read is preferable to adding fences to hot
    /// teardown paths.
    pub fn snapshot(&self) -> WindowsRdpLifecycleSnapshot {
        WindowsRdpLifecycleSnapshot {
            hosts_created: self.hosts_created.load(Ordering::Relaxed),
            hosts_destroyed: self.hosts_destroyed.load(Ordering::Relaxed),
            host_close_failures: self.host_close_failures.load(Ordering::Relaxed),
            wrong_thread_drops: self.wrong_thread_drops.load(Ordering::Relaxed),
            overlays_created: self.overlays_created.load(Ordering::Relaxed),
            overlays_destroyed: self.overlays_destroyed.load(Ordering::Relaxed),
            overlay_destroy_failures: self.overlay_destroy_failures.load(Ordering::Relaxed),
            overlays_abandoned: self.overlays_abandoned.load(Ordering::Relaxed),
            adapters_retired: self.adapters_retired.load(Ordering::Relaxed),
            retired_destroyed: self.retired_destroyed.load(Ordering::Relaxed),
            retired_leaked_at_exit: self.retired_leaked_at_exit.load(Ordering::Relaxed),
        }
    }

    /// Emits one low-frequency lifecycle line at a stable `stage`.
    ///
    /// Callers pass lifecycle boundaries (close timeout, retirement drain, app
    /// quit) rather than per-frame events, so this stays usable in a release
    /// build without flooding the log.
    pub fn log_snapshot(&self, scope: &'static str, stage: &'static str) {
        let snapshot = self.snapshot();
        tracing::info!(
            target: "windows_rdp_host::lifecycle",
            scope,
            stage,
            hosts_created = snapshot.hosts_created,
            hosts_destroyed = snapshot.hosts_destroyed,
            live_hosts = snapshot.live_hosts(),
            host_close_failures = snapshot.host_close_failures,
            wrong_thread_drops = snapshot.wrong_thread_drops,
            overlays_created = snapshot.overlays_created,
            overlays_destroyed = snapshot.overlays_destroyed,
            live_overlays = snapshot.live_overlays(),
            overlay_destroy_failures = snapshot.overlay_destroy_failures,
            overlays_abandoned = snapshot.overlays_abandoned,
            adapters_retired = snapshot.adapters_retired,
            retired_destroyed = snapshot.retired_destroyed,
            retired_leaked_at_exit = snapshot.retired_leaked_at_exit,
            quarantined_adapters = snapshot.quarantined_adapters(),
            "Windows native RDP lifecycle counters"
        );
    }
}

static GLOBAL_LIFECYCLE_COUNTERS: WindowsRdpLifecycleCounters = WindowsRdpLifecycleCounters::new();

/// Returns the process-wide counter set used by the production teardown paths.
pub fn global() -> &'static WindowsRdpLifecycleCounters {
    &GLOBAL_LIFECYCLE_COUNTERS
}

#[cfg(test)]
mod tests {
    use super::WindowsRdpLifecycleCounters;

    #[test]
    fn live_and_quarantined_counts_track_each_terminal_path() {
        let counters = WindowsRdpLifecycleCounters::new();

        counters.record_host_created();
        counters.record_host_created();
        counters.record_overlay_created();
        counters.record_overlay_created();

        let snapshot = counters.snapshot();
        assert_eq!(2, snapshot.live_hosts());
        assert_eq!(2, snapshot.live_overlays());
        assert_eq!(0, snapshot.quarantined_adapters());
        assert!(!snapshot.has_quarantined_resources());

        // One host destroyed normally, one overlay abandoned.
        counters.record_host_destroyed();
        counters.record_overlay_abandoned();

        let snapshot = counters.snapshot();
        assert_eq!(1, snapshot.live_hosts());
        assert_eq!(1, snapshot.live_overlays());
        assert_eq!(1, snapshot.overlays_abandoned);
        assert!(snapshot.has_quarantined_resources());
    }

    #[test]
    fn retired_adapters_stay_counted_until_destroyed_or_leaked() {
        let counters = WindowsRdpLifecycleCounters::new();

        counters.record_host_created();
        counters.record_adapter_retired();
        assert_eq!(1, counters.snapshot().quarantined_adapters());
        // A retired adapter still owns a live host.
        assert_eq!(1, counters.snapshot().live_hosts());

        counters.record_host_destroyed();
        counters.record_retired_destroyed();
        let snapshot = counters.snapshot();
        assert_eq!(0, snapshot.quarantined_adapters());
        assert_eq!(0, snapshot.live_hosts());
        assert!(!snapshot.has_quarantined_resources());
    }

    #[test]
    fn retired_adapter_leaked_at_exit_leaves_no_pending_quarantine_but_stays_visible() {
        let counters = WindowsRdpLifecycleCounters::new();

        counters.record_host_created();
        counters.record_adapter_retired();
        counters.record_retired_leaked_at_exit();

        let snapshot = counters.snapshot();
        assert_eq!(0, snapshot.quarantined_adapters());
        assert_eq!(1, snapshot.retired_leaked_at_exit);
        // The host itself was never destroyed, so it remains live.
        assert_eq!(1, snapshot.live_hosts());
    }

    #[test]
    fn wrong_thread_drop_is_visible_without_inventing_a_destroy() {
        let counters = WindowsRdpLifecycleCounters::new();

        counters.record_host_created();
        counters.record_wrong_thread_drop();

        let snapshot = counters.snapshot();
        assert_eq!(1, snapshot.wrong_thread_drops);
        assert_eq!(1, snapshot.live_hosts());
        assert!(snapshot.has_quarantined_resources());
    }

    #[test]
    fn snapshot_never_underflows_on_out_of_order_records() {
        let counters = WindowsRdpLifecycleCounters::new();

        // Defensive: a stray terminal record must not panic in debug builds.
        counters.record_host_destroyed();
        counters.record_overlay_destroyed();
        counters.record_retired_destroyed();

        let snapshot = counters.snapshot();
        assert_eq!(0, snapshot.live_hosts());
        assert_eq!(0, snapshot.live_overlays());
        assert_eq!(0, snapshot.quarantined_adapters());
    }
}
