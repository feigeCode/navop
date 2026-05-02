use gpui::{Bounds, Pixels, Point, Size, Window, px};

use crate::{Node, PortPosition};

/// Fingerprint of [`Viewport`] fields that affect [`Viewport::is_node_visible`].
/// Used by [`crate::NodePlugin`] to avoid rescanning the full node list every frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ViewportVisibilityCacheKey {
    pub zoom: f32,
    pub offset_x: f32,
    pub offset_y: f32,
    pub has_window: bool,
    pub window_w: f32,
    pub window_h: f32,
}

#[derive(Debug, Clone)]
pub struct Viewport {
    zoom: f32,
    offset: Point<Pixels>,
    window_bounds: Option<Bounds<Pixels>>,
}

impl Viewport {
    pub(crate) fn new() -> Self {
        Self {
            zoom: 1.0,
            offset: Point::new(px(0.0), px(0.0)),
            window_bounds: None,
        }
    }

    /// Sets [`Self::window_bounds`] to the window’s drawable area (`Window::viewport_size`),
    /// origin `(0, 0)`. Skips assignment when width/height are unchanged.
    ///
    /// Prefer this over `Window::bounds()` for hit-testing and overlay layout: the latter is in
    /// global space and can be larger than the content viewport.
    pub fn sync_drawable_bounds(&mut self, window: &Window) {
        let vs = window.viewport_size();
        let unchanged = self
            .window_bounds
            .is_some_and(|b| b.size.width == vs.width && b.size.height == vs.height);
        if !unchanged {
            self.window_bounds = Some(Bounds::new(
                Point::new(px(0.0), px(0.0)),
                Size::new(vs.width, vs.height),
            ));
        }
    }

    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    pub fn set_zoom(&mut self, zoom: f32) {
        self.zoom = zoom;
    }

    /// Compute a new zoom value by multiplying current zoom with `factor`.
    pub fn zoom_scaled_by(&self, factor: f32) -> f32 {
        self.zoom * factor
    }

    pub fn offset(&self) -> Point<Pixels> {
        self.offset
    }

    pub fn set_offset(&mut self, offset: Point<Pixels>) {
        self.offset = offset;
    }

    pub fn set_offset_xy(&mut self, x: Pixels, y: Pixels) {
        self.offset = Point::new(x, y);
    }

    pub fn translate_offset(&mut self, dx: Pixels, dy: Pixels) {
        self.offset.x += dx;
        self.offset.y += dy;
    }

    pub fn window_bounds(&self) -> Option<Bounds<Pixels>> {
        self.window_bounds
    }

    pub fn set_window_bounds(&mut self, bounds: Option<Bounds<Pixels>>) {
        self.window_bounds = bounds;
    }

    /// Convert a world-space scalar length to screen-space scalar length.
    pub fn world_scalar_to_screen(&self, value: f32) -> f32 {
        value * self.zoom
    }

    /// Convert a screen-space scalar length to world-space scalar length.
    pub fn screen_scalar_to_world(&self, value: f32) -> f32 {
        value / self.zoom
    }

    /// Convert a world-space pixel length to screen-space pixel length.
    pub fn world_length_to_screen(&self, value: Pixels) -> Pixels {
        value * self.zoom
    }

    /// Convert a screen-space pixel length to world-space pixel length.
    pub fn screen_length_to_world(&self, value: Pixels) -> Pixels {
        value / self.zoom
    }

    pub fn world_to_screen(&self, p: Point<Pixels>) -> Point<Pixels> {
        Point::new(
            self.world_length_to_screen(p.x) + self.offset.x,
            self.world_length_to_screen(p.y) + self.offset.y,
        )
    }

    pub fn screen_to_world(&self, p: Point<Pixels>) -> Point<Pixels> {
        Point::new(
            self.screen_length_to_world(p.x - self.offset.x),
            self.screen_length_to_world(p.y - self.offset.y),
        )
    }

    /// Bezier control point for an edge tangent at a port direction.
    pub fn edge_control_point(
        &self,
        source: Point<Pixels>,
        position: PortPosition,
    ) -> Point<Pixels> {
        match position {
            PortPosition::Top => {
                source - Point::new(px(0.0), px(self.world_scalar_to_screen(50.0)))
            }
            PortPosition::Left => {
                source - Point::new(px(self.world_scalar_to_screen(50.0)), px(0.0))
            }
            PortPosition::Right => {
                source + Point::new(px(self.world_scalar_to_screen(50.0)), px(0.0))
            }
            PortPosition::Bottom => {
                source + Point::new(px(0.0), px(self.world_scalar_to_screen(50.0)))
            }
        }
    }

    pub fn is_node_visible(&self, node: &Node) -> bool {
        let Some(window_bounds) = self.window_bounds else {
            return false;
        };

        let screen = self.world_to_screen(node.point());
        let size = *node.size_ref();

        screen.x + self.world_length_to_screen(size.width) > px(0.0)
            && screen.x < window_bounds.size.width
            && screen.y + self.world_length_to_screen(size.height) > px(0.0)
            && screen.y < window_bounds.size.height
    }

    pub(crate) fn visibility_cache_key(&self) -> ViewportVisibilityCacheKey {
        match self.window_bounds {
            Some(b) => ViewportVisibilityCacheKey {
                zoom: self.zoom,
                offset_x: self.offset.x.into(),
                offset_y: self.offset.y.into(),
                has_window: true,
                window_w: b.size.width.into(),
                window_h: b.size.height.into(),
            },
            None => ViewportVisibilityCacheKey {
                zoom: self.zoom,
                offset_x: self.offset.x.into(),
                offset_y: self.offset.y.into(),
                has_window: false,
                window_w: 0.0,
                window_h: 0.0,
            },
        }
    }
}
