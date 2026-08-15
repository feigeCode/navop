use crate::{
    plugin::{FlowEvent, InitPluginContext, Plugin, PluginContext, primary_platform_modifier},
    plugins::viewport_frame::{apply_frame_world_rect_to_viewport, frame_world_rect},
};

/// Marker stored in [`crate::SharedState`] by [`FitAllGraphPlugin::setup`]. Cleared on the first
/// [`FlowEvent::DrawableBoundsReady`] after [`crate::Viewport::window_bounds`] is set from GPUI layout
/// (`on_children_prepainted`), then the graph is fitted using that **measured** size.
#[derive(Debug)]
pub struct PendingInitialFitAll;

/// Zoom and pan so **all** nodes fit in the drawable (⌘0 / Ctrl+0). Undo restores the previous view.
///
/// Initial fit runs **after** GPUI has laid out the canvas: [`FitAllGraphPlugin::setup`] only registers
/// [`PendingInitialFitAll`]; the first [`FlowEvent::DrawableBoundsReady`] applies the same geometry as ⌘0
/// using [`Viewport::window_bounds`].
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

fn graph_world_bounds(ctx: &PluginContext) -> Option<(f32, f32, f32, f32)> {
    ctx.graph.nodes_world_aabb()
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
        let _ = ctx.shared_state.insert(PendingInitialFitAll);
    }

    fn priority(&self) -> i32 {
        88
    }

    fn on_event(
        &mut self,
        event: &FlowEvent,
        ctx: &mut PluginContext,
    ) -> crate::plugin::EventResult {
        match event {
            FlowEvent::DrawableBoundsReady => {
                if try_apply_pending_initial_fit_all_ctx(ctx) {
                    ctx.notify();
                }
                crate::plugin::EventResult::Continue
            }
            FlowEvent::Input(crate::plugin::InputEvent::KeyDown(ev))
                if primary_platform_modifier(ev)
                    && !ev.keystroke.modifiers.shift
                    && ev.keystroke.key == "0" =>
            {
                fit_all(ctx);
                crate::plugin::EventResult::Stop
            }
            _ => crate::plugin::EventResult::Continue,
        }
    }
}

/// If [`PendingInitialFitAll`] was registered, apply initial fit using current [`Viewport::window_bounds`].
fn try_apply_pending_initial_fit_all_ctx(ctx: &mut PluginContext<'_>) -> bool {
    if ctx.shared_state.remove::<PendingInitialFitAll>().is_none() {
        return false;
    }
    let Some(b) = ctx.window_bounds() else {
        return false;
    };
    let win_w: f32 = b.size.width.into();
    let win_h: f32 = b.size.height.into();
    let Some((bx, by, bw, bh)) = ctx.graph.nodes_world_aabb() else {
        return false;
    };
    apply_frame_world_rect_to_viewport(ctx.viewport_mut(), win_w, win_h, bx, by, bw, bh);
    true
}
