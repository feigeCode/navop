use super::{
    RecordingConfig, RecordingController, RecordingEvent, RecordingEventKind, RecordingFailure,
    RecordingFileConfig, RecordingFileWriter, RecordingLimit, RecordingMetadata, RecordingState,
    RecordingTransition, partial_recording_path,
};
use crate::TerminalSize;
use std::collections::VecDeque;
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const DEFAULT_MAX_PENDING_RECORDING_EVENTS: usize = 256;
const DEFAULT_MAX_PENDING_RECORDING_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_MAX_PENDING_RECORDING_CONTROLS: usize = 16;

#[derive(Clone, Debug)]
pub struct RecordingQueueLimits {
    pub max_pending_events: usize,
    pub max_pending_bytes: usize,
    pub max_pending_controls: usize,
}

impl Default for RecordingQueueLimits {
    fn default() -> Self {
        Self {
            max_pending_events: DEFAULT_MAX_PENDING_RECORDING_EVENTS,
            max_pending_bytes: DEFAULT_MAX_PENDING_RECORDING_BYTES,
            max_pending_controls: DEFAULT_MAX_PENDING_RECORDING_CONTROLS,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct RecordingRuntimeConfig {
    pub queue: RecordingQueueLimits,
    pub file: RecordingFileConfig,
}

#[derive(Clone, Debug)]
pub struct RecordingStartRequest {
    pub final_path: PathBuf,
    pub metadata: RecordingMetadata,
    pub initial_size: TerminalSize,
    pub recording: RecordingConfig,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordingSnapshot {
    pub state: RecordingState,
    pub elapsed: Duration,
    pub event_count: u64,
    pub payload_bytes: u64,
    pub capture_input: bool,
    pub final_path: Option<PathBuf>,
    pub partial_path: Option<PathBuf>,
    pub failure: Option<RecordingFailure>,
}

impl Default for RecordingSnapshot {
    fn default() -> Self {
        Self {
            state: RecordingState::Idle,
            elapsed: Duration::ZERO,
            event_count: 0,
            payload_bytes: 0,
            capture_input: false,
            final_path: None,
            partial_path: None,
            failure: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecordingQueueSnapshot {
    pub pending_events: usize,
    pub pending_bytes: usize,
    pub peak_pending_events: usize,
    pub peak_pending_bytes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordingRuntimeError {
    InvalidConfig(String),
    ReadOnlyPlayback,
    Closed,
    ControlQueueFull,
    WorkerStopped,
    Recording(RecordingFailure),
}

impl fmt::Display for RecordingRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(reason) => {
                write!(formatter, "invalid recording runtime config: {reason}")
            }
            Self::ReadOnlyPlayback => {
                formatter.write_str("recording playback sessions are read-only")
            }
            Self::Closed => formatter.write_str("recording runtime is closed"),
            Self::ControlQueueFull => formatter.write_str("recording control queue is full"),
            Self::WorkerStopped => formatter.write_str("recording worker stopped"),
            Self::Recording(failure) => failure.fmt(formatter),
        }
    }
}

impl std::error::Error for RecordingRuntimeError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordingTapOutcome {
    Accepted,
    Inactive,
    InputDisabled,
    QueueFull(RecordingLimit),
    Closed,
}

#[derive(Clone)]
pub struct RecordingTap {
    queue: Arc<RuntimeQueue>,
    shared: Arc<RuntimeShared>,
}

impl RecordingTap {
    /// Captures the exact bytes accepted at the terminal parser boundary.
    ///
    /// The payload is copied only while recording is active. This method never
    /// waits for the recording worker or filesystem.
    pub fn record_output(&self, data: &[u8]) -> RecordingTapOutcome {
        self.enqueue(data.len(), false, || {
            RecordingEventKind::Output(data.to_vec())
        })
    }

    /// Returns a lock-free snapshot used to avoid cloning input solely for
    /// post-send recording while recording is inactive or input capture is
    /// disabled. The subsequent enqueue still rechecks all gates.
    pub(crate) fn is_input_capture_active(&self) -> bool {
        !self.queue.closed.load(Ordering::Acquire)
            && self.queue.accepting.load(Ordering::Acquire)
            && self.queue.capture_input.load(Ordering::Acquire)
    }

    /// Captures disclosed user input. Input is not copied or queued unless the
    /// active recording explicitly enabled input capture.
    pub fn record_input(&self, data: &[u8]) -> RecordingTapOutcome {
        self.enqueue(data.len(), true, || {
            RecordingEventKind::Input(data.to_vec())
        })
    }

    pub fn record_resize(&self, size: TerminalSize) -> RecordingTapOutcome {
        self.enqueue(std::mem::size_of::<TerminalSize>(), false, || {
            RecordingEventKind::Resize(size)
        })
    }

    pub fn record_marker(&self, marker: &str) -> RecordingTapOutcome {
        self.enqueue(marker.len(), false, || {
            RecordingEventKind::Marker(marker.to_string())
        })
    }

    fn enqueue(
        &self,
        payload_bytes: usize,
        requires_input: bool,
        build: impl FnOnce() -> RecordingEventKind,
    ) -> RecordingTapOutcome {
        if self.queue.closed.load(Ordering::Acquire) {
            return RecordingTapOutcome::Closed;
        }
        if !self.queue.accepting.load(Ordering::Acquire) {
            return RecordingTapOutcome::Inactive;
        }
        if requires_input && !self.queue.capture_input.load(Ordering::Acquire) {
            return RecordingTapOutcome::InputDisabled;
        }

        match self
            .queue
            .enqueue_data(payload_bytes, requires_input, Instant::now(), build)
        {
            Ok(()) => RecordingTapOutcome::Accepted,
            Err(DataEnqueueError::Inactive) => RecordingTapOutcome::Inactive,
            Err(DataEnqueueError::InputDisabled) => RecordingTapOutcome::InputDisabled,
            Err(DataEnqueueError::Closed) => RecordingTapOutcome::Closed,
            Err(DataEnqueueError::Limit(limit)) => {
                let failure = RecordingFailure::LimitReached(limit);
                self.shared
                    .mark_failed_without_notification(failure.clone(), Instant::now());
                RecordingTapOutcome::QueueFull(limit)
            }
        }
    }
}

#[derive(Clone)]
pub struct RecordingRuntime {
    core: Arc<RuntimeCore>,
}

impl RecordingRuntime {
    pub fn new(config: RecordingRuntimeConfig) -> Result<Self, RecordingRuntimeError> {
        Self::with_observer(config, |_| {})
    }

    pub fn with_observer(
        config: RecordingRuntimeConfig,
        observer: impl Fn(RecordingSnapshot) + Send + Sync + 'static,
    ) -> Result<Self, RecordingRuntimeError> {
        Self::spawn(config, Arc::new(observer), test_gate_none())
    }

    #[cfg(test)]
    pub(super) fn new_with_test_gate(
        config: RecordingRuntimeConfig,
        gate: RecordingWorkerTestGate,
    ) -> Result<Self, RecordingRuntimeError> {
        Self::spawn(config, Arc::new(|_| {}), Some(gate))
    }

    fn spawn(
        config: RecordingRuntimeConfig,
        observer: Arc<dyn Fn(RecordingSnapshot) + Send + Sync>,
        test_gate: TestGateOption,
    ) -> Result<Self, RecordingRuntimeError> {
        validate_runtime_config(&config)?;
        let queue = Arc::new(RuntimeQueue::new(config.queue.clone()));
        let shared = Arc::new(RuntimeShared::new(observer));
        let worker_queue = queue.clone();
        let worker_shared = shared.clone();
        let worker = thread::Builder::new()
            .name("terminal-recording".to_string())
            .spawn(move || recording_worker(worker_queue, worker_shared, config.file, test_gate))
            .map_err(|error| {
                RecordingRuntimeError::InvalidConfig(format!(
                    "failed to spawn recording worker: {error}"
                ))
            })?;
        Ok(Self {
            core: Arc::new(RuntimeCore {
                queue,
                shared,
                worker: Mutex::new(Some(worker)),
            }),
        })
    }

    pub fn tap(&self) -> RecordingTap {
        RecordingTap {
            queue: self.core.queue.clone(),
            shared: self.core.shared.clone(),
        }
    }

    pub fn snapshot(&self) -> RecordingSnapshot {
        self.core.shared.snapshot()
    }

    pub fn queue_snapshot(&self) -> RecordingQueueSnapshot {
        self.core.queue.snapshot()
    }

    pub fn start(
        &self,
        request: RecordingStartRequest,
    ) -> Result<RecordingTransition, RecordingRuntimeError> {
        if request.metadata.capture_input != request.recording.capture_input {
            return Err(RecordingRuntimeError::InvalidConfig(
                "recording metadata and controller disagree about input capture".to_string(),
            ));
        }
        self.submit(ControlAction::Start(request))
    }

    pub fn pause(&self) -> Result<RecordingTransition, RecordingRuntimeError> {
        self.submit(ControlAction::Pause)
    }

    pub fn resume(&self) -> Result<RecordingTransition, RecordingRuntimeError> {
        self.submit(ControlAction::Resume)
    }

    pub fn stop(&self) -> Result<RecordingTransition, RecordingRuntimeError> {
        match self
            .core
            .queue
            .enqueue_control(ControlAction::Stop, Instant::now())
        {
            Ok(receiver) => receive_control_result(receiver),
            Err(ControlEnqueueError::Closed) => self.stop_result_from_snapshot(),
            Err(ControlEnqueueError::ShuttingDown) => self.wait_for_shutdown_then_stop_result(),
            Err(error) => Err(map_control_enqueue_error(error)),
        }
    }

    /// Gracefully finishes an active recording and terminates its worker.
    ///
    /// A recording failure never prevents runtime shutdown. The failure stays
    /// visible in [`RecordingSnapshot`] and the `.partial` file is not
    /// published as a complete recording.
    pub fn shutdown(&self) -> Result<RecordingTransition, RecordingRuntimeError> {
        if let Some(result) = self.core.shared.shutdown_result() {
            self.core.join_worker();
            return result;
        }

        match self
            .core
            .queue
            .enqueue_control(ControlAction::Shutdown, Instant::now())
        {
            Ok(receiver) => {
                let result = receive_control_result(receiver);
                self.core.join_worker();
                result
            }
            Err(ControlEnqueueError::ShuttingDown) => {
                let result = self.core.shared.wait_for_shutdown();
                self.core.join_worker();
                result
            }
            Err(error) => Err(map_control_enqueue_error(error)),
        }
    }

    fn submit(&self, action: ControlAction) -> Result<RecordingTransition, RecordingRuntimeError> {
        match self.core.queue.enqueue_control(action, Instant::now()) {
            Ok(receiver) => receive_control_result(receiver),
            Err(ControlEnqueueError::ShuttingDown) => {
                if matches!(self.snapshot().state, RecordingState::Stopped) {
                    Ok(RecordingTransition::Unchanged)
                } else {
                    Err(RecordingRuntimeError::WorkerStopped)
                }
            }
            Err(error) => Err(map_control_enqueue_error(error)),
        }
    }

    fn wait_for_shutdown_then_stop_result(
        &self,
    ) -> Result<RecordingTransition, RecordingRuntimeError> {
        let _ = self.core.shared.wait_for_shutdown();
        self.stop_result_from_snapshot()
    }

    fn stop_result_from_snapshot(&self) -> Result<RecordingTransition, RecordingRuntimeError> {
        match self.snapshot().state {
            RecordingState::Stopped | RecordingState::Idle => Ok(RecordingTransition::Unchanged),
            RecordingState::Failed(failure) => Err(RecordingRuntimeError::Recording(failure)),
            _ => Err(RecordingRuntimeError::Closed),
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
        if worker.thread().id() == thread::current().id() {
            return;
        }
        let _ = worker.join();
    }
}

impl Drop for RuntimeCore {
    fn drop(&mut self) {
        self.queue.abort();
        self.shared
            .complete_shutdown(Err(RecordingRuntimeError::WorkerStopped));
        self.join_worker();
    }
}

fn receive_control_result(
    receiver: Receiver<Result<RecordingTransition, RecordingRuntimeError>>,
) -> Result<RecordingTransition, RecordingRuntimeError> {
    receiver
        .recv()
        .unwrap_or(Err(RecordingRuntimeError::WorkerStopped))
}

fn map_control_enqueue_error(error: ControlEnqueueError) -> RecordingRuntimeError {
    match error {
        ControlEnqueueError::Closed => RecordingRuntimeError::Closed,
        ControlEnqueueError::Full => RecordingRuntimeError::ControlQueueFull,
        ControlEnqueueError::ShuttingDown => RecordingRuntimeError::WorkerStopped,
    }
}

fn validate_runtime_config(config: &RecordingRuntimeConfig) -> Result<(), RecordingRuntimeError> {
    if config.queue.max_pending_events == 0 {
        return Err(RecordingRuntimeError::InvalidConfig(
            "max_pending_events must be greater than zero".to_string(),
        ));
    }
    if config.queue.max_pending_bytes == 0 {
        return Err(RecordingRuntimeError::InvalidConfig(
            "max_pending_bytes must be greater than zero".to_string(),
        ));
    }
    if config.queue.max_pending_controls == 0 {
        return Err(RecordingRuntimeError::InvalidConfig(
            "max_pending_controls must be greater than zero".to_string(),
        ));
    }
    Ok(())
}

type ControlResult = Result<RecordingTransition, RecordingRuntimeError>;

enum ControlAction {
    Start(RecordingStartRequest),
    Pause,
    Resume,
    Stop,
    Shutdown,
}

struct ControlEnvelope {
    sequence: u64,
    observed_at: Instant,
    action: ControlAction,
    response: SyncSender<ControlResult>,
}

struct DataEnvelope {
    sequence: u64,
    observed_at: Instant,
    payload_bytes: usize,
    kind: RecordingEventKind,
}

enum WorkItem {
    Control(ControlEnvelope),
    Data(DataEnvelope),
    Failure(RecordingFailure),
    Abort,
}

enum DataEnqueueError {
    Inactive,
    InputDisabled,
    Closed,
    Limit(RecordingLimit),
}

enum ControlEnqueueError {
    Closed,
    Full,
    ShuttingDown,
}

struct RuntimeQueue {
    limits: RecordingQueueLimits,
    state: Mutex<QueueState>,
    wake: Condvar,
    accepting: AtomicBool,
    capture_input: AtomicBool,
    closed: AtomicBool,
}

struct QueueState {
    next_sequence: u64,
    data: VecDeque<DataEnvelope>,
    controls: VecDeque<ControlEnvelope>,
    pending_events: usize,
    pending_bytes: usize,
    peak_pending_events: usize,
    peak_pending_bytes: usize,
    pending_failure: Option<RecordingFailure>,
    accepting: bool,
    capture_input: bool,
    shutting_down: bool,
    closed: bool,
    aborted: bool,
}

impl RuntimeQueue {
    fn new(limits: RecordingQueueLimits) -> Self {
        Self {
            limits,
            state: Mutex::new(QueueState {
                next_sequence: 0,
                data: VecDeque::new(),
                controls: VecDeque::new(),
                pending_events: 0,
                pending_bytes: 0,
                peak_pending_events: 0,
                peak_pending_bytes: 0,
                pending_failure: None,
                accepting: false,
                capture_input: false,
                shutting_down: false,
                closed: false,
                aborted: false,
            }),
            wake: Condvar::new(),
            accepting: AtomicBool::new(false),
            capture_input: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        }
    }

    fn enqueue_data(
        &self,
        payload_bytes: usize,
        requires_input: bool,
        observed_at: Instant,
        build: impl FnOnce() -> RecordingEventKind,
    ) -> Result<(), DataEnqueueError> {
        let mut state = lock_unpoisoned(&self.state);
        if state.closed || state.aborted || state.shutting_down {
            return Err(DataEnqueueError::Closed);
        }
        if !state.accepting {
            return Err(DataEnqueueError::Inactive);
        }
        if requires_input && !state.capture_input {
            return Err(DataEnqueueError::InputDisabled);
        }
        if state.pending_events >= self.limits.max_pending_events {
            let limit = RecordingLimit::PendingEvents;
            self.fail_locked(&mut state, RecordingFailure::LimitReached(limit));
            return Err(DataEnqueueError::Limit(limit));
        }
        let Some(next_pending_bytes) = state.pending_bytes.checked_add(payload_bytes) else {
            let limit = RecordingLimit::PendingBytes;
            self.fail_locked(&mut state, RecordingFailure::LimitReached(limit));
            return Err(DataEnqueueError::Limit(limit));
        };
        if next_pending_bytes > self.limits.max_pending_bytes {
            let limit = RecordingLimit::PendingBytes;
            self.fail_locked(&mut state, RecordingFailure::LimitReached(limit));
            return Err(DataEnqueueError::Limit(limit));
        }

        let sequence = take_sequence(&mut state);
        let kind = build();
        state.pending_events += 1;
        state.pending_bytes = next_pending_bytes;
        state.peak_pending_events = state.peak_pending_events.max(state.pending_events);
        state.peak_pending_bytes = state.peak_pending_bytes.max(state.pending_bytes);
        state.data.push_back(DataEnvelope {
            sequence,
            observed_at,
            payload_bytes,
            kind,
        });
        drop(state);
        self.wake.notify_one();
        Ok(())
    }

    fn enqueue_control(
        &self,
        action: ControlAction,
        observed_at: Instant,
    ) -> Result<Receiver<ControlResult>, ControlEnqueueError> {
        let mut state = lock_unpoisoned(&self.state);
        if state.closed || state.aborted {
            return Err(ControlEnqueueError::Closed);
        }
        if state.shutting_down {
            return Err(ControlEnqueueError::ShuttingDown);
        }
        if state.controls.len() >= self.limits.max_pending_controls {
            return Err(ControlEnqueueError::Full);
        }

        let is_shutdown = matches!(&action, ControlAction::Shutdown);
        if matches!(
            &action,
            ControlAction::Pause | ControlAction::Stop | ControlAction::Shutdown
        ) {
            self.set_accepting_locked(&mut state, false, false);
        }
        if is_shutdown {
            state.shutting_down = true;
        }

        let sequence = take_sequence(&mut state);
        let (response, receiver) = mpsc::sync_channel(1);
        state.controls.push_back(ControlEnvelope {
            sequence,
            observed_at,
            action,
            response,
        });
        drop(state);
        self.wake.notify_one();
        Ok(receiver)
    }

    fn next_work(&self) -> WorkItem {
        let mut state = lock_unpoisoned(&self.state);
        loop {
            if state.aborted {
                return WorkItem::Abort;
            }
            if let Some(failure) = state.pending_failure.take() {
                return WorkItem::Failure(failure);
            }
            let control_sequence = state.controls.front().map(|item| item.sequence);
            let data_sequence = state.data.front().map(|item| item.sequence);
            match (control_sequence, data_sequence) {
                (Some(control), Some(data)) if control <= data => {
                    return WorkItem::Control(
                        state.controls.pop_front().expect("control front exists"),
                    );
                }
                (Some(_), Some(_)) => {
                    return WorkItem::Data(state.data.pop_front().expect("data front exists"));
                }
                (Some(_), None) => {
                    return WorkItem::Control(
                        state.controls.pop_front().expect("control front exists"),
                    );
                }
                (None, Some(_)) => {
                    return WorkItem::Data(state.data.pop_front().expect("data front exists"));
                }
                (None, None) if state.closed => return WorkItem::Abort,
                (None, None) => {
                    state = self
                        .wake
                        .wait(state)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
            }
        }
    }

    fn finish_data(&self, payload_bytes: usize) {
        let mut state = lock_unpoisoned(&self.state);
        state.pending_events = state.pending_events.saturating_sub(1);
        state.pending_bytes = state.pending_bytes.saturating_sub(payload_bytes);
    }

    fn has_pending_failure(&self) -> bool {
        lock_unpoisoned(&self.state).pending_failure.is_some()
    }

    fn enable_recording(&self, capture_input: bool) {
        let mut state = lock_unpoisoned(&self.state);
        if !state.closed && !state.aborted && !state.shutting_down {
            self.set_accepting_locked(&mut state, true, capture_input);
        }
    }

    fn disable_recording(&self) {
        let mut state = lock_unpoisoned(&self.state);
        self.set_accepting_locked(&mut state, false, false);
        discard_queued_data(&mut state);
    }

    fn fail_locked(&self, state: &mut QueueState, failure: RecordingFailure) {
        if state.pending_failure.is_none() {
            state.pending_failure = Some(failure);
        }
        self.set_accepting_locked(state, false, false);
        discard_queued_data(state);
        self.wake.notify_one();
    }

    fn set_accepting_locked(&self, state: &mut QueueState, accepting: bool, capture_input: bool) {
        state.accepting = accepting;
        state.capture_input = accepting && capture_input;
        self.accepting.store(state.accepting, Ordering::Release);
        self.capture_input
            .store(state.capture_input, Ordering::Release);
    }

    fn snapshot(&self) -> RecordingQueueSnapshot {
        let state = lock_unpoisoned(&self.state);
        RecordingQueueSnapshot {
            pending_events: state.pending_events,
            pending_bytes: state.pending_bytes,
            peak_pending_events: state.peak_pending_events,
            peak_pending_bytes: state.peak_pending_bytes,
        }
    }

    fn finish_shutdown(&self) {
        let controls = {
            let mut state = lock_unpoisoned(&self.state);
            state.closed = true;
            self.closed.store(true, Ordering::Release);
            self.set_accepting_locked(&mut state, false, false);
            discard_queued_data(&mut state);
            state.controls.drain(..).collect::<Vec<_>>()
        };
        for control in controls {
            let _ = control
                .response
                .send(Err(RecordingRuntimeError::WorkerStopped));
        }
        self.wake.notify_all();
    }

    fn abort(&self) {
        let controls = {
            let mut state = lock_unpoisoned(&self.state);
            if state.aborted {
                return;
            }
            state.aborted = true;
            state.closed = true;
            self.closed.store(true, Ordering::Release);
            self.set_accepting_locked(&mut state, false, false);
            discard_queued_data(&mut state);
            state.controls.drain(..).collect::<Vec<_>>()
        };
        for control in controls {
            let _ = control
                .response
                .send(Err(RecordingRuntimeError::WorkerStopped));
        }
        self.wake.notify_all();
    }
}

fn take_sequence(state: &mut QueueState) -> u64 {
    let sequence = state.next_sequence;
    state.next_sequence = state.next_sequence.saturating_add(1);
    sequence
}

fn discard_queued_data(state: &mut QueueState) {
    let discarded_events = state.data.len();
    let discarded_bytes = state.data.iter().fold(0_usize, |total, item| {
        total.saturating_add(item.payload_bytes)
    });
    state.data.clear();
    state.pending_events = state.pending_events.saturating_sub(discarded_events);
    state.pending_bytes = state.pending_bytes.saturating_sub(discarded_bytes);
}

struct RuntimeShared {
    snapshot: Mutex<SnapshotState>,
    observer: Arc<dyn Fn(RecordingSnapshot) + Send + Sync>,
    shutdown: Mutex<ShutdownState>,
    shutdown_changed: Condvar,
}

struct SnapshotState {
    value: RecordingSnapshot,
    clock: SnapshotClock,
}

#[derive(Default)]
struct SnapshotClock {
    started_at: Option<Instant>,
    paused_at: Option<Instant>,
    paused_total: Duration,
}

#[derive(Default)]
struct ShutdownState {
    result: Option<ControlResult>,
}

enum ClockUpdate {
    Preserve,
    Started(Instant),
    Paused(Instant),
    Resumed(Instant),
    Frozen,
}

impl RuntimeShared {
    fn new(observer: Arc<dyn Fn(RecordingSnapshot) + Send + Sync>) -> Self {
        Self {
            snapshot: Mutex::new(SnapshotState {
                value: RecordingSnapshot::default(),
                clock: SnapshotClock::default(),
            }),
            observer,
            shutdown: Mutex::new(ShutdownState::default()),
            shutdown_changed: Condvar::new(),
        }
    }

    fn snapshot(&self) -> RecordingSnapshot {
        let mut state = lock_unpoisoned(&self.snapshot);
        snapshot_at(&mut state, Instant::now())
    }

    fn publish(
        &self,
        mut value: RecordingSnapshot,
        clock_update: ClockUpdate,
        force_notification: bool,
    ) {
        let should_notify = {
            let mut state = lock_unpoisoned(&self.snapshot);
            let previous_state = state.value.state.clone();
            match clock_update {
                ClockUpdate::Preserve => {}
                ClockUpdate::Started(at) => {
                    state.clock.started_at = Some(at);
                    state.clock.paused_at = None;
                    state.clock.paused_total = Duration::ZERO;
                }
                ClockUpdate::Paused(at) => {
                    state.clock.paused_at = Some(at);
                }
                ClockUpdate::Resumed(at) => {
                    if let Some(paused_at) = state.clock.paused_at.take() {
                        state.clock.paused_total = state
                            .clock
                            .paused_total
                            .saturating_add(at.saturating_duration_since(paused_at));
                    }
                }
                ClockUpdate::Frozen => {
                    if matches!(previous_state, RecordingState::Recording) {
                        let current = snapshot_at(&mut state, Instant::now());
                        value.elapsed = value.elapsed.max(current.elapsed);
                    }
                    state.clock.paused_at = None;
                }
            }
            let state_changed = previous_state != value.state;
            state.value = value.clone();
            state_changed || force_notification
        };
        if should_notify {
            self.notify(value);
        }
    }

    fn mark_failed_without_notification(&self, failure: RecordingFailure, observed_at: Instant) {
        let mut state = lock_unpoisoned(&self.snapshot);
        let mut value = snapshot_at(&mut state, observed_at);
        value.state = RecordingState::Failed(failure.clone());
        value.failure = Some(failure);
        state.value = value;
        state.clock.paused_at = None;
    }

    fn notify(&self, snapshot: RecordingSnapshot) {
        let observer = self.observer.clone();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            observer(snapshot);
        }));
    }

    fn complete_shutdown(&self, result: ControlResult) {
        let mut shutdown = lock_unpoisoned(&self.shutdown);
        if shutdown.result.is_none() {
            shutdown.result = Some(result);
        }
        drop(shutdown);
        self.shutdown_changed.notify_all();
    }

    fn shutdown_result(&self) -> Option<ControlResult> {
        lock_unpoisoned(&self.shutdown).result.clone()
    }

    fn wait_for_shutdown(&self) -> ControlResult {
        let mut shutdown = lock_unpoisoned(&self.shutdown);
        loop {
            if let Some(result) = shutdown.result.clone() {
                return result;
            }
            shutdown = self
                .shutdown_changed
                .wait(shutdown)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }
}

fn snapshot_at(state: &mut SnapshotState, now: Instant) -> RecordingSnapshot {
    let mut value = state.value.clone();
    if matches!(value.state, RecordingState::Recording) {
        if let Some(started_at) = state.clock.started_at {
            let wall_elapsed = now
                .saturating_duration_since(started_at)
                .checked_sub(state.clock.paused_total)
                .unwrap_or(Duration::ZERO);
            value.elapsed = value.elapsed.max(wall_elapsed);
        }
    }
    value
}

struct WorkerSession {
    controller: RecordingController,
    writer: RecordingFileWriter,
    epoch: Instant,
    last_logical_time: Duration,
    capture_input: bool,
    final_path: PathBuf,
    partial_path: PathBuf,
}

impl WorkerSession {
    fn snapshot(&self) -> RecordingSnapshot {
        let state = self.controller.state().clone();
        RecordingSnapshot {
            failure: match &state {
                RecordingState::Failed(failure) => Some(failure.clone()),
                _ => None,
            },
            state,
            elapsed: self.controller.elapsed(),
            event_count: self.controller.event_count(),
            payload_bytes: self.controller.payload_bytes(),
            capture_input: self.capture_input,
            final_path: Some(self.final_path.clone()),
            partial_path: Some(self.partial_path.clone()),
        }
    }

    fn logical_time(&mut self, observed_at: Instant) -> Duration {
        let observed = observed_at.saturating_duration_since(self.epoch);
        self.last_logical_time = self.last_logical_time.max(observed);
        self.last_logical_time
    }

    fn record(
        &mut self,
        observed_at: Instant,
        kind: RecordingEventKind,
    ) -> Result<Option<RecordingEvent>, RecordingFailure> {
        let now = self.logical_time(observed_at);
        match kind {
            RecordingEventKind::Output(data) => self.controller.record_output(now, data),
            RecordingEventKind::Input(data) => self.controller.record_input(now, data),
            RecordingEventKind::Resize(size) => self.controller.record_resize(now, size),
            RecordingEventKind::Marker(marker) => self.controller.record_marker(now, marker),
        }
    }

    fn fail_storage(&mut self, error: impl fmt::Display) -> RecordingFailure {
        let failure = RecordingFailure::Storage(error.to_string());
        self.controller.fail(failure.clone());
        failure
    }
}

fn recording_worker(
    queue: Arc<RuntimeQueue>,
    shared: Arc<RuntimeShared>,
    file_config: RecordingFileConfig,
    test_gate: TestGateOption,
) {
    let mut session: Option<WorkerSession> = None;

    loop {
        match queue.next_work() {
            WorkItem::Abort => break,
            WorkItem::Failure(failure) => {
                queue.disable_recording();
                if let Some(session) = session.as_mut() {
                    session.controller.fail(failure.clone());
                    shared.publish(session.snapshot(), ClockUpdate::Frozen, true);
                } else {
                    shared.publish(failed_snapshot(failure), ClockUpdate::Frozen, true);
                }
            }
            WorkItem::Data(data) => {
                wait_on_test_gate(&test_gate);
                if queue.has_pending_failure() {
                    queue.finish_data(data.payload_bytes);
                    continue;
                }
                process_data(&queue, &shared, session.as_mut(), &data);
                queue.finish_data(data.payload_bytes);
            }
            WorkItem::Control(control) => {
                let is_shutdown = matches!(&control.action, ControlAction::Shutdown);
                let result = match control.action {
                    ControlAction::Start(request) => process_start(
                        &queue,
                        &shared,
                        &mut session,
                        request,
                        control.observed_at,
                        file_config.clone(),
                    ),
                    ControlAction::Pause => {
                        process_pause(&queue, &shared, session.as_mut(), control.observed_at)
                    }
                    ControlAction::Resume => {
                        process_resume(&queue, &shared, session.as_mut(), control.observed_at)
                    }
                    ControlAction::Stop => {
                        process_stop(&queue, &shared, session.as_mut(), control.observed_at)
                    }
                    ControlAction::Shutdown => {
                        process_shutdown(&queue, &shared, session.as_mut(), control.observed_at)
                    }
                };

                if is_shutdown {
                    queue.finish_shutdown();
                    shared.complete_shutdown(result.clone());
                }
                let _ = control.response.send(result);
                if is_shutdown {
                    break;
                }
            }
        }
    }
}

fn process_start(
    queue: &RuntimeQueue,
    shared: &RuntimeShared,
    session: &mut Option<WorkerSession>,
    request: RecordingStartRequest,
    observed_at: Instant,
    file_config: RecordingFileConfig,
) -> ControlResult {
    if session.as_ref().is_some_and(|session| {
        matches!(
            session.controller.state(),
            RecordingState::Recording | RecordingState::Paused | RecordingState::Stopping
        )
    }) {
        return Ok(RecordingTransition::Unchanged);
    }

    let final_path = request.final_path;
    let partial_path = partial_recording_path(&final_path).ok();
    let writer = match RecordingFileWriter::create(
        &final_path,
        request.metadata,
        request.initial_size,
        file_config,
    ) {
        Ok(writer) => writer,
        Err(error) => {
            let failure = RecordingFailure::Storage(error.to_string());
            *session = None;
            shared.publish(
                RecordingSnapshot {
                    state: RecordingState::Failed(failure.clone()),
                    elapsed: Duration::ZERO,
                    event_count: 0,
                    payload_bytes: 0,
                    capture_input: request.recording.capture_input,
                    final_path: Some(final_path),
                    partial_path,
                    failure: Some(failure.clone()),
                },
                ClockUpdate::Frozen,
                true,
            );
            return Err(RecordingRuntimeError::Recording(failure));
        }
    };

    let partial_path = writer.partial_path().to_path_buf();
    let mut controller = RecordingController::new(request.recording.clone());
    if let Err(failure) = controller.start(Duration::ZERO) {
        shared.publish(failed_snapshot(failure.clone()), ClockUpdate::Frozen, true);
        return Err(RecordingRuntimeError::Recording(failure));
    }
    *session = Some(WorkerSession {
        controller,
        writer,
        epoch: observed_at,
        last_logical_time: Duration::ZERO,
        capture_input: request.recording.capture_input,
        final_path,
        partial_path,
    });
    let session = session.as_ref().expect("recording session was installed");
    queue.enable_recording(session.capture_input);
    shared.publish(session.snapshot(), ClockUpdate::Started(observed_at), true);
    Ok(RecordingTransition::Changed)
}

fn process_data(
    queue: &RuntimeQueue,
    shared: &RuntimeShared,
    session: Option<&mut WorkerSession>,
    data: &DataEnvelope,
) {
    let Some(session) = session else {
        return;
    };
    let event = match session.record(data.observed_at, data.kind.clone()) {
        Ok(Some(event)) => event,
        Ok(None) => return,
        Err(failure) => {
            queue.disable_recording();
            shared.publish(session.snapshot(), ClockUpdate::Frozen, true);
            debug_assert_eq!(session.controller.state(), &RecordingState::Failed(failure));
            return;
        }
    };
    if let Err(error) = session.writer.append(&event) {
        let _ = session.fail_storage(error);
        queue.disable_recording();
        shared.publish(session.snapshot(), ClockUpdate::Frozen, true);
        return;
    }
    shared.publish(session.snapshot(), ClockUpdate::Preserve, false);
}

fn process_pause(
    queue: &RuntimeQueue,
    shared: &RuntimeShared,
    session: Option<&mut WorkerSession>,
    observed_at: Instant,
) -> ControlResult {
    let Some(session) = session else {
        return result_from_snapshot(&shared.snapshot());
    };
    if let RecordingState::Failed(failure) = session.controller.state() {
        return Err(RecordingRuntimeError::Recording(failure.clone()));
    }
    let now = session.logical_time(observed_at);
    let transition = match session.controller.pause(now) {
        Ok(transition) => transition,
        Err(failure) => {
            queue.disable_recording();
            shared.publish(session.snapshot(), ClockUpdate::Frozen, true);
            return Err(RecordingRuntimeError::Recording(failure));
        }
    };
    if transition == RecordingTransition::Changed {
        if let Err(error) = session.writer.flush() {
            let failure = session.fail_storage(error);
            queue.disable_recording();
            shared.publish(session.snapshot(), ClockUpdate::Frozen, true);
            return Err(RecordingRuntimeError::Recording(failure));
        }
    }
    shared.publish(
        session.snapshot(),
        if transition == RecordingTransition::Changed {
            ClockUpdate::Paused(observed_at)
        } else {
            ClockUpdate::Preserve
        },
        transition == RecordingTransition::Changed,
    );
    Ok(transition)
}

fn process_resume(
    queue: &RuntimeQueue,
    shared: &RuntimeShared,
    session: Option<&mut WorkerSession>,
    observed_at: Instant,
) -> ControlResult {
    let Some(session) = session else {
        return result_from_snapshot(&shared.snapshot());
    };
    if let RecordingState::Failed(failure) = session.controller.state() {
        return Err(RecordingRuntimeError::Recording(failure.clone()));
    }
    let now = session.logical_time(observed_at);
    let transition = match session.controller.resume(now) {
        Ok(transition) => transition,
        Err(failure) => {
            queue.disable_recording();
            shared.publish(session.snapshot(), ClockUpdate::Frozen, true);
            return Err(RecordingRuntimeError::Recording(failure));
        }
    };
    if matches!(session.controller.state(), RecordingState::Recording) {
        queue.enable_recording(session.capture_input);
    }
    shared.publish(
        session.snapshot(),
        if transition == RecordingTransition::Changed {
            ClockUpdate::Resumed(observed_at)
        } else {
            ClockUpdate::Preserve
        },
        transition == RecordingTransition::Changed,
    );
    Ok(transition)
}

fn process_stop(
    queue: &RuntimeQueue,
    shared: &RuntimeShared,
    session: Option<&mut WorkerSession>,
    observed_at: Instant,
) -> ControlResult {
    let Some(session) = session else {
        return result_from_snapshot(&shared.snapshot());
    };
    if let RecordingState::Failed(failure) = session.controller.state() {
        return Err(RecordingRuntimeError::Recording(failure.clone()));
    }
    if matches!(session.controller.state(), RecordingState::Stopped) {
        return Ok(RecordingTransition::Unchanged);
    }

    let now = session.logical_time(observed_at);
    let transition = match session.controller.request_stop(now) {
        Ok(transition) => transition,
        Err(failure) => {
            queue.disable_recording();
            shared.publish(session.snapshot(), ClockUpdate::Frozen, true);
            return Err(RecordingRuntimeError::Recording(failure));
        }
    };
    if transition == RecordingTransition::Unchanged {
        return Ok(transition);
    }
    shared.publish(session.snapshot(), ClockUpdate::Frozen, true);

    if let Err(error) = session.writer.stop() {
        let failure = session.fail_storage(error);
        queue.disable_recording();
        shared.publish(session.snapshot(), ClockUpdate::Frozen, true);
        return Err(RecordingRuntimeError::Recording(failure));
    }
    session.controller.complete_stop();
    queue.disable_recording();
    shared.publish(session.snapshot(), ClockUpdate::Frozen, true);
    Ok(RecordingTransition::Changed)
}

fn process_shutdown(
    queue: &RuntimeQueue,
    shared: &RuntimeShared,
    session: Option<&mut WorkerSession>,
    observed_at: Instant,
) -> ControlResult {
    match process_stop(queue, shared, session, observed_at) {
        Ok(transition) => Ok(transition),
        Err(RecordingRuntimeError::Recording(_)) => Ok(RecordingTransition::Unchanged),
        Err(error) => Err(error),
    }
}

fn result_from_snapshot(snapshot: &RecordingSnapshot) -> ControlResult {
    match &snapshot.state {
        RecordingState::Failed(failure) => Err(RecordingRuntimeError::Recording(failure.clone())),
        _ => Ok(RecordingTransition::Unchanged),
    }
}

fn failed_snapshot(failure: RecordingFailure) -> RecordingSnapshot {
    RecordingSnapshot {
        state: RecordingState::Failed(failure.clone()),
        failure: Some(failure),
        ..RecordingSnapshot::default()
    }
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
type TestGateOption = Option<RecordingWorkerTestGate>;
#[cfg(not(test))]
type TestGateOption = ();

#[cfg(test)]
fn test_gate_none() -> TestGateOption {
    None
}

#[cfg(not(test))]
fn test_gate_none() -> TestGateOption {}

#[cfg(test)]
fn wait_on_test_gate(gate: &TestGateOption) {
    if let Some(gate) = gate {
        gate.wait_before_data();
    }
}

#[cfg(not(test))]
fn wait_on_test_gate(_gate: &TestGateOption) {}

#[cfg(test)]
#[derive(Clone)]
pub(super) struct RecordingWorkerTestGate {
    inner: Arc<(Mutex<TestGateState>, Condvar)>,
}

#[cfg(test)]
#[derive(Default)]
struct TestGateState {
    blocked: bool,
    worker_waiting: bool,
}

#[cfg(test)]
impl RecordingWorkerTestGate {
    pub(super) fn blocked() -> Self {
        Self {
            inner: Arc::new((
                Mutex::new(TestGateState {
                    blocked: true,
                    worker_waiting: false,
                }),
                Condvar::new(),
            )),
        }
    }

    fn wait_before_data(&self) {
        let (state, changed) = &*self.inner;
        let mut state = lock_unpoisoned(state);
        state.worker_waiting = true;
        changed.notify_all();
        while state.blocked {
            state = changed
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    pub(super) fn wait_until_worker_is_blocked(&self) {
        let (state, changed) = &*self.inner;
        let mut state = lock_unpoisoned(state);
        while !state.worker_waiting {
            state = changed
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    pub(super) fn release(&self) {
        let (state, changed) = &*self.inner;
        lock_unpoisoned(state).blocked = false;
        changed.notify_all();
    }
}
