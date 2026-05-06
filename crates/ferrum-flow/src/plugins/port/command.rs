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
        ctx.remove_edge(&self.edge.id);
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
            GraphOp::NodeOrderInsert { id: self.node.id() },
        ]
    }
    fn undo(&mut self, ctx: &mut crate::canvas::CommandContext) {
        ctx.remove_node(&self.node.id());
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
        ctx.port_offset_cache.clear_node(&self.port.node_id());
    }
    fn to_ops(&self, _ctx: &mut crate::CommandContext) -> Vec<GraphOp> {
        vec![GraphOp::AddPort(self.port.clone())]
    }
    fn undo(&mut self, ctx: &mut crate::canvas::CommandContext) {
        let node_id = self.port.node_id();
        ctx.remove_port(&self.port.id());
        ctx.port_offset_cache.clear_node(&node_id);
    }
}

#[cfg(test)]
mod command_interop_tests {
    use serde_json::json;

    use crate::{
        CreateEdge, CreateNode, CreatePort, Graph, PortBuilder, PortKind, PortPosition, PortType,
        command_interop::assert_command_interop,
    };

    #[test]
    fn create_node_command_interop() {
        let mut base = Graph::new();
        let (node, _ports, _) = base
            .create_node("x")
            .position(100.0, 80.0)
            .data(json!({ "k": "v" }))
            .build_raw();

        assert_command_interop(
            &base,
            || Box::new(CreateNode::new(node.clone())),
            "CreateNode",
        );
    }

    #[test]
    fn create_port_command_interop() {
        let mut base = Graph::new();
        let node_id = base.create_node("x").position(0.0, 0.0).build().unwrap();
        let port = PortBuilder::new(base.next_port_id())
            .kind(PortKind::Output)
            .node_id(node_id)
            .index(0)
            .position(PortPosition::Right)
            .size(12.0, 12.0)
            .port_type(PortType::Any)
            .build();

        assert_command_interop(
            &base,
            || Box::new(CreatePort::new(port.clone())),
            "CreatePort",
        );
    }

    #[test]
    fn create_edge_command_interop() {
        let mut base = Graph::new();
        let n1 = base
            .create_node("a")
            .position(0.0, 0.0)
            .output()
            .build()
            .unwrap();
        let n2 = base
            .create_node("b")
            .position(100.0, 0.0)
            .input()
            .build()
            .unwrap();
        let n1_node = base.get_node(&n1).expect("source node exists");
        let n2_node = base.get_node(&n2).expect("target node exists");
        let edge = base
            .new_edge()
            .source(n1_node.outputs()[0])
            .target(n2_node.inputs()[0]);

        assert_command_interop(
            &base,
            || Box::new(CreateEdge::new(edge.clone())),
            "CreateEdge",
        );
    }
}
