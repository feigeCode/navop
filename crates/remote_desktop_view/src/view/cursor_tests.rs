use super::*;

fn cursor(red: u8) -> RemoteDesktopCursor {
    RemoteDesktopCursor {
        width: 2,
        height: 1,
        hotspot_x: 1,
        hotspot_y: 0,
        rgba: vec![red, 0, 0, 0xff, red, 0, 0, 0xff],
    }
}

#[test]
fn installs_and_promotes_bitmap_cursor_with_remote_geometry() {
    let mut state = RemoteCursorState::default();

    state.install(cursor(1)).unwrap();
    state.set_position(25, 40);
    assert_eq!(RemoteCursorMode::Bitmap, state.mode);
    assert!(state.promote_latest().is_none());

    let paint = state.paint_state(Some((100, 80))).unwrap();
    assert_eq!(
        RemoteCursorGeometry {
            remote_width: 100,
            remote_height: 80,
            x: 25,
            y: 40,
            width: 2,
            height: 1,
            hotspot_x: 1,
            hotspot_y: 0,
        },
        paint.geometry
    );
}

#[test]
fn default_and_hidden_retire_bitmap_without_immediate_drop() {
    let mut state = RemoteCursorState::default();
    state.install(cursor(1)).unwrap();
    state.promote_latest();

    state.hide();

    assert_eq!(RemoteCursorMode::Hidden, state.mode);
    assert!(state.paint_state(Some((100, 80))).is_none());
    assert_eq!(1, state.pending_drops.len());

    state.show_default();
    assert_eq!(RemoteCursorMode::Default, state.mode);
    assert_eq!(1, state.pending_drops.len());
}

#[test]
fn session_reset_clears_position_and_all_cursor_generations() {
    let mut state = RemoteCursorState::default();
    state.install(cursor(1)).unwrap();
    state.set_position(25, 40);
    state.promote_latest();
    state.install(cursor(2)).unwrap();

    state.reset_session();

    assert_eq!(RemoteCursorMode::Default, state.mode);
    assert_eq!(None, state.position);
    assert!(state.latest.is_none());
    assert!(state.rendered.current().is_none());
    assert_eq!(2, state.pending_drops.len());
}

#[test]
fn third_cursor_generation_is_the_first_safe_immediate_retirement() {
    let mut state = RemoteCursorState::default();
    state.install(cursor(1)).unwrap();
    assert!(state.promote_latest().is_none());
    state.install(cursor(2)).unwrap();
    assert!(state.promote_latest().is_none());
    state.install(cursor(3)).unwrap();

    let retired = state.promote_latest().expect("first cursor generation");

    assert_eq!(retired.as_bytes(0).unwrap()[2], 1);
}

#[test]
fn native_cursor_hides_only_when_the_remote_cursor_can_replace_it() {
    assert!(!should_hide_native_cursor(
        RemoteCursorMode::Default,
        true,
        true
    ));
    assert!(!should_hide_native_cursor(
        RemoteCursorMode::Bitmap,
        false,
        true
    ));
    assert!(!should_hide_native_cursor(
        RemoteCursorMode::Bitmap,
        true,
        false
    ));
    assert!(should_hide_native_cursor(
        RemoteCursorMode::Bitmap,
        true,
        true
    ));
    assert!(should_hide_native_cursor(
        RemoteCursorMode::Hidden,
        true,
        false
    ));
}

#[test]
fn bitmap_cursor_is_not_paintable_until_position_and_image_are_ready() {
    let mut state = RemoteCursorState::new(true);
    state.set_pointer_hovered(true);
    state.install(cursor(1)).unwrap();

    assert!(!state.has_paintable_bitmap());

    state.set_position(25, 40);
    assert!(!state.has_paintable_bitmap());

    state.promote_latest();
    assert!(state.has_paintable_bitmap());
}

#[test]
fn repeated_pointer_coordinates_do_not_report_a_position_change() {
    let mut state = RemoteCursorState::default();

    assert!(state.set_position(25, 40));
    assert!(!state.set_position(25, 40));
    assert!(state.set_position(26, 40));
}

#[test]
fn native_cursor_management_is_opt_in_for_rdp_only() {
    assert!(!RemoteCursorState::default().manage_native_cursor);
    assert!(RemoteCursorState::new(true).manage_native_cursor);
}
