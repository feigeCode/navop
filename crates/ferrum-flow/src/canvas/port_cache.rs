use std::collections::HashMap;

use gpui::{Pixels, Point};

use crate::{EdgeId, Graph, NodeId, PortId, RendererRegistry};

#[derive(Debug, Clone)]
pub struct PortLayoutCache {
    map: HashMap<NodeId, HashMap<PortId, Point<Pixels>>>,
}

impl PortLayoutCache {
    pub(crate) fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    pub fn get_offset(&self, node_id: &NodeId, port_id: &PortId) -> Option<Point<Pixels>> {
        self.map.get(node_id)?.get(port_id).copied()
    }

    pub fn is_node_cached(&self, node_id: &NodeId) -> bool {
        self.map.contains_key(node_id)
    }

    /// Port ids whose offsets are cached for `node_id` (after [`Self::ensure_node_ports`]).
    ///
    /// Order follows the inner [`HashMap`] and is not guaranteed stable across runs.
    pub fn cached_port_ids_for_node(&self, node_id: &NodeId) -> impl Iterator<Item = PortId> + '_ {
        self.map
            .get(node_id)
            .into_iter()
            .flat_map(|ports| ports.keys().copied())
    }

    pub fn replace_node_offsets(
        &mut self,
        node_id: NodeId,
        offsets: HashMap<PortId, Point<Pixels>>,
    ) {
        self.map.insert(node_id, offsets);
    }

    pub fn clear_node(&mut self, node_id: &NodeId) {
        self.map.remove(node_id);
    }

    pub fn clear_all(&mut self) {
        self.map.clear();
    }

    /// Fill port layout for `node_id` if not already cached.
    pub fn ensure_node_ports(
        &mut self,
        graph: &Graph,
        renderers: &RendererRegistry,
        node_id: &NodeId,
    ) {
        if self.is_node_cached(node_id) {
            return;
        }

        let Some(node) = graph.get_node(node_id) else {
            return;
        };

        let renderer = renderers.get(node.renderer_key());

        let mut result = HashMap::new();

        for port in graph.ports_values().filter(|p| p.node_id() == node.id()) {
            let pos = renderer.port_offset(node, port, graph);
            result.insert(port.id(), pos);
        }

        self.replace_node_offsets(node.id(), result);
    }

    /// Fill port layout for every node if not already cached.
    pub fn ensure_all_nodes_ports(&mut self, graph: &Graph, renderers: &RendererRegistry) {
        let node_ids = graph.nodes().keys().copied();

        for node_id in node_ids {
            self.ensure_node_ports(graph, renderers, &node_id);
        }
    }

    /// Ensure both endpoint nodes of the edge have port layout cached.
    pub fn ensure_edge_ports(
        &mut self,
        graph: &Graph,
        renderers: &RendererRegistry,
        edge_id: &EdgeId,
    ) {
        let Some(edge) = graph.get_edge(edge_id) else {
            return;
        };

        self.ensure_node_ports_for_port(graph, renderers, &edge.source_port);
        self.ensure_node_ports_for_port(graph, renderers, &edge.target_port);
    }

    /// Ensure the node that owns `port_id` has port layout cached.
    pub fn ensure_node_ports_for_port(
        &mut self,
        graph: &Graph,
        renderers: &RendererRegistry,
        port_id: &PortId,
    ) {
        let Some(port) = graph.get_port(port_id) else {
            return;
        };

        self.ensure_node_ports(graph, renderers, &port.node_id());
    }
}
