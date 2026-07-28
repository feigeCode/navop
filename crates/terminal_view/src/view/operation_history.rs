use super::*;
use terminal::TerminalOperationHistoryRequestKey;

const OPERATION_HISTORY_CURRENT_SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq)]
struct OperationHistoryLoadError {
    key: TerminalOperationHistoryRequestKey,
    message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OperationHistoryCompletion {
    AppliedSuccess,
    AppliedError,
    IgnoredStale,
}

pub(super) struct OperationHistoryLoadState<T> {
    in_flight_key: Option<TerminalOperationHistoryRequestKey>,
    last_completed_key: Option<TerminalOperationHistoryRequestKey>,
    current_load: Option<T>,
    last_error: Option<OperationHistoryLoadError>,
}

impl<T> Default for OperationHistoryLoadState<T> {
    fn default() -> Self {
        Self {
            in_flight_key: None,
            last_completed_key: None,
            current_load: None,
            last_error: None,
        }
    }
}

impl<T> OperationHistoryLoadState<T> {
    /// Starts a load only when the terminal exposes a new request key.
    ///
    /// Ordinary PTY wakeups are intentionally no-ops for an in-flight or
    /// completed key. A future explicit refresh action can clear the completed
    /// key without turning every output event into another disk scan.
    fn begin(&mut self, request_key: Option<&TerminalOperationHistoryRequestKey>) -> bool {
        let Some(request_key) = request_key else {
            *self = Self::default();
            return false;
        };

        if self.in_flight_key.as_ref() == Some(request_key)
            || self.last_completed_key.as_ref() == Some(request_key)
        {
            return false;
        }

        self.in_flight_key = Some(request_key.clone());
        self.last_error = None;
        true
    }

    /// Applies a background result only when both the terminal's current key
    /// and the latest in-flight key still match the completed request.
    fn complete(
        &mut self,
        current_request_key: Option<&TerminalOperationHistoryRequestKey>,
        completed_key: &TerminalOperationHistoryRequestKey,
        result: Result<T, String>,
    ) -> OperationHistoryCompletion {
        let Some(current_request_key) = current_request_key else {
            *self = Self::default();
            return OperationHistoryCompletion::IgnoredStale;
        };
        let in_flight_matches = self.in_flight_key.as_ref() == Some(completed_key);
        let current_request_matches = current_request_key == completed_key;
        if !in_flight_matches || !current_request_matches {
            if in_flight_matches {
                self.in_flight_key = None;
            }
            return OperationHistoryCompletion::IgnoredStale;
        }

        self.in_flight_key = None;
        self.last_completed_key = Some(completed_key.clone());
        match result {
            Ok(load) => {
                self.current_load = Some(load);
                self.last_error = None;
                OperationHistoryCompletion::AppliedSuccess
            }
            Err(message) => {
                self.last_error = Some(OperationHistoryLoadError {
                    key: completed_key.clone(),
                    message,
                });
                OperationHistoryCompletion::AppliedError
            }
        }
    }
}

impl TerminalView {
    pub(super) fn sync_operation_history(&mut self, cx: &mut Context<Self>) {
        let request = self.terminal.read(cx).operation_history_request();
        let Some(request) = request else {
            self.operation_history.begin(None);
            return;
        };
        let request_key = request.key().clone();
        if !self.operation_history.begin(Some(&request_key)) {
            return;
        }

        let task =
            cx.background_spawn(
                async move { request.load(OPERATION_HISTORY_CURRENT_SNAPSHOT_TIMEOUT) },
            );
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let load = task.await;
            let _ = this.update(cx, |this, cx| {
                let current_request_key = this
                    .terminal
                    .read(cx)
                    .operation_history_request()
                    .map(|request| request.key().clone());
                let completion = this.operation_history.complete(
                    current_request_key.as_ref(),
                    &request_key,
                    Ok(load),
                );
                if completion != OperationHistoryCompletion::IgnoredStale {
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::{OperationHistoryCompletion, OperationHistoryLoadState};
    use terminal::TerminalOperationHistoryRequestKey;
    use terminal::operation_journal::{OperationJournalScope, OperationJournalSessionId};

    fn request_key(connection_generation: u64) -> TerminalOperationHistoryRequestKey {
        TerminalOperationHistoryRequestKey::new(
            OperationJournalScope::local(),
            OperationJournalSessionId::from("history-state-test"),
            connection_generation,
        )
    }

    #[test]
    fn matching_success_applies_and_same_key_does_not_rescan() {
        let key = request_key(1);
        let mut state = OperationHistoryLoadState::default();

        assert!(state.begin(Some(&key)));
        assert_eq!(
            state.complete(Some(&key), &key, Ok("generation-one")),
            OperationHistoryCompletion::AppliedSuccess
        );
        assert_eq!(state.current_load, Some("generation-one"));
        assert_eq!(state.last_completed_key.as_ref(), Some(&key));
        assert!(state.last_error.is_none());
        assert!(state.in_flight_key.is_none());
        assert!(!state.begin(Some(&key)));
    }

    #[test]
    fn matching_task_failure_applies_without_discarding_previous_history() {
        let old_key = request_key(1);
        let new_key = request_key(2);
        let mut state = OperationHistoryLoadState::default();

        assert!(state.begin(Some(&old_key)));
        assert_eq!(
            state.complete(Some(&old_key), &old_key, Ok("generation-one")),
            OperationHistoryCompletion::AppliedSuccess
        );
        assert!(state.begin(Some(&new_key)));
        assert_eq!(
            state.complete(
                Some(&new_key),
                &new_key,
                Err("history worker stopped".to_string()),
            ),
            OperationHistoryCompletion::AppliedError
        );

        assert_eq!(state.current_load, Some("generation-one"));
        assert_eq!(state.last_completed_key.as_ref(), Some(&new_key));
        assert_eq!(
            state
                .last_error
                .as_ref()
                .map(|error| error.message.as_str()),
            Some("history worker stopped")
        );
        assert_eq!(
            state.last_error.as_ref().map(|error| &error.key),
            Some(&new_key)
        );
        assert!(state.in_flight_key.is_none());
        assert!(!state.begin(Some(&new_key)));
    }

    #[test]
    fn stale_success_and_failure_cannot_overwrite_newer_request() {
        let old_key = request_key(1);
        let new_key = request_key(2);
        let mut state = OperationHistoryLoadState::default();

        assert!(state.begin(Some(&old_key)));
        assert!(state.begin(Some(&new_key)));

        assert_eq!(
            state.complete(Some(&new_key), &old_key, Ok("stale-success")),
            OperationHistoryCompletion::IgnoredStale
        );
        assert_eq!(
            state.complete(Some(&new_key), &old_key, Err("stale-error".to_string()),),
            OperationHistoryCompletion::IgnoredStale
        );

        assert_eq!(state.in_flight_key.as_ref(), Some(&new_key));
        assert!(state.current_load.is_none());
        assert!(state.last_completed_key.is_none());
        assert!(state.last_error.is_none());
    }

    #[test]
    fn late_stale_completion_cannot_overwrite_completed_newer_history() {
        let old_key = request_key(1);
        let new_key = request_key(2);
        let mut state = OperationHistoryLoadState::default();

        assert!(state.begin(Some(&old_key)));
        assert!(state.begin(Some(&new_key)));
        assert_eq!(
            state.complete(Some(&new_key), &new_key, Ok("generation-two")),
            OperationHistoryCompletion::AppliedSuccess
        );

        assert_eq!(
            state.complete(Some(&new_key), &old_key, Ok("late-stale-success")),
            OperationHistoryCompletion::IgnoredStale
        );
        assert_eq!(
            state.complete(
                Some(&new_key),
                &old_key,
                Err("late-stale-error".to_string()),
            ),
            OperationHistoryCompletion::IgnoredStale
        );

        assert_eq!(state.current_load, Some("generation-two"));
        assert_eq!(state.last_completed_key.as_ref(), Some(&new_key));
        assert!(state.last_error.is_none());
        assert!(state.in_flight_key.is_none());
    }

    #[test]
    fn duplicate_in_flight_is_rejected_but_newer_key_supersedes_it() {
        let old_key = request_key(1);
        let new_key = request_key(2);
        let mut state = OperationHistoryLoadState::<()>::default();

        assert!(state.begin(Some(&old_key)));
        assert!(!state.begin(Some(&old_key)));
        assert!(state.begin(Some(&new_key)));
        assert_eq!(state.in_flight_key.as_ref(), Some(&new_key));
    }

    #[test]
    fn current_terminal_key_is_rechecked_before_applying_completion() {
        let old_key = request_key(1);
        let new_key = request_key(2);
        let mut state = OperationHistoryLoadState::default();

        assert!(state.begin(Some(&old_key)));
        assert_eq!(
            state.complete(Some(&new_key), &old_key, Ok("stale-before-wakeup")),
            OperationHistoryCompletion::IgnoredStale
        );
        assert!(state.in_flight_key.is_none());
        assert!(state.begin(Some(&new_key)));
    }

    #[test]
    fn unavailable_history_cancels_state_and_never_starts_loading() {
        let key = request_key(1);
        let mut state = OperationHistoryLoadState::<()>::default();

        assert!(state.begin(Some(&key)));
        assert!(!state.begin(None));
        assert!(state.in_flight_key.is_none());
        assert!(state.current_load.is_none());
        assert!(state.last_completed_key.is_none());
        assert!(state.last_error.is_none());
    }

    #[test]
    fn unavailable_history_completion_clears_existing_snapshot() {
        let old_key = request_key(1);
        let new_key = request_key(2);
        let mut state = OperationHistoryLoadState::default();

        assert!(state.begin(Some(&old_key)));
        assert_eq!(
            state.complete(Some(&old_key), &old_key, Ok("generation-one")),
            OperationHistoryCompletion::AppliedSuccess
        );
        assert!(state.begin(Some(&new_key)));

        assert_eq!(
            state.complete(None, &new_key, Ok("must-not-apply")),
            OperationHistoryCompletion::IgnoredStale
        );
        assert!(state.in_flight_key.is_none());
        assert!(state.current_load.is_none());
        assert!(state.last_completed_key.is_none());
        assert!(state.last_error.is_none());
    }
}
