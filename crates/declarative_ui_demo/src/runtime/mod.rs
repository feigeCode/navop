mod action;
mod state;

use std::collections::BTreeMap;

use gpui::{Context, EventEmitter};
use thiserror::Error;

pub use action::{ActionContext, ActionError, ActionEvent};
use action::{ActionHandler, invoke_handler};
pub use state::{ActionOutcome, StateChange, StateChangeOrigin, StateStore};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeEvent {
    StateChanged(StateChange),
    ActionCompleted {
        event: ActionEvent,
        outcome: ActionOutcome,
    },
    ActionFailed {
        event: ActionEvent,
        error: RuntimeError,
    },
}

#[derive(Clone, Default)]
pub struct Runtime {
    state: StateStore,
    handlers: BTreeMap<String, ActionHandler>,
    revision: u64,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum RuntimeError {
    #[error("unknown action `{0}`")]
    UnknownAction(String),
    #[error("action `{0}` is already registered")]
    DuplicateAction(String),
    #[error("action `{action}` failed: {message}")]
    HandlerFailed { action: String, message: String },
    #[error("action `{action}` panicked: {message}")]
    HandlerPanicked { action: String, message: String },
}

impl Runtime {
    pub fn new(state: StateStore) -> Self {
        Self {
            state,
            ..Self::default()
        }
    }

    pub fn state(&self) -> &StateStore {
        &self.state
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.state.get(key)
    }

    pub fn set(
        &mut self,
        key: impl Into<String>,
        value: impl Into<String>,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut next = self.state.clone();
        if !next.set(key, value) {
            return false;
        }
        self.commit_external(next, cx);
        true
    }

    pub fn transaction<R>(
        &mut self,
        update: impl FnOnce(&mut StateStore) -> R,
        cx: &mut Context<Self>,
    ) -> R {
        let mut next = self.state.clone();
        let result = update(&mut next);
        if next != self.state {
            self.commit_external(next, cx);
        }
        result
    }

    pub fn on(
        &mut self,
        action: impl Into<String>,
        handler: impl Fn(&mut ActionContext<'_>) -> Result<(), ActionError> + 'static,
    ) -> Result<(), RuntimeError> {
        let action = action.into();
        if self.handlers.contains_key(&action) {
            return Err(RuntimeError::DuplicateAction(action));
        }
        self.handlers.insert(action, std::rc::Rc::new(handler));
        Ok(())
    }

    pub fn dispatch(
        &mut self,
        event: ActionEvent,
        cx: &mut Context<Self>,
    ) -> Result<ActionOutcome, RuntimeError> {
        match self.run_action(&event) {
            Ok(next) => self.complete_action(next, event, cx),
            Err(error) => {
                cx.emit(RuntimeEvent::ActionFailed {
                    event,
                    error: error.clone(),
                });
                cx.notify();
                Err(error)
            }
        }
    }

    fn run_action(&self, event: &ActionEvent) -> Result<StateStore, RuntimeError> {
        let handler = self
            .handlers
            .get(event.name())
            .ok_or_else(|| RuntimeError::UnknownAction(event.name().to_owned()))?;
        let mut next = self.state.clone();
        invoke_handler(handler, &mut next, event)?;
        Ok(next)
    }

    fn complete_action(
        &mut self,
        next: StateStore,
        event: ActionEvent,
        cx: &mut Context<Self>,
    ) -> Result<ActionOutcome, RuntimeError> {
        let state_changed = self.commit_action(next, &event, cx);
        let outcome = ActionOutcome {
            state_changed,
            revision: self.revision,
        };
        cx.emit(RuntimeEvent::ActionCompleted {
            event,
            outcome: outcome.clone(),
        });
        cx.notify();
        Ok(outcome)
    }

    fn commit_external(&mut self, next: StateStore, cx: &mut Context<Self>) {
        let change = self.commit_state(next, StateChangeOrigin::External);
        cx.emit(RuntimeEvent::StateChanged(change));
        cx.notify();
    }

    fn commit_action(
        &mut self,
        next: StateStore,
        event: &ActionEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        if next == self.state {
            return false;
        }
        let origin = StateChangeOrigin::Action {
            name: event.name().to_owned(),
            source_id: event.source_id().to_owned(),
        };
        let change = self.commit_state(next, origin);
        cx.emit(RuntimeEvent::StateChanged(change));
        true
    }

    fn commit_state(&mut self, next: StateStore, origin: StateChangeOrigin) -> StateChange {
        let changed_keys = self.state.changed_keys(&next);
        self.state = next;
        self.revision += 1;
        StateChange {
            revision: self.revision,
            changed_keys,
            origin,
        }
    }
}

impl EventEmitter<RuntimeEvent> for Runtime {}
