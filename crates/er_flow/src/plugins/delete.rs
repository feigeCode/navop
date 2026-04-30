use crate::{
    Edge, GraphOp, Node,
    canvas::Command,
    plugin::{FlowEvent, Plugin},
};

pub struct DeletePlugin;

impl DeletePlugin {
    pub fn new() -> Self {
        Self {}
    }
}

pub(crate) fn delete_selection(ctx: &mut crate::plugin::PluginContext) {
    ctx.execute_command(DeleteCommand::new(ctx));
}

impl Plugin for DeletePlugin {
    fn name(&self) -> &'static str {
        "delete"
    }
    fn setup(&mut self, _ctx: &mut crate::plugin::InitPluginContext) {}
    fn on_event(
        &mut self,
        event: &FlowEvent,
        ctx: &mut crate::plugin::PluginContext,
    ) -> crate::plugin::EventResult {
        if let FlowEvent::Input(crate::plugin::InputEvent::KeyDown(ev)) = event {
            if ev.keystroke.key == "delete" || ev.keystroke.key == "backspace" {
                ctx.execute_command(DeleteCommand::new(&ctx));
                return crate::plugin::EventResult::Stop;
            }
        }
        crate::plugin::EventResult::Continue
    }
}

struct DeleteCommand {
    selected_edge: Vec<Edge>,
    selected_node: Vec<Node>,
}

impl DeleteCommand {
    fn new(ctx: &crate::plugin::PluginContext) -> Self {
        Self {
            selected_edge: ctx
                .graph
                .selected_edge
                .iter()
                .filter_map(|id| ctx.graph.edges.get(id).cloned())
                .collect(),
            selected_node: ctx
                .graph
                .selected_node
                .iter()
                .filter_map(|id| ctx.get_node(id).cloned())
                .collect(),
        }
    }
}

impl Command for DeleteCommand {
    fn name(&self) -> &'static str {
        "delete"
    }
    fn execute(&mut self, ctx: &mut crate::canvas::CommandContext) {
        ctx.remove_selected_edge();
        ctx.remove_selected_node();
    }
    fn undo(&mut self, ctx: &mut crate::canvas::CommandContext) {
        for edge in &self.selected_edge {
            ctx.add_edge(edge.clone());
            ctx.add_selected_edge(edge.id, true);
        }

        for node in &self.selected_node {
            ctx.add_node(node.clone());
            ctx.add_selected_node(node.id, true);
        }
    }

    fn to_ops(&self, ctx: &mut crate::CommandContext) -> Vec<crate::GraphOp> {
        let mut list = vec![];
        for node in &self.selected_node {
            list.push(GraphOp::RemoveNode { id: node.id });
            let mut port_ids = node.inputs.clone();
            port_ids.extend(node.outputs.clone());

            let index = ctx.graph.node_order().iter().position(|v| *v == node.id);
            if let Some(index) = index {
                list.push(GraphOp::NodeOrderRemove { index })
            }

            for port_id in port_ids.iter() {
                let edge1 = ctx
                    .graph
                    .edges
                    .iter()
                    .find(|(_, edge)| edge.source_port == *port_id);
                if let Some((&edge_id, _)) = edge1 {
                    list.push(GraphOp::RemoveEdge(edge_id));
                }

                let edge2 = ctx
                    .graph
                    .edges
                    .iter()
                    .find(|(_, edge)| edge.target_port == *port_id);
                if let Some((&edge_id, _)) = edge2 {
                    list.push(GraphOp::RemoveEdge(edge_id));
                }

                list.push(GraphOp::RemovePort(*port_id));
            }
        }

        for edge in &self.selected_edge {
            list.push(GraphOp::RemoveEdge(edge.id));
        }

        list
    }
}
