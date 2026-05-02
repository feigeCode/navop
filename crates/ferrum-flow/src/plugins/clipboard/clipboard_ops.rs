use std::collections::{HashMap, HashSet};

use gpui::px;

use crate::{CompositeCommand, Edge, Graph, Node, Port, plugin::PluginContext};

use super::copied_subgraph::CopiedSubgraph;
use crate::plugins::{CreateEdge, CreateNode, CreatePort};

#[derive(Clone)]
pub(crate) struct ClipboardShared(pub CopiedSubgraph);

pub(crate) fn set_clipboard_subgraph(ctx: &mut PluginContext, sub: CopiedSubgraph) {
    ctx.shared_state.insert(ClipboardShared(sub));
}

pub(crate) fn get_clipboard_subgraph(ctx: &PluginContext) -> Option<CopiedSubgraph> {
    ctx.shared_state
        .get::<ClipboardShared>()
        .map(|s| s.0.clone())
}

pub(crate) fn has_clipboard_subgraph(ctx: &PluginContext) -> bool {
    ctx.shared_state.contains::<ClipboardShared>()
}

pub(crate) fn extract_subgraph(graph: &Graph) -> Option<CopiedSubgraph> {
    if graph.selected_node_is_empty() {
        return None;
    }
    let node_ids = graph.selected_node();
    if node_ids.is_empty() {
        return None;
    }

    let mut port_ids = HashSet::new();
    let mut nodes = Vec::with_capacity(node_ids.len());
    let mut ports = Vec::new();
    for nid in node_ids {
        let n = graph.get_node(nid)?;
        for pid in n.inputs().iter().chain(n.outputs().iter()) {
            port_ids.insert(*pid);
            if let Some(p) = graph.get_port(pid) {
                ports.push(p.clone());
            }
        }
        nodes.push(n.clone());
    }

    let edges = graph
        .edges_values()
        .filter(|e| port_ids.contains(&e.source_port) && port_ids.contains(&e.target_port))
        .cloned()
        .collect();

    Some(CopiedSubgraph {
        nodes,
        ports,
        edges,
    })
}

pub(crate) fn paste_subgraph(ctx: &mut PluginContext, sub: &CopiedSubgraph) {
    const OFFSET: f32 = 40.0;
    let offset = px(OFFSET);

    let mut node_map = HashMap::new();
    for n in &sub.nodes {
        node_map.insert(n.id(), ctx.graph.next_node_id());
    }
    let mut port_map = HashMap::new();
    for p in &sub.ports {
        port_map.insert(p.id(), ctx.graph.next_port_id());
    }

    let mut composite = CompositeCommand::new();

    let mut new_node_ids = Vec::new();

    for old in &sub.nodes {
        let new_id = node_map[&old.id()];
        let (x, y) = old.position();
        let mut node = Node::new((x + offset).into(), (y + offset).into());
        node.set_renderer_key(old.renderer_key());
        node.set_execute_type(old.execute_type_ref());
        node.set_size_mut(*old.size_ref());
        node.set_data(old.data_ref().clone());
        node.set_id(new_id);

        for pid in old.inputs() {
            node.push_input(port_map[pid]);
        }
        for pid in old.outputs() {
            node.push_output(port_map[pid]);
        }
        new_node_ids.push(new_id);
        composite.push(CreateNode::new(node));
    }

    for old in &sub.ports {
        let port = Port::new(
            port_map[&old.id()],
            old.kind(),
            old.index(),
            node_map[&old.node_id()],
            old.position(),
            *old.size_ref(),
            old.port_type_ref().clone(),
        );
        composite.push(CreatePort::new(port));
    }

    for old in &sub.edges {
        let edge = Edge {
            id: ctx.graph.next_edge_id(),
            source_port: port_map[&old.source_port],
            target_port: port_map[&old.target_port],
        };
        composite.push(CreateEdge::new(edge));
    }

    let pasted_ids = sub.nodes.iter().map(|n| node_map[&n.id()]);

    ctx.execute_command(composite);
    ctx.clear_selected_edge();
    ctx.clear_selected_node();
    for nid in pasted_ids {
        ctx.add_selected_node(nid, true);
    }
    ctx.cache_port_offset_with_node(&new_node_ids);
}
