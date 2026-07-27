use std::{
    collections::BTreeMap,
    panic::{AssertUnwindSafe, catch_unwind},
    rc::Rc,
};

use thiserror::Error;

use crate::NodePath;

use super::{RuntimeError, StateStore};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionEvent {
    name: String,
    source_id: String,
    source_path: NodePath,
    payload: BTreeMap<String, String>,
}

impl ActionEvent {
    pub fn new(
        name: impl Into<String>,
        source_id: impl Into<String>,
        source_path: NodePath,
    ) -> Self {
        Self {
            name: name.into(),
            source_id: source_id.into(),
            source_path,
            payload: BTreeMap::new(),
        }
    }

    pub fn with_payload(mut self, payload: BTreeMap<String, String>) -> Self {
        self.payload = payload;
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    pub fn source_path(&self) -> &NodePath {
        &self.source_path
    }

    pub fn payload(&self) -> &BTreeMap<String, String> {
        &self.payload
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("{message}")]
pub struct ActionError {
    message: String,
}

impl ActionError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

pub struct ActionContext<'a> {
    state: &'a mut StateStore,
    event: &'a ActionEvent,
}

impl<'a> ActionContext<'a> {
    pub(super) fn new(state: &'a mut StateStore, event: &'a ActionEvent) -> Self {
        Self { state, event }
    }

    pub fn event(&self) -> &ActionEvent {
        self.event
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.state.get(key)
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) -> bool {
        self.state.set(key, value)
    }

    pub fn remove(&mut self, key: &str) -> bool {
        self.state.remove(key)
    }
}

pub(super) type ActionHandler = Rc<dyn Fn(&mut ActionContext<'_>) -> Result<(), ActionError>>;

pub(super) fn invoke_handler(
    handler: &ActionHandler,
    state: &mut StateStore,
    event: &ActionEvent,
) -> Result<(), RuntimeError> {
    let result = catch_unwind(AssertUnwindSafe(|| {
        handler(&mut ActionContext::new(state, event))
    }));
    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(RuntimeError::HandlerFailed {
            action: event.name().to_owned(),
            message: error.to_string(),
        }),
        Err(payload) => Err(RuntimeError::HandlerPanicked {
            action: event.name().to_owned(),
            message: panic_message(payload),
        }),
    }
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_owned()
    }
}
