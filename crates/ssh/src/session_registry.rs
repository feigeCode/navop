//! Application-level single-flight slots and leases for shared SSH session
//! managers.
//!
//! This module deliberately stops before transport health, a GPUI global
//! owner, or migration of existing consumers.  It provides the
//! GPUI-independent application service and graceful-shutdown contract that
//! those later integrations build on:
//!
//! - one in-flight manager creation per [`ConnectionKey`];
//! - unrelated keys can make progress independently;
//! - manager creation never runs while the registry state lock is held;
//! - a result from a retired slot can never replace a newer generation;
//! - lease checkout/release is bound to the exact slot generation;
//! - the last release only marks an idle candidate and never disconnects;
//! - one registry-owned task reaps expired generations and disconnects them
//!   outside the registry lock.

use std::collections::HashMap;
use std::fmt;
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use tokio::sync::{Notify, oneshot, watch};
use tokio::task::{JoinError, JoinSet};
use tokio::time::{Instant, sleep_until, timeout_at};
use tokio_util::sync::CancellationToken;

use crate::{ConnectionKey, SshConnectConfig, SshSessionManager};

const DEFAULT_SESSION_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_SERVICE_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// Lifecycle of the application-owned SSH session service.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SshSessionServiceState {
    /// New leases and manager creation are admitted.
    Running,
    /// Admission is closed and registry-owned work is converging.
    ShuttingDown,
    /// The single shutdown attempt has produced its stable report.
    Stopped,
}

/// Credential-free lifecycle and capacity view of the SSH session service.
///
/// This intentionally contains only counters and the lifecycle state.  It
/// never exposes a [`ConnectionKey`], [`SshConnectConfig`], host path,
/// credential revision, password, private key, passphrase, or MFA response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SshSessionServiceSnapshot {
    /// Monotonic-within-this-service change counter (wrapping at `u64::MAX`).
    pub revision: u64,
    /// Current service lifecycle.
    pub state: SshSessionServiceState,
    /// Number of current creating and ready generations.
    pub slot_count: usize,
    /// Number of manager creation flights owned by current slots.
    pub creating_count: usize,
    /// Number of published manager generations owned by current slots.
    pub ready_count: usize,
    /// Leases charged to current published generations.
    ///
    /// Leases retained after a generation is retired or shutdown begins are
    /// intentionally not counted because they can no longer mutate registry
    /// accounting or reconnect their permanently gated manager.
    pub active_lease_count: usize,
    /// Ready generations with no active lease and an idle deadline.
    pub idle_count: usize,
    /// Registry-owned creation and reaper tasks that have not converged.
    pub registry_task_count: usize,
}

impl Default for SshSessionServiceSnapshot {
    fn default() -> Self {
        Self {
            revision: 0,
            state: SshSessionServiceState::Running,
            slot_count: 0,
            creating_count: 0,
            ready_count: 0,
            active_lease_count: 0,
            idle_count: 0,
            registry_task_count: 0,
        }
    }
}

/// Stable, credential-free result of application SSH service shutdown.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SshSessionShutdownReport {
    /// Whether the single total deadline elapsed before full convergence.
    pub timed_out: bool,
    /// Published manager generations whose reconnect gates were closed.
    pub managers_requested: usize,
    /// Manager cleanup futures completed before the deadline, including
    /// futures that completed with an error.
    pub managers_completed: usize,
    /// Completed manager cleanup futures that failed or whose task panicked.
    pub manager_failures: usize,
    /// Manager cleanup futures still pending when the deadline elapsed.
    pub managers_remaining: usize,
    /// Registry-owned creation/reaper tasks still pending at report time.
    pub registry_tasks_remaining: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SlotGeneration(u64);

#[derive(Clone)]
struct CreationToken {
    generation: SlotGeneration,
    identity: Arc<()>,
}

impl CreationToken {
    fn new(generation: SlotGeneration) -> Self {
        Self {
            generation,
            identity: Arc::new(()),
        }
    }

    fn is_same_flight(&self, other: &Self) -> bool {
        self.generation == other.generation && Arc::ptr_eq(&self.identity, &other.identity)
    }
}

#[derive(Clone)]
struct IdleCandidate {
    deadline: Instant,
    identity: Arc<()>,
}

impl IdleCandidate {
    fn new(idle_timeout: Duration) -> Self {
        let now = Instant::now();
        Self {
            deadline: now
                .checked_add(idle_timeout)
                .expect("idle timeout was validated when the SSH registry was constructed"),
            identity: Arc::new(()),
        }
    }

    fn is_same_candidate(&self, other: &Self) -> bool {
        self.deadline == other.deadline && Arc::ptr_eq(&self.identity, &other.identity)
    }
}

enum FlightOutcome {
    Published,
    Failed(Arc<str>),
    Superseded,
}

enum RegistrySlot<M> {
    Creating {
        token: CreationToken,
        waiters: Vec<oneshot::Sender<FlightOutcome>>,
    },
    Ready {
        key: Arc<ConnectionKey>,
        token: CreationToken,
        manager: Arc<M>,
        lease_count: usize,
        idle_candidate: Option<IdleCandidate>,
    },
}

struct RegistryState<M> {
    slots: HashMap<ConnectionKey, RegistrySlot<M>>,
    next_generation: u64,
    lifecycle: SshSessionServiceState,
    snapshot_revision: u64,
}

impl<M> Default for RegistryState<M> {
    fn default() -> Self {
        Self {
            slots: HashMap::new(),
            next_generation: 0,
            lifecycle: SshSessionServiceState::Running,
            snapshot_revision: 0,
        }
    }
}

impl<M> RegistryState<M> {
    fn next_token(&mut self) -> CreationToken {
        self.next_generation = self.next_generation.wrapping_add(1).max(1);
        CreationToken::new(SlotGeneration(self.next_generation))
    }

    fn take_current_creation(
        &mut self,
        key: &ConnectionKey,
        expected: &CreationToken,
    ) -> Option<Vec<oneshot::Sender<FlightOutcome>>> {
        let is_current = matches!(
            self.slots.get(key),
            Some(RegistrySlot::Creating { token, .. }) if token.is_same_flight(expected)
        );
        if !is_current {
            return None;
        }

        match self.slots.remove(key) {
            Some(RegistrySlot::Creating { waiters, .. }) => Some(waiters),
            Some(RegistrySlot::Ready { .. }) | None => {
                unreachable!("slot shape changed while the registry lock was held")
            }
        }
    }
}

struct RegistryShared<M> {
    state: StdMutex<RegistryState<M>>,
    idle_timeout: Duration,
    reaper_notify: Notify,
    reaper_started: AtomicBool,
    reaper_running: AtomicBool,
    reaper_shutdown: AtomicBool,
    reaper_start_count: AtomicUsize,
    reaper_stop_count: AtomicUsize,
    task_cancellation: CancellationToken,
    task_count: AtomicUsize,
    task_notify: Notify,
    snapshot_tx: watch::Sender<SshSessionServiceSnapshot>,
}

impl<M> RegistryShared<M> {
    fn new(idle_timeout: Duration) -> Self {
        let state = RegistryState::default();
        let (snapshot_tx, _) = watch::channel(SshSessionServiceSnapshot::default());
        Self {
            state: StdMutex::new(state),
            idle_timeout,
            reaper_notify: Notify::new(),
            reaper_started: AtomicBool::new(false),
            reaper_running: AtomicBool::new(false),
            reaper_shutdown: AtomicBool::new(false),
            reaper_start_count: AtomicUsize::new(0),
            reaper_stop_count: AtomicUsize::new(0),
            task_cancellation: CancellationToken::new(),
            task_count: AtomicUsize::new(0),
            task_notify: Notify::new(),
            snapshot_tx,
        }
    }

    fn idle_timeout(&self) -> Duration {
        self.idle_timeout
    }

    fn snapshot_from_state(&self, state: &RegistryState<M>) -> SshSessionServiceSnapshot {
        let mut creating_count = 0;
        let mut ready_count = 0;
        let mut active_lease_count = 0;
        let mut idle_count = 0;

        for slot in state.slots.values() {
            match slot {
                RegistrySlot::Creating { .. } => creating_count += 1,
                RegistrySlot::Ready {
                    lease_count,
                    idle_candidate,
                    ..
                } => {
                    ready_count += 1;
                    active_lease_count += *lease_count;
                    if *lease_count == 0 && idle_candidate.is_some() {
                        idle_count += 1;
                    }
                }
            }
        }

        SshSessionServiceSnapshot {
            revision: state.snapshot_revision,
            state: state.lifecycle,
            slot_count: state.slots.len(),
            creating_count,
            ready_count,
            active_lease_count,
            idle_count,
            registry_task_count: self.task_count.load(Ordering::Acquire),
        }
    }

    fn publish_snapshot_locked(&self, state: &mut RegistryState<M>) {
        state.snapshot_revision = state.snapshot_revision.wrapping_add(1);
        self.snapshot_tx
            .send_replace(self.snapshot_from_state(state));
    }

    fn snapshot(&self) -> SshSessionServiceSnapshot {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        self.snapshot_from_state(&state)
    }

    fn subscribe(&self) -> watch::Receiver<SshSessionServiceSnapshot> {
        self.snapshot_tx.subscribe()
    }

    fn register_task_locked(self: &Arc<Self>) -> RegistryTaskGuard<M> {
        self.task_count.fetch_add(1, Ordering::AcqRel);
        RegistryTaskGuard {
            shared: self.clone(),
            active: true,
        }
    }

    async fn wait_for_tasks(&self) {
        loop {
            let notified = self.task_notify.notified();
            if self.task_count.load(Ordering::Acquire) == 0 {
                return;
            }
            notified.await;
        }
    }
}

struct RegistryTaskGuard<M> {
    shared: Arc<RegistryShared<M>>,
    active: bool,
}

impl<M> Drop for RegistryTaskGuard<M> {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        self.active = false;
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let previous = self.shared.task_count.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "SSH registry task count underflow");
        self.shared.publish_snapshot_locked(&mut state);
        drop(state);
        // There is one shutdown driver waiting for registry convergence.
        // `notify_one` retains a permit if completion races its next await,
        // unlike `notify_waiters`, whose edge can be lost without a waiter.
        self.shared.task_notify.notify_one();
    }
}

#[async_trait]
trait SessionManagerFactory<M>: Send + Sync + 'static {
    async fn create(&self, config: SshConnectConfig) -> Result<M>;
}

#[async_trait]
trait RegistryManagedSession: Send + Sync + 'static {
    fn request_shutdown_for_registry(&self) {}

    async fn disconnect_for_registry(&self) -> Result<()>;

    async fn shutdown_for_registry(&self) -> Result<()> {
        self.disconnect_for_registry().await
    }
}

enum AcquirePhase<M> {
    Ready(RegistryLease<M>),
    Wait(oneshot::Receiver<FlightOutcome>),
    Start {
        token: CreationToken,
        receiver: oneshot::Receiver<FlightOutcome>,
        task_guard: RegistryTaskGuard<M>,
    },
}

/// One counted use of a manager from an exact registry slot generation.
///
/// A retired generation may remain alive through this lease's `Arc`, but its
/// release can never mutate the replacement generation for the same key.
struct RegistryLease<M> {
    shared: Arc<RegistryShared<M>>,
    key: Arc<ConnectionKey>,
    token: CreationToken,
    manager: Arc<M>,
    counted: bool,
}

impl<M> RegistryLease<M> {
    fn manager(&self) -> &M {
        &self.manager
    }

    fn release(&mut self) {
        if !self.counted {
            return;
        }
        // Clear this first so even an unexpected early return remains
        // idempotent and `Drop` never attempts a second decrement.
        self.counted = false;

        let became_idle = {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let Some(RegistrySlot::Ready {
                token,
                lease_count,
                idle_candidate,
                ..
            }) = state.slots.get_mut(self.key.as_ref())
            else {
                return;
            };
            if !token.is_same_flight(&self.token) || *lease_count == 0 {
                return;
            }

            *lease_count -= 1;
            let became_idle = if *lease_count == 0 {
                *idle_candidate = Some(IdleCandidate::new(self.shared.idle_timeout()));
                true
            } else {
                false
            };
            self.shared.publish_snapshot_locked(&mut state);
            became_idle
        };

        if became_idle {
            self.shared.reaper_notify.notify_one();
        }
    }
}

impl<M> Clone for RegistryLease<M> {
    fn clone(&self) -> Self {
        let (counted, cancelled_idle) = if self.counted {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            match state.slots.get_mut(self.key.as_ref()) {
                Some(RegistrySlot::Ready {
                    token,
                    lease_count,
                    idle_candidate,
                    ..
                }) if token.is_same_flight(&self.token) => {
                    *lease_count = lease_count
                        .checked_add(1)
                        .expect("SSH session lease count overflow");
                    let cancelled_idle = idle_candidate.take().is_some();
                    self.shared.publish_snapshot_locked(&mut state);
                    (true, cancelled_idle)
                }
                Some(RegistrySlot::Creating { .. }) | Some(RegistrySlot::Ready { .. }) | None => {
                    (false, false)
                }
            }
        } else {
            (false, false)
        };

        if cancelled_idle {
            self.shared.reaper_notify.notify_one();
        }

        Self {
            shared: self.shared.clone(),
            key: self.key.clone(),
            token: self.token.clone(),
            manager: self.manager.clone(),
            counted,
        }
    }
}

impl<M> Drop for RegistryLease<M> {
    fn drop(&mut self) {
        self.release();
    }
}

impl<M> Deref for RegistryLease<M> {
    type Target = M;

    fn deref(&self) -> &Self::Target {
        self.manager()
    }
}

impl<M> fmt::Debug for RegistryLease<M> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RegistryLease")
            .field("connection", &self.key.label())
            .field("generation", &self.token.generation.0)
            .field("active", &self.counted)
            .finish_non_exhaustive()
    }
}

struct SessionRegistryCore<M, F> {
    shared: Arc<RegistryShared<M>>,
    factory: Arc<F>,
}

impl<M, F> SessionRegistryCore<M, F>
where
    M: RegistryManagedSession,
    F: SessionManagerFactory<M>,
{
    fn new(factory: F) -> Self {
        Self::with_idle_timeout(factory, DEFAULT_SESSION_IDLE_TIMEOUT)
    }

    fn with_idle_timeout(factory: F, idle_timeout: Duration) -> Self {
        assert!(
            !idle_timeout.is_zero(),
            "SSH session idle timeout must be greater than zero"
        );
        assert!(
            Instant::now().checked_add(idle_timeout).is_some(),
            "SSH session idle timeout must fit in a Tokio Instant"
        );
        Self {
            shared: Arc::new(RegistryShared::new(idle_timeout)),
            factory: Arc::new(factory),
        }
    }

    /// Acquire the manager for `key`, starting one detached creation flight
    /// when the slot is absent.
    ///
    /// The detached flight is intentional: cancelling the first acquire must
    /// not strand the slot or cause every other waiter to redial.
    async fn acquire(
        self: &Arc<Self>,
        key: &ConnectionKey,
        config: SshConnectConfig,
    ) -> Result<RegistryLease<M>> {
        loop {
            let (phase, cancelled_idle, reaper_task) = {
                let mut state = self
                    .shared
                    .state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if state.lifecycle != SshSessionServiceState::Running {
                    return Err(anyhow!("SSH session service is shutting down"));
                }

                let (phase, cancelled_idle, changed) = match state.slots.get_mut(key) {
                    Some(RegistrySlot::Ready {
                        key: slot_key,
                        token,
                        manager,
                        lease_count,
                        idle_candidate,
                    }) => {
                        *lease_count = lease_count
                            .checked_add(1)
                            .ok_or_else(|| anyhow!("SSH session lease count overflow"))?;
                        let cancelled_idle = idle_candidate.take().is_some();
                        (
                            AcquirePhase::Ready(RegistryLease {
                                shared: self.shared.clone(),
                                key: slot_key.clone(),
                                token: token.clone(),
                                manager: manager.clone(),
                                counted: true,
                            }),
                            cancelled_idle,
                            true,
                        )
                    }
                    Some(RegistrySlot::Creating { waiters, .. }) => {
                        let (sender, receiver) = oneshot::channel();
                        waiters.push(sender);
                        (AcquirePhase::Wait(receiver), false, false)
                    }
                    None => {
                        let token = state.next_token();
                        let (sender, receiver) = oneshot::channel();
                        state.slots.insert(
                            key.clone(),
                            RegistrySlot::Creating {
                                token: token.clone(),
                                waiters: vec![sender],
                            },
                        );
                        (
                            AcquirePhase::Start {
                                token,
                                receiver,
                                task_guard: self.shared.register_task_locked(),
                            },
                            false,
                            true,
                        )
                    }
                };
                // Register the long-lived reaper only after all fallible lease
                // accounting above has completed.  A task guard's Drop takes
                // the state lock to publish its final snapshot, so it must
                // never be unwound while this same lock is still held.
                let reaper_task = if self
                    .shared
                    .reaper_started
                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    self.shared
                        .reaper_start_count
                        .fetch_add(1, Ordering::Relaxed);
                    Some(self.shared.register_task_locked())
                } else {
                    None
                };
                if changed || reaper_task.is_some() {
                    self.shared.publish_snapshot_locked(&mut state);
                }
                (phase, cancelled_idle, reaper_task)
            };

            if let Some(task_guard) = reaper_task {
                let shared = self.shared.clone();
                tokio::spawn(run_idle_reaper(shared, task_guard));
            }
            if cancelled_idle {
                self.shared.reaper_notify.notify_one();
            }

            let receiver = match phase {
                AcquirePhase::Ready(lease) => return Ok(lease),
                AcquirePhase::Wait(receiver) => receiver,
                AcquirePhase::Start {
                    token,
                    receiver,
                    task_guard,
                } => {
                    self.spawn_creation(key.clone(), token, config.clone(), task_guard);
                    receiver
                }
            };

            match receiver.await {
                Ok(FlightOutcome::Published) => {
                    // Checkout must happen while holding the registry lock so
                    // the lease count is charged to the exact generation that
                    // supplies the manager.  A cancelled waiter therefore
                    // never leaves a phantom lease.
                }
                Ok(FlightOutcome::Failed(message)) => {
                    return Err(anyhow!(message.to_string()));
                }
                Ok(FlightOutcome::Superseded) | Err(_) => {
                    // A retire/replacement raced this acquire.  Re-enter the
                    // map and join (or create) the current generation.
                }
            }
        }
    }

    fn spawn_creation(
        &self,
        key: ConnectionKey,
        token: CreationToken,
        config: SshConnectConfig,
        task_guard: RegistryTaskGuard<M>,
    ) {
        let shared = self.shared.clone();
        let factory = self.factory.clone();
        let cancellation = shared.task_cancellation.clone();
        tokio::spawn(async move {
            let _task_guard = task_guard;
            let mut cleanup = CreationCleanup::new(shared.clone(), key.clone(), token.clone());
            let result = tokio::select! {
                biased;
                _ = cancellation.cancelled() => return,
                result = factory.create(config) => result,
            };
            shared.finish_creation(&key, &token, result);
            cleanup.disarm();
        });
    }

    /// Retire only the currently published/in-flight slot.
    ///
    /// Explicit retirement remains a visibility-only operation and returns a
    /// published manager without disconnecting it.  Idle expiration is
    /// handled separately by the registry-owned reaper.  Waiters on an
    /// in-flight generation retry against the new map state instead of
    /// accepting its stale result.
    fn retire(&self, key: &ConnectionKey) -> Option<Arc<M>> {
        let (manager, waiters, removed_slot) = {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let removed = match state.slots.remove(key) {
                Some(RegistrySlot::Ready { manager, .. }) => (Some(manager), Vec::new(), true),
                Some(RegistrySlot::Creating { waiters, .. }) => (None, waiters, true),
                None => (None, Vec::new(), false),
            };
            if removed.2 {
                self.shared.publish_snapshot_locked(&mut state);
            }
            removed
        };
        if removed_slot {
            self.shared.reaper_notify.notify_one();
        }
        notify_waiters(waiters, FlightOutcome::Superseded);
        manager
    }

    fn begin_shutdown(&self) -> Vec<Arc<M>> {
        let (managers, waiters, started) = {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if state.lifecycle != SshSessionServiceState::Running {
                (Vec::new(), Vec::new(), false)
            } else {
                state.lifecycle = SshSessionServiceState::ShuttingDown;
                let mut managers = Vec::new();
                let mut waiters = Vec::new();
                for (_, slot) in state.slots.drain() {
                    match slot {
                        RegistrySlot::Creating {
                            waiters: slot_waiters,
                            ..
                        } => waiters.extend(slot_waiters),
                        RegistrySlot::Ready { manager, .. } => {
                            // Close every reconnect gate before releasing the
                            // registry state lock.  Acquire and retained-lease
                            // client checkout therefore have one synchronous
                            // shutdown linearization point.
                            manager.request_shutdown_for_registry();
                            managers.push(manager);
                        }
                    }
                }
                self.shared.publish_snapshot_locked(&mut state);
                (managers, waiters, true)
            }
        };
        if started {
            self.shared.reaper_shutdown.store(true, Ordering::Release);
            self.shared.task_cancellation.cancel();
            self.shared.reaper_notify.notify_waiters();
            notify_waiters(
                waiters,
                FlightOutcome::Failed(Arc::from("SSH session service is shutting down")),
            );
        }
        managers
    }

    fn finish_shutdown(&self) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.lifecycle != SshSessionServiceState::Stopped {
            state.lifecycle = SshSessionServiceState::Stopped;
            self.shared.publish_snapshot_locked(&mut state);
        }
    }

    fn snapshot(&self) -> SshSessionServiceSnapshot {
        self.shared.snapshot()
    }

    fn subscribe(&self) -> watch::Receiver<SshSessionServiceSnapshot> {
        self.shared.subscribe()
    }
}

impl<M, F> Drop for SessionRegistryCore<M, F> {
    fn drop(&mut self) {
        let waiters = {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.lifecycle = SshSessionServiceState::Stopped;
            let mut waiters = Vec::new();
            for (_, slot) in state.slots.drain() {
                if let RegistrySlot::Creating {
                    waiters: slot_waiters,
                    ..
                } = slot
                {
                    waiters.extend(slot_waiters);
                }
            }
            self.shared.publish_snapshot_locked(&mut state);
            waiters
        };
        self.shared.reaper_shutdown.store(true, Ordering::Release);
        self.shared.task_cancellation.cancel();
        self.shared.reaper_notify.notify_waiters();
        notify_waiters(
            waiters,
            FlightOutcome::Failed(Arc::from("SSH session registry was dropped")),
        );
    }
}

impl<M> RegistryShared<M> {
    fn finish_creation(&self, key: &ConnectionKey, token: &CreationToken, result: Result<M>) {
        let (waiters, outcome, published_idle) = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            let Some(waiters) = state.take_current_creation(key, token) else {
                // The slot was retired, shutdown drained it, or a newer
                // generation owns the key.  Dropping `result` prevents stale
                // publication.
                return;
            };

            let outcome = match result {
                Ok(manager) if state.lifecycle == SshSessionServiceState::Running => {
                    state.slots.insert(
                        key.clone(),
                        RegistrySlot::Ready {
                            key: Arc::new(key.clone()),
                            token: token.clone(),
                            manager: Arc::new(manager),
                            lease_count: 0,
                            idle_candidate: Some(IdleCandidate::new(self.idle_timeout())),
                        },
                    );
                    (waiters, FlightOutcome::Published, true)
                }
                Ok(_) => (
                    waiters,
                    FlightOutcome::Failed(Arc::from("SSH session service is shutting down")),
                    false,
                ),
                Err(error) => (
                    waiters,
                    FlightOutcome::Failed(Arc::from(format!("{error:#}"))),
                    false,
                ),
            };
            self.publish_snapshot_locked(&mut state);
            outcome
        };
        if published_idle {
            self.reaper_notify.notify_one();
        }
        notify_waiters(waiters, outcome);
    }
}

#[derive(Clone)]
struct ScheduledIdle {
    key: Arc<ConnectionKey>,
    creation_token: CreationToken,
    idle_candidate: IdleCandidate,
}

impl<M> RegistryShared<M> {
    fn next_idle_candidate(&self) -> Option<ScheduledIdle> {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state
            .slots
            .values()
            .filter_map(|slot| match slot {
                RegistrySlot::Ready {
                    key,
                    token,
                    lease_count: 0,
                    idle_candidate: Some(idle_candidate),
                    ..
                } => Some(ScheduledIdle {
                    key: key.clone(),
                    creation_token: token.clone(),
                    idle_candidate: idle_candidate.clone(),
                }),
                RegistrySlot::Creating { .. } | RegistrySlot::Ready { .. } => None,
            })
            .min_by_key(|candidate| candidate.idle_candidate.deadline)
    }

    /// Remove only the exact idle generation selected by the reaper.
    ///
    /// The deadline check and all identity checks happen under the registry
    /// lock.  The returned manager is disconnected later, outside this lock.
    fn take_expired_manager(&self, scheduled: &ScheduledIdle, now: Instant) -> Option<Arc<M>> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let is_current_expired = matches!(
            state.slots.get(scheduled.key.as_ref()),
            Some(RegistrySlot::Ready {
                token,
                lease_count: 0,
                idle_candidate: Some(idle_candidate),
                ..
            }) if token.is_same_flight(&scheduled.creation_token)
                && idle_candidate.is_same_candidate(&scheduled.idle_candidate)
                && idle_candidate.deadline <= now
        );
        if !is_current_expired {
            return None;
        }

        match state.slots.remove(scheduled.key.as_ref()) {
            Some(RegistrySlot::Ready { manager, .. }) => {
                self.publish_snapshot_locked(&mut state);
                Some(manager)
            }
            Some(RegistrySlot::Creating { .. }) | None => {
                unreachable!("idle slot shape changed while the registry lock was held")
            }
        }
    }
}

struct DisconnectReport {
    connection: String,
    result: Result<()>,
}

struct ReaperRunGuard<M> {
    shared: Arc<RegistryShared<M>>,
}

impl<M> ReaperRunGuard<M> {
    fn new(shared: Arc<RegistryShared<M>>) -> Self {
        shared.reaper_running.store(true, Ordering::Release);
        Self { shared }
    }
}

impl<M> Drop for ReaperRunGuard<M> {
    fn drop(&mut self) {
        self.shared.reaper_running.store(false, Ordering::Release);
        self.shared
            .reaper_stop_count
            .fetch_add(1, Ordering::Relaxed);
        self.shared.reaper_notify.notify_waiters();
    }
}

async fn run_idle_reaper<M>(shared: Arc<RegistryShared<M>>, _task_guard: RegistryTaskGuard<M>)
where
    M: RegistryManagedSession,
{
    let _run_guard = ReaperRunGuard::new(shared.clone());
    let mut disconnects = JoinSet::new();

    loop {
        if shared.reaper_shutdown.load(Ordering::Acquire) {
            break;
        }

        if let Some(scheduled) = shared.next_idle_candidate() {
            tokio::select! {
                biased;
                _ = shared.task_cancellation.cancelled() => break,
                _ = shared.reaper_notify.notified() => {}
                _ = sleep_until(scheduled.idle_candidate.deadline) => {
                    if let Some(manager) =
                        shared.take_expired_manager(&scheduled, Instant::now())
                    {
                        let connection = scheduled.key.label();
                        disconnects.spawn(async move {
                            DisconnectReport {
                                connection,
                                result: manager.disconnect_for_registry().await,
                            }
                        });
                    }
                }
                completion = disconnects.join_next(), if !disconnects.is_empty() => {
                    handle_disconnect_completion(completion);
                }
            }
        } else if disconnects.is_empty() {
            tokio::select! {
                biased;
                _ = shared.task_cancellation.cancelled() => break,
                _ = shared.reaper_notify.notified() => {}
            }
        } else {
            tokio::select! {
                biased;
                _ = shared.task_cancellation.cancelled() => break,
                _ = shared.reaper_notify.notified() => {}
                completion = disconnects.join_next() => {
                    handle_disconnect_completion(completion);
                }
            }
        }
    }

    disconnects.abort_all();
    while disconnects.join_next().await.is_some() {}
}

fn handle_disconnect_completion(
    completion: Option<std::result::Result<DisconnectReport, JoinError>>,
) {
    match completion {
        Some(Ok(DisconnectReport {
            connection,
            result: Ok(()),
        })) => {
            tracing::debug!(%connection, "reaped idle SSH session generation");
        }
        Some(Ok(DisconnectReport {
            connection,
            result: Err(error),
        })) => {
            tracing::warn!(
                %connection,
                error = %error,
                "failed to disconnect reaped idle SSH session generation"
            );
        }
        Some(Err(error)) if !error.is_cancelled() => {
            tracing::warn!(
                error = %error,
                "idle SSH session disconnect task did not complete"
            );
        }
        Some(Err(_)) | None => {}
    }
}

/// Removes a creation slot if its detached task is cancelled or panics before
/// publishing a result.  The registry state uses a synchronous mutex precisely
/// so this cleanup is possible from `Drop`; no state lock is ever held across
/// async manager creation.
struct CreationCleanup<M> {
    shared: Arc<RegistryShared<M>>,
    key: ConnectionKey,
    token: CreationToken,
    armed: bool,
}

impl<M> CreationCleanup<M> {
    fn new(shared: Arc<RegistryShared<M>>, key: ConnectionKey, token: CreationToken) -> Self {
        Self {
            shared,
            key,
            token,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl<M> Drop for CreationCleanup<M> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }

        let waiters = {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let current = state.take_current_creation(&self.key, &self.token);
            if current.is_some() {
                self.shared.publish_snapshot_locked(&mut state);
            }
            current.unwrap_or_default()
        };
        notify_waiters(
            waiters,
            FlightOutcome::Failed(Arc::from("SSH session manager creation task was aborted")),
        );
    }
}

fn notify_waiters(waiters: Vec<oneshot::Sender<FlightOutcome>>, outcome: FlightOutcome) {
    match outcome {
        FlightOutcome::Published => {
            for waiter in waiters {
                let _ = waiter.send(FlightOutcome::Published);
            }
        }
        FlightOutcome::Failed(message) => {
            for waiter in waiters {
                let _ = waiter.send(FlightOutcome::Failed(message.clone()));
            }
        }
        FlightOutcome::Superseded => {
            for waiter in waiters {
                let _ = waiter.send(FlightOutcome::Superseded);
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
struct DefaultSessionManagerFactory;

#[async_trait]
impl SessionManagerFactory<SshSessionManager> for DefaultSessionManagerFactory {
    async fn create(&self, config: SshConnectConfig) -> Result<SshSessionManager> {
        Ok(SshSessionManager::new(config))
    }
}

#[async_trait]
impl RegistryManagedSession for SshSessionManager {
    fn request_shutdown_for_registry(&self) {
        self.request_shutdown();
    }

    async fn disconnect_for_registry(&self) -> Result<()> {
        self.disconnect().await
    }

    async fn shutdown_for_registry(&self) -> Result<()> {
        self.shutdown().await
    }
}

/// A generation-bound use of a shared [`SshSessionManager`].
///
/// Dropping or explicitly releasing this value synchronously decrements only
/// the slot generation from which it was checked out.  The last release marks
/// that slot as idle and wakes the registry reaper; `Drop` itself never
/// disconnects, blocks, or starts an async task.  Cloning a lease while its
/// generation is current atomically adds another counted use.  A clone of an
/// already retired lease can keep that retired manager alive but is never
/// charged to a replacement generation.  The lease does not expose the
/// registry-owned `Arc<SshSessionManager>`; consumers must retain a lease for
/// the full lifetime of clients or channels obtained from its manager.
#[must_use = "dropping the lease immediately releases the registry use"]
pub struct SshSessionLease {
    inner: RegistryLease<SshSessionManager>,
}

impl SshSessionLease {
    /// Borrow the shared manager while this lease is active.
    ///
    /// Cloning the returned manager is not a substitute for cloning and
    /// retaining this lease, because only the lease participates in registry
    /// lifecycle accounting.
    #[must_use]
    pub fn manager(&self) -> &SshSessionManager {
        self.inner.manager()
    }

    /// Release this use before the end of its lexical scope.
    ///
    /// This is equivalent to dropping the lease.  It may mark a new idle
    /// deadline and wake the registry reaper, but never performs async I/O or
    /// disconnects the underlying transport itself.
    pub fn release(mut self) {
        self.inner.release();
    }
}

impl Clone for SshSessionLease {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl Deref for SshSessionLease {
    type Target = SshSessionManager;

    fn deref(&self) -> &Self::Target {
        self.manager()
    }
}

impl fmt::Debug for SshSessionLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SshSessionLease")
            .field("connection", &self.inner.key.label())
            .field("generation", &self.inner.token.generation.0)
            .field("active", &self.inner.counted)
            .finish_non_exhaustive()
    }
}

/// In-memory registry that coalesces manager creation by [`ConnectionKey`] and
/// returns generation-bound leases.  A single cancellable Tokio task reaps
/// generations that remain unused for the configured idle timeout.
///
/// This low-level type is intentionally not a process-global singleton.
/// Application consumers should use one [`SshSessionService`], which adds
/// admission closure, snapshots, task convergence, and explicit shutdown.
#[derive(Clone)]
pub struct SshSessionRegistry {
    inner: Arc<SessionRegistryCore<SshSessionManager, DefaultSessionManagerFactory>>,
}

impl SshSessionRegistry {
    /// Default time a published generation may remain without a lease before
    /// the registry removes and disconnects it.
    pub const DEFAULT_IDLE_TIMEOUT: Duration = DEFAULT_SESSION_IDLE_TIMEOUT;

    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(SessionRegistryCore::new(DefaultSessionManagerFactory)),
        }
    }

    /// Create a registry with an explicit non-zero, representable idle
    /// timeout.
    ///
    /// The timeout is fixed for the lifetime of this registry.  Changing
    /// application policy should replace the application-owned registry
    /// rather than silently changing deadlines already attached to slots.
    #[must_use]
    pub fn with_idle_timeout(idle_timeout: Duration) -> Self {
        Self {
            inner: Arc::new(SessionRegistryCore::with_idle_timeout(
                DefaultSessionManagerFactory,
                idle_timeout,
            )),
        }
    }

    /// Check out one lease from the single manager slot for `key`.
    ///
    /// Callers must build `key` from the same `config` and current opaque
    /// credential revisions.  Passing changed authentication material without
    /// advancing its revision violates the [`ConnectionKey`] contract.
    pub async fn acquire(
        &self,
        key: &ConnectionKey,
        config: SshConnectConfig,
    ) -> Result<SshSessionLease> {
        self.inner
            .acquire(key, config)
            .await
            .map(|inner| SshSessionLease { inner })
    }

    /// Remove the currently published or in-flight slot for `key`.
    ///
    /// This operation only changes registry visibility.  It deliberately does
    /// not disconnect a returned manager: existing leases keep the retired
    /// generation alive, while their later drops cannot alter a replacement
    /// generation.  Waiters on an in-flight slot transparently join a newer
    /// generation, and that retired flight can no longer publish its result.
    pub fn retire(&self, key: &ConnectionKey) -> Option<Arc<SshSessionManager>> {
        self.inner.retire(key)
    }
}

impl Default for SshSessionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Default)]
struct ServiceShutdownState {
    started: bool,
    report: Option<SshSessionShutdownReport>,
}

struct SessionServiceCore<M, F>
where
    M: RegistryManagedSession,
    F: SessionManagerFactory<M>,
{
    registry: Arc<SessionRegistryCore<M, F>>,
    shutdown_timeout: Duration,
    shutdown_state: StdMutex<ServiceShutdownState>,
    shutdown_complete: CancellationToken,
}

impl<M, F> SessionServiceCore<M, F>
where
    M: RegistryManagedSession,
    F: SessionManagerFactory<M>,
{
    fn new(factory: F, idle_timeout: Duration, shutdown_timeout: Duration) -> Self {
        assert!(
            !shutdown_timeout.is_zero(),
            "SSH service shutdown timeout must be greater than zero"
        );
        assert!(
            Instant::now().checked_add(shutdown_timeout).is_some(),
            "SSH service shutdown timeout must fit in a Tokio Instant"
        );
        Self {
            registry: Arc::new(SessionRegistryCore::with_idle_timeout(
                factory,
                idle_timeout,
            )),
            shutdown_timeout,
            shutdown_state: StdMutex::new(ServiceShutdownState::default()),
            shutdown_complete: CancellationToken::new(),
        }
    }

    async fn shutdown(self: &Arc<Self>) -> SshSessionShutdownReport {
        let should_start = {
            let mut shutdown = self
                .shutdown_state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(report) = &shutdown.report {
                return report.clone();
            }
            if shutdown.started {
                false
            } else {
                shutdown.started = true;
                true
            }
        };

        if should_start {
            // Establish the one total deadline and synchronously close
            // registry admission/reconnect gates before yielding to the
            // detached driver.  Driver scheduling latency is therefore part
            // of the configured shutdown budget.
            let deadline = Instant::now()
                .checked_add(self.shutdown_timeout)
                .expect("shutdown timeout was validated when the SSH service was constructed");
            let managers = self.registry.begin_shutdown();
            let service = self.clone();
            tokio::spawn(async move {
                service.run_shutdown(deadline, managers).await;
            });
        }

        loop {
            if let Some(report) = self
                .shutdown_state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .report
                .clone()
            {
                return report;
            }
            // CancellationToken is a sticky completion signal: a report
            // published between the check above and this await cannot be
            // missed by a concurrent or late shutdown caller.
            self.shutdown_complete.cancelled().await;
        }
    }

    async fn run_shutdown(self: Arc<Self>, deadline: Instant, managers: Vec<Arc<M>>) {
        let managers_requested = managers.len();
        let mut shutdowns = JoinSet::new();
        for manager in managers {
            shutdowns.spawn(async move { manager.shutdown_for_registry().await });
        }

        let mut managers_completed = 0;
        let mut manager_failures = 0;
        let convergence = timeout_at(deadline, async {
            while let Some(completion) = shutdowns.join_next().await {
                match completion {
                    Ok(Ok(())) => managers_completed += 1,
                    Ok(Err(_)) | Err(_) => {
                        managers_completed += 1;
                        manager_failures += 1;
                    }
                }
            }
            self.registry.shared.wait_for_tasks().await;
        })
        .await;

        let timed_out = convergence.is_err();
        let managers_remaining = shutdowns.len();
        if timed_out {
            shutdowns.abort_all();
            while shutdowns.join_next().await.is_some() {}
        }
        let registry_tasks_remaining = self.registry.shared.task_count.load(Ordering::Acquire);
        self.registry.finish_shutdown();

        let report = SshSessionShutdownReport {
            timed_out,
            managers_requested,
            managers_completed,
            manager_failures,
            managers_remaining,
            registry_tasks_remaining,
        };
        {
            let mut shutdown = self
                .shutdown_state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            shutdown.report = Some(report.clone());
        }
        self.shutdown_complete.cancel();
    }
}

impl<M, F> Drop for SessionServiceCore<M, F>
where
    M: RegistryManagedSession,
    F: SessionManagerFactory<M>,
{
    fn drop(&mut self) {
        // No async work is launched from Drop.  Closing admission and each
        // manager's reconnect gate is synchronous; the application exit path
        // is responsible for awaiting `shutdown()` before releasing its owner.
        drop(self.registry.begin_shutdown());
        self.registry.finish_shutdown();
    }
}

/// Application-level owner for shared SSH session managers.
///
/// The application creates exactly one instance and installs a separate GPUI
/// `Global` wrapper around it.  Consumers receive generation-bound
/// [`SshSessionLease`] values rather than a bare manager.  Clones of this
/// service share one lifecycle and one idempotent shutdown result.
#[derive(Clone)]
pub struct SshSessionService {
    inner: Arc<SessionServiceCore<SshSessionManager, DefaultSessionManagerFactory>>,
}

impl SshSessionService {
    pub const DEFAULT_IDLE_TIMEOUT: Duration = DEFAULT_SESSION_IDLE_TIMEOUT;
    pub const DEFAULT_SHUTDOWN_TIMEOUT: Duration = DEFAULT_SERVICE_SHUTDOWN_TIMEOUT;

    #[must_use]
    pub fn new() -> Self {
        Self::with_timeouts(Self::DEFAULT_IDLE_TIMEOUT, Self::DEFAULT_SHUTDOWN_TIMEOUT)
    }

    /// Construct a service with fixed, non-zero idle and total-shutdown
    /// deadlines.
    #[must_use]
    pub fn with_timeouts(idle_timeout: Duration, shutdown_timeout: Duration) -> Self {
        Self {
            inner: Arc::new(SessionServiceCore::new(
                DefaultSessionManagerFactory,
                idle_timeout,
                shutdown_timeout,
            )),
        }
    }

    /// Acquire one generation-bound use of the manager for `key`.
    ///
    /// Once shutdown starts this fails deterministically and can never create
    /// or publish another manager generation.
    pub async fn acquire(
        &self,
        key: &ConnectionKey,
        config: SshConnectConfig,
    ) -> Result<SshSessionLease> {
        self.inner
            .registry
            .acquire(key, config)
            .await
            .map(|inner| SshSessionLease { inner })
    }

    /// Return the current credential-free service snapshot.
    #[must_use]
    pub fn snapshot(&self) -> SshSessionServiceSnapshot {
        self.inner.registry.snapshot()
    }

    /// Subscribe to credential-free lifecycle and capacity snapshots.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<SshSessionServiceSnapshot> {
        self.inner.registry.subscribe()
    }

    /// Close admission, permanently stop every published manager, cancel all
    /// registry-owned work, and return one stable result for every concurrent
    /// or repeated caller.
    pub async fn shutdown(&self) -> SshSessionShutdownReport {
        self.inner.shutdown().await
    }
}

impl Default for SshSessionService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RegistryManagedSession, RegistrySlot, SessionManagerFactory, SessionRegistryCore,
        SessionServiceCore, SshSessionRegistry, SshSessionServiceState,
    };
    use crate::{
        ConnectionCredentialRevisions, ConnectionKey, CredentialRevision, HostKeyVerifier,
        JumpServerConnectConfig, ProxyConnectConfig, SshAuth, SshConnectConfig, SshSessionService,
    };
    use anyhow::{Result, anyhow};
    use async_trait::async_trait;
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex as StdMutex};
    use tokio::sync::Semaphore;
    use tokio::time::{Duration, advance, timeout};

    #[derive(Debug)]
    struct FakeManager {
        id: usize,
        host: String,
        disconnect: Arc<FakeDisconnect>,
    }

    impl FakeManager {
        fn new(id: usize, host: String) -> Self {
            Self {
                id,
                host,
                disconnect: Arc::new(FakeDisconnect::default()),
            }
        }
    }

    #[derive(Debug)]
    struct FakeDisconnect {
        attempts: AtomicUsize,
        shutdown_requested: AtomicBool,
        should_fail: AtomicBool,
        blocked: AtomicBool,
        permits: Semaphore,
    }

    impl Default for FakeDisconnect {
        fn default() -> Self {
            Self {
                attempts: AtomicUsize::new(0),
                shutdown_requested: AtomicBool::new(false),
                should_fail: AtomicBool::new(false),
                blocked: AtomicBool::new(false),
                permits: Semaphore::new(0),
            }
        }
    }

    impl FakeDisconnect {
        fn block(&self) {
            self.blocked.store(true, Ordering::SeqCst);
        }

        fn unblock(&self) {
            self.permits.add_permits(1);
        }
    }

    #[async_trait]
    impl RegistryManagedSession for FakeManager {
        fn request_shutdown_for_registry(&self) {
            self.disconnect
                .shutdown_requested
                .store(true, Ordering::SeqCst);
        }

        async fn disconnect_for_registry(&self) -> Result<()> {
            self.disconnect.attempts.fetch_add(1, Ordering::SeqCst);
            if self.disconnect.blocked.load(Ordering::SeqCst) {
                self.disconnect
                    .permits
                    .acquire()
                    .await
                    .expect("fake disconnect semaphore should remain open")
                    .forget();
            }
            if self.disconnect.should_fail.load(Ordering::SeqCst) {
                Err(anyhow!("fake disconnect failure"))
            } else {
                Ok(())
            }
        }
    }

    #[derive(Default)]
    struct CountingFactory {
        create_count: AtomicUsize,
    }

    #[async_trait]
    impl SessionManagerFactory<FakeManager> for Arc<CountingFactory> {
        async fn create(&self, config: SshConnectConfig) -> Result<FakeManager> {
            let id = self.create_count.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(FakeManager::new(id, config.host))
        }
    }

    struct BlockingFactory {
        create_count: AtomicUsize,
        completed_count: AtomicUsize,
        blocked_hosts: HashSet<String>,
        permits: Semaphore,
    }

    impl BlockingFactory {
        fn new(blocked_hosts: impl IntoIterator<Item = &'static str>) -> Self {
            Self {
                create_count: AtomicUsize::new(0),
                completed_count: AtomicUsize::new(0),
                blocked_hosts: blocked_hosts.into_iter().map(str::to_owned).collect(),
                permits: Semaphore::new(0),
            }
        }

        fn release_one(&self) {
            self.permits.add_permits(1);
        }
    }

    #[async_trait]
    impl SessionManagerFactory<FakeManager> for Arc<BlockingFactory> {
        async fn create(&self, config: SshConnectConfig) -> Result<FakeManager> {
            let id = self.create_count.fetch_add(1, Ordering::SeqCst) + 1;
            if self.blocked_hosts.contains(&config.host) {
                self.permits
                    .acquire()
                    .await
                    .expect("test semaphore should remain open")
                    .forget();
            }
            self.completed_count.fetch_add(1, Ordering::SeqCst);
            Ok(FakeManager::new(id, config.host))
        }
    }

    struct FirstFlightBlockingFactory {
        create_count: AtomicUsize,
        completed_count: AtomicUsize,
        first_permit: Semaphore,
        created_ids: StdMutex<Vec<usize>>,
    }

    impl FirstFlightBlockingFactory {
        fn new() -> Self {
            Self {
                create_count: AtomicUsize::new(0),
                completed_count: AtomicUsize::new(0),
                first_permit: Semaphore::new(0),
                created_ids: StdMutex::new(Vec::new()),
            }
        }
    }

    struct FirstFlightFailingFactory {
        create_count: AtomicUsize,
        first_permit: Semaphore,
    }

    impl FirstFlightFailingFactory {
        fn new() -> Self {
            Self {
                create_count: AtomicUsize::new(0),
                first_permit: Semaphore::new(0),
            }
        }
    }

    #[async_trait]
    impl SessionManagerFactory<FakeManager> for Arc<FirstFlightFailingFactory> {
        async fn create(&self, config: SshConnectConfig) -> Result<FakeManager> {
            let id = self.create_count.fetch_add(1, Ordering::SeqCst) + 1;
            if id == 1 {
                self.first_permit
                    .acquire()
                    .await
                    .expect("test semaphore should remain open")
                    .forget();
                return Err(anyhow!("shared fake creation failure"));
            }
            Ok(FakeManager::new(id, config.host))
        }
    }

    #[async_trait]
    impl SessionManagerFactory<FakeManager> for Arc<FirstFlightBlockingFactory> {
        async fn create(&self, config: SshConnectConfig) -> Result<FakeManager> {
            let id = self.create_count.fetch_add(1, Ordering::SeqCst) + 1;
            if id == 1 {
                self.first_permit
                    .acquire()
                    .await
                    .expect("test semaphore should remain open")
                    .forget();
            }
            self.created_ids
                .lock()
                .expect("created ids lock should not be poisoned")
                .push(id);
            self.completed_count.fetch_add(1, Ordering::SeqCst);
            Ok(FakeManager::new(id, config.host))
        }
    }

    fn test_config(host: &str) -> SshConnectConfig {
        SshConnectConfig {
            host: host.to_owned(),
            port: 22,
            username: "tester".to_owned(),
            auth: SshAuth::Agent,
            timeout: None,
            keepalive_interval: None,
            keepalive_max: None,
            jump_server: None::<JumpServerConnectConfig>,
            proxy: None::<ProxyConnectConfig>,
            keyboard_interactive_responder: None,
            host_key_verifier: HostKeyVerifier::default(),
            x11_forwarding: false,
        }
    }

    fn test_key(config: &SshConnectConfig, slot: u64) -> ConnectionKey {
        ConnectionKey::from_config(
            config,
            ConnectionCredentialRevisions::new(CredentialRevision::new(slot, 1)),
        )
        .expect("test config should produce a key")
    }

    async fn wait_for_count(counter: &AtomicUsize, expected: usize) {
        timeout(Duration::from_secs(2), async {
            while counter.load(Ordering::SeqCst) < expected {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("counter should reach expected value");
    }

    async fn yield_until_count(counter: &AtomicUsize, expected: usize) {
        for _ in 0..100 {
            if counter.load(Ordering::SeqCst) >= expected {
                return;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(
            counter.load(Ordering::SeqCst),
            expected,
            "counter should reach expected value"
        );
    }

    async fn yield_until_true(flag: &AtomicBool) {
        for _ in 0..100 {
            if flag.load(Ordering::SeqCst) {
                return;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            flag.load(Ordering::SeqCst),
            "flag should become true before the yield budget is exhausted"
        );
    }

    async fn wait_for_waiter_count<M, F>(
        registry: &SessionRegistryCore<M, F>,
        key: &ConnectionKey,
        expected: usize,
    ) {
        timeout(Duration::from_secs(2), async {
            loop {
                let waiter_count = {
                    let state = registry
                        .shared
                        .state
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    match state.slots.get(key) {
                        Some(super::RegistrySlot::Creating { waiters, .. }) => waiters.len(),
                        Some(super::RegistrySlot::Ready { .. }) | None => 0,
                    }
                };
                if waiter_count >= expected {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("slot should reach expected waiter count");
    }

    fn ready_lifecycle<M, F>(
        registry: &SessionRegistryCore<M, F>,
        key: &ConnectionKey,
    ) -> (usize, bool) {
        let state = registry
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        match state.slots.get(key) {
            Some(RegistrySlot::Ready {
                lease_count,
                idle_candidate,
                ..
            }) => (*lease_count, idle_candidate.is_some()),
            Some(RegistrySlot::Creating { .. }) => panic!("slot is still being created"),
            None => panic!("slot is not present"),
        }
    }

    fn slot_is_present<M, F>(registry: &SessionRegistryCore<M, F>, key: &ConnectionKey) -> bool {
        registry
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .slots
            .contains_key(key)
    }

    async fn wait_for_ready<M, F>(registry: &SessionRegistryCore<M, F>, key: &ConnectionKey) {
        timeout(Duration::from_secs(2), async {
            loop {
                let is_ready = {
                    let state = registry
                        .shared
                        .state
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    matches!(state.slots.get(key), Some(RegistrySlot::Ready { .. }))
                };
                if is_ready {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("slot should become ready");
    }

    #[tokio::test]
    async fn concurrent_acquires_for_same_key_create_one_manager() {
        let factory = Arc::new(BlockingFactory::new(["shared.example"]));
        let registry = Arc::new(SessionRegistryCore::new(factory.clone()));
        let config = test_config("shared.example");
        let key = test_key(&config, 1);

        let mut acquires = Vec::new();
        for _ in 0..12 {
            let registry = registry.clone();
            let config = config.clone();
            let key = key.clone();
            acquires.push(tokio::spawn(
                async move { registry.acquire(&key, config).await },
            ));
        }

        wait_for_count(&factory.create_count, 1).await;
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
        assert_eq!(factory.create_count.load(Ordering::SeqCst), 1);
        factory.release_one();

        let mut managers = Vec::new();
        for acquire in acquires {
            managers.push(
                acquire
                    .await
                    .expect("acquire task should not panic")
                    .expect("manager creation should succeed"),
            );
        }
        assert!(
            managers
                .iter()
                .all(|manager| Arc::ptr_eq(&manager.manager, &managers[0].manager))
        );
        assert_eq!(factory.create_count.load(Ordering::SeqCst), 1);
        assert_eq!(ready_lifecycle(&registry, &key), (12, false));

        drop(managers);
        assert_eq!(ready_lifecycle(&registry, &key), (0, true));
    }

    #[tokio::test]
    async fn different_keys_never_share_a_manager() {
        let factory = Arc::new(CountingFactory::default());
        let registry = Arc::new(SessionRegistryCore::new(factory.clone()));
        let first_config = test_config("first.example");
        let second_config = test_config("second.example");
        let first_key = test_key(&first_config, 1);
        let second_key = test_key(&second_config, 2);

        let first = registry
            .acquire(&first_key, first_config)
            .await
            .expect("first manager should be created");
        let second = registry
            .acquire(&second_key, second_config)
            .await
            .expect("second manager should be created");

        assert!(!Arc::ptr_eq(&first.manager, &second.manager));
        assert_ne!(first.id, second.id);
        assert_eq!(first.host, "first.example");
        assert_eq!(second.host, "second.example");
        assert_eq!(factory.create_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn blocked_creation_does_not_hold_the_registry_lock() {
        let factory = Arc::new(BlockingFactory::new(["blocked.example"]));
        let registry = Arc::new(SessionRegistryCore::new(factory.clone()));
        let blocked_config = test_config("blocked.example");
        let blocked_key = test_key(&blocked_config, 1);
        let free_config = test_config("free.example");
        let free_key = test_key(&free_config, 2);

        let blocked_registry = registry.clone();
        let blocked =
            tokio::spawn(
                async move { blocked_registry.acquire(&blocked_key, blocked_config).await },
            );
        wait_for_count(&factory.create_count, 1).await;

        let free = timeout(
            Duration::from_secs(1),
            registry.acquire(&free_key, free_config),
        )
        .await
        .expect("unrelated key should not wait for blocked creation")
        .expect("unrelated manager should be created");
        assert_eq!(free.host, "free.example");
        assert_eq!(factory.create_count.load(Ordering::SeqCst), 2);

        factory.release_one();
        blocked
            .await
            .expect("blocked acquire task should not panic")
            .expect("blocked manager should finish");
    }

    #[tokio::test]
    async fn failed_flight_is_shared_and_a_later_acquire_retries() {
        let factory = Arc::new(FirstFlightFailingFactory::new());
        let registry = Arc::new(SessionRegistryCore::new(factory.clone()));
        let config = test_config("retry.example");
        let key = test_key(&config, 1);

        let mut failed_acquires = Vec::new();
        for _ in 0..2 {
            let registry = registry.clone();
            let config = config.clone();
            let key = key.clone();
            failed_acquires.push(tokio::spawn(
                async move { registry.acquire(&key, config).await },
            ));
        }

        wait_for_count(&factory.create_count, 1).await;
        wait_for_waiter_count(&registry, &key, 2).await;
        assert_eq!(factory.create_count.load(Ordering::SeqCst), 1);
        factory.first_permit.add_permits(1);

        for acquire in failed_acquires {
            let error = acquire
                .await
                .expect("acquire task should not panic")
                .expect_err("shared creation should fail");
            assert_eq!(error.to_string(), "shared fake creation failure");
        }

        let second = registry
            .acquire(&key, config)
            .await
            .expect("next acquire should create a fresh flight");
        assert_eq!(second.id, 2);
        assert_eq!(factory.create_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn stale_generation_cannot_replace_the_new_slot() {
        let factory = Arc::new(FirstFlightBlockingFactory::new());
        let registry = Arc::new(SessionRegistryCore::new(factory.clone()));
        let config = test_config("generation.example");
        let key = test_key(&config, 1);

        let first_registry = registry.clone();
        let first_config = config.clone();
        let first_key = key.clone();
        let first =
            tokio::spawn(async move { first_registry.acquire(&first_key, first_config).await });
        wait_for_count(&factory.create_count, 1).await;

        assert!(registry.retire(&key).is_none());
        wait_for_count(&factory.create_count, 2).await;

        let current = registry
            .acquire(&key, config.clone())
            .await
            .expect("replacement generation should succeed");
        assert_eq!(current.id, 2);

        factory.first_permit.add_permits(1);
        wait_for_count(&factory.completed_count, 2).await;
        let first_result = first
            .await
            .expect("first acquire task should not panic")
            .expect("first acquire should join the replacement generation");
        assert!(Arc::ptr_eq(&first_result.manager, &current.manager));

        let after_stale_completion = registry
            .acquire(&key, config)
            .await
            .expect("current slot should remain available");
        assert!(Arc::ptr_eq(
            &after_stale_completion.manager,
            &current.manager
        ));
        assert_eq!(
            *factory
                .created_ids
                .lock()
                .expect("created ids lock should not be poisoned"),
            vec![2, 1],
            "the stale result may finish, but it must not replace manager 2"
        );
    }

    #[tokio::test]
    async fn cancelling_first_waiter_does_not_cancel_shared_creation() {
        let factory = Arc::new(BlockingFactory::new(["cancel.example"]));
        let registry = Arc::new(SessionRegistryCore::new(factory.clone()));
        let config = test_config("cancel.example");
        let key = test_key(&config, 1);

        let cancelled_registry = registry.clone();
        let cancelled_config = config.clone();
        let cancelled_key = key.clone();
        let cancelled = tokio::spawn(async move {
            cancelled_registry
                .acquire(&cancelled_key, cancelled_config)
                .await
        });
        wait_for_count(&factory.create_count, 1).await;
        cancelled.abort();
        let _ = cancelled.await;

        let surviving_registry = registry.clone();
        let surviving_config = config.clone();
        let surviving_key = key.clone();
        let surviving = tokio::spawn(async move {
            surviving_registry
                .acquire(&surviving_key, surviving_config)
                .await
        });
        factory.release_one();
        let manager = surviving
            .await
            .expect("surviving acquire task should not panic")
            .expect("detached creation should satisfy later waiters");

        let cached = registry
            .acquire(&key, config)
            .await
            .expect("completed slot should remain cached");
        assert!(Arc::ptr_eq(&manager.manager, &cached.manager));
        assert_eq!(factory.create_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn last_release_marks_idle_and_reacquire_reuses_the_manager() {
        let factory = Arc::new(CountingFactory::default());
        let registry = Arc::new(SessionRegistryCore::new(factory.clone()));
        let config = test_config("idle.example");
        let key = test_key(&config, 1);

        let first = registry
            .acquire(&key, config.clone())
            .await
            .expect("first lease should be created");
        let manager_id = first.id;
        assert_eq!(ready_lifecycle(&registry, &key), (1, false));

        let cloned = first.clone();
        assert!(Arc::ptr_eq(&first.manager, &cloned.manager));
        assert_eq!(ready_lifecycle(&registry, &key), (2, false));

        drop(first);
        assert_eq!(ready_lifecycle(&registry, &key), (1, false));
        drop(cloned);
        assert_eq!(ready_lifecycle(&registry, &key), (0, true));

        let mut second = registry
            .acquire(&key, config)
            .await
            .expect("idle slot should be reused");
        assert_eq!(second.id, manager_id);
        assert_eq!(factory.create_count.load(Ordering::SeqCst), 1);
        assert_eq!(ready_lifecycle(&registry, &key), (1, false));

        second.release();
        assert_eq!(ready_lifecycle(&registry, &key), (0, true));
    }

    #[tokio::test]
    async fn lease_can_be_released_from_a_non_runtime_thread() {
        let factory = Arc::new(CountingFactory::default());
        let registry = Arc::new(SessionRegistryCore::with_idle_timeout(
            factory.clone(),
            Duration::from_secs(60),
        ));
        let config = test_config("cross-thread-release.example");
        let key = test_key(&config, 1);

        let lease = registry
            .acquire(&key, config.clone())
            .await
            .expect("manager should be created");
        let manager_id = lease.id;
        yield_until_true(&registry.shared.reaper_running).await;

        std::thread::spawn(move || drop(lease))
            .join()
            .expect("lease Drop must not panic outside the Tokio runtime");

        assert_eq!(ready_lifecycle(&registry, &key), (0, true));
        assert_eq!(
            registry.shared.reaper_start_count.load(Ordering::SeqCst),
            1,
            "cross-thread release must only wake the registry-owned reaper"
        );

        let reacquired = registry
            .acquire(&key, config)
            .await
            .expect("cross-thread release should leave an idle reusable slot");
        assert_eq!(reacquired.id, manager_id);
        assert_eq!(factory.create_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    #[should_panic(expected = "SSH session idle timeout must fit in a Tokio Instant")]
    fn registry_rejects_an_unrepresentable_idle_timeout() {
        let _registry = SshSessionRegistry::with_idle_timeout(Duration::MAX);
    }

    #[test]
    #[should_panic(expected = "SSH service shutdown timeout must be greater than zero")]
    fn service_rejects_a_zero_shutdown_timeout() {
        let _service = SshSessionService::with_timeouts(Duration::from_secs(60), Duration::ZERO);
    }

    #[test]
    #[should_panic(expected = "SSH service shutdown timeout must fit in a Tokio Instant")]
    fn service_rejects_an_unrepresentable_shutdown_timeout() {
        let _service = SshSessionService::with_timeouts(Duration::from_secs(60), Duration::MAX);
    }

    #[tokio::test(start_paused = true)]
    async fn idle_generation_is_disconnected_only_after_its_timeout() {
        let factory = Arc::new(CountingFactory::default());
        let registry = Arc::new(SessionRegistryCore::with_idle_timeout(
            factory,
            Duration::from_secs(10),
        ));
        let config = test_config("idle-timeout.example");
        let key = test_key(&config, 1);

        let lease = registry
            .acquire(&key, config)
            .await
            .expect("manager should be created");
        let disconnect = lease.disconnect.clone();
        drop(lease);
        assert_eq!(ready_lifecycle(&registry, &key), (0, true));

        advance(Duration::from_secs(9)).await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert_eq!(disconnect.attempts.load(Ordering::SeqCst), 0);
        assert!(slot_is_present(&registry, &key));

        advance(Duration::from_secs(1)).await;
        yield_until_count(&disconnect.attempts, 1).await;
        assert!(!slot_is_present(&registry, &key));
    }

    #[tokio::test(start_paused = true)]
    async fn reacquire_invalidates_old_idle_work_and_resets_the_deadline() {
        let factory = Arc::new(CountingFactory::default());
        let registry = Arc::new(SessionRegistryCore::with_idle_timeout(
            factory.clone(),
            Duration::from_secs(10),
        ));
        let config = test_config("idle-reacquire.example");
        let key = test_key(&config, 1);

        let first = registry
            .acquire(&key, config.clone())
            .await
            .expect("first lease should be created");
        let manager_id = first.id;
        let disconnect = first.disconnect.clone();
        drop(first);

        advance(Duration::from_secs(4)).await;
        let second = registry
            .acquire(&key, config)
            .await
            .expect("idle generation should be reused");
        assert_eq!(second.id, manager_id);
        assert_eq!(factory.create_count.load(Ordering::SeqCst), 1);

        advance(Duration::from_secs(6)).await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            disconnect.attempts.load(Ordering::SeqCst),
            0,
            "the invalidated first deadline must not disconnect an active lease"
        );

        drop(second);
        advance(Duration::from_secs(9)).await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            disconnect.attempts.load(Ordering::SeqCst),
            0,
            "the second idle period gets a fresh full timeout"
        );

        advance(Duration::from_secs(1)).await;
        yield_until_count(&disconnect.attempts, 1).await;
        assert!(!slot_is_present(&registry, &key));
    }

    #[tokio::test(start_paused = true)]
    async fn reaper_tracks_the_earliest_deadline_across_different_keys() {
        let factory = Arc::new(CountingFactory::default());
        let registry = Arc::new(SessionRegistryCore::with_idle_timeout(
            factory,
            Duration::from_secs(10),
        ));
        let first_config = test_config("first-idle.example");
        let second_config = test_config("second-idle.example");
        let first_key = test_key(&first_config, 1);
        let second_key = test_key(&second_config, 2);

        let first = registry
            .acquire(&first_key, first_config)
            .await
            .expect("first manager should be created");
        let second = registry
            .acquire(&second_key, second_config)
            .await
            .expect("second manager should be created");
        let first_disconnect = first.disconnect.clone();
        let second_disconnect = second.disconnect.clone();

        drop(first);
        advance(Duration::from_secs(4)).await;
        drop(second);

        advance(Duration::from_secs(6)).await;
        yield_until_count(&first_disconnect.attempts, 1).await;
        assert_eq!(second_disconnect.attempts.load(Ordering::SeqCst), 0);
        assert!(!slot_is_present(&registry, &first_key));
        assert!(slot_is_present(&registry, &second_key));

        advance(Duration::from_secs(4)).await;
        yield_until_count(&second_disconnect.attempts, 1).await;
        assert!(!slot_is_present(&registry, &second_key));
    }

    #[tokio::test(start_paused = true)]
    async fn retired_idle_work_cannot_touch_a_replacement_generation() {
        let factory = Arc::new(CountingFactory::default());
        let registry = Arc::new(SessionRegistryCore::with_idle_timeout(
            factory.clone(),
            Duration::from_secs(10),
        ));
        let config = test_config("idle-retire.example");
        let key = test_key(&config, 1);

        let old = registry
            .acquire(&key, config.clone())
            .await
            .expect("old generation should be created");
        let old_disconnect = old.disconnect.clone();
        drop(old);
        advance(Duration::from_secs(4)).await;

        let retired = registry
            .retire(&key)
            .expect("idle generation should be retired");
        let replacement = registry
            .acquire(&key, config)
            .await
            .expect("replacement generation should be created");
        let replacement_disconnect = replacement.disconnect.clone();
        assert_eq!(replacement.id, 2);

        advance(Duration::from_secs(6)).await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert_eq!(old_disconnect.attempts.load(Ordering::SeqCst), 0);
        assert_eq!(
            replacement_disconnect.attempts.load(Ordering::SeqCst),
            0,
            "the stale deadline must not disconnect the active replacement"
        );
        assert_eq!(ready_lifecycle(&registry, &key), (1, false));

        drop(retired);
        drop(replacement);
        advance(Duration::from_secs(10)).await;
        yield_until_count(&replacement_disconnect.attempts, 1).await;
        assert_eq!(factory.create_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn slow_disconnect_runs_outside_the_registry_lock() {
        let factory = Arc::new(CountingFactory::default());
        let registry = Arc::new(SessionRegistryCore::with_idle_timeout(
            factory.clone(),
            Duration::from_secs(10),
        ));
        let config = test_config("slow-disconnect.example");
        let key = test_key(&config, 1);

        let old = registry
            .acquire(&key, config.clone())
            .await
            .expect("old generation should be created");
        let old_disconnect = old.disconnect.clone();
        old_disconnect.block();
        drop(old);

        advance(Duration::from_secs(10)).await;
        yield_until_count(&old_disconnect.attempts, 1).await;
        assert!(!slot_is_present(&registry, &key));

        let replacement = registry
            .acquire(&key, config)
            .await
            .expect("a blocked disconnect must not hold the registry lock");
        assert_eq!(replacement.id, 2);
        assert_eq!(ready_lifecycle(&registry, &key), (1, false));
        assert_eq!(factory.create_count.load(Ordering::SeqCst), 2);

        old_disconnect.unblock();
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        drop(replacement);
    }

    #[tokio::test(start_paused = true)]
    async fn disconnect_failure_is_diagnostic_and_does_not_restore_the_slot() {
        let factory = Arc::new(CountingFactory::default());
        let registry = Arc::new(SessionRegistryCore::with_idle_timeout(
            factory.clone(),
            Duration::from_secs(10),
        ));
        let config = test_config("failed-disconnect.example");
        let key = test_key(&config, 1);

        let failed = registry
            .acquire(&key, config.clone())
            .await
            .expect("first generation should be created");
        let failed_disconnect = failed.disconnect.clone();
        failed_disconnect.should_fail.store(true, Ordering::SeqCst);
        drop(failed);

        advance(Duration::from_secs(10)).await;
        yield_until_count(&failed_disconnect.attempts, 1).await;
        assert!(
            !slot_is_present(&registry, &key),
            "disconnect failure must not resurrect an expired registry slot"
        );

        let replacement = registry
            .acquire(&key, config)
            .await
            .expect("a later acquire should create a clean generation");
        assert_eq!(replacement.id, 2);
        assert_eq!(factory.create_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn repeated_idle_reacquire_uses_one_reaper_task() {
        let factory = Arc::new(CountingFactory::default());
        let registry = Arc::new(SessionRegistryCore::with_idle_timeout(
            factory.clone(),
            Duration::from_secs(10),
        ));
        let config = test_config("single-reaper.example");
        let key = test_key(&config, 1);

        let mut lease = registry
            .acquire(&key, config.clone())
            .await
            .expect("manager should be created");
        let manager_id = lease.id;
        let disconnect = lease.disconnect.clone();

        for _ in 0..20 {
            drop(lease);
            advance(Duration::from_secs(1)).await;
            lease = registry
                .acquire(&key, config.clone())
                .await
                .expect("manager should be reacquired before its deadline");
            assert_eq!(lease.id, manager_id);
        }

        assert_eq!(factory.create_count.load(Ordering::SeqCst), 1);
        assert_eq!(disconnect.attempts.load(Ordering::SeqCst), 0);
        assert_eq!(
            registry.shared.reaper_start_count.load(Ordering::SeqCst),
            1,
            "idle churn must not spawn per-release sleeping tasks"
        );

        drop(lease);
        advance(Duration::from_secs(10)).await;
        yield_until_count(&disconnect.attempts, 1).await;
    }

    #[tokio::test(start_paused = true)]
    async fn dropping_registry_cancels_and_converges_the_reaper() {
        let factory = Arc::new(CountingFactory::default());
        let registry = Arc::new(SessionRegistryCore::with_idle_timeout(
            factory,
            Duration::from_secs(10),
        ));
        let config = test_config("reaper-shutdown.example");
        let key = test_key(&config, 1);

        let lease = registry
            .acquire(&key, config)
            .await
            .expect("manager should be created");
        let disconnect = lease.disconnect.clone();
        drop(lease);

        let shared = registry.shared.clone();
        yield_until_true(&shared.reaper_running).await;
        assert_eq!(shared.reaper_start_count.load(Ordering::SeqCst), 1);

        drop(registry);
        yield_until_count(&shared.reaper_stop_count, 1).await;
        assert!(shared.reaper_shutdown.load(Ordering::SeqCst));
        assert!(!shared.reaper_running.load(Ordering::SeqCst));
        assert_eq!(
            disconnect.attempts.load(Ordering::SeqCst),
            0,
            "registry drop cancels the timer rather than doing async I/O in Drop"
        );
    }

    #[tokio::test]
    async fn retired_lease_drop_cannot_decrement_the_replacement_generation() {
        let factory = Arc::new(CountingFactory::default());
        let registry = Arc::new(SessionRegistryCore::new(factory.clone()));
        let config = test_config("retired-lease.example");
        let key = test_key(&config, 1);

        let old_lease = registry
            .acquire(&key, config.clone())
            .await
            .expect("old generation should be created");
        let old_manager = registry
            .retire(&key)
            .expect("ready generation should be retired");
        assert!(Arc::ptr_eq(&old_lease.manager, &old_manager));

        let replacement = registry
            .acquire(&key, config)
            .await
            .expect("replacement generation should be created");
        assert!(!Arc::ptr_eq(&old_lease.manager, &replacement.manager));
        assert_eq!(ready_lifecycle(&registry, &key), (1, false));

        let stale_clone = old_lease.clone();
        assert!(!stale_clone.counted);
        drop(old_lease);
        drop(stale_clone);
        assert_eq!(
            ready_lifecycle(&registry, &key),
            (1, false),
            "stale release must not touch the replacement count"
        );

        drop(old_manager);
        drop(replacement);
        assert_eq!(ready_lifecycle(&registry, &key), (0, true));
        assert_eq!(factory.create_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn cancelled_waiter_never_creates_a_phantom_lease() {
        let factory = Arc::new(BlockingFactory::new(["phantom.example"]));
        let registry = Arc::new(SessionRegistryCore::new(factory.clone()));
        let config = test_config("phantom.example");
        let key = test_key(&config, 1);

        let cancelled_registry = registry.clone();
        let cancelled_config = config.clone();
        let cancelled_key = key.clone();
        let cancelled = tokio::spawn(async move {
            cancelled_registry
                .acquire(&cancelled_key, cancelled_config)
                .await
        });
        wait_for_count(&factory.create_count, 1).await;
        cancelled.abort();
        let _ = cancelled.await;

        factory.release_one();
        wait_for_count(&factory.completed_count, 1).await;
        wait_for_ready(&registry, &key).await;
        assert_eq!(
            ready_lifecycle(&registry, &key),
            (0, true),
            "publishing a manager does not pre-charge cancelled waiters"
        );

        let lease = registry
            .acquire(&key, config)
            .await
            .expect("published idle manager should remain reusable");
        assert_eq!(lease.id, 1);
        assert_eq!(factory.create_count.load(Ordering::SeqCst), 1);
        assert_eq!(ready_lifecycle(&registry, &key), (1, false));
    }

    #[tokio::test]
    async fn public_lease_debug_is_credential_free() {
        let mut config = test_config("debug.example");
        config.username = "debug-user".to_owned();
        config.auth = SshAuth::PrivateKeyContent {
            private_key: "LEASE-PRIVATE-KEY-SECRET".to_owned(),
            passphrase: Some("LEASE-PASSPHRASE-SECRET".to_owned()),
            certificate_path: Some("/secret/lease-certificate.pub".to_owned()),
        };
        let key = test_key(&config, 1);
        let registry = SshSessionRegistry::new();

        let lease = registry
            .acquire(&key, config)
            .await
            .expect("manager construction should not dial eagerly");
        let cloned: super::SshSessionLease = lease.clone();
        assert_eq!(ready_lifecycle(registry.inner.as_ref(), &key), (2, false));
        drop(cloned);
        assert_eq!(ready_lifecycle(registry.inner.as_ref(), &key), (1, false));

        let debug = format!("{lease:?}");

        assert!(debug.contains("SshSessionLease"));
        assert!(debug.contains("debug-user@debug.example:22"));
        for secret in [
            "LEASE-PRIVATE-KEY-SECRET",
            "LEASE-PASSPHRASE-SECRET",
            "/secret/lease-certificate.pub",
            "SshConnectConfig",
        ] {
            assert!(
                !debug.contains(secret),
                "lease debug leaked {secret}: {debug}"
            );
        }
    }

    #[tokio::test]
    async fn public_service_api_acquires_observes_and_shuts_down() {
        let service =
            SshSessionService::with_timeouts(Duration::from_secs(60), Duration::from_secs(2));
        let mut observer = service.subscribe();
        assert_eq!(observer.borrow().state, SshSessionServiceState::Running);

        let config = test_config("public-service.example");
        let key = test_key(&config, 1);
        let lease = service
            .acquire(&key, config.clone())
            .await
            .expect("public service should return a generation-bound lease");
        assert_eq!(lease.manager().config().host, "public-service.example");
        assert_eq!(service.snapshot().active_lease_count, 1);

        observer
            .changed()
            .await
            .expect("public observer should remain connected");
        assert!(observer.borrow_and_update().revision > 0);
        drop(lease);

        let report = service.shutdown().await;
        assert!(!report.timed_out);
        assert_eq!(report.managers_requested, 1);
        assert_eq!(report.managers_completed, 1);
        assert_eq!(service.snapshot().state, SshSessionServiceState::Stopped);

        let error = service
            .acquire(&key, config)
            .await
            .expect_err("stopped public service must reject acquire");
        assert_eq!(error.to_string(), "SSH session service is shutting down");
        assert_eq!(service.shutdown().await, report);
    }

    #[tokio::test]
    async fn service_snapshot_tracks_leases_without_exposing_connection_data() {
        let factory = Arc::new(CountingFactory::default());
        let service = Arc::new(SessionServiceCore::new(
            factory,
            Duration::from_secs(60),
            Duration::from_secs(2),
        ));
        let mut config = test_config("snapshot-secret.example");
        config.auth = SshAuth::PrivateKeyContent {
            private_key: "SNAPSHOT-PRIVATE-KEY-SECRET".to_owned(),
            passphrase: Some("SNAPSHOT-PASSPHRASE-SECRET".to_owned()),
            certificate_path: Some("/secret/snapshot-certificate.pub".to_owned()),
        };
        let key = test_key(&config, 1);

        let initial = service.registry.snapshot();
        assert_eq!(initial.state, SshSessionServiceState::Running);
        assert_eq!(initial.slot_count, 0);

        let lease = service
            .registry
            .acquire(&key, config)
            .await
            .expect("service acquire should create a lease");
        let active = service.registry.snapshot();
        assert_eq!(active.state, SshSessionServiceState::Running);
        assert_eq!(active.slot_count, 1);
        assert_eq!(active.creating_count, 0);
        assert_eq!(active.ready_count, 1);
        assert_eq!(active.active_lease_count, 1);
        assert_eq!(active.idle_count, 0);
        assert_eq!(active.registry_task_count, 1);
        assert!(active.revision > initial.revision);

        let debug = format!("{active:?}");
        for secret in [
            "snapshot-secret.example",
            "SNAPSHOT-PRIVATE-KEY-SECRET",
            "SNAPSHOT-PASSPHRASE-SECRET",
            "/secret/snapshot-certificate.pub",
            "SshConnectConfig",
            "ConnectionKey",
        ] {
            assert!(
                !debug.contains(secret),
                "service snapshot leaked {secret}: {debug}"
            );
        }

        drop(lease);
        let idle = service.registry.snapshot();
        assert_eq!(idle.active_lease_count, 0);
        assert_eq!(idle.idle_count, 1);

        let report = service.shutdown().await;
        assert!(!report.timed_out);
        assert_eq!(report.registry_tasks_remaining, 0);
        assert_eq!(
            service.registry.snapshot().state,
            SshSessionServiceState::Stopped
        );
    }

    #[tokio::test]
    async fn service_shutdown_rejects_acquire_and_closes_retained_generations() {
        let factory = Arc::new(CountingFactory::default());
        let service = Arc::new(SessionServiceCore::new(
            factory.clone(),
            Duration::from_secs(60),
            Duration::from_secs(2),
        ));
        let config = test_config("service-shutdown.example");
        let key = test_key(&config, 1);
        let lease = service
            .registry
            .acquire(&key, config.clone())
            .await
            .expect("manager should be created");
        let disconnect = lease.disconnect.clone();
        disconnect.block();

        let shutdown_service = service.clone();
        let shutdown = tokio::spawn(async move { shutdown_service.shutdown().await });
        yield_until_true(&disconnect.shutdown_requested).await;

        let error = service
            .registry
            .acquire(&key, config)
            .await
            .expect_err("shutdown must close acquire admission");
        assert_eq!(error.to_string(), "SSH session service is shutting down");
        assert_eq!(factory.create_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            service.registry.snapshot().state,
            SshSessionServiceState::ShuttingDown
        );

        let stale_clone = lease.clone();
        assert!(
            !stale_clone.counted,
            "a lease retained past shutdown cannot revive registry accounting"
        );
        disconnect.unblock();
        let report = shutdown.await.expect("shutdown task should not panic");
        assert!(!report.timed_out);
        assert_eq!(report.managers_requested, 1);
        assert_eq!(report.managers_completed, 1);
        assert_eq!(report.manager_failures, 0);
        assert_eq!(disconnect.attempts.load(Ordering::SeqCst), 1);
        assert_eq!(service.registry.snapshot().slot_count, 0);

        drop(stale_clone);
        drop(lease);
        assert_eq!(service.registry.snapshot().slot_count, 0);
    }

    #[tokio::test]
    async fn concurrent_and_repeated_service_shutdown_share_one_result() {
        let factory = Arc::new(CountingFactory::default());
        let service = Arc::new(SessionServiceCore::new(
            factory,
            Duration::from_secs(60),
            Duration::from_secs(2),
        ));
        let config = test_config("concurrent-shutdown.example");
        let key = test_key(&config, 1);
        let lease = service
            .registry
            .acquire(&key, config)
            .await
            .expect("manager should be created");
        let disconnect = lease.disconnect.clone();
        disconnect.block();

        let mut shutdowns = Vec::new();
        for _ in 0..8 {
            let service = service.clone();
            shutdowns.push(tokio::spawn(async move { service.shutdown().await }));
        }
        yield_until_true(&disconnect.shutdown_requested).await;
        yield_until_count(&disconnect.attempts, 1).await;
        disconnect.unblock();

        let mut reports = Vec::new();
        for shutdown in shutdowns {
            reports.push(shutdown.await.expect("shutdown caller should not panic"));
        }
        assert!(reports.iter().all(|report| report == &reports[0]));
        assert_eq!(disconnect.attempts.load(Ordering::SeqCst), 1);
        assert_eq!(service.shutdown().await, reports[0]);
        assert_eq!(disconnect.attempts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn service_shutdown_cancels_an_inflight_creation_and_its_waiters() {
        let factory = Arc::new(BlockingFactory::new(["creating-shutdown.example"]));
        let service = Arc::new(SessionServiceCore::new(
            factory.clone(),
            Duration::from_secs(60),
            Duration::from_secs(2),
        ));
        let config = test_config("creating-shutdown.example");
        let key = test_key(&config, 1);

        let acquire_service = service.clone();
        let acquire =
            tokio::spawn(async move { acquire_service.registry.acquire(&key, config).await });
        wait_for_count(&factory.create_count, 1).await;

        let report = service.shutdown().await;
        let error = acquire
            .await
            .expect("acquire task should not panic")
            .expect_err("creation waiter should observe shutdown");

        assert_eq!(error.to_string(), "SSH session service is shutting down");
        assert!(!report.timed_out);
        assert_eq!(report.managers_requested, 0);
        assert_eq!(report.registry_tasks_remaining, 0);
        assert_eq!(factory.completed_count.load(Ordering::SeqCst), 0);
        let snapshot = service.registry.snapshot();
        assert_eq!(snapshot.state, SshSessionServiceState::Stopped);
        assert_eq!(snapshot.slot_count, 0);
        assert_eq!(snapshot.registry_task_count, 0);
    }

    #[tokio::test]
    async fn service_shutdown_deadline_bounds_a_stuck_disconnect() {
        let factory = Arc::new(CountingFactory::default());
        let service = Arc::new(SessionServiceCore::new(
            factory,
            Duration::from_secs(60),
            Duration::from_millis(20),
        ));
        let config = test_config("shutdown-deadline.example");
        let key = test_key(&config, 1);
        let lease = service
            .registry
            .acquire(&key, config)
            .await
            .expect("manager should be created");
        let disconnect = lease.disconnect.clone();
        disconnect.block();

        let report = timeout(Duration::from_secs(1), service.shutdown())
            .await
            .expect("service shutdown must have a hard upper bound");

        assert!(report.timed_out);
        assert_eq!(report.managers_requested, 1);
        assert_eq!(report.managers_completed, 0);
        assert_eq!(report.managers_remaining, 1);
        assert_eq!(report.registry_tasks_remaining, 0);
        assert!(disconnect.shutdown_requested.load(Ordering::SeqCst));
        assert_eq!(
            service.registry.snapshot().state,
            SshSessionServiceState::Stopped
        );
        assert_eq!(service.shutdown().await, report);
    }

    #[tokio::test]
    async fn service_shutdown_reports_disconnect_failure_without_restoring_slots() {
        let factory = Arc::new(CountingFactory::default());
        let service = Arc::new(SessionServiceCore::new(
            factory,
            Duration::from_secs(60),
            Duration::from_secs(2),
        ));
        let config = test_config("shutdown-failure.example");
        let key = test_key(&config, 1);
        let lease = service
            .registry
            .acquire(&key, config)
            .await
            .expect("manager should be created");
        lease.disconnect.should_fail.store(true, Ordering::SeqCst);

        let report = service.shutdown().await;

        assert!(!report.timed_out);
        assert_eq!(report.managers_requested, 1);
        assert_eq!(report.managers_completed, 1);
        assert_eq!(report.manager_failures, 1);
        assert_eq!(report.managers_remaining, 0);
        assert_eq!(service.registry.snapshot().slot_count, 0);
    }

    #[tokio::test]
    async fn service_observer_sees_running_shutting_down_and_stopped() {
        let factory = Arc::new(CountingFactory::default());
        let service = Arc::new(SessionServiceCore::new(
            factory,
            Duration::from_secs(60),
            Duration::from_secs(2),
        ));
        let config = test_config("observer.example");
        let key = test_key(&config, 1);
        let lease = service
            .registry
            .acquire(&key, config)
            .await
            .expect("manager should be created");
        let disconnect = lease.disconnect.clone();
        disconnect.block();
        let mut observer = service.registry.subscribe();
        assert_eq!(observer.borrow().state, SshSessionServiceState::Running);

        let shutdown_service = service.clone();
        let shutdown = tokio::spawn(async move { shutdown_service.shutdown().await });
        timeout(Duration::from_secs(1), observer.changed())
            .await
            .expect("observer should change")
            .expect("snapshot sender should remain alive");
        assert_eq!(
            observer.borrow_and_update().state,
            SshSessionServiceState::ShuttingDown
        );

        disconnect.unblock();
        shutdown.await.expect("shutdown task should not panic");
        timeout(Duration::from_secs(1), async {
            loop {
                observer
                    .changed()
                    .await
                    .expect("snapshot sender should remain alive");
                if observer.borrow_and_update().state == SshSessionServiceState::Stopped {
                    break;
                }
            }
        })
        .await
        .expect("observer should see the stopped state");
    }

    #[tokio::test]
    async fn dropping_service_only_requests_synchronous_cleanup() {
        let factory = Arc::new(CountingFactory::default());
        let service = Arc::new(SessionServiceCore::new(
            factory,
            Duration::from_secs(60),
            Duration::from_secs(2),
        ));
        let config = test_config("service-drop.example");
        let key = test_key(&config, 1);
        let lease = service
            .registry
            .acquire(&key, config)
            .await
            .expect("manager should be created");
        let disconnect = lease.disconnect.clone();

        drop(service);
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }

        assert!(disconnect.shutdown_requested.load(Ordering::SeqCst));
        assert_eq!(
            disconnect.attempts.load(Ordering::SeqCst),
            0,
            "Drop must not launch or await transport I/O"
        );
        drop(lease);
    }
}
