use std::collections::HashSet;

use gpui::{Pixels, Point};

use crate::{EdgeId, GraphOp, NodeId, canvas::Command, plugin::PluginContext};

pub struct SelecteNodeCommand {
    node_id: NodeId,
    shift: bool,
    old_node_order: Vec<NodeId>,
    old_selected_edge: HashSet<EdgeId>,
    old_selected_node: HashSet<NodeId>,
}

impl SelecteNodeCommand {
    pub fn new(node_id: NodeId, shift: bool, ctx: &PluginContext) -> Self {
        Self {
            node_id,
            shift,
            old_node_order: ctx.graph.node_order().clone(),
            old_selected_edge: ctx.graph.selected_edge().clone(),
            old_selected_node: ctx.graph.selected_node().clone(),
        }
    }
}

impl Command for SelecteNodeCommand {
    fn name(&self) -> &'static str {
        "select_node"
    }
    fn execute(&mut self, ctx: &mut crate::canvas::CommandContext) {
        if !self.shift {
            ctx.clear_selected_edge();
        }
        ctx.add_selected_node(self.node_id, self.shift);
        ctx.bring_node_to_front(self.node_id);
    }
    fn undo(&mut self, ctx: &mut crate::canvas::CommandContext) {
        ctx.graph.set_selected_node(self.old_selected_node.clone());
        ctx.graph.set_selected_edge(self.old_selected_edge.clone());
        let a = ctx.graph.node_order_mut();
        *a = self.old_node_order.clone();
    }

    fn to_ops(&self, ctx: &mut crate::CommandContext) -> Vec<crate::GraphOp> {
        if !self.shift {
            ctx.clear_selected_edge();
        }
        ctx.add_selected_node(self.node_id, self.shift);

        let mut list = vec![];
        let index = ctx
            .graph
            .node_order()
            .iter()
            .position(|v| *v == self.node_id);
        if let Some(index) = index {
            list.push(GraphOp::NodeOrderRemove { index })
        }
        list.push(GraphOp::NodeOrderInsert { id: self.node_id });
        list
    }
}

pub struct DragNodesCommand {
    from: Vec<(NodeId, Point<Pixels>)>,
    to: Vec<(NodeId, Point<Pixels>)>,
}

impl DragNodesCommand {
    pub fn new(start_positions: &[(NodeId, Point<Pixels>)], ctx: &PluginContext) -> Self {
        let mut to = Vec::new();
        for (node_id, _) in start_positions {
            if let Some(node) = ctx.get_node(node_id) {
                to.push((*node_id, node.point()));
            }
        }
        Self {
            from: start_positions.to_vec(),
            to,
        }
    }

    /// Explicit before/after positions (same node order, same length). Use for align / distribute.
    pub fn from_positions(
        from: Vec<(NodeId, Point<Pixels>)>,
        to: Vec<(NodeId, Point<Pixels>)>,
    ) -> Self {
        Self { from, to }
    }
}

impl Command for DragNodesCommand {
    fn name(&self) -> &'static str {
        "drag_nodes"
    }
    fn execute(&mut self, ctx: &mut crate::canvas::CommandContext) {
        for (id, point) in self.to.iter() {
            if let Some(node) = ctx.get_node_mut(id) {
                node.set_position_with_point(*point);
            }
        }
    }
    fn undo(&mut self, ctx: &mut crate::canvas::CommandContext) {
        for (id, point) in self.from.iter() {
            if let Some(node) = ctx.get_node_mut(id) {
                node.set_position_with_point(*point);
            }
        }
    }

    fn to_ops(&self, _ctx: &mut crate::CommandContext) -> Vec<crate::GraphOp> {
        let mut list = vec![];
        for (id, point) in self.to.iter() {
            list.push(GraphOp::MoveNode {
                id: *id,
                x: Into::<f32>::into(point.x),
                y: Into::<f32>::into(point.y),
            })
        }

        vec![GraphOp::Batch(list)]
    }
}

#[cfg(test)]
mod command_interop_tests {
    use gpui::{Point, px};

    use crate::{Graph, command_interop::assert_command_interop};

    use super::{DragNodesCommand, SelecteNodeCommand};

    #[test]
    fn select_node_command_interop() {
        let mut base = Graph::new();
        let n1 = base.create_node("a").position(0.0, 0.0).build().unwrap();
        let _n2 = base.create_node("b").position(50.0, 0.0).build().unwrap();

        let old_node_order = base.node_order().to_vec();
        let old_selected_edge = base.selected_edge().clone();
        let old_selected_node = base.selected_node().clone();

        assert_command_interop(
            &base,
            || {
                Box::new(SelecteNodeCommand {
                    node_id: n1,
                    shift: false,
                    old_node_order: old_node_order.clone(),
                    old_selected_edge: old_selected_edge.clone(),
                    old_selected_node: old_selected_node.clone(),
                })
            },
            "SelecteNodeCommand",
        );
    }

    #[test]
    fn drag_nodes_command_interop() {
        let mut base = Graph::new();
        let n = base.create_node("n").position(0.0, 0.0).build().unwrap();
        let from = vec![(n, Point::new(px(0.0), px(0.0)))];
        let to = vec![(n, Point::new(px(30.0), px(40.0)))];
        let cmd = DragNodesCommand::from_positions(from, to);

        assert_command_interop(
            &base,
            || {
                Box::new(DragNodesCommand::from_positions(
                    cmd.from.clone(),
                    cmd.to.clone(),
                ))
            },
            "DragNodesCommand",
        );
    }
}
