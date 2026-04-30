use gpui::{Pixels, Point, px};

use crate::{
    Viewport,
    canvas::{Command, CommandContext},
    plugin::PluginContext,
};

/// Same zoom limits as [`crate::plugins::ViewportPlugin`] wheel zoom.
pub(crate) const ZOOM_MIN: f32 = 0.7;
pub(crate) const ZOOM_MAX: f32 = 3.0;
/// Inset from window edges (ratio per side).
pub(crate) const MARGIN_RATIO: f32 = 0.08;

fn frame_params(
    win_w: f32,
    win_h: f32,
    bx: f32,
    by: f32,
    bw: f32,
    bh: f32,
) -> Option<(f32, Point<Pixels>)> {
    if win_w <= 0.0 || win_h <= 0.0 {
        return None;
    }

    let bw_safe = bw.max(1.0);
    let bh_safe = bh.max(1.0);

    let avail_w = win_w * (1.0 - 2.0 * MARGIN_RATIO);
    let avail_h = win_h * (1.0 - 2.0 * MARGIN_RATIO);
    let z = (avail_w / bw_safe)
        .min(avail_h / bh_safe)
        .clamp(ZOOM_MIN, ZOOM_MAX);

    let cx = bx + bw / 2.0;
    let cy = by + bh / 2.0;
    let center_x = win_w / 2.0;
    let center_y = win_h / 2.0;
    let new_offset = Point::new(px(center_x - cx * z), px(center_y - cy * z));

    Some((z, new_offset))
}

pub(crate) struct ViewportFrameCommand {
    pub from_zoom: f32,
    pub from_offset: Point<Pixels>,
    pub to_zoom: f32,
    pub to_offset: Point<Pixels>,
}

impl Command for ViewportFrameCommand {
    fn name(&self) -> &'static str {
        "viewport_frame"
    }

    fn execute(&mut self, ctx: &mut CommandContext) {
        ctx.viewport.zoom = self.to_zoom;
        ctx.viewport.offset.x = self.to_offset.x;
        ctx.viewport.offset.y = self.to_offset.y;
    }

    fn undo(&mut self, ctx: &mut CommandContext) {
        ctx.viewport.zoom = self.from_zoom;
        ctx.viewport.offset.x = self.from_offset.x;
        ctx.viewport.offset.y = self.from_offset.y;
    }

    fn to_ops(&self, ctx: &mut CommandContext) -> Vec<crate::GraphOp> {
        ctx.viewport.zoom = self.to_zoom;
        ctx.viewport.offset.x = self.to_offset.x;
        ctx.viewport.offset.y = self.to_offset.y;
        vec![]
    }
}

/// Pan + zoom so the given world-space axis-aligned box (position + size) fits the window.
pub(crate) fn frame_world_rect(ctx: &mut PluginContext, bx: f32, by: f32, bw: f32, bh: f32) {
    let Some(wb) = ctx.viewport.window_bounds else {
        return;
    };

    let win_w: f32 = wb.size.width.into();
    let win_h: f32 = wb.size.height.into();
    let Some((z, new_offset)) = frame_params(win_w, win_h, bx, by, bw, bh) else {
        return;
    };

    let from_zoom = ctx.viewport.zoom;
    let from_offset = ctx.viewport.offset;
    let zoom_changed = (from_zoom - z).abs() > 1e-4;
    let ox: f32 = from_offset.x.into();
    let oy: f32 = from_offset.y.into();
    let nx: f32 = new_offset.x.into();
    let ny: f32 = new_offset.y.into();
    let offset_changed = (ox - nx).abs() > 0.5 || (oy - ny).abs() > 0.5;
    if !zoom_changed && !offset_changed {
        return;
    }

    ctx.execute_command(ViewportFrameCommand {
        from_zoom,
        from_offset,
        to_zoom: z,
        to_offset: new_offset,
    });
}

/// Same geometry as [`frame_world_rect`], but writes [`Viewport`] directly (no undo stack).
/// For [`crate::plugin::InitPluginContext`] / plugin [`Plugin::setup`].
pub(crate) fn apply_frame_world_rect_direct(
    viewport: &mut Viewport,
    win_w: f32,
    win_h: f32,
    bx: f32,
    by: f32,
    bw: f32,
    bh: f32,
) {
    let Some((z, new_offset)) = frame_params(win_w, win_h, bx, by, bw, bh) else {
        return;
    };

    let zoom_changed = (viewport.zoom - z).abs() > 1e-4;
    let ox: f32 = viewport.offset.x.into();
    let oy: f32 = viewport.offset.y.into();
    let nx: f32 = new_offset.x.into();
    let ny: f32 = new_offset.y.into();
    let offset_changed = (ox - nx).abs() > 0.5 || (oy - ny).abs() > 0.5;
    if !zoom_changed && !offset_changed {
        return;
    }

    viewport.zoom = z;
    viewport.offset = new_offset;
}
