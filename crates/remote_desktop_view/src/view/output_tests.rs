use super::{should_apply_remote_cursor_output, should_commit_prepared_frame};
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

#[test]
fn prepared_frame_from_current_generation_and_ticket_commits() {
    assert!(should_commit_prepared_frame(7, 7, 11, 11));
}

#[test]
fn prepared_frame_superseded_after_background_processing_is_not_committed() {
    assert!(!should_commit_prepared_frame(7, 7, 11, 12));
}

#[test]
fn prepared_frame_from_an_old_generation_is_not_committed() {
    assert!(!should_commit_prepared_frame(6, 7, 12, 12));
}
