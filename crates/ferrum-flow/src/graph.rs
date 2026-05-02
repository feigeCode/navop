use std::collections::hash_map::Values as HashMapValues;
use std::collections::hash_set::Iter as HashSetIter;
use std::collections::{HashMap, HashSet};

use gpui::{Bounds, Pixels, Point, Size, px};
use serde::{Deserialize, Serialize};

use crate::edge::{Edge, EdgeId};
use crate::node::{Node, NodeId, Port, PortId};
use crate::{EdgeBuilder, NodeBuilder, PortKind, PortPosition, Viewport};

mod store;

pub use store::{ChangeSource, GraphChange, GraphChangeKind, GraphOp};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Graph {
    nodes: HashMap<NodeId, Node>,
    node_order: Vec<NodeId>,
    ports: HashMap<PortId, Port>,

    edges: HashMap<EdgeId, Edge>,

    selected_edge: HashSet<EdgeId>,
    selected_node: HashSet<NodeId>,
}

impl Default for Graph {
    fn default() -> Self {
        Self::new()
    }
}

impl Graph {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            node_order: vec![],
            ports: HashMap::new(),
            edges: HashMap::new(),
            selected_edge: HashSet::new(),
            selected_node: HashSet::new(),
        }
    }

    pub fn from_json(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }

    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string(self)
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
            && self.ports.is_empty()
            && self.edges.is_empty()
            && self.node_order.is_empty()
    }

    pub fn apply(&mut self, op: GraphChangeKind) {
        match op {
            GraphChangeKind::NodeAdded(node) => self.add_node(node),
            GraphChangeKind::NodeRemoved { id } => self.remove_node(&id),
            GraphChangeKind::NodeMoved { id, x, y } => {
                if let Some(node) = self.nodes.get_mut(&id) {
                    node.set_position(px(x), px(y));
                }
            }
            GraphChangeKind::NodeSetWidthed { id, width } => {
                if let Some(node) = self.nodes.get_mut(&id) {
                    node.set_size_width(px(width));
                }
            }
            GraphChangeKind::NodeSetHeighted { id, height } => {
                if let Some(node) = self.nodes.get_mut(&id) {
                    node.set_size_height(px(height));
                }
            }
            GraphChangeKind::NodeDataUpdated { id, data } => {
                if let Some(node) = self.nodes.get_mut(&id) {
                    node.set_data(data);
                }
            }
            GraphChangeKind::NodeOrderUpdate(vec) => {
                self.node_order = vec;
            }
            GraphChangeKind::PortAdded(port) => self.add_port(port),
            GraphChangeKind::PortRemoved { id } => {
                self.remove_port(&id);
            }
            GraphChangeKind::EdgeAdded(edge) => self.add_edge(edge),
            GraphChangeKind::EdgeRemoved { id } => self.remove_edge(&id),
            GraphChangeKind::RedrawRequested => {}
            GraphChangeKind::Batch(graph_change_kinds) => {
                for change in graph_change_kinds {
                    self.apply(change);
                }
            }
        }
    }

    pub fn create_node(&mut self, renderer_key: &str) -> NodeBuilder<'_> {
        NodeBuilder::new(renderer_key).graph(self)
    }

    pub fn create_edge(&mut self) -> EdgeBuilder<'_> {
        EdgeBuilder::new().graph(self)
    }

    #[deprecated(note = "use `Graph::create_edge`")]
    pub fn create_dege(&mut self) -> EdgeBuilder<'_> {
        EdgeBuilder::new().graph(self)
    }

    pub fn next_node_id(&self) -> NodeId {
        NodeId::new()
    }

    pub fn next_port_id(&self) -> PortId {
        PortId::new()
    }

    pub fn next_edge_id(&self) -> EdgeId {
        EdgeId::new()
    }

    pub fn add_node(&mut self, node: Node) {
        let node_id = node.id();
        self.nodes.insert(node.id(), node);
        self.node_order.push(node_id);
    }
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn add_node_without_order(&mut self, node: Node) {
        self.nodes.insert(node.id(), node);
    }

    pub fn add_port(&mut self, port: Port) {
        let map = &mut self.ports;
        map.insert(port.id(), port);
    }

    pub fn remove_port(&mut self, id: &PortId) {
        self.ports.remove(id);
    }

    pub fn nodes(&self) -> &HashMap<NodeId, Node> {
        &self.nodes
    }

    pub fn node_order(&self) -> &Vec<NodeId> {
        &self.node_order
    }
    pub fn node_order_mut(&mut self) -> &mut Vec<NodeId> {
        &mut self.node_order
    }
    pub fn ports(&self) -> &HashMap<PortId, Port> {
        &self.ports
    }
    pub fn get_port(&self, id: &PortId) -> Option<&Port> {
        self.ports.get(id)
    }
    pub fn ports_values(&self) -> HashMapValues<'_, PortId, Port> {
        self.ports.values()
    }
    pub fn edges(&self) -> &HashMap<EdgeId, Edge> {
        &self.edges
    }
    pub fn get_edge(&self, id: &EdgeId) -> Option<&Edge> {
        self.edges.get(id)
    }
    pub fn edges_values(&self) -> HashMapValues<'_, EdgeId, Edge> {
        self.edges.values()
    }
    pub fn selected_node(&self) -> &HashSet<NodeId> {
        &self.selected_node
    }
    pub fn selected_node_is_empty(&self) -> bool {
        self.selected_node.is_empty()
    }
    pub fn selected_node_iter(&self) -> HashSetIter<'_, NodeId> {
        self.selected_node.iter()
    }
    pub fn selected_edge(&self) -> &HashSet<EdgeId> {
        &self.selected_edge
    }
    pub fn selected_edge_iter(&self) -> HashSetIter<'_, EdgeId> {
        self.selected_edge.iter()
    }
    pub fn set_selected_node(&mut self, selected: HashSet<NodeId>) {
        self.selected_node = selected;
    }
    pub fn set_selected_edge(&mut self, selected: HashSet<EdgeId>) {
        self.selected_edge = selected;
    }

    pub fn new_edge(&self) -> Edge {
        Edge::new()
    }

    pub fn add_edge(&mut self, edge: Edge) {
        self.edges.insert(edge.id, edge);
    }

    pub fn remove_edge(&mut self, edge_id: &EdgeId) {
        self.edges.remove(edge_id);
        self.selected_edge.remove(edge_id);
    }

    pub fn get_node(&self, id: &NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    pub fn get_node_mut(&mut self, id: &NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id)
    }

    pub fn remove_node(&mut self, id: &NodeId) {
        let Some(node) = &self.nodes.get(id) else {
            return;
        };

        let mut edge_ids_to_remove = HashSet::new();
        for port_id in node.inputs().iter().chain(node.outputs().iter()).copied() {
            edge_ids_to_remove.extend(
                self.edges
                    .iter()
                    .filter(|(_, edge)| edge.source_port == port_id || edge.target_port == port_id)
                    .map(|(id, _)| *id),
            );
            self.ports.remove(&port_id);
        }
        for edge_id in edge_ids_to_remove {
            self.remove_edge(&edge_id);
        }

        self.nodes.remove(id);
        self.selected_node.remove(id);
        let index = self.node_order.iter().position(|v| *v == *id);
        if let Some(index) = index {
            self.node_order.remove(index);
        }
    }

    pub fn add_selected_node(&mut self, id: NodeId, shift: bool) {
        if shift {
            if self.selected_node.contains(&id) {
                self.selected_node.remove(&id);
            } else {
                self.selected_node.insert(id);
            }
        } else {
            self.selected_node.clear();
            self.selected_node.insert(id);
        }
    }
    pub fn clear_selected_node(&mut self) {
        self.selected_node.clear();
    }

    pub fn remove_selected_node(&mut self) -> bool {
        if self.selected_node.is_empty() {
            return false;
        }

        let mut ids = vec![];
        for id in self.selected_node.iter() {
            ids.push(*id);
        }
        for id in ids.iter() {
            self.remove_node(id);
        }
        self.selected_node.clear();
        true
    }

    pub fn add_selected_edge(&mut self, id: EdgeId, shift: bool) {
        if shift {
            if self.selected_edge.contains(&id) {
                self.selected_edge.remove(&id);
            } else {
                self.selected_edge.insert(id);
            }
        } else {
            self.selected_edge.clear();
            self.selected_edge.insert(id);
        }
    }
    pub fn clear_selected_edge(&mut self) {
        self.selected_edge.clear();
    }

    pub fn remove_selected_edge(&mut self) -> bool {
        if self.selected_edge.is_empty() {
            return false;
        }

        let mut ids = vec![];
        for id in self.selected_edge.iter() {
            ids.push(*id);
        }
        for id in ids.iter() {
            self.edges.remove(id);
        }
        self.selected_edge.clear();
        true
    }

    pub fn selection_bounds(&self) -> Option<Bounds<Pixels>> {
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;

        let mut found = false;

        for id in &self.selected_node {
            let node = &self.nodes.get(id)?;
            let (x, y) = node.position();
            let size = *node.size_ref();

            min_x = min_x.min(x.into());
            min_y = min_y.min(y.into());

            max_x = max_x.max((x + size.width).into());
            max_y = max_y.max((y + size.height).into());

            found = true;
        }

        if !found {
            return None;
        }

        Some(Bounds::new(
            Point::new(px(min_x), px(min_y)),
            Size::new(px(max_x - min_x), px(max_y - min_y)),
        ))
    }

    pub fn selected_nodes_with_positions(&self) -> HashMap<NodeId, Point<Pixels>> {
        self.selected_node
            .iter()
            .filter_map(|id| {
                let n = &self.nodes.get(id)?;
                Some((*id, n.point()))
            })
            .collect()
    }

    pub fn hit_node(&self, mouse: Point<Pixels>, viewport: &Viewport) -> Option<NodeId> {
        self.nodes
            .iter()
            .filter(|(_, node)| viewport.is_node_visible(node))
            .find(|(_, n)| n.bounds().contains(&mouse))
            .map(|(id, _)| *id)
    }

    pub fn bring_node_to_front(&mut self, node_id: NodeId) {
        if let Some(index) = self.node_order_mut().iter().position(|id| *id == node_id) {
            self.node_order_mut().remove(index);
        }

        self.node_order_mut().push(node_id);
    }

    pub fn ports_on_node_side(
        &self,
        node_id: NodeId,
        kind: PortKind,
        position: PortPosition,
    ) -> Vec<&Port> {
        self.ports
            .values()
            .filter(|p| p.node_id() == node_id && p.kind() == kind && p.position() == position)
            .collect()
    }
}
