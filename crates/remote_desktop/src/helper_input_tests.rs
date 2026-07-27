use super::*;

#[test]
fn input_key_converts_to_helper_scancode() {
    let request = HelperRequest::from_remote_input(&RemoteDesktopInput::Key {
        key: RemoteKey::Named(RemoteNamedKey::Enter),
        pressed: true,
    });

    assert_eq!(
        Some(HelperRequest::Key {
            code: 0x1c,
            extended: false,
            pressed: true,
        }),
        request
    );
}

#[test]
fn input_prefixed_scancode_converts_to_extended_helper_scancode() {
    let request = HelperRequest::from_remote_input(&RemoteDesktopInput::Key {
        key: RemoteKey::Scancode(0xe048),
        pressed: true,
    });

    assert_eq!(
        Some(HelperRequest::Key {
            code: 0x48,
            extended: true,
            pressed: true,
        }),
        request
    );
}

#[test]
fn vnc_character_key_converts_to_helper_keysym() {
    let request = HelperRequest::from_remote_input_for_protocol(
        &RemoteDesktopInput::Key {
            key: RemoteKey::Character(':'),
            pressed: true,
        },
        RemoteDesktopProtocol::Vnc,
    );

    assert_eq!(
        Some(HelperRequest::KeySym {
            keysym: b':' as u32,
            pressed: true,
        }),
        request
    );
}

#[test]
fn vnc_named_key_converts_to_helper_keysym() {
    let request = HelperRequest::from_remote_input_for_protocol(
        &RemoteDesktopInput::Key {
            key: RemoteKey::Named(RemoteNamedKey::Tab),
            pressed: true,
        },
        RemoteDesktopProtocol::Vnc,
    );

    assert_eq!(
        Some(HelperRequest::KeySym {
            keysym: 0xff09,
            pressed: true,
        }),
        request
    );
}

#[test]
fn input_clipboard_text_converts_to_helper_request() {
    let request = HelperRequest::from_remote_input(&RemoteDesktopInput::ClipboardText {
        text: "hello 中文".to_string(),
    });

    assert_eq!(
        Some(HelperRequest::ClipboardText {
            text: "hello 中文".to_string()
        }),
        request
    );
}

#[test]
fn rdp_clipboard_files_convert_to_helper_request() {
    let request = HelperRequest::from_remote_input_for_protocol(
        &RemoteDesktopInput::ClipboardFiles {
            transfer_id: 11,
            paths: vec!["C:\\tmp\\report.txt".to_string()],
        },
        RemoteDesktopProtocol::Rdp,
    );

    assert_eq!(
        Some(HelperRequest::ClipboardFiles {
            transfer_id: 11,
            paths: vec!["C:\\tmp\\report.txt".to_string()]
        }),
        request
    );
}

#[test]
fn vnc_clipboard_files_are_filtered_before_helper_ipc() {
    let request = HelperRequest::from_remote_input_for_protocol(
        &RemoteDesktopInput::ClipboardFiles {
            transfer_id: 11,
            paths: vec!["C:\\tmp\\report.txt".to_string()],
        },
        RemoteDesktopProtocol::Vnc,
    );

    assert_eq!(None, request);
}

#[test]
fn input_reconnect_is_backend_control_not_helper_request() {
    let request = HelperRequest::from_remote_input(&RemoteDesktopInput::Reconnect);

    assert_eq!(None, request);
}

#[test]
fn clipboard_files_request_debug_does_not_leak_paths() {
    let request = HelperRequest::from_remote_input(&RemoteDesktopInput::ClipboardFiles {
        transfer_id: 42,
        paths: vec!["/tmp/report.txt".to_string()],
    });

    assert_eq!(
        Some(HelperRequest::ClipboardFiles {
            transfer_id: 42,
            paths: vec!["/tmp/report.txt".to_string()]
        }),
        request
    );
    let debug = format!("{request:?}");
    assert!(debug.contains("42"));
    assert!(debug.contains("count"));
    assert!(!debug.contains("/tmp/report.txt"));
}
