//! Owner-thread retirement queue for Windows native RDP adapters.
//!
//! # Why this exists
//!
//! When a close or an initialization cleanup cannot confirm native destruction
//! before its deadline, the previous implementation called `Box::leak` and gave
//! up on the adapter forever. That is safe (pending COM callbacks must never
//! observe a freed host) but it loses *manageable* ownership: the tab is gone
//! from the UI while the native host and its overlay window stay alive for the
//! rest of the process. Repeated failures therefore accumulate.
//!
//! This module replaces "leak immediately at the deadline" with a bounded
//! owner-thread retirement queue:
//!
//! 1. A deadline failure hands the adapter to the queue instead of leaking it.
//! 2. A foreground driver re-attempts `force_close` once per retry interval, on
//!    the owner/UI thread, with **no** `App` borrow held while native COM/Win32
//!    calls run (phases A/B/C mirror the existing borrow-free close runner).
//! 3. An adapter that exhausts the grace period, or that is still queued when
//!    the application quits, is leaked as a last resort and counted
//!    (`retired_leaked_at_exit`) so it cannot be silently mistaken for a clean
//!    release.
//!
//! # Deliberate non-goals
//!
//! * The queue never runs `Drop` on a possibly callback-busy adapter, and it
//!   never destroys anything off the owner thread.
//! * The application-quit fallback performs **no** native cleanup. GPUI has
//!   already committed to quitting by then, and the existing shutdown contract
//!   forbids native calls in that final fallback. It only accounts for, and
//!   keeps alive, whatever the retry loop could not destroy.
//!
//! # Testability
//!
//! The policy and the queue bookkeeping are plain data structures with no
//! Windows dependency, so they are unit-tested on every platform. The GPUI glue
//! and the native adapter type are Windows-only.

use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Pure retirement policy and queue (platform independent, unit-tested)
// ---------------------------------------------------------------------------

/// How often the owner thread re-attempts destruction of retired adapters.
const RETIREMENT_RETRY_INTERVAL: Duration = Duration::from_secs(1);

/// Total grace period before a retired adapter is considered unrecoverable.
///
/// Long enough for a stalled COM callback to drain on its own; short enough
/// that a genuinely dead adapter does not sit in the queue for a whole session.
const RETIREMENT_MAX_AGE: Duration = Duration::from_secs(60);

/// Bounded retry policy for the retirement queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RetirementPolicy {
    /// Minimum delay between two destroy attempts for the same adapter.
    pub(crate) retry_interval: Duration,
    /// Total time an adapter may stay in the queue before it is given up on.
    pub(crate) max_age: Duration,
}

impl Default for RetirementPolicy {
    fn default() -> Self {
        Self {
            retry_interval: RETIREMENT_RETRY_INTERVAL,
            max_age: RETIREMENT_MAX_AGE,
        }
    }
}

/// One adapter (or, in tests, one stand-in) held by the retirement queue.
#[derive(Debug)]
pub(crate) struct Retired<T> {
    /// The retired resource.
    pub(crate) payload: T,
    /// Native generation, for correlated logs.
    pub(crate) generation: u64,
    /// Why the adapter entered the queue.
    pub(crate) reason: &'static str,
    /// When the adapter was retired.
    pub(crate) retired_at: Instant,
    /// How many destroy attempts already ran.
    pub(crate) attempts: u32,
    next_attempt_at: Instant,
}

impl<T> Retired<T> {
    pub(crate) fn new(payload: T, generation: u64, reason: &'static str, now: Instant) -> Self {
        Self {
            payload,
            generation,
            reason,
            retired_at: now,
            attempts: 0,
            // The first attempt is immediate: the adapter usually enters the
            // queue right after a `PendingCallbacks` result, which can clear on
            // its own within milliseconds.
            next_attempt_at: now,
        }
    }

    /// Whether a destroy attempt is due now.
    pub(crate) fn is_due(&self, now: Instant) -> bool {
        now >= self.next_attempt_at
    }

    /// Whether the adapter exhausted its bounded grace period.
    pub(crate) fn is_expired(&self, now: Instant, policy: RetirementPolicy) -> bool {
        now.saturating_duration_since(self.retired_at) >= policy.max_age
    }

    /// Records one attempt and schedules the next one.
    pub(crate) fn note_attempt(&mut self, now: Instant, policy: RetirementPolicy) {
        self.attempts = self.attempts.saturating_add(1);
        self.next_attempt_at = now + policy.retry_interval;
    }
}

/// Bounded queue of retired resources awaiting owner-thread destruction.
#[derive(Debug)]
pub(crate) struct RetirementQueue<T> {
    entries: Vec<Retired<T>>,
}

/// Deliberately hand-written: `#[derive(Default)]` would add a `T: Default`
/// bound, and the queued payload (a live Windows native adapter) has no
/// meaningful default.
impl<T> Default for RetirementQueue<T> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl<T> RetirementQueue<T> {
    pub(crate) fn push(&mut self, retired: Retired<T>) {
        self.entries.push(retired);
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Removes and returns every entry whose retry interval elapsed.
    ///
    /// Expired entries are deliberately left in place: they are handled by
    /// [`Self::take_expired`] so that an adapter is never both retried and
    /// given up on in the same pass.
    pub(crate) fn take_due(&mut self, now: Instant, policy: RetirementPolicy) -> Vec<Retired<T>> {
        let mut due = Vec::new();
        let mut kept = Vec::with_capacity(self.entries.len());
        for mut entry in self.entries.drain(..) {
            if entry.is_expired(now, policy) {
                kept.push(entry);
            } else if entry.is_due(now) {
                entry.note_attempt(now, policy);
                due.push(entry);
            } else {
                kept.push(entry);
            }
        }
        self.entries = kept;
        due
    }

    /// Puts an adapter that survived its destroy attempt back in the queue.
    pub(crate) fn requeue(&mut self, retired: Retired<T>) {
        self.entries.push(retired);
    }

    /// Removes and returns every entry that exhausted its grace period.
    pub(crate) fn take_expired(&mut self, now: Instant, policy: RetirementPolicy) -> Vec<Retired<T>> {
        let mut expired = Vec::new();
        let mut kept = Vec::with_capacity(self.entries.len());
        for entry in self.entries.drain(..) {
            if entry.is_expired(now, policy) {
                expired.push(entry);
            } else {
                kept.push(entry);
            }
        }
        self.entries = kept;
        expired
    }

    /// Removes and returns every entry, regardless of state.
    pub(crate) fn take_all(&mut self) -> Vec<Retired<T>> {
        std::mem::take(&mut self.entries)
    }
}
// ---------------------------------------------------------------------------
// Windows/GPUI glue
// ---------------------------------------------------------------------------

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
mod platform {
    use std::time::Instant;

    use gpui::{App, BorrowAppContext as _, Global};

    use super::{Retired, RetirementPolicy, RetirementQueue};
    use crate::view::windows_native::{NativeDestroyProgress, WindowsNativeAdapter};

    const POLICY: RetirementPolicy = RetirementPolicy {
        retry_interval: super::RETIREMENT_RETRY_INTERVAL,
        max_age: super::RETIREMENT_MAX_AGE,
    };

    /// The retired resource: a complete adapter that could not be closed yet.
    pub(crate) struct PendingRetiredAdapter {
        native: WindowsNativeAdapter,
    }

    impl PendingRetiredAdapter {
        /// One bounded owner-thread destroy attempt.
        fn attempt_destroy(&mut self) -> Result<(), anyhow::Error> {
            let mut focus_parent = || {};
            match self.native.force_close(&mut focus_parent) {
                Ok(NativeDestroyProgress::Destroyed) => Ok(()),
                Ok(NativeDestroyProgress::PendingCallbacks) => {
                    Err(anyhow::anyhow!("native callbacks are still in flight"))
                }
                // Mirrors the close runner: a native error whose host already
                // reports itself closed has still released the resource.
                Err(error) => {
                    if self.native.is_destroyed() {
                        Ok(())
                    } else {
                        Err(error)
                    }
                }
            }
        }

        fn into_adapter(self) -> WindowsNativeAdapter {
            self.native
        }
    }

    /// Result of one retirement poll, used to decide whether to keep the driver
    /// alive.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum PollOutcome {
        /// Adapters remain queued; wait for the next retry interval.
        Pending,
        /// The queue is empty and the driver should stop.
        Empty,
        /// The shutdown controller disappeared; there is nowhere to requeue to.
        ControllerGone,
    }

    pub(crate) struct GlobalWindowsNativeRdpRetirement {
        queue: RetirementQueue<PendingRetiredAdapter>,
        /// Whether a driver task is already polling this queue.
        ///
        /// The flag is set inside the same `update_global` that pushes a new
        /// adapter, and cleared inside the same `update_global` that observes an
        /// empty queue, so a concurrent `retire` can never be left without a
        /// driver.
        driver_running: bool,
    }

    impl Default for GlobalWindowsNativeRdpRetirement {
        fn default() -> Self {
            Self {
                queue: RetirementQueue::default(),
                driver_running: false,
            }
        }
    }

    impl Global for GlobalWindowsNativeRdpRetirement {}

    /// Installs the retirement controller and its application-quit accounting.
    pub(crate) fn init(cx: &mut App) {
        if cx.has_global::<GlobalWindowsNativeRdpRetirement>() {
            return;
        }
        cx.set_global(GlobalWindowsNativeRdpRetirement::default());
        cx.on_app_quit(|cx| {
            finalize_for_app_quit(cx);
            async {}
        })
        .detach();
    }

    /// Hands a close-timeout adapter to the owner-thread retirement queue.
    ///
    /// Never drops the adapter: a pending native callback must never observe a
    /// freed host, so the fallback is to keep it alive and count the leak.
    pub(crate) fn retire(
        cx: &gpui::AsyncApp,
        native: WindowsNativeAdapter,
        generation: u64,
        reason: &'static str,
    ) {
        if cx
            .try_read_global::<GlobalWindowsNativeRdpRetirement, _>(|_, _| ())
            .is_none()
        {
            leak_unretirable(native, generation, reason, "no_retirement_controller");
            return;
        }
        let now = Instant::now();
        // `spawn` must stay outside this closure: `AsyncApp::update_global`
        // already holds the app borrow, and spawning inside it would re-enter
        // that borrow.
        let start_driver = cx.update_global::<GlobalWindowsNativeRdpRetirement, _>(
            |retirement, _| {
                retirement.queue.push(Retired::new(
                    PendingRetiredAdapter { native },
                    generation,
                    reason,
                    now,
                ));
                if retirement.driver_running {
                    false
                } else {
                    retirement.driver_running = true;
                    true
                }
            },
        );
        if start_driver {
            cx.spawn(async move |cx| drive_retirement(cx).await).detach();
        }
        windows_rdp_host::lifecycle_counters().record_adapter_retired();
        windows_rdp_host::lifecycle_counters().log_snapshot("retirement", "adapter_retired");
    }

    /// Number of adapters currently awaiting destruction.
    pub(crate) fn pending_count(cx: &gpui::AsyncApp) -> usize {
        cx.try_read_global::<GlobalWindowsNativeRdpRetirement, _>(|retirement, _| {
            retirement.queue.len()
        })
        .unwrap_or(0)
    }

    /// Low-frequency owner-thread retry driver.
    ///
    /// Detached on purpose: it owns no handle that a later `retire` could
    /// replace, and it exits by itself once the queue drains.
    async fn drive_retirement(cx: &mut gpui::AsyncApp) {
        loop {
            match poll_retirement(cx).await {
                PollOutcome::Pending => {
                    cx.background_executor().timer(POLICY.retry_interval).await;
                }
                PollOutcome::Empty => {
                    let keep_running = cx.update_global::<GlobalWindowsNativeRdpRetirement, _>(
                        |retirement, _| {
                            if retirement.queue.is_empty() {
                                retirement.driver_running = false;
                                false
                            } else {
                                true
                            }
                        },
                    );
                    if !keep_running {
                        windows_rdp_host::lifecycle_counters()
                            .log_snapshot("retirement", "driver_stopped");
                        return;
                    }
                }
                PollOutcome::ControllerGone => return,
            }
        }
    }

    /// One retirement pass.
    ///
    /// Phase A takes adapters out of the global with pure moves, phase B runs
    /// the native close state machine **after** the borrow is released, and
    /// phase C puts survivors back. This mirrors the borrow-free close runner:
    /// `force_close` pumps Win32/COM messages and must never run while an `App`
    /// or entity borrow is held.
    async fn poll_retirement(cx: &mut gpui::AsyncApp) -> PollOutcome {
        let now = Instant::now();
        if cx
            .try_read_global::<GlobalWindowsNativeRdpRetirement, _>(|_, _| ())
            .is_none()
        {
            return PollOutcome::ControllerGone;
        }
        let (expired, due, remaining) = cx.update_global::<GlobalWindowsNativeRdpRetirement, _>(
            |retirement, _| {
                let expired = retirement.queue.take_expired(now, POLICY);
                let due = retirement.queue.take_due(now, POLICY);
                (expired, due, retirement.queue.len())
            },
        );

        for retired in expired {
            leak_unretirable(
                retired.payload.into_adapter(),
                retired.generation,
                retired.reason,
                "grace_period_exhausted",
            );
        }

        if due.is_empty() {
            return if remaining == 0 {
                PollOutcome::Empty
            } else {
                PollOutcome::Pending
            };
        }

        // Phase B: no borrow is held across native work.
        let mut survivors = Vec::new();
        for mut retired in due {
            let generation = retired.generation;
            let attempts = retired.attempts;
            match retired.payload.attempt_destroy() {
                Ok(()) => {
                    windows_rdp_host::lifecycle_counters().record_retired_destroyed();
                    tracing::info!(
                        target: "remote_desktop_view::lifecycle",
                        stage = "retired_adapter_destroyed",
                        generation,
                        attempts,
                        "destroyed a retired Windows native RDP adapter"
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        target: "remote_desktop_view::lifecycle",
                        stage = "retired_adapter_retry_failed",
                        generation,
                        attempts,
                        ?error,
                        "failed to destroy a retired Windows native RDP adapter; retrying"
                    );
                    survivors.push(retired);
                }
            }
        }

        if survivors.is_empty() {
            return if remaining == 0 {
                PollOutcome::Empty
            } else {
                PollOutcome::Pending
            };
        }

        // Phase C: put survivors back, or account for them if the controller is
        // gone (e.g. because GPUI already began quitting).
        if cx
            .try_read_global::<GlobalWindowsNativeRdpRetirement, _>(|_, _| ())
            .is_none()
        {
            for retired in survivors {
                leak_unretirable(
                    retired.payload.into_adapter(),
                    retired.generation,
                    retired.reason,
                    "retirement_controller_lost",
                );
            }
            return PollOutcome::ControllerGone;
        }
        cx.update_global::<GlobalWindowsNativeRdpRetirement, _>(|retirement, _| {
            for retired in survivors {
                retirement.queue.requeue(retired);
            }
        });
        PollOutcome::Pending
    }

    /// Final, bounded accounting for adapters still queued at application quit.
    ///
    /// This performs **no** native cleanup: GPUI has already committed to
    /// quitting, and the shutdown contract explicitly forbids owner-thread
    /// native calls in the late platform-quit fallback. Surviving adapters are
    /// kept alive (never dropped) and counted, so the leak is visible in the
    /// diagnostics instead of looking like a clean release.
    ///
    /// Returns the number of adapters given up on.
    pub(crate) fn finalize_for_app_quit(cx: &mut App) -> usize {
        if !cx.has_global::<GlobalWindowsNativeRdpRetirement>() {
            return 0;
        }
        let pending = cx.update_global::<GlobalWindowsNativeRdpRetirement, _>(
            |retirement, _| {
                retirement.driver_running = false;
                retirement.queue.take_all()
            },
        );
        if pending.is_empty() {
            return 0;
        }
        let count = pending.len();
        tracing::error!(
            target: "remote_desktop_view::lifecycle",
            stage = "retirement_app_exit",
            pending = count,
            "Windows native RDP adapters are still retired at application exit; \
             keeping them alive instead of dropping a possibly callback-busy host"
        );
        for retired in pending {
            leak_unretirable(
                retired.payload.into_adapter(),
                retired.generation,
                retired.reason,
                "app_exit",
            );
        }
        windows_rdp_host::lifecycle_counters().log_snapshot("retirement", "app_exit_leaked");
        count
    }

    /// Last-resort leak: keeps the native resource alive and counts it.
    fn leak_unretirable(
        native: WindowsNativeAdapter,
        generation: u64,
        reason: &'static str,
        stage: &'static str,
    ) {
        windows_rdp_host::lifecycle_counters().record_retired_leaked_at_exit();
        tracing::error!(
            target: "remote_desktop_view::lifecycle",
            stage,
            generation,
            reason,
            "leaking a Windows native RDP adapter that could not be destroyed"
        );
        let _ = Box::leak(Box::new(native));
    }
}

#[cfg(all(feature = "windows-native-rdp", target_os = "windows"))]
pub(crate) use platform::{init, pending_count, retire};

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{Retired, RetirementPolicy, RetirementQueue};

    fn policy() -> RetirementPolicy {
        RetirementPolicy {
            retry_interval: Duration::from_secs(1),
            max_age: Duration::from_secs(10),
        }
    }

    fn retired(payload: u32, now: Instant) -> Retired<u32> {
        Retired::new(payload, u64::from(payload), "test", now)
    }

    #[test]
    fn first_attempt_is_immediate_and_later_attempts_wait_for_the_interval() {
        let now = Instant::now();
        let entry = retired(1, now);
        assert!(entry.is_due(now), "retiring must schedule an immediate retry");
        assert!(!entry.is_expired(now, policy()));

        let mut queue = RetirementQueue::default();
        queue.push(entry);

        let due = queue.take_due(now, policy());
        assert_eq!(1, due.len());
        assert_eq!(1, due[0].attempts);
        assert!(queue.is_empty(), "a taken entry must leave the queue");

        queue.requeue(due.into_iter().next().unwrap());
        assert!(
            queue.take_due(now, policy()).is_empty(),
            "the retry interval must gate the next attempt"
        );
        let later = now + Duration::from_secs(1);
        assert_eq!(1, queue.take_due(later, policy()).len());
    }

    #[test]
    fn expired_entries_are_never_retried_and_are_handed_back_separately() {
        let now = Instant::now();
        let mut queue = RetirementQueue::default();
        queue.push(retired(1, now));

        let expired_at = now + Duration::from_secs(10);
        assert!(
            queue.take_due(expired_at, policy()).is_empty(),
            "an expired adapter must not consume another retry"
        );
        let expired = queue.take_expired(expired_at, policy());
        assert_eq!(1, expired.len());
        assert_eq!(1, expired[0].payload);
        assert!(queue.is_empty());
    }

    #[test]
    fn take_all_drains_every_entry_regardless_of_state() {
        let now = Instant::now();
        let mut queue = RetirementQueue::default();
        queue.push(retired(1, now));
        queue.push(retired(2, now + Duration::from_secs(5)));
        assert_eq!(2, queue.len());

        let all = queue.take_all();
        assert_eq!(2, all.len());
        assert!(queue.is_empty());
        assert_eq!(0, queue.len());
    }

    #[test]
    fn requeued_entries_keep_their_attempt_count_and_retirement_time() {
        let now = Instant::now();
        let mut queue = RetirementQueue::default();
        queue.push(retired(1, now));

        // `take_due` already records the attempt, so the popped entry is at 1.
        let entry = queue.take_due(now, policy()).pop().unwrap();
        assert_eq!(1, entry.attempts);
        let retired_at = entry.retired_at;
        queue.requeue(entry);

        assert!(
            queue.take_due(now, policy()).is_empty(),
            "the retry interval must survive a requeue"
        );

        let later = now + Duration::from_secs(1);
        let entry = queue.take_due(later, policy()).pop().unwrap();
        assert_eq!(2, entry.attempts, "requeueing must not reset the attempt count");
        assert_eq!(
            retired_at, entry.retired_at,
            "requeueing must not reset the grace period clock"
        );
        assert!(!entry.is_expired(later, policy()));
    }
}
