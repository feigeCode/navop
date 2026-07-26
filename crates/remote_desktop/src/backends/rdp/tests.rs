use crate::helper_protocol::HelperEvent;
use crate::{RemoteDesktopFrameRect, ResizeSupport};

use super::input::{
    coalesce_remote_inputs, reconnect_delay, reconnect_event, remember_reconnect_state,
};
use super::transport::{
    HelperOutput, forward_helper_output, helper_disconnect_message, helper_event_to_output,
    read_helper_output, reconnect_replay_requests,
};
use super::*;

#[test]
fn converts_helper_connected_event_to_rdp_capabilities() {
    let output = helper_event_to_output(
        HelperEvent::Connected {
            width: 1280,
            height: 720,
        },
        RemoteDesktopProtocol::Rdp,
    )
    .expect("event converts");

    assert_eq!(
        output,
        RemoteDesktopOutput::Connected {
            width: 1280,
            height: 720,
            capabilities: crate::RemoteDesktopCapabilities {
                resize: ResizeSupport::RemoteResize,
                clipboard_text: true,
                cursor_shape: false,
                audio: true,
                file_transfer: true,
            }
        }
    );
}

#[test]
fn converts_helper_connected_event_to_vnc_capabilities() {
    let output = helper_event_to_output(
        HelperEvent::Connected {
            width: 1280,
            height: 720,
        },
        RemoteDesktopProtocol::Vnc,
    )
    .expect("event converts");

    assert_eq!(
        output,
        RemoteDesktopOutput::Connected {
            width: 1280,
            height: 720,
            capabilities: crate::RemoteDesktopCapabilities::vnc_mvp(),
        }
    );
}

#[test]
fn converts_helper_clipboard_text_event_to_output() {
    let output = helper_event_to_output(
        HelperEvent::ClipboardText {
            text: "remote 中文".to_string(),
        },
        RemoteDesktopProtocol::Rdp,
    )
    .expect("event converts");

    assert_eq!(
        output,
        RemoteDesktopOutput::ClipboardText {
            text: "remote 中文".to_string()
        }
    );
}

#[test]
fn reconnect_delay_uses_bounded_backoff() {
    assert_eq!(std::time::Duration::from_secs(1), reconnect_delay(0));
    assert_eq!(std::time::Duration::from_secs(2), reconnect_delay(1));
    assert_eq!(std::time::Duration::from_secs(5), reconnect_delay(2));
    assert_eq!(std::time::Duration::from_secs(10), reconnect_delay(3));
    assert_eq!(std::time::Duration::from_secs(10), reconnect_delay(20));
}

#[test]
fn helper_events_identify_disconnect_signals() {
    assert_eq!(
        Some("network".to_string()),
        helper_disconnect_message(&HelperEvent::ConnectionFailure {
            message: "network".to_string(),
        })
    );
    assert_eq!(
        Some("closed".to_string()),
        helper_disconnect_message(&HelperEvent::Terminated {
            message: "closed".to_string(),
        })
    );
    assert_eq!(
        None,
        helper_disconnect_message(&HelperEvent::Connected {
            width: 1,
            height: 1
        })
    );
}

#[test]
fn reads_binary_frame_event_from_helper_stream() {
    let mut input = std::io::Cursor::new(
        b"{\"type\":\"FrameBytes\",\"width\":2,\"height\":1,\"rgba_len\":8}\n\
          \x01\x02\x03\xff\x04\x05\x06\xff"
            .to_vec(),
    );

    let output = read_helper_output(&mut input, RemoteDesktopProtocol::Rdp)
        .expect("helper output reads")
        .expect("helper output exists")
        .output;

    assert_eq!(
        RemoteDesktopOutput::Frame {
            width: 2,
            height: 1,
            rgba: vec![1, 2, 3, 255, 4, 5, 6, 255],
        },
        output
    );
}

#[test]
fn reads_bgra_frame_event_from_helper_stream() {
    let mut input = std::io::Cursor::new(
        b"{\"type\":\"FrameBgraBytes\",\"width\":2,\"height\":1,\"bgra_len\":8}\n\
          \x03\x02\x01\xff\x06\x05\x04\xff"
            .to_vec(),
    );

    let output = read_helper_output(&mut input, RemoteDesktopProtocol::Rdp)
        .expect("helper output reads")
        .expect("helper output exists")
        .output;

    assert_eq!(
        RemoteDesktopOutput::FrameBgra {
            width: 2,
            height: 1,
            bgra: vec![3, 2, 1, 255, 6, 5, 4, 255],
        },
        output
    );
}

#[test]
fn reads_bgra_rectangles_from_helper_stream() {
    let mut input = std::io::Cursor::new(
        b"{\"type\":\"FrameBgraRects\",\"width\":2,\"height\":2,\"rects\":[{\"x\":1,\"y\":1,\"width\":1,\"height\":1,\"byte_len\":4}],\"bgra_len\":4}\n\
          \x03\x02\x01\xff"
            .to_vec(),
    );

    let output = read_helper_output(&mut input, RemoteDesktopProtocol::Rdp)
        .expect("helper output reads")
        .expect("helper output exists")
        .output;

    assert_eq!(
        RemoteDesktopOutput::FrameBgraRects {
            width: 2,
            height: 2,
            rects: vec![RemoteDesktopFrameRect {
                x: 1,
                y: 1,
                width: 1,
                height: 1,
                byte_len: 4,
            }],
            bgra: vec![3, 2, 1, 255],
        },
        output
    );
}

#[test]
fn reads_legacy_base64_frame_event_from_helper_stream() {
    let mut input = std::io::Cursor::new(
        b"{\"type\":\"Frame\",\"width\":2,\"height\":1,\"rgba_base64\":\"AQID/wQFBv8=\"}\n"
            .to_vec(),
    );

    let output = read_helper_output(&mut input, RemoteDesktopProtocol::Rdp)
        .expect("helper output reads")
        .expect("helper output exists")
        .output;

    assert_eq!(
        RemoteDesktopOutput::Frame {
            width: 2,
            height: 1,
            rgba: vec![1, 2, 3, 255, 4, 5, 6, 255],
        },
        output
    );
}

#[test]
fn forwarded_frames_keep_only_latest_pending_output() {
    let (output_tx, output_rx) = crate::output_mailbox::output_mailbox();
    let (signal_tx, _signal_rx) = std::sync::mpsc::channel();

    forward_helper_output(frame_output(1), &output_tx, &signal_tx);
    forward_helper_output(frame_output(2), &output_tx, &signal_tx);

    assert_eq!(Some(frame(2)), output_rx.drain().latest_frame);
}

#[test]
fn reconnect_event_classifies_fast_path_without_exposing_internal_details() {
    let reconnect = reconnect_event(
        "[Fast-Path @ /Users/hufei/.cargo/git/checkouts/ironrdp/src/lib.rs:98] custom error",
        std::time::Duration::from_secs(1),
    );

    assert_eq!(
        RemoteDesktopReconnect {
            reason: RemoteDesktopReconnectReason::DisplayUpdate,
            delay_secs: Some(1),
        },
        reconnect
    );
    assert!(!format!("{reconnect:?}").contains("/Users/"));
    assert!(!format!("{reconnect:?}").contains(".cargo/git/checkouts"));
}

#[test]
fn reconnect_event_uses_a_protocol_neutral_connection_lost_reason() {
    assert_eq!(
        RemoteDesktopReconnect {
            reason: RemoteDesktopReconnectReason::ConnectionLost,
            delay_secs: Some(2),
        },
        reconnect_event("socket closed", std::time::Duration::from_secs(2),)
    );
}

#[test]
fn coalesces_consecutive_mouse_moves_without_reordering_actions() {
    let inputs = vec![
        RemoteDesktopInput::MouseMove { x: 10, y: 10 },
        RemoteDesktopInput::MouseMove { x: 20, y: 20 },
        RemoteDesktopInput::MouseButton {
            button: crate::RemoteMouseButton::Left,
            pressed: true,
        },
        RemoteDesktopInput::MouseMove { x: 30, y: 30 },
        RemoteDesktopInput::MouseMove { x: 40, y: 40 },
        RemoteDesktopInput::Key {
            key: crate::RemoteKey::Named(crate::RemoteNamedKey::Enter),
            pressed: true,
        },
    ];

    assert_eq!(
        vec![
            RemoteDesktopInput::MouseMove { x: 20, y: 20 },
            RemoteDesktopInput::MouseButton {
                button: crate::RemoteMouseButton::Left,
                pressed: true,
            },
            RemoteDesktopInput::MouseMove { x: 40, y: 40 },
            RemoteDesktopInput::Key {
                key: crate::RemoteKey::Named(crate::RemoteNamedKey::Enter),
                pressed: true,
            },
        ],
        coalesce_remote_inputs(inputs)
    );
}

#[test]
fn rdp_remembers_file_clipboard_for_reconnect() {
    let mut connect = connect_request();
    let mut latest_clipboard_text = None;
    let mut latest_clipboard_files = None;

    remember_reconnect_state(
        &RemoteDesktopInput::ClipboardFiles {
            paths: vec!["C:\\tmp\\report.txt".to_string()],
        },
        &mut connect,
        &mut latest_clipboard_text,
        &mut latest_clipboard_files,
        RemoteDesktopProtocol::Rdp,
    );

    assert_eq!(
        Some(vec!["C:\\tmp\\report.txt".to_string()]),
        latest_clipboard_files
    );
}

#[test]
fn vnc_does_not_remember_file_clipboard_for_reconnect() {
    let mut connect = connect_request();
    let mut latest_clipboard_text = None;
    let mut latest_clipboard_files = None;

    remember_reconnect_state(
        &RemoteDesktopInput::ClipboardFiles {
            paths: vec!["C:\\tmp\\report.txt".to_string()],
        },
        &mut connect,
        &mut latest_clipboard_text,
        &mut latest_clipboard_files,
        RemoteDesktopProtocol::Vnc,
    );

    assert_eq!(None, latest_clipboard_files);
}

#[test]
fn reconnect_replay_sends_text_before_rdp_files() {
    let requests = reconnect_replay_requests(
        &Some("clipboard text".to_string()),
        &Some(vec!["C:\\tmp\\report.txt".to_string()]),
        RemoteDesktopProtocol::Rdp,
    );

    assert_eq!(
        vec![
            HelperRequest::ClipboardText {
                text: "clipboard text".to_string(),
            },
            HelperRequest::ClipboardFiles {
                paths: vec!["C:\\tmp\\report.txt".to_string()],
            },
        ],
        requests
    );
}

#[test]
fn reconnect_replay_never_sends_files_to_vnc() {
    let requests = reconnect_replay_requests(
        &Some("clipboard text".to_string()),
        &Some(vec!["C:\\tmp\\report.txt".to_string()]),
        RemoteDesktopProtocol::Vnc,
    );

    assert_eq!(
        vec![HelperRequest::ClipboardText {
            text: "clipboard text".to_string(),
        }],
        requests
    );
}

fn connect_request() -> HelperRequest {
    HelperRequest::Connect {
        destination: "127.0.0.1:3389".to_string(),
        username: None,
        password: None,
        domain: None,
        width: 1280,
        height: 720,
        scale_factor: 100,
        audio_playback: false,
    }
}

fn frame_output(value: u8) -> HelperOutput {
    HelperOutput {
        output: frame(value),
        connected: false,
        disconnect_message: None,
    }
}

fn frame(value: u8) -> RemoteDesktopOutput {
    RemoteDesktopOutput::FrameBgra {
        width: 1,
        height: 1,
        bgra: vec![value, 0, 0, 255],
    }
}
