use gpui::KeyDownEvent;

use crate::{Edge, Graph, GraphChangeKind, NodeId, Viewport, canvas::PortLayoutCache};

/// Clears [`PortLayoutCache`] entries affected by an incoming graph change. Call **before**
/// [`Graph::apply`](crate::graph::Graph::apply) so `PortRemoved` can still resolve `node_id`.
pub fn invalidate_port_layout_cache_for_graph_change(
    cache: &mut PortLayoutCache,
    graph: &Graph,
    kind: &GraphChangeKind,
) {
    match kind {
        GraphChangeKind::NodeRemoved { id } => cache.clear_node(id),
        GraphChangeKind::NodeAdded(node) => cache.clear_node(&node.id()),
        GraphChangeKind::NodeSetWidthed { id, .. }
        | GraphChangeKind::NodeSetHeighted { id, .. }
        | GraphChangeKind::NodeDataUpdated { id, .. } => cache.clear_node(id),
        GraphChangeKind::PortAdded(port) => cache.clear_node(&port.node_id()),
        GraphChangeKind::PortRemoved { id } => {
            if let Some(p) = graph.get_port(id) {
                cache.clear_node(&p.node_id());
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

    let Some(port) = graph.get_port(source_port) else {
        return false;
    };
    let n1 = port.node_id();

    let Some(port) = graph.get_port(target_port) else {
        return false;
    };
    let n2 = port.node_id();

    let node1_visible = graph
        .get_node(&n1)
        .map(|n| viewport.is_node_visible(n))
        .unwrap_or(false);

    let node2_visible = graph
        .get_node(&n2)
        .map(|n| viewport.is_node_visible(n))
        .unwrap_or(false);

    node1_visible || node2_visible
}
