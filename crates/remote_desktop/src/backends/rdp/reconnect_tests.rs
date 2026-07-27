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
    let reconnect = reconnect_event(
        "[Fast-Path @ /Users/hufei/.cargo/git/checkouts/ironrdp/src/lib.rs:98] custom error",
        Duration::from_secs(1),
    );

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
        reconnect_event("socket closed", Duration::from_secs(2))
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
