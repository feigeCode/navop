use std::time::Duration;

use ferrum_flow::{
    EventResult, FlowEvent, InputEvent, Plugin, PluginContext, RenderContext, RenderLayer,
};
use gpui::{MouseButton, Pixels, Point, px};

use crate::er_diagram::scrollbar_plugin::{ScrollbarDragInteraction, ScrollbarState};

pub struct ErDiagramScrollPanPlugin {
    scrollbars: ScrollbarState,
    refresh_scheduled: bool,
}

impl ErDiagramScrollPanPlugin {
    pub fn new() -> Self {
        Self {
            scrollbars: ScrollbarState::default(),
            refresh_scheduled: false,
        }
    }
}

impl Plugin for ErDiagramScrollPanPlugin {
    fn name(&self) -> &'static str {
        "er_diagram_scroll_pan"
    }

    fn on_event(&mut self, event: &FlowEvent, ctx: &mut PluginContext) -> EventResult {
        match event {
            FlowEvent::DrawableBoundsReady => {
                if !self.refresh_scheduled {
                    self.refresh_scheduled = true;
                    ctx.schedule_after(Duration::from_millis(16));
                }
                EventResult::Continue
            }
            FlowEvent::Input(InputEvent::Wheel(ev)) => {
                let delta = ev.delta.pixel_delta(px(1.0));
                let pan = wheel_delta_to_pan(delta);
                let dx = pan.x;
                let dy = pan.y;
                if dx != px(0.0) || dy != px(0.0) {
                    ctx.translate_offset(dx, dy);
                    ctx.notify();
                    return EventResult::Stop;
                }
                EventResult::Continue
            }
            FlowEvent::Input(InputEvent::MouseDown(ev)) if ev.button == MouseButton::Left => {
                let pointer_position = ctx.window_pointer_to_canvas_local(ev.position);
                if let Some(axis) = self.scrollbars.axis_at(pointer_position) {
                    ctx.start_interaction(ScrollbarDragInteraction::new(axis, ev.position, ctx));
                    return EventResult::Stop;
                }
                EventResult::Continue
            }
            _ => EventResult::Continue,
        }
    }

    fn render(&mut self, ctx: &mut RenderContext) -> Option<gpui::AnyElement> {
        self.scrollbars.render(ctx)
    }

    fn priority(&self) -> i32 {
        130
    }

    fn render_layer(&self) -> RenderLayer {
        RenderLayer::Overlay
    }
}

fn wheel_delta_to_pan(delta: Point<Pixels>) -> Point<Pixels> {
    Point::new(-delta.x, delta.y)
}

#[cfg(test)]
mod tests {
    use super::wheel_delta_to_pan;
    use crate::er_diagram::scrollbar_plugin::thumb_length;
    use gpui::{Point, px};

    #[test]
    fn vertical_wheel_delta_is_not_inverted() {
        assert_eq!(
            wheel_delta_to_pan(Point::new(px(0.0), px(24.0))),
            Point::new(px(0.0), px(24.0))
        );
    }

    #[test]
    fn thumb_length_has_minimum_size() {
        assert_eq!(thumb_length(px(100.0), px(10.0), px(1000.0)), px(32.0));
    }
}
