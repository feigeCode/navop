use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::watch;

use crate::RemoteDesktopOutput;

#[derive(Debug, Default)]
pub struct OutputBatch {
    pub control: Vec<RemoteDesktopOutput>,
    pub latest_frame: Option<RemoteDesktopOutput>,
    pub latest_delta: Option<RemoteDesktopOutput>,
    pub stats: OutputMailboxStats,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OutputMailboxStats {
    pub outputs_received: u64,
    pub full_frames_received: u64,
    pub delta_frames_received: u64,
    pub full_frames_coalesced: u64,
    pub delta_frames_merged: u64,
    pub frames_dropped: u64,
    pub payload_bytes: u64,
    pub dirty_rects: u64,
    pub wakeups: u64,
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

pub struct OutputMailboxSubscription {
    shared: Arc<Mutex<State>>,
    notifications: watch::Receiver<u64>,
    observed_epoch: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputMailboxClosed;

struct State {
    control: Vec<RemoteDesktopOutput>,
    latest_frame: Option<RemoteDesktopOutput>,
    latest_delta: Option<RemoteDesktopOutput>,
    pending_stats: OutputMailboxStats,
    /// Frames are accepted only while a helper session is known to be
    /// connected. A reconnect barrier flips this off so late output from the
    /// previous helper cannot be applied after the view has reset.
    accepting_frames: bool,
    next_session_generation: u64,
    active_session_generation: Option<u64>,
    receiver_alive: bool,
    notification_epoch: u64,
    notifications: watch::Sender<u64>,
}

pub fn output_mailbox() -> (OutputMailboxSender, OutputMailboxReceiver) {
    let (notifications, _) = watch::channel(0);
    let shared = Arc::new(Mutex::new(State {
        control: Vec::new(),
        latest_frame: None,
        latest_delta: None,
        pending_stats: OutputMailboxStats::default(),
        receiver_alive: true,
        accepting_frames: true,
        next_session_generation: 0,
        active_session_generation: None,
        notification_epoch: 0,
        notifications,
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
        state.pending_stats.record_received(&output);
        if let Some(session_generation) = self.session_generation
            && state.active_session_generation != Some(session_generation)
        {
            state.pending_stats.record_dropped(&output);
            return Ok(());
        }
        let was_empty = state.is_empty();
        match output {
            frame @ (RemoteDesktopOutput::Frame { .. } | RemoteDesktopOutput::FrameBgra { .. }) => {
                if state.accepting_frames {
                    if state.latest_frame.is_some() {
                        state.pending_stats.full_frames_coalesced =
                            state.pending_stats.full_frames_coalesced.saturating_add(1);
                        state.pending_stats.frames_dropped =
                            state.pending_stats.frames_dropped.saturating_add(1);
                    }
                    if state.latest_delta.is_some() {
                        state.pending_stats.frames_dropped =
                            state.pending_stats.frames_dropped.saturating_add(1);
                    }
                    state.latest_frame = Some(frame);
                    state.latest_delta = None;
                } else {
                    state.pending_stats.frames_dropped =
                        state.pending_stats.frames_dropped.saturating_add(1);
                }
            }
            delta @ RemoteDesktopOutput::FrameBgraRects { .. } => {
                if state.accepting_frames {
                    if state.latest_delta.is_some() {
                        state.pending_stats.delta_frames_merged =
                            state.pending_stats.delta_frames_merged.saturating_add(1);
                    }
                    state.latest_delta = Some(match state.latest_delta.take() {
                        Some(previous) => merge_deltas(previous, delta),
                        None => delta,
                    });
                } else {
                    state.pending_stats.frames_dropped =
                        state.pending_stats.frames_dropped.saturating_add(1);
                }
            }
            connected @ RemoteDesktopOutput::Connected { .. } => {
                state.accepting_frames = true;
                state.control.push(connected);
            }
            terminal @ (RemoteDesktopOutput::ConnectionFailure(_)
            | RemoteDesktopOutput::Terminated(_)) => {
                state.accepting_frames = false;
                state.pending_stats.frames_dropped = state
                    .pending_stats
                    .frames_dropped
                    .saturating_add(u64::from(state.latest_frame.is_some()))
                    .saturating_add(u64::from(state.latest_delta.is_some()));
                state.latest_frame = None;
                state.latest_delta = None;
                discard_pending_cursor_outputs(&mut state.control);
                state.control.push(terminal);
            }
            reconnecting @ RemoteDesktopOutput::Reconnecting(_) => {
                // A frame queued by the old helper session must not be
                // accepted after the view has reset for the next session.
                state.accepting_frames = false;
                state.pending_stats.frames_dropped = state
                    .pending_stats
                    .frames_dropped
                    .saturating_add(u64::from(state.latest_frame.is_some()))
                    .saturating_add(u64::from(state.latest_delta.is_some()));
                state.latest_frame = None;
                state.latest_delta = None;
                discard_pending_cursor_outputs(&mut state.control);
                state.control.push(reconnecting);
            }
            control => enqueue_control(&mut state.control, control),
        }
        if was_empty && !state.is_empty() {
            state.notify_output_ready();
        }
        Ok(())
    }
}

impl OutputMailboxReceiver {
    pub fn subscribe(&self) -> OutputMailboxSubscription {
        let state = lock(&self.shared);
        OutputMailboxSubscription {
            shared: self.shared.clone(),
            notifications: state.notifications.subscribe(),
            observed_epoch: 0,
        }
    }

    pub fn drain(&self) -> OutputBatch {
        let mut state = lock(&self.shared);
        OutputBatch {
            control: std::mem::take(&mut state.control),
            latest_frame: state.latest_frame.take(),
            latest_delta: state.latest_delta.take(),
            stats: std::mem::take(&mut state.pending_stats),
        }
    }
}

impl OutputMailboxSubscription {
    /// Wait until the mailbox transitions from empty to non-empty.
    ///
    /// The mailbox remains latest-only: a burst produces one UI wakeup while
    /// frames and deltas continue to coalesce until the receiver drains them.
    pub async fn wait(&mut self) -> Result<(), OutputMailboxClosed> {
        loop {
            let (receiver_alive, notification_epoch) = {
                let state = lock(&self.shared);
                (state.receiver_alive, state.notification_epoch)
            };
            if !receiver_alive {
                return Err(OutputMailboxClosed);
            }
            if notification_epoch != self.observed_epoch {
                self.observed_epoch = notification_epoch;
                return Ok(());
            }
            if self.notifications.changed().await.is_err() {
                return Err(OutputMailboxClosed);
            }
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
        state.notify_output_ready();
    }
}

impl State {
    fn is_empty(&self) -> bool {
        self.control.is_empty() && self.latest_frame.is_none() && self.latest_delta.is_none()
    }

    fn notify_output_ready(&mut self) {
        self.notification_epoch = self.notification_epoch.wrapping_add(1);
        self.pending_stats.wakeups = self.pending_stats.wakeups.saturating_add(1);
        self.notifications.send_replace(self.notification_epoch);
    }
}

impl OutputMailboxStats {
    fn record_received(&mut self, output: &RemoteDesktopOutput) {
        self.outputs_received = self.outputs_received.saturating_add(1);
        match output {
            RemoteDesktopOutput::Frame { rgba, .. } => {
                self.full_frames_received = self.full_frames_received.saturating_add(1);
                self.payload_bytes = self
                    .payload_bytes
                    .saturating_add(u64::try_from(rgba.len()).unwrap_or(u64::MAX));
            }
            RemoteDesktopOutput::FrameBgra { bgra, .. } => {
                self.full_frames_received = self.full_frames_received.saturating_add(1);
                self.payload_bytes = self
                    .payload_bytes
                    .saturating_add(u64::try_from(bgra.len()).unwrap_or(u64::MAX));
            }
            RemoteDesktopOutput::FrameBgraRects { rects, bgra, .. } => {
                self.delta_frames_received = self.delta_frames_received.saturating_add(1);
                self.payload_bytes = self
                    .payload_bytes
                    .saturating_add(u64::try_from(bgra.len()).unwrap_or(u64::MAX));
                self.dirty_rects = self
                    .dirty_rects
                    .saturating_add(u64::try_from(rects.len()).unwrap_or(u64::MAX));
            }
            _ => {}
        }
    }

    fn record_dropped(&mut self, output: &RemoteDesktopOutput) {
        if matches!(
            output,
            RemoteDesktopOutput::Frame { .. }
                | RemoteDesktopOutput::FrameBgra { .. }
                | RemoteDesktopOutput::FrameBgraRects { .. }
        ) {
            self.frames_dropped = self.frames_dropped.saturating_add(1);
        }
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

fn enqueue_control(control: &mut Vec<RemoteDesktopOutput>, output: RemoteDesktopOutput) {
    match (control.last_mut(), output) {
        (
            Some(RemoteDesktopOutput::CursorPosition { x, y }),
            RemoteDesktopOutput::CursorPosition {
                x: next_x,
                y: next_y,
            },
        ) => {
            *x = next_x;
            *y = next_y;
        }
        (
            Some(previous @ RemoteDesktopOutput::CursorBitmap(_)),
            next @ RemoteDesktopOutput::CursorBitmap(_),
        ) => *previous = next,
        (_, output) => control.push(output),
    }
}

fn discard_pending_cursor_outputs(control: &mut Vec<RemoteDesktopOutput>) {
    control.retain(|output| {
        !matches!(
            output,
            RemoteDesktopOutput::CursorDefault
                | RemoteDesktopOutput::CursorHidden
                | RemoteDesktopOutput::CursorPosition { .. }
                | RemoteDesktopOutput::CursorBitmap(_)
        )
    });
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

impl fmt::Debug for OutputMailboxSubscription {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("OutputMailboxSubscription").finish()
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
