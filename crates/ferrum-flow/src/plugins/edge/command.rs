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
            old_selected_edge: ctx.graph.selected_edge().clone(),
            old_selected_node: ctx.graph.selected_node().clone(),
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
        ctx.graph.set_selected_node(self.old_selected_node.clone());
        ctx.graph.set_selected_edge(self.old_selected_edge.clone());
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
            old_selected_edge: ctx.graph.selected_edge().clone(),
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
        ctx.graph.set_selected_edge(self.old_selected_edge.clone());
    }

    fn to_ops(&self, ctx: &mut crate::CommandContext) -> Vec<crate::GraphOp> {
        ctx.clear_selected_edge();
        vec![]
    }
}

#[cfg(test)]
mod command_interop_tests {
    use crate::{Graph, command_interop::assert_command_interop};

    use super::{ClearEdgeCommand, SelectEdgeCommand};

    #[test]
    fn select_edge_command_interop() {
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
        let n1_node = base.get_node(&n1).expect("n1");
        let n2_node = base.get_node(&n2).expect("n2");
        let source_port = n1_node.outputs()[0];
        let target_port = n2_node.inputs()[0];
        let edge_id = base
            .create_edge()
            .source(source_port)
            .target(target_port)
            .build()
            .expect("edge");

        let old_selected_edge = base.selected_edge().clone();
        let old_selected_node = base.selected_node().clone();

        assert_command_interop(
            &base,
            || {
                Box::new(SelectEdgeCommand {
                    edge_id,
                    shift: false,
                    old_selected_edge: old_selected_edge.clone(),
                    old_selected_node: old_selected_node.clone(),
                })
            },
            "SelectEdgeCommand",
        );
    }

    #[test]
    fn clear_edge_command_interop() {
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
        let n1_node = base.get_node(&n1).expect("n1");
        let n2_node = base.get_node(&n2).expect("n2");
        let source_port = n1_node.outputs()[0];
        let target_port = n2_node.inputs()[0];
        let edge_id = base
            .create_edge()
            .source(source_port)
            .target(target_port)
            .build()
            .expect("edge");
        base.add_selected_edge(edge_id, false);

        let old_selected_edge = base.selected_edge().clone();

        assert_command_interop(
            &base,
            || {
                Box::new(ClearEdgeCommand {
                    old_selected_edge: old_selected_edge.clone(),
                })
            },
            "ClearEdgeCommand",
        );
    }
}
