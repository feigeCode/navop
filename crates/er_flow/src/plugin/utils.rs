use std::collections::HashMap;

use gpui::{KeyDownEvent, Pixels, Point};

use crate::{
    Edge, EdgeId, Graph, GraphChangeKind, NodeId, Port, PortId, RendererRegistry, Viewport,
    canvas::PortLayoutCache,
};

/// Clears [`PortLayoutCache`] entries affected by an incoming graph change. Call **before**
/// [`Graph::apply`](crate::graph::Graph::apply) so `PortRemoved` can still resolve `node_id`.
pub fn invalidate_port_layout_cache_for_graph_change(
    cache: &mut PortLayoutCache,
    graph: &Graph,
    kind: &GraphChangeKind,
) {
    match kind {
        GraphChangeKind::NodeRemoved { id } => cache.clear_node(id),
        GraphChangeKind::NodeAdded(node) => cache.clear_node(&node.id),
        GraphChangeKind::NodeSetWidthed { id, .. }
        | GraphChangeKind::NodeSetHeighted { id, .. }
        | GraphChangeKind::NodeDataUpdated { id, .. } => cache.clear_node(id),
        GraphChangeKind::PortAdded(port) => cache.clear_node(&port.node_id),
        GraphChangeKind::PortRemoved { id } => {
            if let Some(p) = graph.ports.get(id) {
                cache.clear_node(&p.node_id);
            }
        }
        GraphChangeKind::NodeMoved { .. }
        | GraphChangeKind::NodeOrderUpdate(_)
        | GraphChangeKind::EdgeAdded(_)
        | GraphChangeKind::EdgeRemoved { .. }
        | GraphChangeKind::RedrawRequested => {}
        GraphChangeKind::Batch(changes) => {
            for c in changes {
                invalidate_port_layout_cache_for_graph_change(cache, graph, c);
            }
        }
    }
}

/// Primary shortcut modifier: ⌘ on macOS, Ctrl on other platforms.
pub fn primary_platform_modifier(ev: &KeyDownEvent) -> bool {
    #[cfg(target_os = "macos")]
    {
        ev.keystroke.modifiers.platform
    }
    #[cfg(not(target_os = "macos"))]
    {
        ev.keystroke.modifiers.control
    }
}

pub fn is_node_visible(graph: &Graph, viewport: &Viewport, node_id: &NodeId) -> bool {
    let Some(node) = graph.get_node(node_id) else {
        return false;
    };

    viewport.is_node_visible(node)
}

pub fn is_edge_visible(graph: &Graph, viewport: &Viewport, edge: &Edge) -> bool {
    let Edge {
        source_port,
        target_port,
        ..
    } = edge;

    let Some(Port { node_id: n1, .. }) = graph.ports.get(source_port) else {
        return false;
    };

    let Some(Port { node_id: n2, .. }) = graph.ports.get(target_port) else {
        return false;
    };

    let node1_visible = graph
        .get_node(n1)
        .map(|n| viewport.is_node_visible(n))
        .unwrap_or(false);

    let node2_visible = graph
        .get_node(n2)
        .map(|n| viewport.is_node_visible(n))
        .unwrap_or(false);

    node1_visible || node2_visible
}

pub fn port_offset_cached(
    cache: &PortLayoutCache,
    node_id: &NodeId,
    port_id: &PortId,
) -> Option<Point<Pixels>> {
    cache.map.get(node_id)?.get(port_id).copied()
}

pub fn cache_node_port_offset(
    graph: &Graph,
    renderers: &RendererRegistry,
    cache: &mut PortLayoutCache,
    node_id: &NodeId,
) {
    if cache.map.contains_key(node_id) {
        return;
    }

    let Some(node) = graph.get_node(node_id) else {
        return;
    };

    let renderer = renderers.get(&node.node_type);

    let mut result = HashMap::new();

    for port in graph.ports.values().filter(|p| p.node_id == node.id) {
        let pos = renderer.port_offset(node, port, graph);
        result.insert(port.id, pos);
    }

    cache.map.insert(node.id, result);
}

pub fn cache_all_node_port_offset(
    graph: &Graph,
    renderers: &RendererRegistry,
    cache: &mut PortLayoutCache,
) {
    let node_ids: Vec<NodeId> = graph.nodes().iter().map(|(id, _)| *id).collect();

    for node_id in node_ids {
        cache_node_port_offset(graph, renderers, cache, &node_id);
    }
}

pub fn cache_port_offset_with_edge(
    graph: &Graph,
    renderers: &RendererRegistry,
    cache: &mut PortLayoutCache,
    edge_id: &EdgeId,
) {
    let Some(edge) = graph.edges.get(edge_id) else {
        return;
    };

    cache_port_offset_with_port(graph, renderers, cache, &edge.source_port);
    cache_port_offset_with_port(graph, renderers, cache, &edge.target_port);
}

pub fn cache_port_offset_with_port(
    graph: &Graph,
    renderers: &RendererRegistry,
    cache: &mut PortLayoutCache,
    port_id: &PortId,
) {
    let Some(port) = graph.ports.get(port_id) else {
        return;
    };

    cache_node_port_offset(graph, renderers, cache, &port.node_id);
}
