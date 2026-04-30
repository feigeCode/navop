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
            old_selected_edge: ctx.graph.selected_edge.clone(),
            old_selected_node: ctx.graph.selected_node.clone(),
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
        ctx.graph.selected_node = self.old_selected_node.clone();
        ctx.graph.selected_edge = self.old_selected_edge.clone();
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
    pub fn new(start_positions: &Vec<(NodeId, Point<Pixels>)>, ctx: &PluginContext) -> Self {
        let mut to = Vec::new();
        for (node_id, _) in start_positions.iter() {
            if let Some(node) = ctx.get_node(node_id) {
                to.push((*node_id, node.point()));
            }
        }
        Self {
            from: start_positions.clone(),
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
                node.x = point.x;
                node.y = point.y;
            }
        }
    }
    fn undo(&mut self, ctx: &mut crate::canvas::CommandContext) {
        for (id, point) in self.from.iter() {
            if let Some(node) = ctx.get_node_mut(id) {
                node.x = point.x;
                node.y = point.y;
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
