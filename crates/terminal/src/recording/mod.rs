mod controller;
mod model;

pub use controller::RecordingController;
pub use model::{
    RecordingConfig, RecordingEvent, RecordingEventKind, RecordingFailure, RecordingLimit,
    RecordingLimits, RecordingState, RecordingTransition,
};

#[cfg(test)]
mod tests;
