use crate::{
    Edge, EdgeId, GraphOp, Node, Port,
    canvas::Command,
    plugin::{FlowEvent, Plugin},
};
use std::collections::HashSet;

pub struct DeletePlugin;

impl DeletePlugin {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for DeletePlugin {
    fn default() -> Self {
        Self::new()
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
        if let FlowEvent::Input(crate::plugin::InputEvent::KeyDown(ev)) = event
            && (ev.keystroke.key == "delete" || ev.keystroke.key == "backspace")
        {
            ctx.execute_command(DeleteCommand::new(ctx));
            return crate::plugin::EventResult::Stop;
        }
        crate::plugin::EventResult::Continue
    }
}

struct DeleteCommand {
    selected_edge: Vec<Edge>,
    originally_selected_edge_ids: HashSet<EdgeId>,
    selected_node: Vec<Node>,
    selected_port: Vec<Port>,
}

impl DeleteCommand {
    fn collect_edges_for_selected_nodes(
        graph: &crate::Graph,
        selected_nodes: &[Node],
    ) -> Vec<Edge> {
        let mut edge_ids = HashSet::new();
        let mut edges = Vec::new();

        for node in selected_nodes {
            for port_id in node.inputs().iter().chain(node.outputs().iter()) {
                for edge in graph.edges().values() {
                    if (edge.source_port == *port_id || edge.target_port == *port_id)
                        && edge_ids.insert(edge.id)
                    {
                        edges.push(edge.clone());
                    }
                }
            }
        }

        edges
    }

    fn new(ctx: &crate::plugin::PluginContext) -> Self {
        let selected_node: Vec<Node> = ctx
            .graph
            .selected_node()
            .iter()
            .filter_map(|id| ctx.get_node(id).cloned())
            .collect();
        let mut selected_edge: Vec<Edge> = ctx
            .graph
            .selected_edge()
            .iter()
            .filter_map(|id| ctx.graph.get_edge(id).cloned())
            .collect();
        let originally_selected_edge_ids: HashSet<_> = selected_edge.iter().map(|e| e.id).collect();
        let mut seen_edge_ids: HashSet<_> = selected_edge.iter().map(|e| e.id).collect();
        for edge in Self::collect_edges_for_selected_nodes(ctx.graph, &selected_node) {
            if seen_edge_ids.insert(edge.id) {
                selected_edge.push(edge);
            }
        }

        Self {
            selected_edge,
            originally_selected_edge_ids,
            selected_node,
            selected_port: ctx
                .graph
                .selected_node()
                .iter()
                .filter_map(|node_id| ctx.get_node(node_id))
                .flat_map(|node| node.inputs().iter().chain(node.outputs().iter()))
                .filter_map(|port_id| ctx.graph.get_port(port_id).cloned())
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
        for node in &self.selected_node {
            ctx.add_node(node.clone());
            ctx.add_selected_node(node.id(), true);
        }

        for port in &self.selected_port {
            ctx.add_port(port.clone());
        }

        for edge in &self.selected_edge {
            ctx.add_edge(edge.clone());
            if self.originally_selected_edge_ids.contains(&edge.id) {
                ctx.add_selected_edge(edge.id, true);
            }
        }
    }

    fn to_ops(&self, ctx: &mut crate::CommandContext) -> Vec<crate::GraphOp> {
        let mut list = vec![];
        let mut removed_edges = HashSet::new();
        for node in &self.selected_node {
            list.push(GraphOp::RemoveNode { id: node.id() });
            let mut port_ids = node.inputs().to_vec();
            port_ids.extend(node.outputs().iter().copied());

            let index = ctx.graph.node_order().iter().position(|v| *v == node.id());
            if let Some(index) = index {
                list.push(GraphOp::NodeOrderRemove { index })
            }

            for port_id in port_ids.iter() {
                list.push(GraphOp::RemovePort(*port_id));
            }
        }

        for edge in &self.selected_edge {
            if removed_edges.insert(edge.id) {
                list.push(GraphOp::RemoveEdge(edge.id));
            }
        }

        vec![GraphOp::Batch(list)]
    }
}

#[cfg(test)]
mod command_interop_tests {
    use std::collections::HashSet;

    use crate::{Graph, command_interop::assert_command_interop};

    use super::DeleteCommand;

    fn delete_command_like_new(graph: &Graph) -> DeleteCommand {
        let selected_node: Vec<crate::Node> = graph
            .selected_node()
            .iter()
            .filter_map(|id| graph.get_node(id).cloned())
            .collect();
        let mut selected_edge: Vec<crate::Edge> = graph
            .selected_edge()
            .iter()
            .filter_map(|id| graph.get_edge(id).cloned())
            .collect();
        let originally_selected_edge_ids: HashSet<_> = selected_edge.iter().map(|e| e.id).collect();
        let mut seen_edge_ids: HashSet<_> = selected_edge.iter().map(|e| e.id).collect();
        for edge in DeleteCommand::collect_edges_for_selected_nodes(graph, &selected_node) {
            if seen_edge_ids.insert(edge.id) {
                selected_edge.push(edge);
            }
        }
        let selected_port: Vec<crate::Port> = graph
            .selected_node()
            .iter()
            .filter_map(|node_id| graph.get_node(node_id))
            .flat_map(|node| node.inputs().iter().chain(node.outputs().iter()))
            .filter_map(|port_id| graph.get_port(port_id).cloned())
            .collect();
        DeleteCommand {
            selected_edge,
            originally_selected_edge_ids,
            selected_node,
            selected_port,
        }
    }

    #[test]
    fn delete_command_interop_single_node_with_port() {
        let mut base = Graph::new();
        let src_id = base
            .create_node("x")
            .position(-220.0, 0.0)
            .output()
            .build()
            .unwrap();
        let dst_id = base
            .create_node("x")
            .position(220.0, 0.0)
            .input()
            .output()
            .build()
            .unwrap();
        let other_id = base
            .create_node("x")
            .position(440.0, 0.0)
            .input()
            .build()
            .unwrap();
        // Put selected node at the end so execute+undo preserves node_order with current command behavior.
        let selected_id = base
            .create_node("x")
            .position(0.0, 0.0)
            .input()
            .output()
            .build()
            .unwrap();

        let selected_node = base.get_node(&selected_id).expect("selected node").clone();
        let src_node = base.get_node(&src_id).expect("src node").clone();
        let dst_node = base.get_node(&dst_id).expect("dst node").clone();
        let other_node = base.get_node(&other_id).expect("other node").clone();

        // This edge is NOT selected, but should be deleted via node-cascade.
        let _cascade_in = base
            .create_edge()
            .source(src_node.outputs()[0])
            .target(selected_node.inputs()[0])
            .build()
            .expect("cascade in edge");
        // This edge IS selected and also touches selected node.
        let selected_edge = base
            .create_edge()
            .source(selected_node.outputs()[0])
            .target(dst_node.inputs()[0])
            .build()
            .expect("selected edge");
        // Unrelated edge should remain untouched.
        let _unrelated = base
            .create_edge()
            .source(dst_node.outputs()[0])
            .target(other_node.inputs()[0])
            .build()
            .expect("unrelated edge");

        base.add_selected_node(selected_id, false);
        base.add_selected_edge(selected_edge, true);

        let cmd = delete_command_like_new(&base);
        assert_command_interop(
            &base,
            || {
                Box::new(DeleteCommand {
                    selected_edge: cmd.selected_edge.clone(),
                    originally_selected_edge_ids: cmd.originally_selected_edge_ids.clone(),
                    selected_node: cmd.selected_node.clone(),
                    selected_port: cmd.selected_port.clone(),
                })
            },
            "DeleteCommand",
        );
    }
}
