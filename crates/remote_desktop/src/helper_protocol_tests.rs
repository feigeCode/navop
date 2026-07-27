use super::*;
use crate::{RemoteDesktopConnectionOptions, RemoteDesktopProtocol, RemoteDesktopSize};

fn rdp_options() -> RemoteDesktopConnectionOptions {
    RemoteDesktopConnectionOptions {
        protocol: RemoteDesktopProtocol::Rdp,
        destination: "10.2.178.12:3389".to_string(),
        username: Some("administrator".to_string()),
        password: Some("Seeyon123@cd".to_string()),
        domain: Some("private.example".to_string()),
        read_only: false,
        audio_playback: true,
        audio_capture: false,
        shared_folders: vec![RemoteDesktopSharedFolder {
            name: "workspace".to_string(),
            path: std::path::PathBuf::from("/Users/rachel/private-project"),
            read_only: true,
        }],
        proxy: None,
    }
}

#[test]
fn connect_request_debug_redacts_credentials_and_shared_folders() {
    let request = HelperRequest::connect_from_options(
        &rdp_options(),
        RemoteDesktopSize {
            width: 1280,
            height: 720,
            scale_factor: 100,
        },
    );
    let debug = format!("{request:?}");

    assert!(debug.contains("10.2.178.12:3389"));
    assert!(debug.contains("username_present: true"));
    assert!(debug.contains("username_len: Some(13)"));
    assert!(debug.contains("password_present: true"));
    assert!(debug.contains("domain_present: true"));
    assert!(debug.contains("domain_len: Some(15)"));
    assert!(debug.contains("shared_folder_count: 1"));
    for secret in [
        "administrator",
        "Seeyon123@cd",
        "private.example",
        "workspace",
        "private-project",
    ] {
        assert!(!debug.contains(secret));
    }
}

#[test]
fn text_request_debug_reports_length_without_content() {
    for request in [
        HelperRequest::Text {
            text: "typed-secret".to_string(),
        },
        HelperRequest::ClipboardText {
            text: "clipboard-secret".to_string(),
        },
    ] {
        let debug = format!("{request:?}");
        assert!(debug.contains("text_len"));
        assert!(!debug.contains("typed-secret"));
        assert!(!debug.contains("clipboard-secret"));
    }
}

#[test]
fn helper_event_debug_reports_metadata_without_payloads() {
    let events = [
        HelperEvent::Status {
            message: "private-status".to_string(),
        },
        HelperEvent::Frame {
            width: 1,
            height: 1,
            rgba_base64: "private-base64".to_string(),
        },
        HelperEvent::CursorRgbaBytes {
            width: 1,
            height: 1,
            hotspot_x: 0,
            hotspot_y: 0,
            rgba_len: "private-cursor".len(),
        },
        HelperEvent::ClipboardText {
            text: "private-clipboard".to_string(),
        },
        HelperEvent::ClipboardFilesReady {
            transfer_id: 7,
            paths: vec!["/Users/rachel/private-file".to_string()],
        },
        HelperEvent::ClipboardTransferFailed {
            transfer_id: 8,
            message: "private-transfer-error".to_string(),
        },
        HelperEvent::ConnectionFailure {
            message: "private-connection-error".to_string(),
        },
        HelperEvent::Terminated {
            message: "private-termination".to_string(),
        },
    ];

    for event in events {
        assert!(!format!("{event:?}").contains("private-"));
    }
}

#[test]
fn binary_cursor_header_decodes_without_embedding_pixels() {
    let event = decode_event_line(
        r#"{"type":"CursorRgbaBytes","width":2,"height":1,"hotspot_x":1,"hotspot_y":0,"rgba_len":8}"#,
    )
    .expect("binary cursor header decodes");

    assert_eq!(
        HelperEvent::CursorRgbaBytes {
            width: 2,
            height: 1,
            hotspot_x: 1,
            hotspot_y: 0,
            rgba_len: 8,
        },
        event
    );
    assert!(
        event
            .into_rgba()
            .expect_err("cursor payload is not part of the JSON header")
            .to_string()
            .contains("binary cursor payload")
    );
}

#[test]
fn connect_request_round_trips_audio_playback() {
    let request: HelperRequest = serde_json::from_str(
        r#"{
            "type":"Connect",
            "destination":"10.2.178.12:3389",
            "username":"administrator",
            "password":"secret",
            "domain":null,
            "width":1280,
            "height":720,
            "scale_factor":100,
            "audio_playback":true
        }"#,
    )
    .expect("connect request decodes");

    let encoded = serde_json::to_value(request).expect("connect request encodes");
    assert_eq!(
        Some(&serde_json::Value::Bool(true)),
        encoded.get("audio_playback")
    );
}

#[test]
fn legacy_connect_request_defaults_optional_features() {
    let request: HelperRequest = serde_json::from_str(
        r#"{
            "type":"Connect",
            "destination":"10.2.178.12:3389",
            "username":"administrator",
            "password":"secret",
            "domain":null,
            "width":1280,
            "height":720,
            "scale_factor":100
        }"#,
    )
    .expect("legacy connect request decodes");

    let encoded = serde_json::to_value(request).expect("connect request encodes");
    assert_eq!(
        Some(&serde_json::Value::Bool(false)),
        encoded.get("audio_playback")
    );
    assert_eq!(
        Some(&serde_json::Value::Bool(false)),
        encoded.get("audio_capture")
    );
    assert_eq!(
        Some(&serde_json::Value::Array(Vec::new())),
        encoded.get("shared_folders")
    );
}

#[test]
fn helper_frame_roundtrips_rgba_payload() {
    let event = HelperEvent::frame(2, 1, vec![1, 2, 3, 255, 4, 5, 6, 255]);
    let line = encode_event_line(&event).expect("event encodes");
    let decoded = decode_event_line(&line).expect("event decodes");

    assert_eq!(
        HelperEvent::Frame {
            width: 2,
            height: 1,
            rgba_base64: "AQID/wQFBv8=".to_string()
        },
        decoded
    );
    assert_eq!(
        vec![1, 2, 3, 255, 4, 5, 6, 255],
        decoded.into_rgba().expect("rgba decodes")
    );
}

#[test]
fn reconnecting_wire_contract_is_stable() {
    let event = HelperEvent::Reconnecting {
        reason: HelperReconnectReason::ConnectionLost,
        delay_secs: Some(2),
    };
    assert_eq!(
        "{\"type\":\"Reconnecting\",\"reason\":\"connection_lost\",\"delay_secs\":2}\n",
        encode_event_line(&event).expect("event encodes")
    );

    let manual = HelperEvent::Reconnecting {
        reason: HelperReconnectReason::Manual,
        delay_secs: None,
    };
    assert_eq!(
        "{\"type\":\"Reconnecting\",\"reason\":\"manual\",\"delay_secs\":null}\n",
        encode_event_line(&manual).expect("event encodes")
    );
}

#[test]
fn helper_clipboard_text_event_decodes() {
    let event =
        decode_event_line(r#"{"type":"ClipboardText","text":"remote 中文"}"#).expect("decodes");
    assert_eq!(
        HelperEvent::ClipboardText {
            text: "remote 中文".to_string()
        },
        event
    );
}

#[test]
fn legacy_clipboard_files_request_defaults_transfer_id_to_zero() {
    let request =
        decode_request_line(r#"{"type":"ClipboardFiles","paths":["C:\\tmp\\report.txt"]}"#)
            .expect("legacy clipboard request decodes");

    assert_eq!(
        HelperRequest::ClipboardFiles {
            transfer_id: 0,
            paths: vec!["C:\\tmp\\report.txt".to_string()],
        },
        request
    );
}

#[test]
fn clipboard_transfer_events_and_cancel_request_round_trip() {
    let ready = HelperEvent::ClipboardFilesReady {
        transfer_id: (1_u64 << 63) | 7,
        paths: vec!["C:\\Temp\\navop-rdp-clipboard\\transfer-7\\report.txt".to_string()],
    };
    assert_eq!(
        ready,
        decode_event_line(&encode_event_line(&ready).expect("ready event encodes"))
            .expect("ready event decodes")
    );

    let failed = HelperEvent::ClipboardTransferFailed {
        transfer_id: (1_u64 << 63) | 8,
        message: "remote clipboard transfer failed".to_string(),
    };
    assert_eq!(
        failed,
        decode_event_line(&encode_event_line(&failed).expect("failed event encodes"))
            .expect("failed event decodes")
    );

    let cancel = HelperRequest::CancelClipboardTransfer { transfer_id: 9 };
    assert_eq!(
        cancel,
        decode_request_line(&encode_request_line(&cancel).expect("cancel request encodes"))
            .expect("cancel request decodes")
    );
}
