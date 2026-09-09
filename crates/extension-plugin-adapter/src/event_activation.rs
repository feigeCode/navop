//! Host-owned lifecycle for provider event streams.
//!
//! Providers own event production, but Navop owns stream identity, pending
//! read state, cancellation, and cleanup. IDs are scoped to an exact runtime
//! generation so a replacement process cannot read or close a stale stream.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use extension_host::{HostError, HostResult};
use extension_protocol::event_stream::{EventCloseParams, EventOpenResult, EventReadParams};
use parking_lot::Mutex as SyncMutex;

pub const DEFAULT_MAX_OPEN_EVENT_STREAMS: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventStreamKey {
    pub extension_id: String,
    pub runtime_id: String,
    pub generation: u64,
    pub stream_id: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EventActivationError {
    #[error("runtime `{runtime_id}` generation `{generation}` is not active")]
    StaleGeneration { runtime_id: String, generation: u64 },
    #[error("event stream `{stream_id}` is already open")]
    DuplicateStream { stream_id: String },
    #[error("runtime `{runtime_id}` has reached its open event stream limit")]
    TooManyOpenStreams { runtime_id: String },
}

#[derive(Default)]
struct EventActivationState {
    runtime_generations: BTreeMap<String, u64>,
    open: BTreeMap<EventStreamKey, ()>,
}

/// Tracks provider event stream ownership and lifecycle.
pub struct EventActivationManager {
    max_open_per_runtime: usize,
    state: SyncMutex<EventActivationState>,
}

impl EventActivationManager {
    pub fn new() -> Self {
        Self {
            max_open_per_runtime: DEFAULT_MAX_OPEN_EVENT_STREAMS,
            state: SyncMutex::new(EventActivationState::default()),
        }
    }

    pub fn with_max_open_per_runtime(mut self, limit: usize) -> Self {
        self.max_open_per_runtime = limit.max(1);
        self
    }

    pub fn shared(self) -> Arc<Self> {
        Arc::new(self)
    }

    pub fn mark_runtime_active(&self, runtime_id: &str, generation: u64) {
        self.state
            .lock()
            .runtime_generations
            .insert(runtime_id.to_owned(), generation);
    }

    pub fn open(
        &self,
        extension_id: &str,
        runtime_id: &str,
        generation: u64,
        result: &EventOpenResult,
    ) -> HostResult<EventStreamKey> {
        let key = EventStreamKey {
            extension_id: extension_id.to_owned(),
            runtime_id: runtime_id.to_owned(),
            generation,
            stream_id: result.stream_id.clone(),
        };
        let mut state = self.state.lock();
        if state.runtime_generations.get(runtime_id).copied() != Some(generation) {
            return Err(event_protocol_error(
                EventActivationError::StaleGeneration {
                    runtime_id: runtime_id.to_owned(),
                    generation,
                },
            ));
        }
        if state.open.contains_key(&key) {
            return Err(event_protocol_error(
                EventActivationError::DuplicateStream {
                    stream_id: key.stream_id,
                },
            ));
        }
        if self.open_count_locked(&state, runtime_id) >= self.max_open_per_runtime {
            return Err(event_protocol_error(
                EventActivationError::TooManyOpenStreams {
                    runtime_id: runtime_id.to_owned(),
                },
            ));
        }
        state.open.insert(key.clone(), ());
        Ok(key)
    }

    pub fn validate_read(
        &self,
        extension_id: &str,
        runtime_id: &str,
        generation: u64,
        params: &EventReadParams,
    ) -> HostResult<()> {
        self.ensure_open(extension_id, runtime_id, generation, &params.stream_id)
    }

    pub fn validate_close(
        &self,
        extension_id: &str,
        runtime_id: &str,
        generation: u64,
        params: &EventCloseParams,
    ) -> HostResult<()> {
        self.ensure_open(extension_id, runtime_id, generation, &params.stream_id)
    }

    pub fn complete(&self, extension_id: &str, runtime_id: &str, generation: u64, stream_id: &str) {
        self.state.lock().open.remove(&EventStreamKey {
            extension_id: extension_id.to_owned(),
            runtime_id: runtime_id.to_owned(),
            generation,
            stream_id: stream_id.to_owned(),
        });
    }

    pub fn remove_runtime(&self, runtime_id: &str) -> BTreeSet<EventStreamKey> {
        let mut state = self.state.lock();
        state.runtime_generations.remove(runtime_id);
        let removed = state
            .open
            .keys()
            .filter(|key| key.runtime_id == runtime_id)
            .cloned()
            .collect::<BTreeSet<_>>();
        for key in &removed {
            state.open.remove(key);
        }
        removed
    }

    pub fn retire_generation(&self, runtime_id: &str, generation: u64) -> BTreeSet<EventStreamKey> {
        let mut state = self.state.lock();
        if state
            .runtime_generations
            .get(runtime_id)
            .is_none_or(|active| *active == generation)
        {
            state.runtime_generations.remove(runtime_id);
        }
        let removed = state
            .open
            .keys()
            .filter(|key| key.runtime_id == runtime_id && key.generation == generation)
            .cloned()
            .collect::<BTreeSet<_>>();
        for key in &removed {
            state.open.remove(key);
        }
        removed
    }

    pub fn remove_extension(&self, extension_id: &str) -> BTreeSet<EventStreamKey> {
        let mut state = self.state.lock();
        let runtime_ids = state
            .open
            .keys()
            .filter(|key| key.extension_id == extension_id)
            .map(|key| key.runtime_id.clone())
            .collect::<BTreeSet<_>>();
        for runtime_id in &runtime_ids {
            state.runtime_generations.remove(runtime_id);
        }
        let removed = state
            .open
            .keys()
            .filter(|key| key.extension_id == extension_id)
            .cloned()
            .collect::<BTreeSet<_>>();
        for key in &removed {
            state.open.remove(key);
        }
        removed
    }

    pub fn open_count(&self, runtime_id: &str) -> usize {
        self.open_count_locked(&self.state.lock(), runtime_id)
    }

    pub fn open_keys(&self) -> BTreeSet<EventStreamKey> {
        self.state.lock().open.keys().cloned().collect()
    }

    fn ensure_open(
        &self,
        extension_id: &str,
        runtime_id: &str,
        generation: u64,
        stream_id: &str,
    ) -> HostResult<()> {
        let state = self.state.lock();
        if state.runtime_generations.get(runtime_id).copied() != Some(generation) {
            return Err(event_protocol_error(
                EventActivationError::StaleGeneration {
                    runtime_id: runtime_id.to_owned(),
                    generation,
                },
            ));
        }
        if !state.open.contains_key(&EventStreamKey {
            extension_id: extension_id.to_owned(),
            runtime_id: runtime_id.to_owned(),
            generation,
            stream_id: stream_id.to_owned(),
        }) {
            return Err(HostError::protocol(extension_protocol::ProtocolError::new(
                extension_protocol::error::error_codes::INVALID_PARAMS,
                format!("event stream `{stream_id}` is not open for this runtime generation"),
            )));
        }
        Ok(())
    }

    fn open_count_locked(&self, state: &EventActivationState, runtime_id: &str) -> usize {
        state
            .open
            .keys()
            .filter(|key| key.runtime_id == runtime_id)
            .count()
    }
}

impl Default for EventActivationManager {
    fn default() -> Self {
        Self::new()
    }
}

fn event_protocol_error(error: EventActivationError) -> HostError {
    let code = match &error {
        EventActivationError::StaleGeneration { .. } => {
            extension_protocol::error::error_codes::PERMISSION_DENIED
        }
        EventActivationError::DuplicateStream { .. }
        | EventActivationError::TooManyOpenStreams { .. } => {
            extension_protocol::error::error_codes::RESOURCE_BUSY
        }
    };
    HostError::protocol(extension_protocol::ProtocolError::new(
        code,
        error.to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opened(id: &str) -> EventOpenResult {
        EventOpenResult {
            stream_id: id.into(),
        }
    }

    #[test]
    fn open_streams_are_owner_checked_and_deduplicated() {
        let manager = EventActivationManager::new();
        manager.mark_runtime_active("runtime", 0);
        let key = manager
            .open("extension", "runtime", 0, &opened("stream-1"))
            .unwrap();
        assert_eq!("stream-1", key.stream_id);

        let error = manager
            .open("extension", "runtime", 0, &opened("stream-1"))
            .unwrap_err();
        assert!(error.to_string().contains("already open"));
        assert!(
            manager
                .validate_read(
                    "other-extension",
                    "runtime",
                    0,
                    &EventReadParams {
                        stream_id: "stream-1".into(),
                        max_events: None,
                        wait_ms: Some(0),
                    }
                )
                .is_err()
        );
    }

    #[test]
    fn close_removes_only_the_exact_stream() {
        let manager = EventActivationManager::new();
        manager.mark_runtime_active("runtime", 0);
        manager
            .open("extension", "runtime", 0, &opened("one"))
            .unwrap();
        manager
            .open("extension", "runtime", 0, &opened("two"))
            .unwrap();
        manager.complete("extension", "runtime", 0, "one");

        assert_eq!(1, manager.open_count("runtime"));
        assert!(
            manager
                .validate_close(
                    "extension",
                    "runtime",
                    0,
                    &EventCloseParams {
                        stream_id: "two".into()
                    }
                )
                .is_ok()
        );
    }

    #[test]
    fn generation_retirement_removes_only_that_generation() {
        let manager = EventActivationManager::new();
        manager.mark_runtime_active("runtime", 0);
        manager
            .open("extension", "runtime", 0, &opened("old"))
            .unwrap();
        let removed = manager.retire_generation("runtime", 0);
        manager.mark_runtime_active("runtime", 1);
        manager
            .open("extension", "runtime", 1, &opened("new"))
            .unwrap();

        assert_eq!(1, removed.len());
        assert_eq!("old", removed.iter().next().unwrap().stream_id);
        assert!(
            manager
                .validate_read(
                    "extension",
                    "runtime",
                    0,
                    &EventReadParams {
                        stream_id: "old".into(),
                        max_events: None,
                        wait_ms: Some(0),
                    }
                )
                .is_err()
        );
        assert!(
            manager
                .validate_read(
                    "extension",
                    "runtime",
                    1,
                    &EventReadParams {
                        stream_id: "new".into(),
                        max_events: None,
                        wait_ms: Some(0),
                    }
                )
                .is_ok()
        );
    }

    #[test]
    fn per_runtime_limit_is_enforced() {
        let manager = EventActivationManager::new().with_max_open_per_runtime(1);
        manager.mark_runtime_active("runtime", 0);
        manager
            .open("extension", "runtime", 0, &opened("one"))
            .unwrap();
        let error = manager
            .open("extension", "runtime", 0, &opened("two"))
            .unwrap_err();
        assert!(error.to_string().contains("open event stream limit"));
    }
}
