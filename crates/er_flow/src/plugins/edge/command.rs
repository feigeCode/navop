use std::{collections::HashSet, vec};

use crate::{EdgeId, NodeId, canvas::Command, plugin::PluginContext};

pub(super) struct SelectEdgeCommand {
    edge_id: EdgeId,
    shift: bool,
    old_selected_edge: HashSet<EdgeId>,
    old_selected_node: HashSet<NodeId>,
}

impl SelectEdgeCommand {
    pub(super) fn new(edge_id: EdgeId, shift: bool, ctx: &PluginContext) -> Self {
        Self {
            edge_id,
            shift,
            old_selected_edge: ctx.graph.selected_edge.clone(),
            old_selected_node: ctx.graph.selected_node.clone(),
        }
    }
}

impl Command for SelectEdgeCommand {
    fn name(&self) -> &'static str {
        "select_edge"
    }
    fn execute(&mut self, ctx: &mut crate::canvas::CommandContext) {
        if !self.shift {
            ctx.clear_selected_node();
        }
        ctx.add_selected_edge(self.edge_id, self.shift);
    }
    fn undo(&mut self, ctx: &mut crate::canvas::CommandContext) {
        ctx.graph.selected_node = self.old_selected_node.clone();
        ctx.graph.selected_edge = self.old_selected_edge.clone();
    }
    fn to_ops(&self, ctx: &mut crate::CommandContext) -> Vec<crate::GraphOp> {
        if !self.shift {
            ctx.clear_selected_node();
        }
        ctx.add_selected_edge(self.edge_id, self.shift);
        vec![]
    }
}

pub(super) struct ClearEdgeCommand {
    old_selected_edge: HashSet<EdgeId>,
}

impl ClearEdgeCommand {
    pub(super) fn new(ctx: &PluginContext) -> Self {
        Self {
            old_selected_edge: ctx.graph.selected_edge.clone(),
        }
    }
}

impl Command for ClearEdgeCommand {
    fn name(&self) -> &'static str {
        "clear_edge"
    }
    fn execute(&mut self, ctx: &mut crate::canvas::CommandContext) {
        ctx.clear_selected_edge();
    }
    fn undo(&mut self, ctx: &mut crate::canvas::CommandContext) {
        ctx.graph.selected_edge = self.old_selected_edge.clone();
    }

    fn to_ops(&self, ctx: &mut crate::CommandContext) -> Vec<crate::GraphOp> {
        ctx.clear_selected_edge();
        vec![]
    }
}
