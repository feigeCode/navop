use super::{
    RecordingConfig, RecordingEvent, RecordingEventKind, RecordingFailure, RecordingLimit,
    RecordingState, RecordingTransition,
};
use crate::TerminalSize;
use std::time::Duration;

pub struct RecordingController {
    config: RecordingConfig,
    state: RecordingState,
    started_at: Option<Duration>,
    last_observed_at: Option<Duration>,
    paused_at: Option<Duration>,
    paused_total: Duration,
    elapsed: Duration,
    event_count: u64,
    payload_bytes: u64,
}

impl RecordingController {
    pub fn new(config: RecordingConfig) -> Self {
        Self {
            config,
            state: RecordingState::Idle,
            started_at: None,
            last_observed_at: None,
            paused_at: None,
            paused_total: Duration::ZERO,
            elapsed: Duration::ZERO,
            event_count: 0,
            payload_bytes: 0,
        }
    }

    pub fn state(&self) -> &RecordingState {
        &self.state
    }

    pub fn event_count(&self) -> u64 {
        self.event_count
    }

    pub fn payload_bytes(&self) -> u64 {
        self.payload_bytes
    }

    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    pub fn start(&mut self, now: Duration) -> Result<RecordingTransition, RecordingFailure> {
        if self.state != RecordingState::Idle {
            return Ok(RecordingTransition::Unchanged);
        }
        self.started_at = Some(now);
        self.last_observed_at = Some(now);
        self.state = RecordingState::Recording;
        Ok(RecordingTransition::Changed)
    }

    pub fn pause(&mut self, now: Duration) -> Result<RecordingTransition, RecordingFailure> {
        if self.state != RecordingState::Recording {
            return Ok(RecordingTransition::Unchanged);
        }
        self.observe_active_time(now)?;
        self.paused_at = Some(now);
        self.state = RecordingState::Paused;
        Ok(RecordingTransition::Changed)
    }

    pub fn resume(&mut self, now: Duration) -> Result<RecordingTransition, RecordingFailure> {
        if self.state != RecordingState::Paused {
            return Ok(RecordingTransition::Unchanged);
        }
        self.ensure_monotonic(now)?;
        let paused_at = self.paused_at.take().expect("paused state has a timestamp");
        let paused_for = now
            .checked_sub(paused_at)
            .ok_or(RecordingFailure::ClockMovedBackwards)?;
        self.paused_total = self.paused_total.saturating_add(paused_for);
        self.state = RecordingState::Recording;
        Ok(RecordingTransition::Changed)
    }

    pub fn request_stop(&mut self, now: Duration) -> Result<RecordingTransition, RecordingFailure> {
        match self.state {
            RecordingState::Recording => {
                self.observe_active_time(now)?;
            }
            RecordingState::Paused => self.ensure_monotonic(now)?,
            _ => return Ok(RecordingTransition::Unchanged),
        }
        self.paused_at = None;
        self.state = RecordingState::Stopping;
        Ok(RecordingTransition::Changed)
    }

    pub fn complete_stop(&mut self) -> RecordingTransition {
        if self.state != RecordingState::Stopping {
            return RecordingTransition::Unchanged;
        }
        self.state = RecordingState::Stopped;
        RecordingTransition::Changed
    }

    pub fn fail(&mut self, failure: RecordingFailure) -> RecordingTransition {
        if matches!(
            self.state,
            RecordingState::Stopped | RecordingState::Failed(_)
        ) {
            return RecordingTransition::Unchanged;
        }
        self.state = RecordingState::Failed(failure);
        RecordingTransition::Changed
    }

    pub fn record_output(
        &mut self,
        now: Duration,
        data: Vec<u8>,
    ) -> Result<Option<RecordingEvent>, RecordingFailure> {
        self.record_event(now, RecordingEventKind::Output(data))
    }

    pub fn record_input(
        &mut self,
        now: Duration,
        data: Vec<u8>,
    ) -> Result<Option<RecordingEvent>, RecordingFailure> {
        if !self.config.capture_input {
            return Ok(None);
        }
        self.record_event(now, RecordingEventKind::Input(data))
    }

    pub fn record_resize(
        &mut self,
        now: Duration,
        size: TerminalSize,
    ) -> Result<Option<RecordingEvent>, RecordingFailure> {
        self.record_event(now, RecordingEventKind::Resize(size))
    }

    pub fn record_marker(
        &mut self,
        now: Duration,
        marker: String,
    ) -> Result<Option<RecordingEvent>, RecordingFailure> {
        self.record_event(now, RecordingEventKind::Marker(marker))
    }

    fn record_event(
        &mut self,
        now: Duration,
        kind: RecordingEventKind,
    ) -> Result<Option<RecordingEvent>, RecordingFailure> {
        if self.state != RecordingState::Recording {
            return Ok(None);
        }
        let payload_len = kind.payload_len();
        let next_payload_bytes = self.enforce_event_limits(payload_len)?;
        let elapsed = self.observe_active_time(now)?;
        self.event_count += 1;
        self.payload_bytes = next_payload_bytes;
        Ok(Some(RecordingEvent { elapsed, kind }))
    }

    fn enforce_event_limits(&mut self, payload_len: usize) -> Result<u64, RecordingFailure> {
        if self.event_count >= self.config.limits.max_events {
            return self.fail_limit(RecordingLimit::EventCount);
        }
        if payload_len > self.config.limits.max_event_bytes {
            return self.fail_limit(RecordingLimit::EventBytes);
        }
        let Some(next_payload_bytes) = u64::try_from(payload_len)
            .ok()
            .and_then(|payload_len| self.payload_bytes.checked_add(payload_len))
        else {
            return self.fail_limit(RecordingLimit::PayloadBytes);
        };
        if next_payload_bytes > self.config.limits.max_payload_bytes {
            return self.fail_limit(RecordingLimit::PayloadBytes);
        }
        Ok(next_payload_bytes)
    }

    fn observe_active_time(&mut self, now: Duration) -> Result<Duration, RecordingFailure> {
        self.ensure_monotonic(now)?;
        let started_at = self.started_at.expect("active recording has a start time");
        let elapsed = now
            .checked_sub(started_at)
            .and_then(|elapsed| elapsed.checked_sub(self.paused_total))
            .ok_or(RecordingFailure::ClockMovedBackwards)?;
        if elapsed > self.config.limits.max_duration {
            return self.fail_limit(RecordingLimit::Duration);
        }
        self.elapsed = elapsed;
        Ok(elapsed)
    }

    fn ensure_monotonic(&mut self, now: Duration) -> Result<(), RecordingFailure> {
        if self.last_observed_at.is_some_and(|last| now < last) {
            return self.fail_with(RecordingFailure::ClockMovedBackwards);
        }
        self.last_observed_at = Some(now);
        Ok(())
    }

    fn fail_limit<T>(&mut self, limit: RecordingLimit) -> Result<T, RecordingFailure> {
        self.fail_with(RecordingFailure::LimitReached(limit))
    }

    fn fail_with<T>(&mut self, failure: RecordingFailure) -> Result<T, RecordingFailure> {
        self.fail(failure.clone());
        Err(failure)
    }
}
