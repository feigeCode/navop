use super::*;

#[test]
fn scales_and_clamps_coordinate() {
    assert_eq!(500, scale_coordinate(50.0, 100.0, 1000));
    assert_eq!(0, scale_coordinate(-10.0, 100.0, 1000));
    assert_eq!(999, scale_coordinate(120.0, 100.0, 1000));
}

#[test]
fn scales_pointer_inside_centered_letterboxed_frame() {
    assert_eq!(
        Some((640, 360)),
        scale_pointer_position(640.0, 360.0, 1280.0, 720.0, 1280, 720)
    );
    assert_eq!(
        Some((0, 0)),
        scale_pointer_position(280.0, 0.0, 1280.0, 720.0, 720, 720)
    );
    assert_eq!(
        None,
        scale_pointer_position(279.0, 0.0, 1280.0, 720.0, 720, 720)
    );
}

#[test]
fn subtracts_content_bounds_before_scaling_window_position() {
    assert_eq!(
        Some((640, 360)),
        scale_window_pointer_position(
            640.0,
            456.0,
            LocalBounds {
                left: 0.0,
                top: 96.0,
                width: 1280.0,
                height: 720.0,
            },
            1280,
            720,
        )
    );
}

#[test]
fn scales_filled_pointer_against_full_content_bounds() {
    assert_eq!(
        Some((0, 0)),
        scale_filled_pointer_position(0.0, 0.0, 1280.0, 720.0, 1024, 768)
    );
    assert_eq!(
        Some((512, 384)),
        scale_filled_pointer_position(640.0, 360.0, 1280.0, 720.0, 1024, 768)
    );
}

#[test]
fn filled_window_position_subtracts_header_bounds() {
    assert_eq!(
        Some((512, 384)),
        scale_filled_window_pointer_position(
            640.0,
            456.0,
            LocalBounds {
                left: 0.0,
                top: 96.0,
                width: 1280.0,
                height: 720.0,
            },
            1024,
            768,
        )
    );
}

#[test]
fn scales_remote_cursor_with_filled_non_uniform_geometry_and_hotspot() {
    let bounds = LocalBounds {
        left: 10.0,
        top: 20.0,
        width: 200.0,
        height: 50.0,
    };
    let cursor = RemoteCursorGeometry {
        remote_width: 100,
        remote_height: 100,
        x: 25,
        y: 40,
        width: 10,
        height: 20,
        hotspot_x: 2,
        hotspot_y: 5,
    };

    assert_eq!(
        Some(LocalBounds {
            left: 56.0,
            top: 37.5,
            width: 20.0,
            height: 10.0,
        }),
        scale_filled_remote_cursor_bounds(bounds, cursor)
    );
}

#[test]
fn remote_cursor_bounds_keep_partial_overflow_for_canvas_clipping() {
    let bounds = LocalBounds {
        left: 100.0,
        top: 50.0,
        width: 200.0,
        height: 100.0,
    };
    let cursor = RemoteCursorGeometry {
        remote_width: 100,
        remote_height: 100,
        x: 0,
        y: 0,
        width: 16,
        height: 16,
        hotspot_x: 8,
        hotspot_y: 8,
    };

    assert_eq!(
        Some(LocalBounds {
            left: 84.0,
            top: 42.0,
            width: 32.0,
            height: 16.0,
        }),
        scale_filled_remote_cursor_bounds(bounds, cursor)
    );
}

#[test]
fn remote_cursor_bounds_require_valid_frame_and_cursor_dimensions() {
    let bounds = LocalBounds {
        left: 0.0,
        top: 0.0,
        width: 100.0,
        height: 100.0,
    };
    let cursor = RemoteCursorGeometry {
        remote_width: 0,
        remote_height: 100,
        x: 0,
        y: 0,
        width: 1,
        height: 1,
        hotspot_x: 0,
        hotspot_y: 0,
    };

    assert_eq!(None, scale_filled_remote_cursor_bounds(bounds, cursor));
    assert_eq!(
        None,
        scale_filled_remote_cursor_bounds(
            bounds,
            RemoteCursorGeometry {
                remote_width: 100,
                width: 0,
                ..cursor
            }
        )
    );
}

#[test]
fn remote_cursor_bounds_reject_invalid_hotspots_and_non_finite_bounds() {
    let cursor = RemoteCursorGeometry {
        remote_width: 100,
        remote_height: 100,
        x: 50,
        y: 50,
        width: 2,
        height: 2,
        hotspot_x: 2,
        hotspot_y: 0,
    };
    let bounds = LocalBounds {
        left: 0.0,
        top: 0.0,
        width: 100.0,
        height: 100.0,
    };

    assert_eq!(None, scale_filled_remote_cursor_bounds(bounds, cursor));
    assert_eq!(
        None,
        scale_filled_remote_cursor_bounds(
            LocalBounds {
                width: f32::NAN,
                ..bounds
            },
            RemoteCursorGeometry {
                hotspot_x: 0,
                ..cursor
            }
        )
    );
}
