use remote_desktop::{RemoteDesktopProtocol, RemoteDesktopProviderVersionError};

use super::{
    SessionResetReason, close_runtime_once, preserve_presented_frame_during_session_reset,
    remote_desktop_error_message, remote_desktop_tab_title,
};

#[test]
fn closes_runtime_only_once() {
    let (input_tx, mut input_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut input_tx = Some(input_tx);

    close_runtime_once(&mut input_tx);
    close_runtime_once(&mut input_tx);

    assert_eq!(
        Some(remote_desktop::RemoteDesktopInput::Close),
        input_rx.blocking_recv()
    );
    assert!(input_rx.try_recv().is_err());
}

#[test]
fn tab_title_uses_connection_name_and_duplicate_index() {
    assert_eq!("prod-rdp", remote_desktop_tab_title("prod-rdp", None));
    assert_eq!("prod-rdp(2)", remote_desktop_tab_title("prod-rdp", Some(2)));
}

#[test]
fn provider_version_error_message_is_localized_after_context() {
    let error = anyhow::Error::new(RemoteDesktopProviderVersionError {
        protocol: RemoteDesktopProtocol::Vnc,
        installed: "0.1.0".to_string(),
        required: "0.1.1".to_string(),
        invalid: false,
    })
    .context("VNC remote desktop provider");

    assert_eq!(
        "VNC provider version 0.1.0 is too old. Please update the provider to 0.1.1 or newer.",
        remote_desktop_error_message(&error)
    );
}

#[test]
fn only_transient_reconnect_preserves_the_presented_frame() {
    assert!(preserve_presented_frame_during_session_reset(
        SessionResetReason::Reconnecting
    ));
    assert!(!preserve_presented_frame_during_session_reset(
        SessionResetReason::ConnectionFailure
    ));
    assert!(!preserve_presented_frame_during_session_reset(
        SessionResetReason::Terminated
    ));
}
