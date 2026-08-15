use std::collections::hash_map::Values as HashMapValues;
use std::collections::hash_set::Iter as HashSetIter;
use std::collections::{HashMap, HashSet};

use gpui::{Bounds, Pixels, Point, Size, px};
use serde::{Deserialize, Serialize};

use crate::edge::{Edge, EdgeBuilderInGraph, EdgeId};
use crate::plugin::CanvasMessage;
use crate::{EdgeBuilder, FlowEvent, Viewport};
use node::{Node, NodeBuilder, NodeBuilderInGraph, NodeId, Port, PortId, PortKind, PortPosition};

pub mod node;
mod store;

pub use store::{ChangeSource, GraphChange, GraphChangeKind, GraphOp};

/// Hierarchy and graph invariant violations when linking parent/child nodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphError {
    /// A referenced node id is not in the graph.
    NodeNotFound(NodeId),
    /// A node cannot be its own parent or child.
    SelfReference { node: NodeId },
    /// Linking `child` under `parent` would create a cycle in the tree.
    WouldCreateCycle { parent: NodeId, child: NodeId },
    /// `child` is not a direct child of `parent`.
    NotParentChild { parent: NodeId, child: NodeId },
}

impl std::fmt::Display for GraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphError::NodeNotFound(id) => write!(f, "node not found: {id}"),
            GraphError::SelfReference { node } => write!(f, "node cannot reference itself: {node}"),
            GraphError::WouldCreateCycle { parent, child } => {
                write!(f, "would create cycle: parent {parent}, child {child}")
            }
            GraphError::NotParentChild { parent, child } => {
                write!(f, "node {child} is not a child of {parent}")
            }
        }
    }
}

impl std::error::Error for GraphError {}

impl From<GraphError> for CanvasMessage {
    fn from(error: GraphError) -> Self {
        CanvasMessage::error(error.to_string()).with_source(error)
    }
}

impl From<GraphError> for FlowEvent {
    fn from(error: GraphError) -> Self {
        FlowEvent::Message(error.into())
    }
}

/// Source for [`crate::canvas::FlowCanvas::builder`]: an owned [`Graph`] or `Result<Graph, GraphError>`.
pub trait IntoFlowGraph: private::Sealed {
    fn into_flow_graph(self) -> Result<Graph, GraphError>;
}

mod private {
    use super::*;

    pub trait Sealed {}

    impl Sealed for Graph {}
    impl Sealed for Result<Graph, GraphError> {}
}

impl IntoFlowGraph for Graph {
    fn into_flow_graph(self) -> Result<Graph, GraphError> {
        Ok(self)
    }
}

impl IntoFlowGraph for Result<Graph, GraphError> {
    fn into_flow_graph(self) -> Result<Graph, GraphError> {
        self
    }
}

/// Walks from `node`'s parent upward (excludes `node` itself).
struct AncestorsIter<'a> {
    graph: &'a Graph,
    next: Option<NodeId>,
}

impl Iterator for AncestorsIter<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<Self::Item> {
        let id = self.next?;
        self.next = self.graph.nodes.get(&id).and_then(|n| n.parent());
        Some(id)
    }
}

/// Pre-order walk of one subtree from `root` (includes `root`); sibling order follows
/// [`Graph::children_index`].
pub struct PaintPreorderIter<'a> {
    graph: &'a Graph,
    stack: Vec<NodeId>,
}

impl<'a> PaintPreorderIter<'a> {
    fn new(graph: &'a Graph, root: NodeId) -> Self {
        Self {
            graph,
            stack: vec![root],
        }
    }
}

impl Iterator for PaintPreorderIter<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<Self::Item> {
        let id = self.stack.pop()?;
        for &child in self.graph.children_of(id).iter().rev() {
            self.stack.push(child);
        }
        Some(id)
    }
}

/// Depth-first draw order from [`Graph::roots`] and [`Graph::children_index`].
///
/// Same sequence as [`Graph::paint_order`]: each node before its descendants; sibling order
/// follows `children_index`.
pub struct PaintOrderIter<'a> {
    graph: &'a Graph,
    roots: std::slice::Iter<'a, NodeId>,
    subtree: Option<PaintPreorderIter<'a>>,
}

impl Iterator for PaintOrderIter<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.subtree.is_none() {
                let root = self.roots.next()?;
                self.subtree = Some(PaintPreorderIter::new(self.graph, *root));
            }
            if let Some(id) = self.subtree.as_mut()?.next() {
                return Some(id);
            }
            self.subtree = None;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParentDeletePolicy {
    /// Delete the parent node and all its children.
    Cascade,
    /// Promote the children to the parent's level and delete the parent.
    Promote,
}

impl std::fmt::Display for ParentDeletePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParentDeletePolicy::Cascade => write!(f, "cascade"),
            ParentDeletePolicy::Promote => write!(f, "promote"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Graph {
    nodes: HashMap<NodeId, Node>,
    node_order: Vec<NodeId>,
    ports: HashMap<PortId, Port>,
    /// Map of node id to its children node ids
    children_index: HashMap<NodeId, Vec<NodeId>>,
    /// List of root node ids
    roots: Vec<NodeId>,

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
            children_index: HashMap::new(),
            roots: vec![],
            edges: HashMap::new(),
            selected_edge: HashSet::new(),
            selected_node: HashSet::new(),
        }
    }

    /// Runs `f` on a fresh graph and returns it. Useful with [`crate::NodeBuilderInGraph::build_with_ports`]
    /// so node/edge setup stays inside one closure.
    pub fn build(f: impl FnOnce(&mut Self)) -> Self {
        let mut g = Self::new();
        f(&mut g);
        g
    }

    pub fn try_build(
        f: impl FnOnce(&mut Self) -> Result<(), GraphError>,
    ) -> Result<Self, GraphError> {
        let mut g = Self::new();
        f(&mut g)?;
        Ok(g)
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

    pub fn apply(&mut self, op: GraphChangeKind) -> Result<(), GraphError> {
        match op {
            GraphChangeKind::NodeAdded(node) => self.add_node(node),
            GraphChangeKind::NodeRemoved { id } => {
                self.remove_node(&id, ParentDeletePolicy::Promote)?
            }
            GraphChangeKind::NodeRemovedWithPolicy { id, policy } => {
                self.remove_node(&id, policy)?
            }
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
            GraphChangeKind::NodeParentChanged { id, parent } => {
                self.reparent(id, parent)?;
            }
            GraphChangeKind::NodePushedChild { id, child_id } => self.add_child(id, child_id)?,
            GraphChangeKind::NodePoppedChild { id, child_id } => self.remove_child(id, child_id),
            GraphChangeKind::PortAdded(port) => self.add_port(port),
            GraphChangeKind::PortRemoved { id } => {
                self.remove_port(&id);
            }
            GraphChangeKind::EdgeAdded(edge) => self.add_edge(edge),
            GraphChangeKind::EdgeRemoved { id } => self.remove_edge(&id),
            GraphChangeKind::RedrawRequested => {}
            GraphChangeKind::Batch(graph_change_kinds) => {
                for change in graph_change_kinds {
                    self.apply(change)?;
                }
            }
        }

        Ok(())
    }

    pub fn create_node(&mut self, renderer_key: &str) -> NodeBuilderInGraph<'_> {
        NodeBuilder::new(renderer_key).graph(self)
    }

    pub fn create_edge(&mut self) -> EdgeBuilderInGraph<'_> {
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

    pub fn add_node(&mut self, mut node: Node) {
        let node_id = node.id();
        node.set_parent(None);
        node.clear_children();
        self.nodes.insert(node_id, node);
        self.node_order.push(node_id);
        self.children_index.entry(node_id).or_default();
        self.roots_push(node_id);
    }

    fn roots_push(&mut self, node_id: NodeId) {
        if !self.roots.contains(&node_id) {
            self.roots.push(node_id);
        }
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

    /// Nodes with no parent (top-level / canvas roots).
    pub fn roots(&self) -> &[NodeId] {
        &self.roots
    }

    pub fn children_index(&self) -> &HashMap<NodeId, Vec<NodeId>> {
        &self.children_index
    }

    /// Direct children of `parent` (empty if unknown parent or no children).
    pub fn children_of(&self, parent: NodeId) -> &[NodeId] {
        self.children_index
            .get(&parent)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Depth-first draw order from [`Self::roots`] and [`Self::children_index`].
    ///
    /// Each node appears before its descendants; among siblings, later entries in
    /// `children_index` (or `roots` for top-level nodes) are painted later (on top).
    pub fn paint_order(&self) -> Vec<NodeId> {
        self.paint_order_iter().collect()
    }

    /// Lazy version of [`Self::paint_order`].
    pub fn paint_order_iter(&self) -> PaintOrderIter<'_> {
        PaintOrderIter {
            graph: self,
            roots: self.roots.iter(),
            subtree: None,
        }
    }

    /// World-space origin of `id`, composing local [`Node::position`] along the parent chain.
    ///
    /// Root nodes use their stored position as world coordinates. A child stores offsets
    /// relative to its parent's origin, so `world(child) = world(parent) + local(child)`.
    pub fn node_world_point(&self, id: NodeId) -> Option<Point<Pixels>> {
        let node = self.nodes.get(&id)?;
        self.node_world_point_with_node(node)
    }

    pub fn node_world_point_with_node(&self, node: &Node) -> Option<Point<Pixels>> {
        let mut world = node.point();
        let mut current = node.id();
        while let Some(parent_id) = self.nodes.get(&current)?.parent() {
            let parent_pos = self.nodes.get(&parent_id)?.point();
            world.x += parent_pos.x;
            world.y += parent_pos.y;
            current = parent_id;
        }
        Some(world)
    }

    /// World-space axis-aligned bounds: [`Self::node_world_point`] origin with the node's size.
    pub fn node_world_bounds(&self, id: NodeId) -> Option<Bounds<Pixels>> {
        let node = self.nodes.get(&id)?;
        let origin = self.node_world_point(id)?;
        Some(Bounds::new(origin, *node.size_ref()))
    }

    pub fn node_world_bounds_with_node(&self, node: &Node) -> Option<Bounds<Pixels>> {
        let origin = self.node_world_point_with_node(node)?;
        Some(Bounds::new(origin, *node.size_ref()))
    }

    /// World-space port anchor: [`Self::node_world_point`] + `local_offset` from [`NodeRenderer::port_offset`].
    pub fn port_world_point(
        &self,
        id: NodeId,
        local_offset: Point<Pixels>,
    ) -> Option<Point<Pixels>> {
        Some(self.node_world_point(id)? + local_offset)
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

    pub fn remove_node(
        &mut self,
        id: &NodeId,
        policy: ParentDeletePolicy,
    ) -> Result<(), GraphError> {
        match policy {
            ParentDeletePolicy::Cascade => {
                self.remove_node_cascade(id);
                Ok(())
            }
            ParentDeletePolicy::Promote => self.remove_node_promote(id),
        }
    }

    pub fn remove_node_cascade(&mut self, id: &NodeId) {
        if self.ensure_node(*id).is_err() {
            return;
        }
        let mut order = Vec::new();
        self.collect_descendants_postorder(*id, &mut order);
        order.push(*id);
        for node_id in order {
            self.remove_node_from_graph(&node_id);
        }
    }

    pub fn remove_node_promote(&mut self, id: &NodeId) -> Result<(), GraphError> {
        self.ensure_node(*id)?;
        let grandparent = self.nodes.get(id).and_then(|n| n.parent());
        let children: Vec<NodeId> = self
            .nodes
            .get(id)
            .map(|n| n.children().to_vec())
            .unwrap_or_default();

        for child in children {
            let world = self
                .node_world_point(child)
                .ok_or(GraphError::NodeNotFound(child))?;
            self.reparent(child, grandparent)?;
            let local = self.local_point_from_world(world, grandparent)?;
            let child_node = self
                .nodes
                .get_mut(&child)
                .ok_or(GraphError::NodeNotFound(child))?;
            child_node.set_position_with_point(local);
        }
        self.remove_node_from_graph(id);

        Ok(())
    }

    /// Local origin under `parent` that places a node at `world` (same as [`Self::node_world_point`] inverse).
    pub fn local_point_from_world(
        &self,
        world: Point<Pixels>,
        parent: Option<NodeId>,
    ) -> Result<Point<Pixels>, GraphError> {
        let Some(parent) = parent else {
            return Ok(world);
        };
        let parent_world = self
            .node_world_point(parent)
            .ok_or(GraphError::NodeNotFound(parent))?;
        Ok(Point::new(
            world.x - parent_world.x,
            world.y - parent_world.y,
        ))
    }

    /// Detach hierarchy links, drop ports/edges, and remove the node record (no child promotion).
    fn remove_node_from_graph(&mut self, id: &NodeId) {
        let Some(node) = self.nodes.get(id).cloned() else {
            return;
        };

        self.detach_from_parent(*id);
        self.children_index.remove(id);
        self.roots
            .iter()
            .position(|root| *root == *id)
            .map(|index| self.roots.remove(index));

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
        if let Some(index) = self.node_order.iter().position(|v| *v == *id) {
            self.node_order.remove(index);
        }
    }

    /// Descendants of `id` in post-order (each node before its ancestors in the subtree).
    fn collect_descendants_postorder(&self, id: NodeId, out: &mut Vec<NodeId>) {
        let Some(node) = self.nodes.get(&id) else {
            return;
        };
        for child in node.children() {
            self.collect_descendants_postorder(*child, out);
            out.push(*child);
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

    pub fn remove_selected_node(&mut self, policy: ParentDeletePolicy) -> Result<bool, GraphError> {
        if self.selected_node.is_empty() {
            return Ok(false);
        }

        let mut ids = vec![];
        for id in self.selected_node.iter() {
            ids.push(*id);
        }
        for id in ids.iter() {
            self.remove_node(id, policy)?;
        }
        self.selected_node.clear();
        Ok(true)
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

    /// World-space axis-aligned bounds of **all** nodes: `(min_x, min_y, width, height)`, or `None` if there are no nodes.
    pub fn nodes_world_aabb(&self) -> Option<(f32, f32, f32, f32)> {
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;
        let mut any = false;

        for n in self.nodes.values() {
            let (nx, ny) = n.position();
            let size = *n.size_ref();
            let x: f32 = nx.into();
            let y: f32 = ny.into();
            let w: f32 = size.width.into();
            let h: f32 = size.height.into();
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x + w);
            max_y = max_y.max(y + h);
            any = true;
        }

        if !any {
            return None;
        }

        Some((
            min_x,
            min_y,
            (max_x - min_x).max(1.0),
            (max_y - min_y).max(1.0),
        ))
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
        self.paint_order().into_iter().rev().find(|id| {
            let Some(bounds) = self.node_world_bounds(*id) else {
                return false;
            };
            viewport.is_world_bounds_visible(&bounds) && bounds.contains(&mouse)
        })
    }

    fn vec_bring_to_end(vec: &mut Vec<NodeId>, id: NodeId) {
        if let Some(index) = vec.iter().position(|x| *x == id) {
            let id = vec.remove(index);
            vec.push(id);
        }
    }

    /// Move `node_id` to the end of its sibling list (top among peers under the same parent).
    pub fn bring_sibling_to_front(&mut self, node_id: NodeId) {
        let parent = self.nodes.get(&node_id).and_then(|n| n.parent());
        match parent {
            Some(parent) => {
                if let Some(children) = self.children_index.get_mut(&parent) {
                    Self::vec_bring_to_end(children, node_id);
                }
            }
            None => {
                if self.nodes.contains_key(&node_id) {
                    Self::vec_bring_to_end(&mut self.roots, node_id);
                }
            }
        }
    }

    /// Raise within siblings, then repeat for each ancestor so the subtree moves forward.
    pub fn bring_node_to_front(&mut self, node_id: NodeId) {
        if !self.nodes.contains_key(&node_id) {
            return;
        }
        let mut current = Some(node_id);
        while let Some(id) = current {
            self.bring_sibling_to_front(id);
            current = self.nodes.get(&id).and_then(|n| n.parent());
        }
        *self.node_order_mut() = self.paint_order();
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

    pub fn add_child(&mut self, parent: NodeId, child: NodeId) -> Result<(), GraphError> {
        self.ensure_node(parent)?;
        self.ensure_node(child)?;

        if parent == child {
            return Err(GraphError::SelfReference { node: parent });
        }
        if self.is_ancestor(child, parent) {
            return Err(GraphError::WouldCreateCycle { parent, child });
        }

        if self.nodes.get(&child).and_then(|n| n.parent()) == Some(parent) {
            return Ok(());
        }

        self.detach_from_parent(child);
        self.link_child_under_parent(parent, child);
        Ok(())
    }

    pub fn remove_child(&mut self, parent: NodeId, child: NodeId) {
        let Ok(()) = self.ensure_node(parent) else {
            return;
        };
        let Ok(()) = self.ensure_node(child) else {
            return;
        };

        if self.nodes.get(&child).and_then(|n| n.parent()) != Some(parent) {
            return;
        }

        self.unlink_child_from_parent(parent, child);
    }

    pub fn reparent(&mut self, node: NodeId, new_parent: Option<NodeId>) -> Result<(), GraphError> {
        self.ensure_node(node)?;

        match new_parent {
            None => {
                self.detach_from_parent(node);
            }
            Some(parent) => {
                self.add_child(parent, node)?;
            }
        }

        Ok(())
    }

    pub fn ancestors(&self, node: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        let start = self.nodes.get(&node).and_then(|n| n.parent());
        AncestorsIter {
            graph: self,
            next: start,
        }
    }

    pub fn descendants(&self, node: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        (self.nodes.contains_key(&node))
            .then(|| PaintPreorderIter::new(self, node))
            .into_iter()
            .flat_map(|it| it.skip(1))
    }

    pub fn is_ancestor(&self, ancestor: NodeId, target: NodeId) -> bool {
        self.ancestors(target).any(|id| id == ancestor)
    }

    pub(crate) fn ensure_node(&self, id: NodeId) -> Result<(), GraphError> {
        if self.nodes.contains_key(&id) {
            Ok(())
        } else {
            Err(GraphError::NodeNotFound(id))
        }
    }

    /// Detach `child` from its current parent and register it as a root node.
    fn detach_from_parent(&mut self, child: NodeId) {
        let Some(old_parent) = self.nodes.get(&child).and_then(|n| n.parent()) else {
            self.roots_push(child);
            return;
        };
        self.unlink_child_from_parent(old_parent, child);
    }

    fn unlink_child_from_parent(&mut self, parent: NodeId, child: NodeId) {
        if let Some(p) = self.nodes.get_mut(&parent) {
            p.remove_child_ref(child);
        }
        if let Some(children) = self.children_index.get_mut(&parent) {
            children
                .iter()
                .position(|id| *id == child)
                .map(|index| children.remove(index));
        }
        if let Some(c) = self.nodes.get_mut(&child) {
            c.set_parent(None);
        }
        self.roots_push(child);
    }

    /// Link `child` under `parent` without validation (caller must check invariants).
    fn link_child_under_parent(&mut self, parent: NodeId, child: NodeId) {
        if let Some(c) = self.nodes.get_mut(&child) {
            c.set_parent(Some(parent));
        }
        if let Some(p) = self.nodes.get_mut(&parent) {
            p.push_child(child);
        }
        let children = self.children_index.entry(parent).or_default();
        if !children.contains(&child) {
            children.push(child);
        }
        self.roots
            .iter()
            .position(|id| *id == child)
            .map(|index| self.roots.remove(index));
    }
}

#[cfg(test)]
mod hierarchy_tests {
    use super::*;
    use serde_json::json;

    fn graph_with_nodes() -> (Graph, NodeId, NodeId, NodeId) {
        let mut g = Graph::new();
        let a = g
            .create_node("default")
            .position(0.0, 0.0)
            .data(json!({ "label": "A" }))
            .build();
        let b = g
            .create_node("default")
            .position(100.0, 0.0)
            .data(json!({ "label": "B" }))
            .build();
        let c = g
            .create_node("default")
            .position(200.0, 0.0)
            .data(json!({ "label": "C" }))
            .build();
        (g, a, b, c)
    }

    #[test]
    fn add_child_links_parent_and_removes_from_roots() {
        let (mut g, a, b, _) = graph_with_nodes();
        assert!(g.roots().contains(&a));
        assert!(g.roots().contains(&b));

        g.add_child(a, b).unwrap();

        assert_eq!(g.get_node(&b).unwrap().parent(), Some(a));
        assert!(g.get_node(&a).unwrap().children().contains(&b));
        assert!(!g.roots().contains(&b));
        assert!(g.roots().contains(&a));
    }

    #[test]
    fn add_child_rejects_cycle() {
        let (mut g, a, b, c) = graph_with_nodes();
        g.add_child(a, b).unwrap();
        g.add_child(b, c).unwrap();

        let err = g.add_child(c, a).unwrap_err();
        assert!(matches!(
            err,
            GraphError::WouldCreateCycle {
                parent,
                child,
            } if parent == c && child == a
        ));
    }

    #[test]
    fn reparent_moves_between_parents() {
        let (mut g, a, b, c) = graph_with_nodes();
        g.add_child(a, b).unwrap();
        g.reparent(c, Some(a)).unwrap();

        assert_eq!(g.get_node(&c).unwrap().parent(), Some(a));
        assert!(g.get_node(&a).unwrap().children().contains(&c));
    }

    #[test]
    fn remove_child_promotes_to_roots() {
        let (mut g, a, b, _) = graph_with_nodes();
        g.add_child(a, b).unwrap();
        g.remove_child(a, b);

        assert_eq!(g.get_node(&b).unwrap().parent(), None);
        assert!(g.roots().contains(&b));
        assert!(!g.get_node(&a).unwrap().children().contains(&b));
    }

    #[test]
    fn ancestors_and_descendants() {
        let (mut g, a, b, c) = graph_with_nodes();
        g.add_child(a, b).unwrap();
        g.add_child(b, c).unwrap();

        assert_eq!(g.ancestors(c).collect::<Vec<_>>(), vec![b, a]);
        assert_eq!(g.descendants(a).collect::<Vec<_>>(), vec![b, c]);
        assert!(g.is_ancestor(a, c));
        assert!(!g.is_ancestor(c, a));
    }

    #[test]
    fn remove_node_promotes_children_to_grandparent() {
        let (mut g, a, b, c) = graph_with_nodes();
        g.add_child(a, b).unwrap();
        g.add_child(b, c).unwrap();
        g.remove_node_promote(&b).unwrap();

        assert!(g.get_node(&a).is_some());
        assert!(g.get_node(&c).is_some());
        assert_eq!(g.get_node(&c).unwrap().parent(), Some(a));
        assert!(!g.roots().contains(&c));
        assert!(g.get_node(&a).unwrap().children().contains(&c));
    }

    #[test]
    fn remove_node_promote_preserves_world_position() {
        let mut g = Graph::new();
        let a = g.create_node("default").position(100.0, 100.0).build();
        let b = g.create_node("default").position(10.0, 10.0).build();
        let c = g.create_node("default").position(5.0, 5.0).build();
        g.add_child(a, b).unwrap();
        g.add_child(b, c).unwrap();

        let world_c = g.node_world_point(c).unwrap();
        g.remove_node_promote(&b).unwrap();

        assert_eq!(g.node_world_point(c), Some(world_c));
        assert_eq!(
            g.get_node(&c).unwrap().point(),
            Point::new(px(15.0), px(15.0))
        );
    }

    #[test]
    fn remove_node_promote_root_children_become_roots_with_world_local() {
        let (mut g, a, b, _) = graph_with_nodes();
        g.add_child(a, b).unwrap();
        let world_b = g.node_world_point(b).unwrap();

        g.remove_node_promote(&a).unwrap();

        assert_eq!(g.get_node(&b).unwrap().parent(), None);
        assert!(g.roots().contains(&b));
        assert_eq!(g.node_world_point(b), Some(world_b));
        assert_eq!(g.get_node(&b).unwrap().point(), world_b);
    }

    #[test]
    fn node_world_point_is_local_for_roots() {
        let (g, a, b, _) = graph_with_nodes();
        assert_eq!(g.node_world_point(a), Some(g.get_node(&a).unwrap().point()));
        assert_eq!(g.node_world_point(b), Some(g.get_node(&b).unwrap().point()));
    }

    #[test]
    fn node_world_point_accumulates_ancestor_positions() {
        let mut g = Graph::new();
        let a = g.create_node("default").position(100.0, 20.0).build();
        let b = g.create_node("default").position(10.0, 5.0).build();
        let c = g.create_node("default").position(3.0, 2.0).build();
        g.add_child(a, b).unwrap();
        g.add_child(b, c).unwrap();

        let world_c = g.node_world_point(c).expect("c exists");
        assert_eq!(world_c.x, px(113.0));
        assert_eq!(world_c.y, px(27.0));

        let world_b = g.node_world_point(b).expect("b exists");
        assert_eq!(world_b.x, px(110.0));
        assert_eq!(world_b.y, px(25.0));
    }

    #[test]
    fn node_world_point_returns_none_for_missing_node() {
        let g = Graph::new();
        assert_eq!(g.node_world_point(NodeId::new()), None);
    }

    #[test]
    fn node_world_bounds_uses_world_origin_and_local_size() {
        let mut g = Graph::new();
        let a = g
            .create_node("default")
            .position(50.0, 10.0)
            .size(200.0, 80.0)
            .build();
        let b = g
            .create_node("default")
            .position(20.0, 15.0)
            .size(60.0, 40.0)
            .build();
        g.add_child(a, b).unwrap();

        let bounds_b = g.node_world_bounds(b).expect("b exists");
        assert_eq!(bounds_b.origin.x, px(70.0));
        assert_eq!(bounds_b.origin.y, px(25.0));
        assert_eq!(bounds_b.size.width, px(60.0));
        assert_eq!(bounds_b.size.height, px(40.0));

        let bounds_a = g.node_world_bounds(a).expect("a exists");
        assert_eq!(bounds_a, g.get_node(&a).unwrap().bounds());
    }

    #[test]
    fn node_world_bounds_returns_none_for_missing_node() {
        let g = Graph::new();
        assert_eq!(g.node_world_bounds(NodeId::new()), None);
    }

    #[test]
    fn port_world_point_offsets_from_node_world_origin() {
        let mut g = Graph::new();
        let a = g.create_node("default").position(100.0, 20.0).build();
        let b = g.create_node("default").position(10.0, 5.0).build();
        g.add_child(a, b).unwrap();

        let local = Point::new(px(40.0), px(30.0));
        let world = g.port_world_point(b, local).expect("b exists");
        assert_eq!(world.x, px(150.0));
        assert_eq!(world.y, px(55.0));
    }

    #[test]
    fn paint_order_matches_node_order_for_flat_roots() {
        let (g, a, b, c) = graph_with_nodes();
        assert_eq!(g.paint_order(), vec![a, b, c]);
        assert_eq!(g.paint_order(), g.node_order().clone());
    }

    #[test]
    fn paint_order_depth_first_follows_children_index() {
        let (mut g, a, b, c) = graph_with_nodes();
        g.add_child(a, b).unwrap();
        g.add_child(a, c).unwrap();

        assert_eq!(g.paint_order(), vec![a, b, c]);

        if let Some(children) = g.children_index.get_mut(&a) {
            children.swap(0, 1);
        }
        assert_eq!(g.paint_order(), vec![a, c, b]);
    }

    #[test]
    fn bring_sibling_to_front_reorders_roots() {
        let (mut g, a, b, c) = graph_with_nodes();
        g.bring_sibling_to_front(b);
        assert_eq!(g.roots(), &[a, c, b]);
    }

    #[test]
    fn bring_sibling_to_front_reorders_children_index() {
        let (mut g, a, b, c) = graph_with_nodes();
        g.add_child(a, b).unwrap();
        g.add_child(a, c).unwrap();
        g.bring_sibling_to_front(b);
        assert_eq!(g.children_of(a), &[c, b]);
        let child_set: HashSet<_> = g.get_node(&a).unwrap().children().iter().copied().collect();
        assert_eq!(child_set, HashSet::from([b, c]));
    }

    #[test]
    fn bring_node_to_front_bubbles_ancestors() {
        let (mut g, a, b, c) = graph_with_nodes();
        g.add_child(a, b).unwrap();
        g.add_child(a, c).unwrap();
        g.bring_node_to_front(c);
        assert_eq!(g.children_of(a), &[b, c]);
        assert_eq!(g.roots(), &[a]);
        assert_eq!(g.node_order(), &g.paint_order());
    }

    #[test]
    fn paint_order_nested_chain() {
        let (mut g, a, b, c) = graph_with_nodes();
        g.add_child(a, b).unwrap();
        g.add_child(b, c).unwrap();

        assert_eq!(g.paint_order(), vec![a, b, c]);
        assert_eq!(g.paint_order().len(), g.nodes.len());
    }

    #[test]
    fn paint_preorder_iter_single_node() {
        let (g, a, _, _) = graph_with_nodes();
        assert_eq!(PaintPreorderIter::new(&g, a).collect::<Vec<_>>(), vec![a]);
    }

    #[test]
    fn paint_preorder_iter_nested_chain() {
        let (mut g, a, b, c) = graph_with_nodes();
        g.add_child(a, b).unwrap();
        g.add_child(b, c).unwrap();

        assert_eq!(
            PaintPreorderIter::new(&g, a).collect::<Vec<_>>(),
            vec![a, b, c]
        );
    }

    #[test]
    fn paint_preorder_iter_from_interior_node() {
        let (mut g, a, b, c) = graph_with_nodes();
        g.add_child(a, b).unwrap();
        g.add_child(b, c).unwrap();

        assert_eq!(
            PaintPreorderIter::new(&g, b).collect::<Vec<_>>(),
            vec![b, c]
        );
        assert_eq!(PaintPreorderIter::new(&g, c).collect::<Vec<_>>(), vec![c]);
    }

    #[test]
    fn paint_preorder_iter_respects_children_index() {
        let (mut g, a, b, c) = graph_with_nodes();
        g.add_child(a, b).unwrap();
        g.add_child(a, c).unwrap();

        assert_eq!(
            PaintPreorderIter::new(&g, a).collect::<Vec<_>>(),
            vec![a, b, c]
        );

        if let Some(children) = g.children_index.get_mut(&a) {
            children.swap(0, 1);
        }
        assert_eq!(
            PaintPreorderIter::new(&g, a).collect::<Vec<_>>(),
            vec![a, c, b]
        );
    }

    #[test]
    fn paint_preorder_iter_stops_at_subtree_boundary() {
        let (mut g, a, b, c) = graph_with_nodes();
        g.add_child(a, b).unwrap();

        let order: Vec<_> = PaintPreorderIter::new(&g, a).collect();
        assert_eq!(order, vec![a, b]);
        assert!(!order.contains(&c));
    }

    #[test]
    fn paint_order_iter_matches_paint_order() {
        let (mut g, a, b, c) = graph_with_nodes();
        g.add_child(a, b).unwrap();
        g.add_child(b, c).unwrap();

        assert_eq!(g.paint_order_iter().collect::<Vec<_>>(), g.paint_order());
        assert_eq!(g.paint_order_iter().collect::<Vec<_>>(), vec![a, b, c]);
    }

    #[test]
    fn paint_order_iter_lazy_next() {
        let (g, a, b, c) = graph_with_nodes();
        let mut iter = g.paint_order_iter();
        assert_eq!(iter.next(), Some(a));
        assert_eq!(iter.next(), Some(b));
        assert_eq!(iter.next(), Some(c));
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn paint_order_iter_walks_root_forests_in_roots_order() {
        let (mut g, a, b, c) = graph_with_nodes();
        g.add_child(a, b).unwrap();
        g.bring_sibling_to_front(a);

        assert_eq!(g.roots(), &[c, a]);
        assert_eq!(g.paint_order_iter().collect::<Vec<_>>(), vec![c, a, b]);
    }

    #[test]
    fn paint_order_iter_visits_every_node_in_hierarchy() {
        let (mut g, a, b, c) = graph_with_nodes();
        g.add_child(a, b).unwrap();
        g.add_child(b, c).unwrap();

        assert_eq!(g.paint_order_iter().count(), g.nodes.len());
    }

    #[test]
    fn remove_node_cascade_deletes_entire_subtree() {
        let (mut g, a, b, c) = graph_with_nodes();
        g.add_child(a, b).unwrap();
        g.add_child(b, c).unwrap();

        g.remove_node_cascade(&a);

        assert!(g.get_node(&a).is_none());
        assert!(g.get_node(&b).is_none());
        assert!(g.get_node(&c).is_none());
        assert!(!g.roots().contains(&a));
        assert!(!g.roots().contains(&b));
        assert!(!g.roots().contains(&c));
    }
}
