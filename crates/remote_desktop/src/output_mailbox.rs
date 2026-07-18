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
    receiver_alive: bool,
}

pub fn output_mailbox() -> (OutputMailboxSender, OutputMailboxReceiver) {
    let shared = Arc::new(Mutex::new(State {
        receiver_alive: true,
        ..State::default()
    }));
    (
        OutputMailboxSender {
            shared: shared.clone(),
        },
        OutputMailboxReceiver { shared },
    )
}

impl OutputMailboxSender {
    pub fn send(&self, output: RemoteDesktopOutput) -> Result<(), OutputMailboxClosed> {
        let mut state = lock(&self.shared);
        if !state.receiver_alive {
            return Err(OutputMailboxClosed);
        }
        match output {
            frame @ (RemoteDesktopOutput::Frame { .. } | RemoteDesktopOutput::FrameBgra { .. }) => {
                state.latest_frame = Some(frame);
                state.latest_delta = None;
            }
            delta @ RemoteDesktopOutput::FrameBgraRects { .. } => {
                state.latest_delta = Some(match state.latest_delta.take() {
                    Some(previous) => merge_deltas(previous, delta),
                    None => delta,
                });
            }
            terminal @ (RemoteDesktopOutput::ConnectionFailure(_)
            | RemoteDesktopOutput::Terminated(_)) => {
                state.latest_frame = None;
                state.latest_delta = None;
                state.control.push(terminal);
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
