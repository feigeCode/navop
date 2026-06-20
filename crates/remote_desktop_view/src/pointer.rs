pub fn scale_coordinate(local: f32, local_size: f32, remote_size: u16) -> u16 {
    if local_size <= 0.0 || remote_size == 0 {
        return 0;
    }

    let scaled = (local / local_size) * remote_size as f32;
    scaled.clamp(0.0, remote_size.saturating_sub(1) as f32) as u16
}

pub fn scale_pointer_position(
    local_x: f32,
    local_y: f32,
    local_width: f32,
    local_height: f32,
    remote_width: u16,
    remote_height: u16,
) -> Option<(u16, u16)> {
    if local_width <= 0.0 || local_height <= 0.0 || remote_width == 0 || remote_height == 0 {
        return None;
    }

    let scale =
        (local_width / f32::from(remote_width)).min(local_height / f32::from(remote_height));
    let frame_width = f32::from(remote_width) * scale;
    let frame_height = f32::from(remote_height) * scale;
    let offset_x = (local_width - frame_width) / 2.0;
    let offset_y = (local_height - frame_height) / 2.0;
    let frame_x = local_x - offset_x;
    let frame_y = local_y - offset_y;

    if frame_x < 0.0 || frame_y < 0.0 || frame_x > frame_width || frame_y > frame_height {
        return None;
    }

    Some((
        scale_coordinate(frame_x, frame_width, remote_width),
        scale_coordinate(frame_y, frame_height, remote_height),
    ))
}

pub fn scale_filled_pointer_position(
    local_x: f32,
    local_y: f32,
    local_width: f32,
    local_height: f32,
    remote_width: u16,
    remote_height: u16,
) -> Option<(u16, u16)> {
    if local_width <= 0.0 || local_height <= 0.0 || remote_width == 0 || remote_height == 0 {
        return None;
    }

    Some((
        scale_coordinate(local_x, local_width, remote_width),
        scale_coordinate(local_y, local_height, remote_height),
    ))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalBounds {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}

pub fn scale_window_pointer_position(
    window_x: f32,
    window_y: f32,
    bounds: LocalBounds,
    remote_width: u16,
    remote_height: u16,
) -> Option<(u16, u16)> {
    scale_pointer_position(
        window_x - bounds.left,
        window_y - bounds.top,
        bounds.width,
        bounds.height,
        remote_width,
        remote_height,
    )
}

pub fn scale_filled_window_pointer_position(
    window_x: f32,
    window_y: f32,
    bounds: LocalBounds,
    remote_width: u16,
    remote_height: u16,
) -> Option<(u16, u16)> {
    scale_filled_pointer_position(
        window_x - bounds.left,
        window_y - bounds.top,
        bounds.width,
        bounds.height,
        remote_width,
        remote_height,
    )
}

#[cfg(test)]
mod tests {
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
}
