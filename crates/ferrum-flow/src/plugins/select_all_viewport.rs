use std::collections::HashSet;

use crate::plugin::{FlowEvent, Plugin, PluginContext, primary_platform_modifier};

/// Select every node and edge that intersects the current window viewport (⌘A / Ctrl+A).
pub struct SelectAllViewportPlugin;

impl SelectAllViewportPlugin {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SelectAllViewportPlugin {
    fn default() -> Self {
        Self::new()
    }
}

fn select_visible(ctx: &mut PluginContext) {
    let visible_nodes: HashSet<_> = ctx
        .graph
        .node_order()
        .iter()
        .filter(|id| ctx.is_node_visible(id))
        .copied()
        .collect();

    let visible_edges: HashSet<_> = ctx
        .graph
        .edges_values()
        .filter(|e| ctx.is_edge_visible(e))
        .map(|e| e.id)
        .collect();

    ctx.graph.set_selected_node(visible_nodes);
    ctx.graph.set_selected_edge(visible_edges);
}

pub(crate) fn select_all_in_viewport(ctx: &mut PluginContext) {
    select_visible(ctx);
}

impl Plugin for SelectAllViewportPlugin {
    fn name(&self) -> &'static str {
        "select_all_viewport"
    }

    fn setup(&mut self, _ctx: &mut crate::plugin::InitPluginContext) {}

    fn priority(&self) -> i32 {
        93
    }

    fn on_event(
        &mut self,
        event: &FlowEvent,
        ctx: &mut PluginContext,
    ) -> crate::plugin::EventResult {
        if let FlowEvent::Input(crate::plugin::InputEvent::KeyDown(ev)) = event
            && primary_platform_modifier(ev)
            && ev.keystroke.key == "a"
        {
            select_visible(ctx);
            ctx.notify();
            return crate::plugin::EventResult::Stop;
        }
        crate::plugin::EventResult::Continue
    }
}
