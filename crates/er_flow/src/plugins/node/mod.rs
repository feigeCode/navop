mod command;
mod drag_events;
mod interaction;

pub use command::DragNodesCommand;
pub use drag_events::{NODE_DRAG_TICK_INTERVAL, NodeDragEvent};
use gpui::{Element, ParentElement, div};
pub use interaction::NodeInteractionPlugin;

use crate::{RenderContext, plugin::Plugin};

pub struct NodePlugin {}

impl NodePlugin {
    pub fn new() -> Self {
        Self {}
    }
}

impl Plugin for NodePlugin {
    fn name(&self) -> &'static str {
        "node"
    }
    fn setup(&mut self, _ctx: &mut crate::plugin::InitPluginContext) {}
    fn on_event(
        &mut self,
        _event: &crate::plugin::FlowEvent,
        _context: &mut crate::plugin::PluginContext,
    ) -> crate::plugin::EventResult {
        crate::plugin::EventResult::Continue
    }
    fn priority(&self) -> i32 {
        60
    }
    fn render_layer(&self) -> crate::plugin::RenderLayer {
        crate::plugin::RenderLayer::Nodes
    }
    fn render(&mut self, ctx: &mut RenderContext) -> Option<gpui::AnyElement> {
        let node_ids: Vec<_> = ctx
            .graph
            .node_order()
            .iter()
            .filter(|node_id| ctx.is_node_visible(node_id))
            .cloned()
            .collect();

        ctx.cache_port_offset_with_nodes(&node_ids);

        let list = node_ids.iter().filter_map(|node_id| {
            let node = ctx.graph.nodes().get(node_id)?;
            let render = ctx.renderers.get(&node.node_type);

            let node_render = render.render(node, ctx);

            let ports = ctx
                .graph
                .ports
                .iter()
                .filter(|(_, port)| port.node_id == *node_id)
                .filter_map(|(_, port)| render.port_render(node, port, ctx));

            Some(div().child(node_render).children(ports))
        });

        Some(div().children(list).into_any())
    }
}
