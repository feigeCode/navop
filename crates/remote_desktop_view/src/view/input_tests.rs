use super::should_track_local_cursor_position;
use remote_desktop::RemoteDesktopProtocol;

#[test]
fn tracks_local_cursor_position_for_connected_writable_vnc() {
    assert!(should_track_local_cursor_position(
        RemoteDesktopProtocol::Vnc,
        true,
        false,
    ));
}

#[test]
fn does_not_track_local_cursor_position_for_read_only_vnc() {
    assert!(!should_track_local_cursor_position(
        RemoteDesktopProtocol::Vnc,
        true,
        true,
    ));
}

#[test]
fn does_not_track_local_cursor_position_for_disconnected_vnc() {
    assert!(!should_track_local_cursor_position(
        RemoteDesktopProtocol::Vnc,
        false,
        false,
    ));
}

#[test]
fn does_not_override_rdp_server_cursor_position() {
    assert!(!should_track_local_cursor_position(
        RemoteDesktopProtocol::Rdp,
        true,
        false,
    ));
}
