use crate::plugin::{FlowEvent, Plugin, PluginContext, primary_platform_modifier};

use super::clipboard_ops::{
    extract_subgraph, get_clipboard_subgraph, paste_subgraph, set_clipboard_subgraph,
};

/// Copy / paste selected nodes, their ports, and edges **between** those ports (one undo on paste).
pub struct ClipboardPlugin;

impl ClipboardPlugin {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ClipboardPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for ClipboardPlugin {
    fn name(&self) -> &'static str {
        "clipboard"
    }

    fn priority(&self) -> i32 {
        95
    }

    fn on_event(
        &mut self,
        event: &FlowEvent,
        ctx: &mut PluginContext,
    ) -> crate::plugin::EventResult {
        if let FlowEvent::Input(crate::plugin::InputEvent::KeyDown(ev)) = event {
            if !primary_platform_modifier(ev) {
                return crate::plugin::EventResult::Continue;
            }
            match ev.keystroke.key.as_str() {
                "c" => {
                    if let Some(sub) = extract_subgraph(ctx.graph) {
                        set_clipboard_subgraph(ctx, sub);
                    }
                    return crate::plugin::EventResult::Stop;
                }
                "v" => {
                    if let Some(sub) = get_clipboard_subgraph(ctx) {
                        paste_subgraph(ctx, &sub);
                    }
                    return crate::plugin::EventResult::Stop;
                }
                _ => {}
            }
        }
        crate::plugin::EventResult::Continue
    }
}
