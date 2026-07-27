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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemoteCursorGeometry {
    pub remote_width: u16,
    pub remote_height: u16,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub hotspot_x: u16,
    pub hotspot_y: u16,
}

pub fn scale_filled_remote_cursor_bounds(
    bounds: LocalBounds,
    cursor: RemoteCursorGeometry,
) -> Option<LocalBounds> {
    if !valid_cursor_geometry(bounds, cursor) {
        return None;
    }
    let scale_x = bounds.width / f32::from(cursor.remote_width);
    let scale_y = bounds.height / f32::from(cursor.remote_height);
    let hotspot_x = bounds.left + f32::from(cursor.x) * scale_x;
    let hotspot_y = bounds.top + f32::from(cursor.y) * scale_y;

    Some(LocalBounds {
        left: hotspot_x - f32::from(cursor.hotspot_x) * scale_x,
        top: hotspot_y - f32::from(cursor.hotspot_y) * scale_y,
        width: f32::from(cursor.width) * scale_x,
        height: f32::from(cursor.height) * scale_y,
    })
}

fn valid_cursor_geometry(bounds: LocalBounds, cursor: RemoteCursorGeometry) -> bool {
    bounds.left.is_finite()
        && bounds.top.is_finite()
        && bounds.width.is_finite()
        && bounds.height.is_finite()
        && bounds.width > 0.0
        && bounds.height > 0.0
        && cursor.remote_width > 0
        && cursor.remote_height > 0
        && cursor.width > 0
        && cursor.height > 0
        && cursor.hotspot_x < cursor.width
        && cursor.hotspot_y < cursor.height
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
#[path = "pointer_tests.rs"]
mod tests;
