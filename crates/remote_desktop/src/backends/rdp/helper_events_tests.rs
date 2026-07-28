use crate::helper_protocol::{HelperEvent, HelperReconnectReason};

use super::*;

#[test]
fn converts_helper_connected_event_to_protocol_capabilities() {
    for (protocol, capabilities) in [
        (
            RemoteDesktopProtocol::Rdp,
            RemoteDesktopCapabilities::rdp_mvp(),
        ),
        (
            RemoteDesktopProtocol::Vnc,
            RemoteDesktopCapabilities::vnc_mvp(),
        ),
    ] {
        let output = helper_event_to_output(
            HelperEvent::Connected {
                width: 1280,
                height: 720,
            },
            protocol,
        )
        .expect("event converts");

        assert_eq!(
            Some(RemoteDesktopOutput::Connected {
                width: 1280,
                height: 720,
                capabilities,
            }),
            output
        );
    }
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
        Some(RemoteDesktopOutput::ClipboardText {
            text: "remote 中文".to_string()
        }),
        output
    );
}

#[test]
fn helper_events_identify_disconnect_signals() {
    assert_eq!(
        Some(HelperDisconnect {
            kind: Some(HelperDisconnectKind::ConnectionFailure),
            reason: "network".to_string(),
        }),
        helper_disconnect(&HelperEvent::ConnectionFailure {
            message: "network".to_string(),
        })
    );
    assert_eq!(
        Some(HelperDisconnect {
            kind: Some(HelperDisconnectKind::Terminated),
            reason: "closed".to_string(),
        }),
        helper_disconnect(&HelperEvent::Terminated {
            message: "closed".to_string(),
        })
    );
    assert_eq!(
        None,
        helper_disconnect(&HelperEvent::Connected {
            width: 1,
            height: 1
        })
    );
}

#[test]
fn converts_helper_reconnect_reasons_to_structured_outputs() {
    let cases = [
        (
            HelperReconnectReason::DisplayUpdate,
            RemoteDesktopReconnectReason::DisplayUpdate,
            Some(1),
        ),
        (
            HelperReconnectReason::SessionError,
            RemoteDesktopReconnectReason::SessionError,
            Some(2),
        ),
        (
            HelperReconnectReason::ConnectionLost,
            RemoteDesktopReconnectReason::ConnectionLost,
            Some(5),
        ),
        (
            HelperReconnectReason::Manual,
            RemoteDesktopReconnectReason::Manual,
            None,
        ),
    ];

    for (helper_reason, reason, delay_secs) in cases {
        let event = HelperEvent::Reconnecting {
            reason: helper_reason,
            delay_secs,
        };

        assert_eq!(None, helper_disconnect(&event));
        assert_eq!(
            Some(RemoteDesktopOutput::Reconnecting(RemoteDesktopReconnect {
                reason,
                delay_secs
            })),
            helper_event_to_output(event, RemoteDesktopProtocol::Rdp).expect("event converts")
        );
    }
}

#[test]
fn terminal_helper_events_do_not_create_user_visible_outputs() {
    for event in [
        HelperEvent::ConnectionFailure {
            message: "[CredSSP @ /Users/runner/connector.rs:107] CredSSP".to_string(),
        },
        HelperEvent::Terminated {
            message: "Another user connected to the server".to_string(),
        },
    ] {
        assert_eq!(
            None,
            helper_event_to_output(event, RemoteDesktopProtocol::Rdp).expect("event converts")
        );
    }
}

#[test]
fn helper_status_diagnostics_do_not_create_user_visible_outputs() {
    let event = HelperEvent::Status {
        message:
            "[CredSSP @ /Users/runner/.cargo/git/checkouts/ironrdp/src/connector.rs:107] CredSSP"
                .to_string(),
    };

    assert_eq!(
        None,
        helper_event_to_output(event, RemoteDesktopProtocol::Rdp).expect("event converts")
    );
}
