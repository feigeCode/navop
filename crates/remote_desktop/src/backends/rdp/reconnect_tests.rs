use super::*;

#[test]
fn reconnect_delay_uses_bounded_backoff() {
    assert_eq!(Duration::from_secs(1), reconnect_delay(0));
    assert_eq!(Duration::from_secs(2), reconnect_delay(1));
    assert_eq!(Duration::from_secs(5), reconnect_delay(2));
    assert_eq!(Duration::from_secs(10), reconnect_delay(3));
    assert_eq!(Duration::from_secs(10), reconnect_delay(20));
}

#[test]
fn reconnect_event_classifies_fast_path_without_exposing_internal_details() {
    let decision = reconnect_decision(
        "[Fast-Path @ /Users/hufei/.cargo/git/checkouts/ironrdp/src/lib.rs:98] custom error",
        true,
        None,
    );
    let ReconnectDecision::Retry(reason) = decision else {
        panic!("fast-path failure should remain retryable");
    };
    let reconnect = reconnect_event(reason, Duration::from_secs(1));

    assert_eq!(
        RemoteDesktopReconnect {
            reason: RemoteDesktopReconnectReason::DisplayUpdate,
            delay_secs: Some(1),
        },
        reconnect
    );
    let debug = format!("{reconnect:?}");
    assert!(!debug.contains("/Users/"));
    assert!(!debug.contains(".cargo/git/checkouts"));
}

#[test]
fn reconnect_event_uses_a_protocol_neutral_connection_lost_reason() {
    assert_eq!(
        RemoteDesktopReconnect {
            reason: RemoteDesktopReconnectReason::ConnectionLost,
            delay_secs: Some(2),
        },
        reconnect_event(
            RemoteDesktopReconnectReason::ConnectionLost,
            Duration::from_secs(2)
        )
    );
}

#[test]
fn another_user_disconnect_is_a_terminal_session_takeover() {
    let decision = reconnect_decision(
        "[Protocol independent error] Another user connected to the server, \
         forcing the disconnection of the current connection",
        true,
        Some(HelperDisconnectKind::Terminated),
    );

    assert_eq!(
        ReconnectDecision::Terminated(RemoteDesktopFailure::SessionTakenOver),
        decision
    );
}

#[test]
fn credssp_failure_is_a_terminal_authentication_failure_without_internal_details() {
    let decision = reconnect_decision(
        "[CredSSP @ /Users/runner/.cargo/git/checkouts/ironrdp/src/connector.rs:107] CredSSP",
        false,
        Some(HelperDisconnectKind::ConnectionFailure),
    );

    assert_eq!(
        ReconnectDecision::ConnectionFailure(RemoteDesktopFailure::AuthenticationFailed),
        decision
    );
    let debug = format!("{decision:?}");
    assert!(!debug.contains("/Users/"));
    assert!(!debug.contains(".cargo/git/checkouts"));
    assert!(!debug.contains("connector.rs"));
    assert!(!debug.contains("CredSSP"));
}

#[test]
fn initial_connection_refused_is_a_terminal_host_failure() {
    assert_eq!(
        ReconnectDecision::ConnectionFailure(RemoteDesktopFailure::HostUnreachable),
        reconnect_decision(
            "connection refused",
            false,
            Some(HelperDisconnectKind::ConnectionFailure),
        )
    );
}

#[test]
fn established_socket_disconnect_remains_retryable() {
    assert_eq!(
        ReconnectDecision::Retry(RemoteDesktopReconnectReason::ConnectionLost),
        reconnect_decision("socket closed", true, None)
    );
}

#[test]
fn explicit_server_termination_does_not_retry() {
    assert_eq!(
        ReconnectDecision::Terminated(RemoteDesktopFailure::ServerEndedSession),
        reconnect_decision(
            "remote session terminated",
            true,
            Some(HelperDisconnectKind::Terminated),
        )
    );
}

#[test]
fn rdp_remembers_file_clipboard_for_reconnect() {
    let mut connect = connect_request();
    let mut latest_clipboard_text = None;
    let mut latest_clipboard_files = None;

    remember_reconnect_state(
        &RemoteDesktopInput::ClipboardFiles {
            transfer_id: 17,
            paths: vec!["C:\\tmp\\report.txt".to_string()],
        },
        &mut connect,
        &mut latest_clipboard_text,
        &mut latest_clipboard_files,
        RemoteDesktopProtocol::Rdp,
    );

    assert_eq!(
        Some(ClipboardFilesSnapshot {
            transfer_id: 17,
            paths: vec!["C:\\tmp\\report.txt".to_string()],
        }),
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
            transfer_id: 17,
            paths: vec!["C:\\tmp\\report.txt".to_string()],
        },
        &mut connect,
        &mut latest_clipboard_text,
        &mut latest_clipboard_files,
        RemoteDesktopProtocol::Vnc,
    );

    assert_eq!(None, latest_clipboard_files);
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
        audio_capture: false,
        shared_folders: Vec::new(),
    }
}
