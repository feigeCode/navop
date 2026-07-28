use super::model::{
    OperationGenerationId, OperationId, OperationJournal, OperationJournalError,
    OperationJournalSessionId, OperationKind, OperationStatus,
};
use super::persistence::{
    OperationJournalFileStore, OperationJournalPersistenceConfig, OperationJournalPersistenceError,
};
use super::redaction::RedactedOperationPayload;
use super::session_history::{
    OperationJournalHistoryConfig, OperationJournalHistoryError, OperationJournalHistoryStore,
    OperationJournalScope, OperationJournalSessionManifest,
};
use std::collections::VecDeque;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const DEFAULT_MAX_PENDING_OPERATIONS: usize = 1_024;
const DEFAULT_MAX_PENDING_OPERATION_BYTES: usize = 1024 * 1024;
const DEFAULT_MAX_PENDING_CONTROLS: usize = 16;
const MAX_PENDING_OPERATIONS: usize = 65_536;
const MAX_PENDING_OPERATION_BYTES: usize = 64 * 1024 * 1024;
const MAX_PENDING_CONTROLS: usize = 1_024;
const QUEUED_OPERATION_OVERHEAD_BYTES: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationJournalQueueLimits {
    pub max_pending_operations: usize,
    pub max_pending_bytes: usize,
    pub max_pending_controls: usize,
}

impl Default for OperationJournalQueueLimits {
    fn default() -> Self {
        Self {
            max_pending_operations: DEFAULT_MAX_PENDING_OPERATIONS,
            max_pending_bytes: DEFAULT_MAX_PENDING_OPERATION_BYTES,
            max_pending_controls: DEFAULT_MAX_PENDING_CONTROLS,
        }
    }
}

#[derive(Clone, Debug)]
pub struct OperationJournalRuntimeConfig {
    pub root: PathBuf,
    pub queue: OperationJournalQueueLimits,
    pub persistence: OperationJournalPersistenceConfig,
}

impl OperationJournalRuntimeConfig {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            queue: OperationJournalQueueLimits::default(),
            persistence: OperationJournalPersistenceConfig::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationJournalRuntimeHealth {
    Initializing,
    Healthy,
    QueueFull,
    PersistenceFailed,
    Unavailable,
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationJournalRuntimeSnapshot {
    pub health: OperationJournalRuntimeHealth,
    pub pending_operations: usize,
    pub pending_bytes: usize,
    pub dropped_operations: u64,
    pub last_error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OperationJournalAttempt {
    status: OperationStatus,
    occurred_at_unix_ms: u64,
}

impl OperationJournalAttempt {
    pub fn sent(occurred_at_unix_ms: u64) -> Self {
        Self {
            status: OperationStatus::Sent,
            occurred_at_unix_ms,
        }
    }

    pub fn failed(occurred_at_unix_ms: u64) -> Self {
        Self {
            status: OperationStatus::Failed,
            occurred_at_unix_ms,
        }
    }

    pub fn canceled(occurred_at_unix_ms: u64) -> Self {
        Self {
            status: OperationStatus::Canceled,
            occurred_at_unix_ms,
        }
    }

    pub fn status(self) -> OperationStatus {
        self.status
    }

    pub fn occurred_at_unix_ms(self) -> u64 {
        self.occurred_at_unix_ms
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperationJournalRuntimeError {
    InvalidConfig(String),
    InvalidOperation(OperationJournalError),
    WorkerSpawn(String),
    Closed,
    QueueFull,
    ControlQueueFull,
    WorkerStopped,
    Unavailable(String),
    PersistenceFailed(String),
    TimedOut,
}

impl fmt::Display for OperationJournalRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(reason) => {
                write!(
                    formatter,
                    "invalid operation journal runtime config: {reason}"
                )
            }
            Self::InvalidOperation(error) => {
                write!(formatter, "invalid operation journal item: {error}")
            }
            Self::WorkerSpawn(reason) => {
                write!(
                    formatter,
                    "failed to spawn operation journal worker: {reason}"
                )
            }
            Self::Closed => formatter.write_str("operation journal runtime is closed"),
            Self::QueueFull => formatter.write_str("operation journal queue is full"),
            Self::ControlQueueFull => {
                formatter.write_str("operation journal control queue is full")
            }
            Self::WorkerStopped => formatter.write_str("operation journal worker stopped"),
            Self::Unavailable(reason) => {
                write!(formatter, "operation journal is unavailable: {reason}")
            }
            Self::PersistenceFailed(reason) => {
                write!(formatter, "operation journal persistence failed: {reason}")
            }
            Self::TimedOut => formatter.write_str("operation journal worker timed out"),
        }
    }
}

impl std::error::Error for OperationJournalRuntimeError {}

#[derive(Clone)]
pub struct OperationJournalRuntime {
    core: Arc<RuntimeCore>,
}

impl OperationJournalRuntime {
    pub fn new(
        config: OperationJournalRuntimeConfig,
        session_id: OperationJournalSessionId,
        scope: OperationJournalScope,
        initial_generation_id: OperationGenerationId,
        started_at_unix_ms: u64,
    ) -> Result<Self, OperationJournalRuntimeError> {
        Self::with_observer(
            config,
            session_id,
            scope,
            initial_generation_id,
            started_at_unix_ms,
            |_| {},
        )
    }

    pub fn with_observer(
        config: OperationJournalRuntimeConfig,
        session_id: OperationJournalSessionId,
        scope: OperationJournalScope,
        initial_generation_id: OperationGenerationId,
        started_at_unix_ms: u64,
        observer: impl Fn(OperationJournalRuntimeHealth) + Send + Sync + 'static,
    ) -> Result<Self, OperationJournalRuntimeError> {
        Self::spawn(
            config,
            session_id,
            scope,
            initial_generation_id,
            started_at_unix_ms,
            Arc::new(observer),
            test_gate_none(),
        )
    }

    #[cfg(test)]
    pub(super) fn new_with_test_gate(
        config: OperationJournalRuntimeConfig,
        session_id: OperationJournalSessionId,
        scope: OperationJournalScope,
        initial_generation_id: OperationGenerationId,
        started_at_unix_ms: u64,
        test_gate: OperationJournalWorkerTestGate,
    ) -> Result<Self, OperationJournalRuntimeError> {
        Self::spawn(
            config,
            session_id,
            scope,
            initial_generation_id,
            started_at_unix_ms,
            Arc::new(|_| {}),
            Some(test_gate),
        )
    }

    fn spawn(
        config: OperationJournalRuntimeConfig,
        session_id: OperationJournalSessionId,
        scope: OperationJournalScope,
        initial_generation_id: OperationGenerationId,
        started_at_unix_ms: u64,
        observer: Arc<dyn Fn(OperationJournalRuntimeHealth) + Send + Sync>,
        test_gate: TestGateOption,
    ) -> Result<Self, OperationJournalRuntimeError> {
        validate_runtime_config(&config)?;

        let queue = Arc::new(RuntimeQueue::new(config.queue.clone()));
        let shared = Arc::new(RuntimeShared::new(observer));
        let worker_queue = queue.clone();
        let worker_shared = shared.clone();
        let worker = thread::Builder::new()
            .name("terminal-operation-journal".to_string())
            .spawn(move || {
                let _exit_guard = WorkerExitGuard::new(test_gate.clone());
                let worker_result = catch_unwind(AssertUnwindSafe(|| {
                    operation_journal_worker(
                        worker_queue.clone(),
                        worker_shared.clone(),
                        WorkerStart {
                            root: config.root,
                            persistence: config.persistence,
                            session_id,
                            scope,
                            initial_generation_id,
                            started_at_unix_ms,
                        },
                        test_gate,
                    )
                }));
                if worker_result.is_err() {
                    let error = OperationJournalRuntimeError::Unavailable(
                        "operation journal worker panicked".to_string(),
                    );
                    worker_shared.mark_failure(OperationJournalRuntimeHealth::Unavailable, &error);
                    worker_queue.abort(error);
                }
            })
            .map_err(|error| OperationJournalRuntimeError::WorkerSpawn(error.to_string()))?;

        Ok(Self {
            core: Arc::new(RuntimeCore {
                queue,
                shared,
                worker: Mutex::new(Some(worker)),
            }),
        })
    }

    /// Queues a fully redacted operation attempt without waiting for disk I/O.
    ///
    /// Raw terminal bytes cannot cross this API boundary. Callers must redact
    /// sensitive input before constructing the work item. Queue exhaustion is
    /// fail-closed for the journal only: the runtime rejects all later data, but
    /// callers remain free to continue the live terminal write path.
    pub fn record_attempt(
        &self,
        kind: OperationKind,
        parent_operation_id: Option<&OperationId>,
        redacted_payload: Option<RedactedOperationPayload>,
        queued_at_unix_ms: u64,
        attempt: OperationJournalAttempt,
    ) -> Result<OperationId, OperationJournalRuntimeError> {
        kind.validate_redacted_payload(redacted_payload.as_ref())
            .map_err(OperationJournalRuntimeError::InvalidOperation)?;
        let operation_id = OperationId::new();
        let cost_bytes = queued_operation_cost(
            &operation_id,
            parent_operation_id,
            redacted_payload.as_ref(),
        )?;
        let action = OperationAction {
            operation_id: operation_id.clone(),
            kind,
            parent_operation_id: parent_operation_id.cloned(),
            redacted_payload,
            queued_at_unix_ms,
            attempt,
        };

        match self.core.queue.enqueue_operation(action, cost_bytes) {
            Ok(()) => Ok(operation_id),
            Err(QueueEnqueueError::Full) => {
                self.core.shared.mark_queue_full();
                Err(OperationJournalRuntimeError::QueueFull)
            }
            Err(error) => Err(error.into_runtime_error()),
        }
    }

    /// Enqueues a reconnect generation boundary in the same FIFO as operations.
    pub fn begin_generation(
        &self,
        generation_id: OperationGenerationId,
        started_at_unix_ms: u64,
    ) -> Result<(), OperationJournalRuntimeError> {
        match self.core.queue.enqueue_control(
            ControlAction::BeginGeneration {
                generation_id,
                started_at_unix_ms,
            },
            true,
        ) {
            Ok(()) => Ok(()),
            Err(QueueEnqueueError::Full) => {
                self.core.shared.mark_queue_full();
                Err(OperationJournalRuntimeError::ControlQueueFull)
            }
            Err(error) => Err(error.into_runtime_error()),
        }
    }

    pub fn flush(&self, timeout: Duration) -> Result<(), OperationJournalRuntimeError> {
        let (sender, receiver) = mpsc::sync_channel(1);
        match self
            .core
            .queue
            .enqueue_control(ControlAction::Flush(sender), false)
        {
            Ok(()) => {}
            Err(QueueEnqueueError::Full) => {
                return Err(OperationJournalRuntimeError::ControlQueueFull);
            }
            Err(error) => return Err(error.into_runtime_error()),
        }
        receive_control_result(receiver, timeout)
    }

    /// Stops accepting new operation data and waits at most `timeout` for all
    /// already accepted work to be durably published.
    pub fn shutdown(&self, timeout: Duration) -> Result<(), OperationJournalRuntimeError> {
        let (sender, receiver) = mpsc::sync_channel(1);
        match self
            .core
            .queue
            .enqueue_control(ControlAction::Shutdown(sender), false)
        {
            Ok(()) => {}
            Err(error) => {
                self.core.join_worker_if_finished();
                return Err(error.into_runtime_error());
            }
        }

        match receive_control_result(receiver, timeout) {
            Ok(()) => {
                self.core.join_worker();
                Ok(())
            }
            Err(OperationJournalRuntimeError::TimedOut) => {
                self.core
                    .queue
                    .abort(OperationJournalRuntimeError::TimedOut);
                self.core.shared.mark_closed();
                self.core.detach_worker();
                Err(OperationJournalRuntimeError::TimedOut)
            }
            Err(error) => {
                self.core.join_worker_if_finished();
                Err(error)
            }
        }
    }

    pub fn snapshot(&self) -> OperationJournalRuntimeSnapshot {
        let QueueSnapshot {
            pending_operations,
            pending_bytes,
            dropped_operations,
        } = self.core.queue.snapshot();
        let (health, last_error) = self.core.shared.snapshot();
        OperationJournalRuntimeSnapshot {
            health,
            pending_operations,
            pending_bytes,
            dropped_operations,
            last_error,
        }
    }
}

struct RuntimeCore {
    queue: Arc<RuntimeQueue>,
    shared: Arc<RuntimeShared>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl RuntimeCore {
    fn join_worker(&self) {
        let worker = lock_unpoisoned(&self.worker).take();
        let Some(worker) = worker else {
            return;
        };
        if worker.thread().id() != thread::current().id() {
            let _ = worker.join();
        }
    }

    fn join_worker_if_finished(&self) {
        let worker = {
            let mut worker = lock_unpoisoned(&self.worker);
            if worker.as_ref().is_some_and(JoinHandle::is_finished) {
                worker.take()
            } else {
                None
            }
        };
        if let Some(worker) = worker {
            let _ = worker.join();
        }
    }

    fn detach_worker(&self) {
        let _ = lock_unpoisoned(&self.worker).take();
    }
}

impl Drop for RuntimeCore {
    fn drop(&mut self) {
        self.queue
            .abort(OperationJournalRuntimeError::WorkerStopped);
        self.shared.mark_closed();
        let _ = lock_unpoisoned(&self.worker).take();
    }
}

fn validate_runtime_config(
    config: &OperationJournalRuntimeConfig,
) -> Result<(), OperationJournalRuntimeError> {
    if config.queue.max_pending_operations == 0 {
        return Err(OperationJournalRuntimeError::InvalidConfig(
            "max_pending_operations must be greater than zero".to_string(),
        ));
    }
    if config.queue.max_pending_bytes == 0 {
        return Err(OperationJournalRuntimeError::InvalidConfig(
            "max_pending_bytes must be greater than zero".to_string(),
        ));
    }
    if config.queue.max_pending_controls == 0 {
        return Err(OperationJournalRuntimeError::InvalidConfig(
            "max_pending_controls must be greater than zero".to_string(),
        ));
    }
    if config.queue.max_pending_operations > MAX_PENDING_OPERATIONS {
        return Err(OperationJournalRuntimeError::InvalidConfig(
            "max_pending_operations exceeds the hard limit".to_string(),
        ));
    }
    if config.queue.max_pending_bytes > MAX_PENDING_OPERATION_BYTES {
        return Err(OperationJournalRuntimeError::InvalidConfig(
            "max_pending_bytes exceeds the hard limit".to_string(),
        ));
    }
    if config.queue.max_pending_controls > MAX_PENDING_CONTROLS {
        return Err(OperationJournalRuntimeError::InvalidConfig(
            "max_pending_controls exceeds the hard limit".to_string(),
        ));
    }
    config
        .persistence
        .validate()
        .map_err(|error| OperationJournalRuntimeError::InvalidConfig(error.to_string()))
}

fn queued_operation_cost(
    operation_id: &OperationId,
    parent_operation_id: Option<&OperationId>,
    redacted_payload: Option<&RedactedOperationPayload>,
) -> Result<usize, OperationJournalRuntimeError> {
    let payload_bytes = redacted_payload
        .map(serde_json::to_vec)
        .transpose()
        .map_err(|error| {
            OperationJournalRuntimeError::InvalidConfig(format!(
                "failed to size redacted operation payload: {error}"
            ))
        })?
        .map_or(0, |payload| payload.len());
    QUEUED_OPERATION_OVERHEAD_BYTES
        .checked_add(operation_id.as_str().len())
        .and_then(|total| {
            total.checked_add(
                parent_operation_id
                    .map(OperationId::as_str)
                    .map_or(0, str::len),
            )
        })
        .and_then(|total| total.checked_add(payload_bytes))
        .ok_or_else(|| {
            OperationJournalRuntimeError::InvalidConfig(
                "redacted operation payload size overflowed".to_string(),
            )
        })
}

fn receive_control_result(
    receiver: Receiver<Result<(), OperationJournalRuntimeError>>,
    timeout: Duration,
) -> Result<(), OperationJournalRuntimeError> {
    receiver
        .recv_timeout(timeout)
        .unwrap_or_else(|error| match error {
            mpsc::RecvTimeoutError::Timeout => Err(OperationJournalRuntimeError::TimedOut),
            mpsc::RecvTimeoutError::Disconnected => {
                Err(OperationJournalRuntimeError::WorkerStopped)
            }
        })
}

struct RuntimeShared {
    state: Mutex<RuntimeSharedState>,
    observer: Arc<dyn Fn(OperationJournalRuntimeHealth) + Send + Sync>,
}

struct RuntimeSharedState {
    health: OperationJournalRuntimeHealth,
    last_error: Option<String>,
}

impl RuntimeShared {
    fn new(observer: Arc<dyn Fn(OperationJournalRuntimeHealth) + Send + Sync>) -> Self {
        Self {
            state: Mutex::new(RuntimeSharedState {
                health: OperationJournalRuntimeHealth::Initializing,
                last_error: None,
            }),
            observer,
        }
    }

    fn snapshot(&self) -> (OperationJournalRuntimeHealth, Option<String>) {
        let state = lock_unpoisoned(&self.state);
        (state.health, state.last_error.clone())
    }

    fn mark_healthy(&self) {
        let changed = {
            let mut state = lock_unpoisoned(&self.state);
            if state.health == OperationJournalRuntimeHealth::Initializing {
                state.health = OperationJournalRuntimeHealth::Healthy;
                true
            } else {
                false
            }
        };
        if changed {
            self.notify_observer(OperationJournalRuntimeHealth::Healthy);
        }
    }

    fn mark_queue_full(&self) {
        self.update(
            OperationJournalRuntimeHealth::QueueFull,
            Some("operation journal queue exhausted its bounded capacity".to_string()),
        );
    }

    fn mark_failure(
        &self,
        health: OperationJournalRuntimeHealth,
        error: &OperationJournalRuntimeError,
    ) {
        self.update(health, Some(error.to_string()));
    }

    fn mark_closed(&self) {
        self.update(OperationJournalRuntimeHealth::Closed, None);
    }

    fn update(&self, health: OperationJournalRuntimeHealth, last_error: Option<String>) {
        let changed = {
            let mut state = lock_unpoisoned(&self.state);
            if state.health == health && (last_error.is_none() || state.last_error == last_error) {
                false
            } else {
                state.health = health;
                if last_error.is_some() {
                    state.last_error = last_error;
                }
                true
            }
        };
        if changed {
            self.notify_observer(health);
        }
    }

    fn notify_observer(&self, health: OperationJournalRuntimeHealth) {
        let _ = catch_unwind(AssertUnwindSafe(|| (self.observer)(health)));
    }
}

struct RuntimeQueue {
    limits: OperationJournalQueueLimits,
    state: Mutex<RuntimeQueueState>,
    ready: Condvar,
}

struct RuntimeQueueState {
    entries: VecDeque<QueueEntry>,
    pending_operations: usize,
    pending_bytes: usize,
    pending_controls: usize,
    dropped_operations: u64,
    accepting_data: bool,
    shutdown_enqueued: bool,
    stopped: bool,
    worker_error: Option<OperationJournalRuntimeError>,
}

#[derive(Clone, Copy)]
struct QueueSnapshot {
    pending_operations: usize,
    pending_bytes: usize,
    dropped_operations: u64,
}

impl RuntimeQueue {
    fn new(limits: OperationJournalQueueLimits) -> Self {
        Self {
            limits,
            state: Mutex::new(RuntimeQueueState {
                entries: VecDeque::new(),
                pending_operations: 0,
                pending_bytes: 0,
                pending_controls: 0,
                dropped_operations: 0,
                accepting_data: true,
                shutdown_enqueued: false,
                stopped: false,
                worker_error: None,
            }),
            ready: Condvar::new(),
        }
    }

    fn enqueue_operation(
        &self,
        action: OperationAction,
        cost_bytes: usize,
    ) -> Result<(), QueueEnqueueError> {
        let mut state = lock_unpoisoned(&self.state);
        if let Some(error) = state.worker_error.clone() {
            return Err(QueueEnqueueError::Worker(error));
        }
        if !state.accepting_data || state.stopped {
            return Err(QueueEnqueueError::Closed);
        }

        let next_operations = state.pending_operations.saturating_add(1);
        let next_bytes = state.pending_bytes.saturating_add(cost_bytes);
        if next_operations > self.limits.max_pending_operations
            || next_bytes > self.limits.max_pending_bytes
        {
            state.dropped_operations = state.dropped_operations.saturating_add(1);
            state.accepting_data = false;
            return Err(QueueEnqueueError::Full);
        }

        state
            .entries
            .push_back(QueueEntry::Operation { action, cost_bytes });
        state.pending_operations = next_operations;
        state.pending_bytes = next_bytes;
        self.ready.notify_one();
        Ok(())
    }

    fn enqueue_control(
        &self,
        action: ControlAction,
        requires_live_data: bool,
    ) -> Result<(), QueueEnqueueError> {
        let mut state = lock_unpoisoned(&self.state);
        if let Some(error) = state.worker_error.clone() {
            return Err(QueueEnqueueError::Worker(error));
        }
        if state.stopped {
            return Err(QueueEnqueueError::Closed);
        }
        if requires_live_data && !state.accepting_data {
            return Err(QueueEnqueueError::Closed);
        }

        let is_shutdown = matches!(action, ControlAction::Shutdown(_));
        if is_shutdown {
            if state.shutdown_enqueued {
                return Err(QueueEnqueueError::Closed);
            }
            state.shutdown_enqueued = true;
            state.accepting_data = false;
        } else if state.pending_controls >= self.limits.max_pending_controls {
            if requires_live_data {
                state.accepting_data = false;
            }
            return Err(QueueEnqueueError::Full);
        }

        state.entries.push_back(QueueEntry::Control(action));
        state.pending_controls = state.pending_controls.saturating_add(1);
        self.ready.notify_one();
        Ok(())
    }

    fn pop(&self) -> Option<QueueEntry> {
        let mut state = lock_unpoisoned(&self.state);
        loop {
            if state.stopped {
                return None;
            }
            if let Some(entry) = state.entries.pop_front() {
                if matches!(entry, QueueEntry::Control(_)) {
                    state.pending_controls = state.pending_controls.saturating_sub(1);
                }
                return Some(entry);
            }
            state = wait_unpoisoned(&self.ready, state);
        }
    }

    fn drain_consecutive_operations(&self, destination: &mut Vec<OperationAction>) -> usize {
        let mut state = lock_unpoisoned(&self.state);
        let mut drained_bytes = 0usize;
        while matches!(state.entries.front(), Some(QueueEntry::Operation { .. })) {
            let entry = state
                .entries
                .pop_front()
                .expect("front operation exists while draining");
            let QueueEntry::Operation { action, cost_bytes } = entry else {
                unreachable!("operation queue front was checked")
            };
            drained_bytes = drained_bytes.saturating_add(cost_bytes);
            destination.push(action);
        }
        drained_bytes
    }

    fn finish_operations(&self, operation_count: usize, cost_bytes: usize) {
        let mut state = lock_unpoisoned(&self.state);
        state.pending_operations = state.pending_operations.saturating_sub(operation_count);
        state.pending_bytes = state.pending_bytes.saturating_sub(cost_bytes);
    }

    fn snapshot(&self) -> QueueSnapshot {
        let state = lock_unpoisoned(&self.state);
        QueueSnapshot {
            pending_operations: state.pending_operations,
            pending_bytes: state.pending_bytes,
            dropped_operations: state.dropped_operations,
        }
    }

    fn finish(&self) {
        let mut state = lock_unpoisoned(&self.state);
        state.accepting_data = false;
        state.stopped = true;
        state.worker_error = Some(OperationJournalRuntimeError::Closed);
        fail_pending_controls(&mut state, OperationJournalRuntimeError::Closed);
        self.ready.notify_all();
    }

    fn abort(&self, error: OperationJournalRuntimeError) {
        let mut state = lock_unpoisoned(&self.state);
        if state.stopped {
            return;
        }
        state.accepting_data = false;
        state.stopped = true;
        state.worker_error = Some(error.clone());
        fail_pending_controls(&mut state, error);
        self.ready.notify_all();
    }
}

fn fail_pending_controls(state: &mut RuntimeQueueState, error: OperationJournalRuntimeError) {
    for entry in state.entries.drain(..) {
        if let QueueEntry::Control(action) = entry {
            action.fail(error.clone());
        }
    }
    state.pending_operations = 0;
    state.pending_bytes = 0;
    state.pending_controls = 0;
}

enum QueueEnqueueError {
    Closed,
    Full,
    Worker(OperationJournalRuntimeError),
}

impl QueueEnqueueError {
    fn into_runtime_error(self) -> OperationJournalRuntimeError {
        match self {
            Self::Closed => OperationJournalRuntimeError::Closed,
            Self::Full => OperationJournalRuntimeError::QueueFull,
            Self::Worker(error) => error,
        }
    }
}

enum QueueEntry {
    Operation {
        action: OperationAction,
        cost_bytes: usize,
    },
    Control(ControlAction),
}

struct OperationAction {
    operation_id: OperationId,
    kind: OperationKind,
    parent_operation_id: Option<OperationId>,
    redacted_payload: Option<RedactedOperationPayload>,
    queued_at_unix_ms: u64,
    attempt: OperationJournalAttempt,
}

enum ControlAction {
    BeginGeneration {
        generation_id: OperationGenerationId,
        started_at_unix_ms: u64,
    },
    Flush(SyncSender<Result<(), OperationJournalRuntimeError>>),
    Shutdown(SyncSender<Result<(), OperationJournalRuntimeError>>),
}

impl ControlAction {
    fn fail(self, error: OperationJournalRuntimeError) {
        match self {
            Self::Flush(sender) | Self::Shutdown(sender) => {
                let _ = sender.send(Err(error));
            }
            Self::BeginGeneration { .. } => {}
        }
    }
}

struct WorkerStart {
    root: PathBuf,
    persistence: OperationJournalPersistenceConfig,
    session_id: OperationJournalSessionId,
    scope: OperationJournalScope,
    initial_generation_id: OperationGenerationId,
    started_at_unix_ms: u64,
}

struct WorkerState {
    journal: OperationJournal,
    file_store: OperationJournalFileStore,
    history_store: OperationJournalHistoryStore,
    manifest: OperationJournalSessionManifest,
}

impl WorkerState {
    fn open(start: WorkerStart) -> Result<Self, OperationJournalRuntimeError> {
        let history_store = OperationJournalHistoryStore::new(
            &start.root,
            OperationJournalHistoryConfig {
                persistence: start.persistence.clone(),
                ..OperationJournalHistoryConfig::default()
            },
        )
        .map_err(unavailable_history_error)?;
        let paths = history_store.paths_for_session(&start.session_id);
        let (mut file_store, recovery) = OperationJournalFileStore::open(paths, start.persistence)
            .map_err(unavailable_persistence_error)?;
        if recovery.journal().is_some() {
            return Err(OperationJournalRuntimeError::Unavailable(
                "session storage already contains a journal snapshot".to_string(),
            ));
        }

        let journal = OperationJournal::new(
            start.session_id.clone(),
            start.initial_generation_id,
            start.started_at_unix_ms,
        );
        let manifest = OperationJournalSessionManifest::new(
            start.session_id,
            start.scope,
            start.started_at_unix_ms,
        )
        .map_err(unavailable_history_error)?;
        history_store
            .write_manifest(&manifest)
            .map_err(persistence_history_error)?;
        file_store.persist(&journal).map_err(persistence_error)?;

        Ok(Self {
            journal,
            file_store,
            history_store,
            manifest,
        })
    }

    fn apply_operations(
        &mut self,
        actions: Vec<OperationAction>,
    ) -> Result<(), OperationJournalRuntimeError> {
        let mut updated_at_unix_ms = self.manifest.updated_at_unix_ms();
        for action in actions {
            let attempt_at_unix_ms = action.attempt.occurred_at_unix_ms();
            self.journal
                .queue_operation_with_id(
                    action.operation_id.clone(),
                    action.kind,
                    action.parent_operation_id.as_ref(),
                    action.redacted_payload,
                    action.queued_at_unix_ms,
                )
                .map_err(|error| {
                    OperationJournalRuntimeError::Unavailable(format!(
                        "invalid queued operation {}: {error}",
                        action.operation_id
                    ))
                })?;
            self.journal
                .transition_operation(
                    &action.operation_id,
                    action.attempt.status(),
                    attempt_at_unix_ms,
                )
                .map_err(|error| {
                    OperationJournalRuntimeError::Unavailable(format!(
                        "invalid operation attempt {}: {error}",
                        action.operation_id
                    ))
                })?;
            updated_at_unix_ms = updated_at_unix_ms
                .max(action.queued_at_unix_ms)
                .max(attempt_at_unix_ms);
        }
        self.persist(updated_at_unix_ms)
    }

    fn begin_generation(
        &mut self,
        generation_id: OperationGenerationId,
        started_at_unix_ms: u64,
    ) -> Result<(), OperationJournalRuntimeError> {
        self.journal
            .begin_generation(generation_id, started_at_unix_ms)
            .map_err(|error| {
                OperationJournalRuntimeError::Unavailable(format!(
                    "invalid reconnect generation: {error}"
                ))
            })?;
        self.persist(started_at_unix_ms)
    }

    fn persist(&mut self, updated_at_unix_ms: u64) -> Result<(), OperationJournalRuntimeError> {
        self.file_store
            .persist(&self.journal)
            .map_err(persistence_error)?;
        self.manifest
            .touch(updated_at_unix_ms)
            .map_err(persistence_history_error)?;
        self.history_store
            .write_manifest(&self.manifest)
            .map_err(persistence_history_error)
    }
}

fn operation_journal_worker(
    queue: Arc<RuntimeQueue>,
    shared: Arc<RuntimeShared>,
    start: WorkerStart,
    test_gate: TestGateOption,
) {
    let mut state = match WorkerState::open(start) {
        Ok(state) => state,
        Err(error) => {
            shared.mark_failure(health_for_runtime_error(&error), &error);
            queue.abort(error);
            return;
        }
    };
    shared.mark_healthy();
    wait_on_test_gate_before_work(&test_gate);

    while let Some(entry) = queue.pop() {
        let result = match entry {
            QueueEntry::Operation { action, cost_bytes } => {
                let mut actions = vec![action];
                let cost_bytes =
                    cost_bytes.saturating_add(queue.drain_consecutive_operations(&mut actions));
                let operation_count = actions.len();
                wait_on_test_gate_before_persist(&test_gate);
                let result = state.apply_operations(actions);
                if result.is_ok() {
                    queue.finish_operations(operation_count, cost_bytes);
                }
                result
            }
            QueueEntry::Control(ControlAction::BeginGeneration {
                generation_id,
                started_at_unix_ms,
            }) => state.begin_generation(generation_id, started_at_unix_ms),
            QueueEntry::Control(ControlAction::Flush(sender)) => {
                let _ = sender.send(Ok(()));
                continue;
            }
            QueueEntry::Control(ControlAction::Shutdown(sender)) => {
                queue.finish();
                shared.mark_closed();
                let _ = sender.send(Ok(()));
                return;
            }
        };

        if let Err(error) = result {
            shared.mark_failure(health_for_runtime_error(&error), &error);
            queue.abort(error);
            return;
        }
    }
}

fn health_for_runtime_error(error: &OperationJournalRuntimeError) -> OperationJournalRuntimeHealth {
    match error {
        OperationJournalRuntimeError::PersistenceFailed(_) => {
            OperationJournalRuntimeHealth::PersistenceFailed
        }
        _ => OperationJournalRuntimeHealth::Unavailable,
    }
}

fn unavailable_persistence_error(
    error: OperationJournalPersistenceError,
) -> OperationJournalRuntimeError {
    OperationJournalRuntimeError::Unavailable(error.to_string())
}

fn persistence_error(error: OperationJournalPersistenceError) -> OperationJournalRuntimeError {
    OperationJournalRuntimeError::PersistenceFailed(error.to_string())
}

fn unavailable_history_error(error: OperationJournalHistoryError) -> OperationJournalRuntimeError {
    OperationJournalRuntimeError::Unavailable(error.to_string())
}

fn persistence_history_error(error: OperationJournalHistoryError) -> OperationJournalRuntimeError {
    OperationJournalRuntimeError::PersistenceFailed(error.to_string())
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

fn wait_unpoisoned<'a, T>(condvar: &Condvar, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
    condvar
        .wait(guard)
        .unwrap_or_else(|error| error.into_inner())
}

#[cfg(test)]
type TestGateOption = Option<OperationJournalWorkerTestGate>;
#[cfg(not(test))]
type TestGateOption = ();

#[cfg(test)]
fn test_gate_none() -> TestGateOption {
    None
}

#[cfg(not(test))]
fn test_gate_none() -> TestGateOption {}

#[cfg(test)]
fn wait_on_test_gate_before_work(gate: &TestGateOption) {
    if let Some(gate) = gate {
        gate.wait_at(OperationJournalWorkerTestGatePoint::BeforeWork);
    }
}

#[cfg(not(test))]
fn wait_on_test_gate_before_work(_gate: &TestGateOption) {}

#[cfg(test)]
fn wait_on_test_gate_before_persist(gate: &TestGateOption) {
    if let Some(gate) = gate {
        gate.wait_at(OperationJournalWorkerTestGatePoint::BeforePersist);
    }
}

#[cfg(not(test))]
fn wait_on_test_gate_before_persist(_gate: &TestGateOption) {}

struct WorkerExitGuard {
    test_gate: TestGateOption,
}

impl WorkerExitGuard {
    fn new(test_gate: TestGateOption) -> Self {
        Self { test_gate }
    }
}

impl Drop for WorkerExitGuard {
    fn drop(&mut self) {
        mark_test_worker_exited(&self.test_gate);
    }
}

#[cfg(test)]
fn mark_test_worker_exited(gate: &TestGateOption) {
    if let Some(gate) = gate {
        gate.mark_worker_exited();
    }
}

#[cfg(not(test))]
fn mark_test_worker_exited(_gate: &TestGateOption) {}

#[cfg(test)]
#[derive(Clone)]
pub(super) struct OperationJournalWorkerTestGate {
    inner: Arc<(Mutex<OperationJournalWorkerTestGateState>, Condvar)>,
}

#[cfg(test)]
struct OperationJournalWorkerTestGateState {
    point: OperationJournalWorkerTestGatePoint,
    blocked: bool,
    worker_waiting: bool,
    worker_exited: bool,
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum OperationJournalWorkerTestGatePoint {
    BeforeWork,
    BeforePersist,
}

#[cfg(test)]
impl OperationJournalWorkerTestGate {
    pub(super) fn blocked_before_work() -> Self {
        Self::blocked_at(OperationJournalWorkerTestGatePoint::BeforeWork)
    }

    pub(super) fn blocked_before_persist() -> Self {
        Self::blocked_at(OperationJournalWorkerTestGatePoint::BeforePersist)
    }

    fn blocked_at(point: OperationJournalWorkerTestGatePoint) -> Self {
        Self {
            inner: Arc::new((
                Mutex::new(OperationJournalWorkerTestGateState {
                    point,
                    blocked: true,
                    worker_waiting: false,
                    worker_exited: false,
                }),
                Condvar::new(),
            )),
        }
    }

    fn wait_at(&self, point: OperationJournalWorkerTestGatePoint) {
        let (state, changed) = &*self.inner;
        let mut state = lock_unpoisoned(state);
        if state.point != point {
            return;
        }
        state.worker_waiting = true;
        changed.notify_all();
        while state.blocked {
            state = wait_unpoisoned(changed, state);
        }
    }

    pub(super) fn wait_until_worker_is_blocked(&self) {
        let (state, changed) = &*self.inner;
        let mut state = lock_unpoisoned(state);
        while !state.worker_waiting {
            state = wait_unpoisoned(changed, state);
        }
    }

    pub(super) fn release(&self) {
        let (state, changed) = &*self.inner;
        lock_unpoisoned(state).blocked = false;
        changed.notify_all();
    }

    pub(super) fn wait_until_worker_exited(&self, timeout: Duration) -> bool {
        let (state, changed) = &*self.inner;
        let state = lock_unpoisoned(state);
        let (state, _) = changed
            .wait_timeout_while(state, timeout, |state| !state.worker_exited)
            .unwrap_or_else(|error| error.into_inner());
        state.worker_exited
    }

    fn mark_worker_exited(&self) {
        let (state, changed) = &*self.inner;
        lock_unpoisoned(state).worker_exited = true;
        changed.notify_all();
    }
}
