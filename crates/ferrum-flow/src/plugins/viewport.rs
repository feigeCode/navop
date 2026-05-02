use gpui::{MouseButton, Pixels, Point, px};

use crate::{
    canvas::{Command, Interaction, InteractionResult},
    plugin::{EventResult, FlowEvent, InputEvent, Plugin},
};

pub struct ViewportPlugin;

impl ViewportPlugin {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for ViewportPlugin {
    fn default() -> Self {
        Self::new()
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
        if let FlowEvent::Input(InputEvent::MouseDown(ev)) = event
            && ((ev.button == MouseButton::Left && ev.modifiers.shift)
                || ev.button == MouseButton::Middle)
        {
            ctx.start_interaction(Panning {
                start_mouse: ev.position,
                start_offset: ctx.offset(),
            });
            return EventResult::Stop;
        } else if let FlowEvent::Input(InputEvent::Wheel(ev)) = event {
            let cursor = ev.position;

            let before = ctx.screen_to_world(cursor);

            let delta = f32::from(ev.delta.pixel_delta(px(1.0)).y);
            if delta == 0.0 {
                return EventResult::Continue;
            }

            let zoom_delta = if delta > 0.0 { 0.9 } else { 1.1 };

            ctx.set_zoom(ctx.zoom_scaled_by(zoom_delta).clamp(0.1, 3.0));

            let after = ctx.world_to_screen(before);

            ctx.translate_offset(cursor.x - after.x, cursor.y - after.y);
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

        ctx.set_offset(Point::new(
            self.start_offset.x + dx,
            self.start_offset.y + dy,
        ));
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
            to: ctx.offset(),
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
        ctx.set_offset(self.to);
    }
    fn undo(&mut self, ctx: &mut crate::canvas::CommandContext) {
        ctx.set_offset(self.from);
    }

    fn to_ops(&self, _ctx: &mut crate::CommandContext) -> Vec<crate::GraphOp> {
        vec![]
    }
}

#[cfg(test)]
mod command_interop_tests {
    use gpui::{Point, px};

    use crate::{Graph, command_interop::assert_command_interop};

    use super::PanningCommand;

    #[test]
    fn panning_command_interop() {
        let base = Graph::new();
        let cmd = PanningCommand {
            from: Point::new(px(0.0), px(0.0)),
            to: Point::new(px(12.0), px(34.0)),
        };
        assert_command_interop(
            &base,
            || {
                Box::new(PanningCommand {
                    from: cmd.from,
                    to: cmd.to,
                })
            },
            "PanningCommand",
        );
    }
}
