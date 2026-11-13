use super::*;

#[test]
fn request_batch_is_written_and_flushed_once() {
    #[derive(Default)]
    struct BatchWriter {
        write_calls: usize,
        flush_calls: usize,
        bytes: Vec<u8>,
    }

    impl std::io::Write for BatchWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.write_calls += 1;
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.flush_calls += 1;
            Ok(())
        }
    }

    let requests = [
        HelperRequest::MouseMove { x: 20, y: 30 },
        HelperRequest::MouseButton {
            button: crate::helper_protocol::HelperMouseButton::Left,
            pressed: true,
        },
        HelperRequest::MouseButton {
            button: crate::helper_protocol::HelperMouseButton::Left,
            pressed: false,
        },
    ];
    let mut writer = BatchWriter::default();

    encode_and_write_requests(&mut writer, requests.iter()).expect("request batch writes");

    assert_eq!(1, writer.write_calls);
    assert_eq!(1, writer.flush_calls);
    let lines: Vec<_> = writer
        .bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .collect();
    assert_eq!(3, lines.len());
    assert_eq!(
        HelperRequest::MouseMove { x: 20, y: 30 },
        crate::helper_protocol::decode_request_line(std::str::from_utf8(lines[0]).unwrap())
            .unwrap()
    );
}

#[test]
fn handshake_requires_the_matching_protocol_version() {
    let (output_tx, _output_rx) = crate::output_mailbox::output_mailbox();
    let (signal_tx, signal_rx) = std::sync::mpsc::channel();
    signal_tx
        .send(BackendSignal::ServerHello(
            crate::helper_protocol::HelperServerHello {
                protocol_version: crate::helper_protocol::HELPER_PROTOCOL_VERSION,
                provider_version: "0.4.0".to_string(),
                capabilities: crate::helper_protocol::HelperServerCapabilities {
                    heartbeat: true,
                    typed_errors: true,
                },
            },
        ))
        .unwrap();

    let hello = await_server_hello(&signal_rx, &output_tx, RemoteDesktopProtocol::Rdp)
        .expect("matching protocol version is accepted");
    assert!(hello.capabilities.heartbeat);

    let (signal_tx, signal_rx) = std::sync::mpsc::channel();
    signal_tx
        .send(BackendSignal::ServerHello(
            crate::helper_protocol::HelperServerHello {
                protocol_version: crate::helper_protocol::HELPER_PROTOCOL_VERSION + 1,
                provider_version: "9.9.9".to_string(),
                capabilities: Default::default(),
            },
        ))
        .unwrap();
    assert!(await_server_hello(&signal_rx, &output_tx, RemoteDesktopProtocol::Rdp).is_none());
}

#[cfg(unix)]
#[test]
fn failed_helper_startup_reaps_the_spawned_process() {
    let mut helper = std::process::Command::new("sleep")
        .arg("60")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .expect("spawn test helper");

    terminate_failed_helper(&mut helper, RemoteDesktopProtocol::Rdp);

    assert!(
        helper.try_wait().expect("query helper status").is_some(),
        "failed helper startup must not leave the process running"
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
fn forwarded_terminal_diagnostic_only_reaches_the_backend_signal() {
    let (output_tx, output_rx) = crate::output_mailbox::output_mailbox();
    let (signal_tx, signal_rx) = std::sync::mpsc::channel();
    let disconnect = HelperDisconnect {
        kind: Some(HelperDisconnectKind::Terminated),
        reason: "Another user connected to the server".to_string(),
        failure: None,
    };

    forward_helper_output(
        HelperOutput {
            output: None,
            connected: false,
            disconnect: Some(disconnect.clone()),
            server_hello: None,
            pong: None,
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
        server_hello: None,
        pong: None,
    }
}

fn frame(value: u8) -> RemoteDesktopOutput {
    RemoteDesktopOutput::FrameBgra {
        width: 1,
        height: 1,
        bgra: vec![value, 0, 0, 255],
    }
}
