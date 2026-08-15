use crate::{
    Edge, EdgeId, Graph, GraphError, GraphOp, Node, NodeId, ParentDeletePolicy, Port,
    canvas::Command,
    plugin::{FlowEvent, Plugin},
};
use gpui::Point;
use std::collections::{HashMap, HashSet};

pub struct DeletePlugin {
    policy: ParentDeletePolicy,
}

impl DeletePlugin {
    pub fn new(policy: ParentDeletePolicy) -> Self {
        Self { policy }
    }
}

impl Default for DeletePlugin {
    fn default() -> Self {
        Self::new(ParentDeletePolicy::Promote)
    }
}

pub(crate) fn delete_selection(ctx: &mut crate::plugin::PluginContext, policy: ParentDeletePolicy) {
    let cmd = DeleteCommand::new(ctx, policy);
    match cmd {
        Ok(cmd) => ctx.execute_command(cmd),
        Err(e) => {
            ctx.emit(e.into());
        }
    }
}

impl Plugin for DeletePlugin {
    fn name(&self) -> &'static str {
        "delete"
    }

    fn on_event(
        &mut self,
        event: &FlowEvent,
        ctx: &mut crate::plugin::PluginContext,
    ) -> crate::plugin::EventResult {
        if let FlowEvent::Input(crate::plugin::InputEvent::KeyDown(ev)) = event
            && (ev.keystroke.key == "delete" || ev.keystroke.key == "backspace")
        {
            let cmd = DeleteCommand::new(ctx, self.policy);
            match cmd {
                Ok(cmd) => ctx.execute_command(cmd),
                Err(e) => {
                    ctx.emit(e.into());
                }
            }
            return crate::plugin::EventResult::Stop;
        }
        crate::plugin::EventResult::Continue
    }
}

/// Child promoted on delete ([`ParentDeletePolicy::Promote`]); stores local offset under the removed parent.
#[derive(Clone, Copy)]
struct PromotedChildSnapshot {
    parent_id: NodeId,
    child_id: NodeId,
    local: Point<gpui::Pixels>,
}

struct DeleteCommand {
    selected_edge: Vec<Edge>,
    originally_selected_edge_ids: HashSet<EdgeId>,
    selected_node: Vec<Node>,
    selected_port: Vec<Port>,
    policy: ParentDeletePolicy,
    /// Children reparented away when a selected parent is removed with promote policy.
    promoted_children: Vec<PromotedChildSnapshot>,
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

    fn new(
        ctx: &crate::plugin::PluginContext,
        policy: ParentDeletePolicy,
    ) -> Result<Self, GraphError> {
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

        Self::validate_delete_nodes(&selected_node, ctx.graph, policy)?;

        let to_remove = Self::deletion_set(&selected_node, ctx.graph, policy);
        let mut promoted_children = Vec::new();
        if matches!(policy, ParentDeletePolicy::Promote) {
            for node in &selected_node {
                for &child_id in node.children() {
                    if to_remove.contains(&child_id) {
                        continue;
                    }
                    let Some(child) = ctx.graph.get_node(&child_id) else {
                        continue;
                    };
                    promoted_children.push(PromotedChildSnapshot {
                        parent_id: node.id(),
                        child_id,
                        local: child.point(),
                    });
                }
            }
        }

        Ok(Self {
            selected_edge,
            originally_selected_edge_ids,
            selected_port: selected_node
                .iter()
                .flat_map(|node| node.inputs().iter().chain(node.outputs().iter()))
                .filter_map(|port_id| ctx.graph.get_port(port_id).cloned())
                .collect(),
            selected_node,
            policy,
            promoted_children,
        })
    }

    fn local_under_removed_parent(
        &self,
        parent_id: NodeId,
        child_id: NodeId,
    ) -> Option<Point<gpui::Pixels>> {
        self.promoted_children
            .iter()
            .find(|p| p.parent_id == parent_id && p.child_id == child_id)
            .map(|p| p.local)
            .or_else(|| {
                self.selected_node
                    .iter()
                    .find(|n| n.id() == child_id)
                    .map(|n| n.point())
            })
    }

    fn snapshot_depth(
        node: &Node,
        by_id: &HashMap<NodeId, &Node>,
        restored: &HashSet<NodeId>,
    ) -> usize {
        let mut depth = 0;
        let mut cur = node.parent();
        while let Some(p) = cur {
            if !restored.contains(&p) {
                break;
            }
            depth += 1;
            cur = by_id.get(&p).and_then(|n| n.parent());
        }
        depth
    }

    fn restore_node_hierarchy(&self, ctx: &mut crate::canvas::CommandContext) {
        let restored: HashSet<_> = self.selected_node.iter().map(|n| n.id()).collect();
        let by_id: HashMap<_, _> = self.selected_node.iter().map(|n| (n.id(), n)).collect();
        let mut nodes: Vec<_> = self.selected_node.iter().collect();
        nodes.sort_by_key(|n| Self::snapshot_depth(n, &by_id, &restored));

        for node in nodes {
            if let Some(parent) = node.parent()
                && ctx.graph.get_node(&parent).is_some()
            {
                ctx.graph.add_child(parent, node.id()).unwrap_or_else(|e| {
                    log::error!("restore deleted node parent: {e}");
                });
            }

            for &child_id in node.children() {
                if ctx.graph.get_node(&child_id).is_none() {
                    continue;
                }
                let Some(local) = self.local_under_removed_parent(node.id(), child_id) else {
                    continue;
                };
                if let Err(e) = ctx.graph.add_child(node.id(), child_id) {
                    log::error!("restore deleted node child: {e}");
                    continue;
                }
                if let Some(child) = ctx.graph.get_node_mut(&child_id) {
                    child.set_position_with_point(local);
                }
                ctx.port_offset_cache.clear_node(&child_id);
            }
            ctx.port_offset_cache.clear_node(&node.id());
        }
    }

    /// All node ids that `remove_node` will touch for the given selection and policy.
    fn deletion_set(
        nodes: &[Node],
        graph: &Graph,
        policy: ParentDeletePolicy,
    ) -> HashSet<crate::NodeId> {
        let mut set: HashSet<_> = nodes.iter().map(|n| n.id()).collect();
        if matches!(policy, ParentDeletePolicy::Cascade) {
            let mut stack: Vec<_> = set.iter().copied().collect();
            while let Some(id) = stack.pop() {
                let Some(node) = graph.get_node(&id) else {
                    continue;
                };
                for &child in node.children() {
                    if set.insert(child) {
                        stack.push(child);
                    }
                }
            }
        }
        set
    }

    fn validate_delete_nodes(
        nodes: &[Node],
        graph: &Graph,
        policy: ParentDeletePolicy,
    ) -> Result<(), GraphError> {
        if nodes.is_empty() {
            return Ok(());
        }

        let selected: HashSet<_> = nodes.iter().map(|n| n.id()).collect();
        let to_remove = Self::deletion_set(nodes, graph, policy);

        for id in &to_remove {
            graph.ensure_node(*id)?;
        }

        if matches!(policy, ParentDeletePolicy::Promote) {
            for node in nodes {
                graph.ensure_node(node.id())?;
                let children = graph
                    .get_node(&node.id())
                    .map(|n| n.children().to_vec())
                    .unwrap_or_default();
                for child in children {
                    if selected.contains(&child) {
                        continue;
                    }
                    graph.ensure_node(child)?;
                }
            }
        }

        Ok(())
    }
}

impl Command for DeleteCommand {
    fn name(&self) -> &'static str {
        "delete"
    }
    fn execute(&mut self, ctx: &mut crate::canvas::CommandContext) {
        ctx.remove_selected_edge();
        if let Err(e) = ctx.remove_selected_node(self.policy) {
            log::error!("failed to remove selected node: {e}");
        }
    }
    fn undo(&mut self, ctx: &mut crate::canvas::CommandContext) {
        for node in &self.selected_node {
            ctx.add_node(node.clone());
            ctx.add_selected_node(node.id(), true);
        }
        self.restore_node_hierarchy(ctx);

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

            let index = ctx.graph.node_order().iter().position(|v| *v == node.id());
            if let Some(index) = index {
                list.push(GraphOp::NodeOrderRemove { index })
            }
        }

        for port in &self.selected_port {
            list.push(GraphOp::RemovePort(port.id()));
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
mod tests {
    use std::collections::HashSet;

    use crate::{
        Command, CommandContext, Graph, ParentDeletePolicy, RendererRegistry, SharedState,
        Viewport, canvas::PortLayoutCache,
    };

    use super::DeleteCommand;

    fn with_ctx<R>(graph: &mut Graph, f: impl FnOnce(&mut CommandContext<'_>) -> R) -> R {
        let mut port_offset_cache = PortLayoutCache::new();
        let mut viewport = Viewport::new();
        let mut renderers = RendererRegistry::new();
        let mut shared_state = SharedState::new();
        let mut notify = || {};
        let mut ctx = CommandContext::new(
            graph,
            &mut port_offset_cache,
            &mut viewport,
            &mut renderers,
            &mut shared_state,
            &mut notify,
        );
        f(&mut ctx)
    }

    #[test]
    fn undo_delete_parent_restores_promoted_children() {
        let mut graph = Graph::new();
        let parent = graph.create_node("default").position(100.0, 100.0).build();
        let child = graph.create_node("default").position(12.0, 18.0).build();
        graph.add_child(parent, child).unwrap();
        graph.add_selected_node(parent, false);

        let selected_node = vec![graph.get_node(&parent).expect("parent").clone()];
        let to_remove =
            DeleteCommand::deletion_set(&selected_node, &graph, ParentDeletePolicy::Promote);
        let mut promoted_children = Vec::new();
        for node in &selected_node {
            for &child_id in node.children() {
                if to_remove.contains(&child_id) {
                    continue;
                }
                let c = graph.get_node(&child_id).expect("child");
                promoted_children.push(super::PromotedChildSnapshot {
                    parent_id: node.id(),
                    child_id,
                    local: c.point(),
                });
            }
        }
        let mut cmd = DeleteCommand {
            selected_edge: Vec::new(),
            originally_selected_edge_ids: HashSet::new(),
            selected_port: Vec::new(),
            selected_node,
            policy: ParentDeletePolicy::Promote,
            promoted_children,
        };

        with_ctx(&mut graph, |ctx| cmd.execute(ctx));
        assert!(graph.get_node(&parent).is_none());
        assert_eq!(graph.get_node(&child).unwrap().parent(), None);

        with_ctx(&mut graph, |ctx| cmd.undo(ctx));
        assert!(graph.get_node(&parent).is_some());
        assert_eq!(graph.get_node(&child).unwrap().parent(), Some(parent));
        assert_eq!(graph.get_node(&child).unwrap().point().x, gpui::px(12.0));
        assert_eq!(graph.get_node(&child).unwrap().point().y, gpui::px(18.0));
    }
}

#[cfg(test)]
mod command_interop_tests {
    use std::collections::HashSet;

    use crate::{Graph, ParentDeletePolicy, command_interop::assert_command_interop};

    use super::{DeleteCommand, PromotedChildSnapshot};

    fn delete_command_like_new(graph: &Graph, policy: ParentDeletePolicy) -> DeleteCommand {
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
        let to_remove = DeleteCommand::deletion_set(&selected_node, graph, policy);
        let mut promoted_children = Vec::new();
        if matches!(policy, ParentDeletePolicy::Promote) {
            for node in &selected_node {
                for &child_id in node.children() {
                    if to_remove.contains(&child_id) {
                        continue;
                    }
                    let Some(child) = graph.get_node(&child_id) else {
                        continue;
                    };
                    promoted_children.push(PromotedChildSnapshot {
                        parent_id: node.id(),
                        child_id,
                        local: child.point(),
                    });
                }
            }
        }
        DeleteCommand {
            selected_edge,
            originally_selected_edge_ids,
            selected_node,
            selected_port,
            policy,
            promoted_children,
        }
    }

    #[test]
    fn delete_command_interop_single_node_with_port() {
        let mut base = Graph::new();
        let src_id = base.create_node("x").position(-220.0, 0.0).output().build();
        let dst_id = base
            .create_node("x")
            .position(220.0, 0.0)
            .input()
            .output()
            .build();
        let other_id = base.create_node("x").position(440.0, 0.0).input().build();
        // Put selected node at the end so execute+undo preserves node_order with current command behavior.
        let selected_id = base
            .create_node("x")
            .position(0.0, 0.0)
            .input()
            .output()
            .build();

        let selected_node = base.get_node(&selected_id).expect("selected node").clone();
        let src_node = base.get_node(&src_id).expect("src node").clone();
        let dst_node = base.get_node(&dst_id).expect("dst node").clone();
        let other_node = base.get_node(&other_id).expect("other node").clone();

        // This edge is NOT selected, but should be deleted via node-cascade.
        let _cascade_in = base
            .create_edge()
            .source(src_node.outputs()[0])
            .target(selected_node.inputs()[0])
            .build();
        // This edge IS selected and also touches selected node.
        let selected_edge = base
            .create_edge()
            .source(selected_node.outputs()[0])
            .target(dst_node.inputs()[0])
            .build();
        // Unrelated edge should remain untouched.
        let _unrelated = base
            .create_edge()
            .source(dst_node.outputs()[0])
            .target(other_node.inputs()[0])
            .build();

        base.add_selected_node(selected_id, false);
        base.add_selected_edge(selected_edge, true);

        let cmd = delete_command_like_new(&base, ParentDeletePolicy::Promote);
        assert_command_interop(
            &base,
            || {
                Box::new(DeleteCommand {
                    selected_edge: cmd.selected_edge.clone(),
                    originally_selected_edge_ids: cmd.originally_selected_edge_ids.clone(),
                    selected_node: cmd.selected_node.clone(),
                    selected_port: cmd.selected_port.clone(),
                    policy: ParentDeletePolicy::Promote,
                    promoted_children: cmd.promoted_children.clone(),
                })
            },
            "DeleteCommand",
        );
    }
}
