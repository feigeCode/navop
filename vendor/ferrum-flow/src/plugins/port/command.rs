use crate::{Edge, Graph, GraphError, GraphOp, Node, NodeId, Port, canvas::Command};

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
        ctx.remove_node_cascade(&self.node.id());
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

/// Link an existing node under `parent` (after [`CreateNode`]).
pub struct AttachChildCommand {
    parent: NodeId,
    child: NodeId,
}

/// Same preconditions as [`Graph::add_child`] (excluding the no-op “already linked” case).
pub fn validate_attach_child(
    graph: &Graph,
    parent: NodeId,
    child: NodeId,
) -> Result<(), GraphError> {
    graph.ensure_node(parent)?;
    graph.ensure_node(child)?;
    if parent == child {
        return Err(GraphError::SelfReference { node: parent });
    }
    if graph.is_ancestor(child, parent) {
        return Err(GraphError::WouldCreateCycle { parent, child });
    }
    Ok(())
}

impl AttachChildCommand {
    /// Validates against `graph` so [`Command::execute`] can call [`Graph::add_child`] without
    /// handling [`GraphError`] (call right after the child node exists in `graph`).
    pub fn new(graph: &Graph, parent: NodeId, child: NodeId) -> Result<Self, GraphError> {
        validate_attach_child(graph, parent, child)?;
        Ok(Self::link(parent, child))
    }

    /// Parent and child must already exist when this command runs (e.g. earlier steps in a
    /// [`CompositeCommand`](crate::CompositeCommand)).
    pub fn link(parent: NodeId, child: NodeId) -> Self {
        Self { parent, child }
    }
}

impl Command for AttachChildCommand {
    fn name(&self) -> &'static str {
        "attach_child"
    }
    fn execute(&mut self, ctx: &mut crate::canvas::CommandContext) {
        ctx.graph
            .add_child(self.parent, self.child)
            .unwrap_or_else(|e| {
                log::error!("Failed to attach child: {}", e);
            });
        ctx.port_offset_cache.clear_node(&self.child);
    }
    fn undo(&mut self, ctx: &mut crate::canvas::CommandContext) {
        ctx.graph.remove_child(self.parent, self.child);
        ctx.port_offset_cache.clear_node(&self.child);
    }
    fn to_ops(&self, _ctx: &mut crate::canvas::CommandContext) -> Vec<GraphOp> {
        vec![GraphOp::PushChildNode {
            id: self.parent,
            child_id: self.child,
        }]
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
        let node_id = base.create_node("x").position(0.0, 0.0).build();
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
        let n1 = base.create_node("a").position(0.0, 0.0).output().build();
        let n2 = base.create_node("b").position(100.0, 0.0).input().build();
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
