use ferrum_flow::{EventResult, FlowEvent, InputEvent, Plugin, PluginContext};
use gpui::{Pixels, Point, px};

pub struct ErDiagramScrollPanPlugin;

impl ErDiagramScrollPanPlugin {
    pub fn new() -> Self {
        Self
    }
}

impl Plugin for ErDiagramScrollPanPlugin {
    fn name(&self) -> &'static str {
        "er_diagram_scroll_pan"
    }

    fn on_event(&mut self, event: &FlowEvent, ctx: &mut PluginContext) -> EventResult {
        if let FlowEvent::Input(InputEvent::Wheel(ev)) = event {
            let delta = ev.delta.pixel_delta(px(1.0));
            let pan = wheel_delta_to_pan(delta);
            let dx = pan.x;
            let dy = pan.y;
            if dx != px(0.0) || dy != px(0.0) {
                ctx.translate_offset(dx, dy);
                ctx.notify();
                return EventResult::Stop;
            }
        }
        EventResult::Continue
    }

    fn priority(&self) -> i32 {
        130
    }
}

fn wheel_delta_to_pan(delta: Point<Pixels>) -> Point<Pixels> {
    Point::new(-delta.x, delta.y)
}

#[cfg(test)]
mod tests {
    use super::wheel_delta_to_pan;
    use gpui::{Point, px};

    #[test]
    fn vertical_wheel_delta_is_not_inverted() {
        assert_eq!(
            wheel_delta_to_pan(Point::new(px(0.0), px(24.0))),
            Point::new(px(0.0), px(24.0))
        );
    }
}
