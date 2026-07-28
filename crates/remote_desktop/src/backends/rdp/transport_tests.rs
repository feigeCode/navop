use super::*;

#[test]
fn forwarded_frames_keep_only_latest_pending_output() {
    let (output_tx, output_rx) = crate::output_mailbox::output_mailbox();
    let (signal_tx, _signal_rx) = std::sync::mpsc::channel();

    forward_helper_output(frame_output(1), &output_tx, &signal_tx);
    forward_helper_output(frame_output(2), &output_tx, &signal_tx);

    assert_eq!(Some(frame(2)), output_rx.drain().latest_frame);
}

#[test]
fn forwarded_terminal_diagnostic_only_reaches_the_backend_signal() {
    let (output_tx, output_rx) = crate::output_mailbox::output_mailbox();
    let (signal_tx, signal_rx) = std::sync::mpsc::channel();
    let disconnect = HelperDisconnect {
        kind: Some(HelperDisconnectKind::Terminated),
        reason: "Another user connected to the server".to_string(),
    };

    forward_helper_output(
        HelperOutput {
            output: None,
            connected: false,
            disconnect: Some(disconnect.clone()),
        },
        &output_tx,
        &signal_tx,
    );

    assert!(output_rx.drain().control.is_empty());
    assert_eq!(
        BackendSignal::Disconnected(disconnect),
        signal_rx.try_recv().expect("disconnect signal")
    );
}

#[test]
fn reconnect_replay_sends_text_before_rdp_files() {
    let requests = reconnect_replay_requests(
        &Some("clipboard text".to_string()),
        &Some(ClipboardFilesSnapshot {
            transfer_id: 17,
            paths: vec!["C:\\tmp\\report.txt".to_string()],
        }),
        RemoteDesktopProtocol::Rdp,
    );

    assert_eq!(
        vec![
            HelperRequest::ClipboardText {
                text: "clipboard text".to_string(),
            },
            HelperRequest::ClipboardFiles {
                transfer_id: 17,
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
        &Some(ClipboardFilesSnapshot {
            transfer_id: 17,
            paths: vec!["C:\\tmp\\report.txt".to_string()],
        }),
        RemoteDesktopProtocol::Vnc,
    );

    assert_eq!(
        vec![HelperRequest::ClipboardText {
            text: "clipboard text".to_string(),
        }],
        requests
    );
}

fn frame_output(value: u8) -> HelperOutput {
    HelperOutput {
        output: Some(frame(value)),
        connected: false,
        disconnect: None,
    }
}

fn frame(value: u8) -> RemoteDesktopOutput {
    RemoteDesktopOutput::FrameBgra {
        width: 1,
        height: 1,
        bgra: vec![value, 0, 0, 255],
    }
}
