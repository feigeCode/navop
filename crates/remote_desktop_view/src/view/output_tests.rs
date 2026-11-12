use super::{
    should_apply_remote_cursor_output, should_commit_prepared_frame, should_fallback_to_reconnect,
};
use crate::view::frame_sync::FrameSyncPhase;
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

#[test]
fn full_frame_recovery_falls_back_to_reconnect_only_after_the_deadline() {
    assert!(should_fallback_to_reconnect(
        7,
        7,
        FrameSyncPhase::Recovering
    ));
    assert!(should_fallback_to_reconnect(
        7,
        7,
        FrameSyncPhase::AwaitingBase
    ));
    assert!(!should_fallback_to_reconnect(7, 7, FrameSyncPhase::Ready));
    assert!(!should_fallback_to_reconnect(
        6,
        7,
        FrameSyncPhase::Recovering
    ));
}
