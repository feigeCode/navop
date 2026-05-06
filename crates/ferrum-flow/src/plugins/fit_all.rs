use gpui::{Bounds, Point, px};

use crate::{
    Node,
    plugin::{FlowEvent, InitPluginContext, Plugin, PluginContext, primary_platform_modifier},
    plugins::viewport_frame::{apply_frame_world_rect_direct, frame_world_rect},
};

/// Zoom and pan so **all** nodes fit in the window (⌘0 / Ctrl+0). Undo restores the previous view.
///
/// On [`Plugin::setup`], fits once using [`InitPluginContext::drawable_size`] (does not push an undo
/// entry; the initial view is not recorded as a command).
pub struct FitAllGraphPlugin;

impl FitAllGraphPlugin {
    pub fn new() -> Self {
        Self
    }
}

impl Default for FitAllGraphPlugin {
    fn default() -> Self {
        Self::new()
    }
}

fn graph_world_bounds_graph<'a>(
    nodes: impl Iterator<Item = &'a Node> + 'a,
) -> Option<(f32, f32, f32, f32)> {
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    let mut any = false;

    for n in nodes {
        let (nx, ny) = n.position();
        let size = *n.size_ref();
        let x: f32 = nx.into();
        let y: f32 = ny.into();
        let w: f32 = size.width.into();
        let h: f32 = size.height.into();
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x + w);
        max_y = max_y.max(y + h);
        any = true;
    }

    if !any {
        return None;
    }

    Some((
        min_x,
        min_y,
        (max_x - min_x).max(1.0),
        (max_y - min_y).max(1.0),
    ))
}

fn graph_world_bounds(ctx: &PluginContext) -> Option<(f32, f32, f32, f32)> {
    graph_world_bounds_graph(ctx.nodes().values())
}

fn fit_all(ctx: &mut PluginContext) {
    let Some((bx, by, bw, bh)) = graph_world_bounds(ctx) else {
        return;
    };
    frame_world_rect(ctx, bx, by, bw, bh);
}

pub(crate) fn fit_entire_graph(ctx: &mut PluginContext) {
    fit_all(ctx);
}

impl Plugin for FitAllGraphPlugin {
    fn name(&self) -> &'static str {
        "fit_all_graph"
    }

    fn setup(&mut self, ctx: &mut InitPluginContext) {
        let Some((bx, by, bw, bh)) = graph_world_bounds_graph(ctx.nodes().values()) else {
            return;
        };
        let win_w: f32 = ctx.drawable_size.width.into();
        let win_h: f32 = ctx.drawable_size.height.into();
        apply_frame_world_rect_direct(ctx, win_w, win_h, bx, by, bw, bh);
        ctx.set_window_bounds(Some(Bounds::new(
            Point::new(px(0.0), px(0.0)),
            ctx.drawable_size,
        )));
    }

    fn priority(&self) -> i32 {
        88
    }

    fn on_event(
        &mut self,
        event: &FlowEvent,
        ctx: &mut PluginContext,
    ) -> crate::plugin::EventResult {
        if let FlowEvent::Input(crate::plugin::InputEvent::KeyDown(ev)) = event
            && primary_platform_modifier(ev)
            && !ev.keystroke.modifiers.shift
            && ev.keystroke.key == "0"
        {
            fit_all(ctx);
            return crate::plugin::EventResult::Stop;
        }
        crate::plugin::EventResult::Continue
    }
}
