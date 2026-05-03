use ferrum_flow::{EventResult, FlowEvent, InputEvent, Plugin, PluginContext};
use gpui::px;

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
            let dx = -delta.x;
            let dy = -delta.y;
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