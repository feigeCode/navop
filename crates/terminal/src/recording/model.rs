use crate::TerminalSize;
use std::fmt;
use std::time::Duration;

pub const DEFAULT_MAX_RECORDING_DURATION: Duration = Duration::from_secs(8 * 60 * 60);
pub const DEFAULT_MAX_RECORDING_EVENTS: u64 = 1_000_000;
pub const DEFAULT_MAX_RECORDING_EVENT_BYTES: usize = 4 * 1024 * 1024;
pub const DEFAULT_MAX_RECORDING_PAYLOAD_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordingLimit {
    Duration,
    EventCount,
    EventBytes,
    PayloadBytes,
    PendingEvents,
    PendingBytes,
}

impl fmt::Display for RecordingLimit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Duration => "duration",
            Self::EventCount => "event_count",
            Self::EventBytes => "event_bytes",
            Self::PayloadBytes => "payload_bytes",
            Self::PendingEvents => "pending_events",
            Self::PendingBytes => "pending_bytes",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordingFailure {
    ClockMovedBackwards,
    LimitReached(RecordingLimit),
    Storage(String),
}

impl fmt::Display for RecordingFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ClockMovedBackwards => formatter.write_str("recording_clock_moved_backwards"),
            Self::LimitReached(limit) => write!(formatter, "recording_{limit}_limit_reached"),
            Self::Storage(message) => write!(formatter, "recording_storage_failed: {message}"),
        }
    }
}

impl std::error::Error for RecordingFailure {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordingState {
    Idle,
    Recording,
    Paused,
    Stopping,
    Stopped,
    Failed(RecordingFailure),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordingTransition {
    Changed,
    Unchanged,
}

#[derive(Clone, Debug)]
pub struct RecordingLimits {
    pub max_duration: Duration,
    pub max_events: u64,
    pub max_event_bytes: usize,
    pub max_payload_bytes: u64,
}

impl Default for RecordingLimits {
    fn default() -> Self {
        Self {
            max_duration: DEFAULT_MAX_RECORDING_DURATION,
            max_events: DEFAULT_MAX_RECORDING_EVENTS,
            max_event_bytes: DEFAULT_MAX_RECORDING_EVENT_BYTES,
            max_payload_bytes: DEFAULT_MAX_RECORDING_PAYLOAD_BYTES,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct RecordingConfig {
    pub capture_input: bool,
    pub limits: RecordingLimits,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordingEvent {
    pub elapsed: Duration,
    pub kind: RecordingEventKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordingEventKind {
    Output(Vec<u8>),
    Input(Vec<u8>),
    Resize(TerminalSize),
    Marker(String),
}

impl RecordingEventKind {
    pub(crate) fn payload_len(&self) -> usize {
        match self {
            Self::Output(data) | Self::Input(data) => data.len(),
            Self::Resize(_) => std::mem::size_of::<TerminalSize>(),
            Self::Marker(marker) => marker.len(),
        }
    }
}
