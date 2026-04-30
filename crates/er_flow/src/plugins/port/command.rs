use crate::{Edge, GraphOp, Node, Port, canvas::Command};

pub struct CreateEdge {
    edge: Edge,
}

impl CreateEdge {
    pub fn new(edge: Edge) -> Self {
        Self { edge }
    }
}

impl Command for CreateEdge {
    fn name(&self) -> &'static str {
        "create_edge"
    }
    fn execute(&mut self, ctx: &mut crate::canvas::CommandContext) {
        ctx.add_edge(self.edge.clone());
    }
    fn undo(&mut self, ctx: &mut crate::canvas::CommandContext) {
        ctx.remove_edge(self.edge.id);
    }

    fn to_ops(&self, _ctx: &mut crate::CommandContext) -> Vec<crate::GraphOp> {
        vec![GraphOp::AddEdge(self.edge.clone())]
    }
}

pub struct CreateNode {
    node: Node,
}

impl CreateNode {
    pub fn new(node: Node) -> Self {
        Self { node }
    }
}

impl Command for CreateNode {
    fn name(&self) -> &'static str {
        "create_node"
    }
    fn execute(&mut self, ctx: &mut crate::canvas::CommandContext) {
        ctx.add_node(self.node.clone());
    }
    fn to_ops(&self, _ctx: &mut crate::CommandContext) -> Vec<GraphOp> {
        vec![
            GraphOp::AddNode(self.node.clone()),
            GraphOp::NodeOrderInsert { id: self.node.id },
        ]
    }
    fn undo(&mut self, ctx: &mut crate::canvas::CommandContext) {
        ctx.remove_node(&self.node.id);
    }
}

pub struct CreatePort {
    port: Port,
}

impl CreatePort {
    pub fn new(port: Port) -> Self {
        Self { port }
    }
}

impl Command for CreatePort {
    fn name(&self) -> &'static str {
        "create_port"
    }
    fn execute(&mut self, ctx: &mut crate::canvas::CommandContext) {
        ctx.add_port(self.port.clone());
        ctx.port_offset_cache.clear_node(&self.port.node_id);
    }
    fn to_ops(&self, _ctx: &mut crate::CommandContext) -> Vec<GraphOp> {
        vec![GraphOp::AddPort(self.port.clone())]
    }
    fn undo(&mut self, ctx: &mut crate::canvas::CommandContext) {
        let node_id = self.port.node_id;
        ctx.remove_port(&self.port.id);
        ctx.port_offset_cache.clear_node(&node_id);
    }
}
