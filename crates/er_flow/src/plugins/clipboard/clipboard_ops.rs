use std::collections::{HashMap, HashSet};

use gpui::px;

use crate::{CompositeCommand, Edge, Graph, Node, NodeId, Port, plugin::PluginContext};

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
    if graph.selected_node.is_empty() {
        return None;
    }
    let selected = &graph.selected_node;
    let node_ids: Vec<NodeId> = graph
        .node_order()
        .iter()
        .filter(|id| selected.contains(id))
        .copied()
        .collect();
    if node_ids.is_empty() {
        return None;
    }

    let mut port_ids = HashSet::new();
    let mut nodes = Vec::new();
    for nid in &node_ids {
        let n = graph.nodes.get(nid)?.clone();
        for pid in n.inputs.iter().chain(n.outputs.iter()) {
            port_ids.insert(*pid);
        }
        nodes.push(n);
    }

    let mut ports = Vec::new();
    for nid in &node_ids {
        let n = graph.nodes.get(nid)?;
        for pid in n.inputs.iter().chain(n.outputs.iter()) {
            if let Some(p) = graph.ports.get(pid) {
                ports.push(p.clone());
            }
        }
    }

    let mut edges = Vec::new();
    for e in graph.edges.values() {
        if port_ids.contains(&e.source_port) && port_ids.contains(&e.target_port) {
            edges.push(e.clone());
        }
    }

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
        node_map.insert(n.id, ctx.graph.next_node_id());
    }
    let mut port_map = HashMap::new();
    for p in &sub.ports {
        port_map.insert(p.id, ctx.graph.next_port_id());
    }

    let mut composite = CompositeCommand::new();

    let mut new_node_ids = Vec::new();

    for old in &sub.nodes {
        let new_id = node_map[&old.id];
        let node = Node {
            id: new_id,
            node_type: old.node_type.clone(),
            execute_type: old.execute_type.clone(),
            x: old.x + offset,
            y: old.y + offset,
            size: old.size,
            inputs: old.inputs.iter().map(|p| port_map[p]).collect(),
            outputs: old.outputs.iter().map(|p| port_map[p]).collect(),
            data: old.data.clone(),
        };
        new_node_ids.push(new_id);
        composite.push(CreateNode::new(node));
    }

    for old in &sub.ports {
        let port = Port {
            id: port_map[&old.id],
            kind: old.kind,
            index: old.index,
            node_id: node_map[&old.node_id],
            position: old.position,
            size: old.size,
        };
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

    let pasted_ids: Vec<NodeId> = sub.nodes.iter().map(|n| node_map[&n.id]).collect();

    ctx.execute_command(composite);
    ctx.clear_selected_edge();
    ctx.clear_selected_node();
    for (i, nid) in pasted_ids.iter().enumerate() {
        ctx.add_selected_node(*nid, i != 0);
    }
    ctx.cache_port_offset_with_node(&new_node_ids);
}
