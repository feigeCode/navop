use crate::{
    Edge, EdgeId, NodeId,
    plugin::{
        FlowEvent, Plugin, PluginContext, is_edge_visible, is_node_visible,
        primary_platform_modifier,
    },
};

/// Select every node and edge that intersects the current window viewport (⌘A / Ctrl+A).
pub struct SelectAllViewportPlugin;

impl SelectAllViewportPlugin {
    pub fn new() -> Self {
        Self
    }
}

fn select_visible(ctx: &mut PluginContext) {
    let order: Vec<NodeId> = ctx.graph.node_order().to_vec();
    let visible_nodes: Vec<NodeId> = order
        .into_iter()
        .filter(|id| is_node_visible(ctx.graph, ctx.viewport, id))
        .collect();

    let edges: Vec<Edge> = ctx.graph.edges.values().cloned().collect();
    let visible_edges: Vec<EdgeId> = edges
        .into_iter()
        .filter(|e| is_edge_visible(ctx.graph, ctx.viewport, &e))
        .map(|e| e.id)
        .collect();

    ctx.clear_selected_node();
    ctx.clear_selected_edge();

    for id in visible_nodes.iter() {
        ctx.add_selected_node(*id, true);
    }
    for id in visible_edges.iter() {
        ctx.add_selected_edge(*id, true);
    }
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
        if let FlowEvent::Input(crate::plugin::InputEvent::KeyDown(ev)) = event {
            if primary_platform_modifier(ev) && ev.keystroke.key == "a" {
                select_visible(ctx);
                ctx.notify();
                return crate::plugin::EventResult::Stop;
            }
        }
        crate::plugin::EventResult::Continue
    }
}
