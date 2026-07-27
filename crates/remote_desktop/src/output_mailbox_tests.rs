use super::*;
use crate::{
    RemoteDesktopCursor, RemoteDesktopFrameRect, RemoteDesktopReconnect,
    RemoteDesktopReconnectReason,
};

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
fn preserves_clipboard_transfer_events_without_coalescing() {
    let (tx, rx) = output_mailbox();
    let ready = RemoteDesktopOutput::ClipboardFilesReady {
        transfer_id: (1_u64 << 63) | 7,
        paths: vec!["/tmp/navop-rdp-clipboard/transfer-7/report.txt".into()],
    };
    let failed = RemoteDesktopOutput::ClipboardTransferFailed {
        transfer_id: (1_u64 << 63) | 8,
        message: "transfer failed".into(),
    };

    tx.send(ready.clone()).unwrap();
    tx.send(failed.clone()).unwrap();

    assert_eq!(vec![ready, failed], rx.drain().control);
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
    tx.send(reconnecting()).unwrap();

    let batch = rx.drain();

    assert_eq!(None, batch.latest_frame);
    assert_eq!(None, batch.latest_delta);
    assert_eq!(vec![reconnecting()], batch.control);
}

#[test]
fn drops_late_frames_until_the_next_session_connects() {
    let (tx, rx) = output_mailbox();
    tx.send(reconnecting()).unwrap();
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
    root_tx.send(reconnecting()).unwrap();
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
            reconnecting(),
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
fn coalesces_adjacent_cursor_positions() {
    let (tx, rx) = output_mailbox();
    tx.send(RemoteDesktopOutput::CursorPosition { x: 1, y: 2 })
        .unwrap();
    tx.send(RemoteDesktopOutput::CursorPosition { x: 3, y: 4 })
        .unwrap();

    assert_eq!(
        vec![RemoteDesktopOutput::CursorPosition { x: 3, y: 4 }],
        rx.drain().control
    );
}

#[test]
fn coalesces_adjacent_cursor_bitmaps_without_crossing_state_boundaries() {
    let (tx, rx) = output_mailbox();
    tx.send(cursor(1)).unwrap();
    tx.send(cursor(2)).unwrap();
    tx.send(RemoteDesktopOutput::CursorHidden).unwrap();
    tx.send(cursor(3)).unwrap();

    assert_eq!(
        vec![cursor(2), RemoteDesktopOutput::CursorHidden, cursor(3)],
        rx.drain().control
    );
}

#[test]
fn reconnect_barrier_discards_pending_cursor_state() {
    let (tx, rx) = output_mailbox();
    tx.send(RemoteDesktopOutput::CursorPosition { x: 1, y: 2 })
        .unwrap();
    tx.send(cursor(1)).unwrap();
    tx.send(reconnecting()).unwrap();

    assert_eq!(vec![reconnecting()], rx.drain().control);
}

#[test]
fn terminal_barrier_discards_pending_cursor_state() {
    let (tx, rx) = output_mailbox();
    tx.send(RemoteDesktopOutput::CursorHidden).unwrap();
    tx.send(RemoteDesktopOutput::ConnectionFailure("closed".into()))
        .unwrap();

    assert_eq!(
        vec![RemoteDesktopOutput::ConnectionFailure("closed".into())],
        rx.drain().control
    );
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

fn reconnecting() -> RemoteDesktopOutput {
    RemoteDesktopOutput::Reconnecting(RemoteDesktopReconnect {
        reason: RemoteDesktopReconnectReason::ConnectionLost,
        delay_secs: Some(1),
    })
}

fn cursor(value: u8) -> RemoteDesktopOutput {
    RemoteDesktopOutput::CursorBitmap(RemoteDesktopCursor {
        width: 1,
        height: 1,
        hotspot_x: 0,
        hotspot_y: 0,
        rgba: vec![value, 0, 0, 255],
    })
}
