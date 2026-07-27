use super::should_apply_remote_cursor_output;
use remote_desktop::RemoteDesktopProtocol;

#[test]
fn rdp_applies_server_cursor_outputs() {
    assert!(should_apply_remote_cursor_output(
        RemoteDesktopProtocol::Rdp
    ));
}

#[test]
fn vnc_keeps_the_native_cursor_and_ignores_server_cursor_outputs() {
    assert!(!should_apply_remote_cursor_output(
        RemoteDesktopProtocol::Vnc
    ));
}
