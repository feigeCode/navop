use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::RemoteDesktopOutput;

#[derive(Debug, Default)]
pub struct OutputBatch {
    pub control: Vec<RemoteDesktopOutput>,
    pub latest_frame: Option<RemoteDesktopOutput>,
    pub latest_delta: Option<RemoteDesktopOutput>,
}

#[derive(Clone)]
pub struct OutputMailboxSender {
    shared: Arc<Mutex<State>>,
    /// `None` is the backend-owned lifecycle sender. Helper sessions receive
    /// a generation-scoped clone so a detached reader from an old process
    /// cannot publish into a newer session.
    session_generation: Option<u64>,
}

pub struct OutputMailboxReceiver {
    shared: Arc<Mutex<State>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputMailboxClosed;

#[derive(Default)]
struct State {
    control: Vec<RemoteDesktopOutput>,
    latest_frame: Option<RemoteDesktopOutput>,
    latest_delta: Option<RemoteDesktopOutput>,
    /// Frames are accepted only while a helper session is known to be
    /// connected. A reconnect barrier flips this off so late output from the
    /// previous helper cannot be applied after the view has reset.
    accepting_frames: bool,
    next_session_generation: u64,
    active_session_generation: Option<u64>,
    receiver_alive: bool,
}

pub fn output_mailbox() -> (OutputMailboxSender, OutputMailboxReceiver) {
    let shared = Arc::new(Mutex::new(State {
        receiver_alive: true,
        accepting_frames: true,
        ..State::default()
    }));
    (
        OutputMailboxSender {
            shared: shared.clone(),
            session_generation: None,
        },
        OutputMailboxReceiver { shared },
    )
}

impl OutputMailboxSender {
    /// Create the sender for one helper-process session.
    ///
    /// Output readers are detached threads because joining them can block
    /// indefinitely when a descendant inherits the helper's stdout handle.
    /// Scoping their sender lets the backend retire a session synchronously
    /// without waiting for that reader to exit.
    pub fn begin_session(&self) -> Self {
        let mut state = lock(&self.shared);
        state.next_session_generation = state.next_session_generation.wrapping_add(1);
        let session_generation = state.next_session_generation;
        state.active_session_generation = Some(session_generation);
        Self {
            shared: self.shared.clone(),
            session_generation: Some(session_generation),
        }
    }

    /// Retire this helper session before the backend publishes its reconnect
    /// barrier. Any output still buffered by the detached reader is ignored.
    pub fn end_session(&self) {
        let Some(session_generation) = self.session_generation else {
            return;
        };
        let mut state = lock(&self.shared);
        if state.active_session_generation == Some(session_generation) {
            state.active_session_generation = None;
        }
    }

    pub fn send(&self, output: RemoteDesktopOutput) -> Result<(), OutputMailboxClosed> {
        let mut state = lock(&self.shared);
        if !state.receiver_alive {
            return Err(OutputMailboxClosed);
        }
        if let Some(session_generation) = self.session_generation
            && state.active_session_generation != Some(session_generation)
        {
            return Ok(());
        }
        match output {
            frame @ (RemoteDesktopOutput::Frame { .. } | RemoteDesktopOutput::FrameBgra { .. }) => {
                if state.accepting_frames {
                    state.latest_frame = Some(frame);
                    state.latest_delta = None;
                }
            }
            delta @ RemoteDesktopOutput::FrameBgraRects { .. } => {
                if state.accepting_frames {
                    state.latest_delta = Some(match state.latest_delta.take() {
                        Some(previous) => merge_deltas(previous, delta),
                        None => delta,
                    });
                }
            }
            connected @ RemoteDesktopOutput::Connected { .. } => {
                state.accepting_frames = true;
                state.control.push(connected);
            }
            terminal @ (RemoteDesktopOutput::ConnectionFailure(_)
            | RemoteDesktopOutput::Terminated(_)) => {
                state.accepting_frames = false;
                state.latest_frame = None;
                state.latest_delta = None;
                state.control.push(terminal);
            }
            reconnecting @ RemoteDesktopOutput::Reconnecting(_) => {
                // A frame queued by the old helper session must not be
                // accepted after the view has reset for the next session.
                state.accepting_frames = false;
                state.latest_frame = None;
                state.latest_delta = None;
                state.control.push(reconnecting);
            }
            control => state.control.push(control),
        }
        Ok(())
    }
}

impl OutputMailboxReceiver {
    pub fn drain(&self) -> OutputBatch {
        let mut state = lock(&self.shared);
        OutputBatch {
            control: std::mem::take(&mut state.control),
            latest_frame: state.latest_frame.take(),
            latest_delta: state.latest_delta.take(),
        }
    }
}

impl Drop for OutputMailboxReceiver {
    fn drop(&mut self) {
        let mut state = lock(&self.shared);
        state.receiver_alive = false;
        state.control.clear();
        state.latest_frame = None;
        state.latest_delta = None;
    }
}

fn merge_deltas(previous: RemoteDesktopOutput, next: RemoteDesktopOutput) -> RemoteDesktopOutput {
    match (previous, next) {
        (
            RemoteDesktopOutput::FrameBgraRects {
                width,
                height,
                mut rects,
                mut bgra,
            },
            RemoteDesktopOutput::FrameBgraRects {
                width: next_width,
                height: next_height,
                rects: next_rects,
                bgra: next_bgra,
            },
        ) if width == next_width && height == next_height => {
            rects.extend(next_rects);
            bgra.extend(next_bgra);
            RemoteDesktopOutput::FrameBgraRects {
                width,
                height,
                rects,
                bgra,
            }
        }
        (_, next) => next,
    }
}

impl fmt::Debug for OutputMailboxSender {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("OutputMailboxSender").finish()
    }
}

impl fmt::Debug for OutputMailboxReceiver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("OutputMailboxReceiver").finish()
    }
}

impl fmt::Display for OutputMailboxClosed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("remote desktop output mailbox is closed")
    }
}

impl std::error::Error for OutputMailboxClosed {}

fn lock(shared: &Mutex<State>) -> MutexGuard<'_, State> {
    shared.lock().unwrap_or_else(|error| error.into_inner())
}

#[cfg(test)]
#[path = "output_mailbox_tests.rs"]
mod tests;
