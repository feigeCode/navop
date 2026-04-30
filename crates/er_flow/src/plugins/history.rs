use crate::plugin::{FlowEvent, Plugin, primary_platform_modifier};

pub struct HistoryPlugin;

impl HistoryPlugin {
    pub fn new() -> Self {
        Self {}
    }
}

impl Plugin for HistoryPlugin {
    fn name(&self) -> &'static str {
        "history"
    }
    fn setup(&mut self, _ctx: &mut crate::plugin::InitPluginContext) {}
    fn on_event(
        &mut self,
        event: &FlowEvent,
        ctx: &mut crate::plugin::PluginContext,
    ) -> crate::plugin::EventResult {
        if let FlowEvent::Input(crate::plugin::InputEvent::KeyDown(ev)) = event {
            let primary = primary_platform_modifier(ev);
            if ev.keystroke.key == "z" && primary && ev.keystroke.modifiers.shift {
                ctx.redo();
                return crate::plugin::EventResult::Stop;
            } else if ev.keystroke.key == "z" && primary {
                ctx.undo();
                return crate::plugin::EventResult::Stop;
            }
        }
        crate::plugin::EventResult::Continue
    }
}
