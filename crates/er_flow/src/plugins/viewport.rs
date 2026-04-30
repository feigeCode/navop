use gpui::{MouseButton, Pixels, Point, px};

use crate::{
    canvas::{Command, Interaction, InteractionResult},
    plugin::{EventResult, FlowEvent, InputEvent, Plugin},
    plugins::current_tool_mode,
};

pub struct ViewportPlugin {
    pan_with_blank_left_drag: bool,
}

impl ViewportPlugin {
    pub fn new() -> Self {
        Self {
            pan_with_blank_left_drag: false,
        }
    }

    pub fn with_blank_left_drag() -> Self {
        Self {
            pan_with_blank_left_drag: true,
        }
    }
}

impl Plugin for ViewportPlugin {
    fn name(&self) -> &'static str {
        "viewport"
    }
    fn setup(&mut self, _ctx: &mut crate::plugin::InitPluginContext) {}
    fn on_event(
        &mut self,
        event: &crate::plugin::FlowEvent,
        ctx: &mut crate::plugin::PluginContext,
    ) -> EventResult {
        if let FlowEvent::Input(InputEvent::MouseDown(ev)) = event {
            if ev.button == MouseButton::Left {
                let mouse_world = ctx.screen_to_world(ev.position);
                let node_under_mouse = ctx.hit_node(mouse_world).is_some();
                let pan_tool_active = current_tool_mode(ctx.shared_state).is_pan();
                if should_start_pan(
                    ev.modifiers.shift,
                    self.pan_with_blank_left_drag,
                    node_under_mouse,
                    pan_tool_active,
                ) {
                    ctx.start_interaction(Panning {
                        start_mouse: ev.position,
                        start_offset: ctx.viewport.offset,
                    });
                    return EventResult::Stop;
                }
            }
        } else if let FlowEvent::Input(InputEvent::Wheel(ev)) = event {
            let cursor = ev.position;

            let before = ctx.screen_to_world(cursor);

            let delta = f32::from(ev.delta.pixel_delta(px(1.0)).y);
            if delta == 0.0 {
                return EventResult::Continue;
            }

            let zoom_delta = if delta > 0.0 { 0.9 } else { 1.1 };

            ctx.viewport.zoom *= zoom_delta;

            ctx.viewport.zoom = ctx.viewport.zoom.clamp(0.7, 3.0);

            let after = ctx.world_to_screen(before);

            ctx.viewport.offset.x += cursor.x - after.x;
            ctx.viewport.offset.y += cursor.y - after.y;
            ctx.notify();
        }
        EventResult::Continue
    }
    fn priority(&self) -> i32 {
        10
    }
    fn render(&mut self, _context: &mut crate::plugin::RenderContext) -> Option<gpui::AnyElement> {
        None
    }
}

fn should_start_pan(
    shift: bool,
    blank_left_drag: bool,
    node_under_mouse: bool,
    pan_tool_active: bool,
) -> bool {
    pan_tool_active || shift || (blank_left_drag && !node_under_mouse)
}

struct Panning {
    start_mouse: Point<Pixels>,
    start_offset: Point<Pixels>,
}

impl Interaction for Panning {
    fn on_mouse_move(
        &mut self,
        ev: &gpui::MouseMoveEvent,
        ctx: &mut crate::plugin::PluginContext,
    ) -> InteractionResult {
        let dx = ev.position.x - self.start_mouse.x;
        let dy = ev.position.y - self.start_mouse.y;

        ctx.viewport.offset.x = self.start_offset.x + dx;
        ctx.viewport.offset.y = self.start_offset.y + dy;
        ctx.notify();

        InteractionResult::Continue
    }
    fn on_mouse_up(
        &mut self,
        _event: &gpui::MouseUpEvent,
        ctx: &mut crate::plugin::PluginContext,
    ) -> crate::canvas::InteractionResult {
        ctx.execute_command(PanningCommand {
            from: self.start_offset,
            to: ctx.viewport.offset,
        });
        ctx.cancel_interaction();
        InteractionResult::End
    }
    fn render(&self, _ctx: &mut crate::plugin::RenderContext) -> Option<gpui::AnyElement> {
        None
    }
}

struct PanningCommand {
    from: Point<Pixels>,
    to: Point<Pixels>,
}

impl Command for PanningCommand {
    fn name(&self) -> &'static str {
        "panning"
    }
    fn execute(&mut self, ctx: &mut crate::canvas::CommandContext) {
        ctx.viewport.offset.x = self.to.x;
        ctx.viewport.offset.y = self.to.y;
    }
    fn undo(&mut self, ctx: &mut crate::canvas::CommandContext) {
        ctx.viewport.offset.x = self.from.x;
        ctx.viewport.offset.y = self.from.y;
    }

    fn to_ops(&self, _ctx: &mut crate::CommandContext) -> Vec<crate::GraphOp> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::should_start_pan;

    #[test]
    fn default_viewport_keeps_shift_drag_pan_behavior() {
        assert!(should_start_pan(true, false, false, false));
        assert!(!should_start_pan(false, false, false, false));
    }

    #[test]
    fn blank_left_drag_pans_only_when_not_over_node() {
        assert!(should_start_pan(false, true, false, false));
        assert!(!should_start_pan(false, true, true, false));
    }

    #[test]
    fn pan_tool_pans_even_when_over_node() {
        assert!(should_start_pan(false, false, true, true));
    }
}
