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
fn prepared_frame_from_the_current_generation_commits_even_when_it_was_superseded() {
    // The frame tracker has to stay in step with the worker's framebuffer: a base that
    // a newer frame superseded was still applied there, and skipping its accounting
    // would leave the tracker without the base the framebuffer already holds.
    assert!(should_commit_prepared_frame(7, 7));
}

#[test]
fn prepared_frame_from_an_old_generation_is_not_committed() {
    assert!(!should_commit_prepared_frame(6, 7));
}
