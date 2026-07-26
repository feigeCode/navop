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
mod tests {
    use super::*;
    use crate::RemoteDesktopFrameRect;

    #[test]
    fn keeps_only_latest_pending_frame() {
        let (tx, rx) = output_mailbox();
        tx.send(frame(1)).unwrap();
        tx.send(frame(2)).unwrap();
        tx.send(frame(3)).unwrap();

        let batch = rx.drain();

        assert_eq!(Vec::<RemoteDesktopOutput>::new(), batch.control);
        assert_eq!(Some(frame(3)), batch.latest_frame);
    }

    #[test]
    fn preserves_control_event_order_while_replacing_frames() {
        let (tx, rx) = output_mailbox();
        tx.send(RemoteDesktopOutput::Status("one".into())).unwrap();
        tx.send(frame(1)).unwrap();
        tx.send(RemoteDesktopOutput::ClipboardText { text: "two".into() })
            .unwrap();
        tx.send(frame(2)).unwrap();

        let batch = rx.drain();

        assert_eq!(
            vec![
                RemoteDesktopOutput::Status("one".into()),
                RemoteDesktopOutput::ClipboardText { text: "two".into() },
            ],
            batch.control
        );
        assert_eq!(Some(frame(2)), batch.latest_frame);
    }

    #[test]
    fn terminal_event_discards_pending_frame() {
        let (tx, rx) = output_mailbox();
        tx.send(frame(7)).unwrap();
        tx.send(RemoteDesktopOutput::Terminated("closed".into()))
            .unwrap();

        let batch = rx.drain();

        assert_eq!(None, batch.latest_frame);
        assert_eq!(
            vec![RemoteDesktopOutput::Terminated("closed".into())],
            batch.control
        );
    }

    #[test]
    fn reconnecting_event_discards_frames_from_the_previous_session() {
        let (tx, rx) = output_mailbox();
        tx.send(frame(7)).unwrap();
        tx.send(RemoteDesktopOutput::FrameBgraRects {
            width: 1,
            height: 1,
            rects: vec![RemoteDesktopFrameRect {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
                byte_len: 4,
            }],
            bgra: vec![1, 2, 3, 255],
        })
        .unwrap();
        tx.send(RemoteDesktopOutput::Reconnecting("network lost".into()))
            .unwrap();

        let batch = rx.drain();

        assert_eq!(None, batch.latest_frame);
        assert_eq!(None, batch.latest_delta);
        assert_eq!(
            vec![RemoteDesktopOutput::Reconnecting("network lost".into())],
            batch.control
        );
    }

    #[test]
    fn drops_late_frames_until_the_next_session_connects() {
        let (tx, rx) = output_mailbox();
        tx.send(RemoteDesktopOutput::Reconnecting("network lost".into()))
            .unwrap();
        tx.send(frame(7)).unwrap();
        tx.send(RemoteDesktopOutput::FrameBgraRects {
            width: 1,
            height: 1,
            rects: vec![RemoteDesktopFrameRect {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
                byte_len: 4,
            }],
            bgra: vec![1, 2, 3, 255],
        })
        .unwrap();
        tx.send(RemoteDesktopOutput::Connected {
            width: 1,
            height: 1,
            capabilities: crate::RemoteDesktopCapabilities::rdp_mvp(),
        })
        .unwrap();
        tx.send(frame(8)).unwrap();

        let batch = rx.drain();

        assert_eq!(None, batch.latest_delta);
        assert_eq!(Some(frame(8)), batch.latest_frame);
        assert!(matches!(
            batch.control.as_slice(),
            [
                RemoteDesktopOutput::Reconnecting(_),
                RemoteDesktopOutput::Connected { .. }
            ]
        ));
    }

    #[test]
    fn old_session_output_is_ignored_after_the_next_session_starts() {
        let (root_tx, rx) = output_mailbox();
        let first_session = root_tx.begin_session();
        first_session
            .send(RemoteDesktopOutput::Connected {
                width: 1,
                height: 1,
                capabilities: crate::RemoteDesktopCapabilities::rdp_mvp(),
            })
            .unwrap();
        first_session.send(frame(1)).unwrap();
        let first_batch = rx.drain();
        assert_eq!(Some(frame(1)), first_batch.latest_frame);

        first_session.end_session();
        root_tx
            .send(RemoteDesktopOutput::Reconnecting("network lost".into()))
            .unwrap();
        let second_session = root_tx.begin_session();
        second_session
            .send(RemoteDesktopOutput::Connected {
                width: 2,
                height: 2,
                capabilities: crate::RemoteDesktopCapabilities::rdp_mvp(),
            })
            .unwrap();
        second_session.send(frame(2)).unwrap();

        first_session
            .send(RemoteDesktopOutput::Terminated(
                "late output from the old helper".into(),
            ))
            .unwrap();
        first_session.send(frame(3)).unwrap();

        let second_batch = rx.drain();
        assert_eq!(Some(frame(2)), second_batch.latest_frame);
        assert_eq!(
            vec![
                RemoteDesktopOutput::Reconnecting("network lost".into()),
                RemoteDesktopOutput::Connected {
                    width: 2,
                    height: 2,
                    capabilities: crate::RemoteDesktopCapabilities::rdp_mvp(),
                },
            ],
            second_batch.control
        );
    }

    #[test]
    fn keeps_keyframe_when_coalescing_dirty_rectangles() {
        let (tx, rx) = output_mailbox();
        tx.send(RemoteDesktopOutput::FrameBgra {
            width: 128,
            height: 128,
            bgra: vec![0; 128 * 128 * 4],
        })
        .unwrap();
        tx.send(RemoteDesktopOutput::FrameBgraRects {
            width: 128,
            height: 128,
            rects: vec![RemoteDesktopFrameRect {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
                byte_len: 4,
            }],
            bgra: vec![1, 2, 3, 255],
        })
        .unwrap();

        let batch = rx.drain();
        assert!(matches!(
            batch.latest_frame,
            Some(RemoteDesktopOutput::FrameBgra { .. })
        ));
        assert!(matches!(
            batch.latest_delta,
            Some(RemoteDesktopOutput::FrameBgraRects { .. })
        ));
    }

    #[test]
    fn send_fails_after_receiver_is_dropped() {
        let (tx, rx) = output_mailbox();
        drop(rx);

        assert!(tx.send(frame(1)).is_err());
    }

    fn frame(value: u8) -> RemoteDesktopOutput {
        RemoteDesktopOutput::FrameBgra {
            width: 1,
            height: 1,
            bgra: vec![value, 0, 0, 255],
        }
    }
}
