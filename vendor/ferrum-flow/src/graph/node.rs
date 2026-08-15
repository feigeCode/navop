use std::{collections::HashMap, fmt::Display, marker::PhantomData, str::FromStr};

use gpui::{Bounds, Pixels, Point, Size, px};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::Graph;
use crate::builder_state::{Set, Unset};

pub const DEFAULT_NODE_WIDTH: Pixels = px(120.0);
pub const DEFAULT_NODE_HEIGHT: Pixels = px(60.0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(Uuid);

impl Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Default for NodeId {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
    pub fn from_string(s: impl Into<String>) -> Option<Self> {
        let string = s.into();
        Uuid::from_str(&string).ok().map(Self)
    }
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    id: NodeId,
    node_type: String,
    execute_type: String,
    x: Pixels,
    y: Pixels,
    size: Size<Pixels>,
    inputs: Vec<PortId>,
    outputs: Vec<PortId>,
    data: serde_json::Value,

    parent: Option<NodeId>,
    /// Child ids for hierarchy membership only; **sibling z-order** lives on
    /// [`Graph::children_index`](crate::graph::Graph::children_index).
    children: Vec<NodeId>,
}

impl Node {
    pub fn new(x: f32, y: f32) -> Self {
        Self {
            id: NodeId::new(),
            node_type: String::new(),
            execute_type: String::new(),
            x: x.into(),
            y: y.into(),
            size: Size {
                width: DEFAULT_NODE_WIDTH,
                height: DEFAULT_NODE_HEIGHT,
            },
            inputs: vec![],
            outputs: vec![],
            data: json!({}),

            parent: None,
            children: vec![],
        }
    }

    pub fn id(&self) -> NodeId {
        self.id
    }

    pub(crate) fn set_id(&mut self, id: NodeId) {
        self.id = id;
    }

    pub fn renderer_key(&self) -> &str {
        &self.node_type
    }

    pub fn execute_type_ref(&self) -> &str {
        &self.execute_type
    }

    pub fn set_renderer_key(&mut self, node_type: impl Into<String>) {
        self.node_type = node_type.into();
    }

    pub fn set_execute_type(&mut self, execute_type: impl Into<String>) {
        self.execute_type = execute_type.into();
    }

    pub fn position(&self) -> (Pixels, Pixels) {
        (self.x, self.y)
    }

    #[deprecated(since = "0.1.0", note = "Use `point` instead")]
    pub fn position_point(&self) -> Point<Pixels> {
        Point::new(self.x, self.y)
    }

    pub fn size_ref(&self) -> &Size<Pixels> {
        &self.size
    }

    pub fn inputs(&self) -> &[PortId] {
        &self.inputs
    }

    pub fn outputs(&self) -> &[PortId] {
        &self.outputs
    }

    pub fn data_ref(&self) -> &serde_json::Value {
        &self.data
    }

    pub fn data_mut(&mut self) -> &mut serde_json::Value {
        &mut self.data
    }

    pub fn set_position(&mut self, x: Pixels, y: Pixels) {
        self.x = x;
        self.y = y;
    }

    pub fn set_position_with_point(&mut self, point: Point<Pixels>) {
        self.x = point.x;
        self.y = point.y;
    }

    pub fn set_size_mut(&mut self, size: Size<Pixels>) {
        self.size = size;
    }

    pub fn set_size_width(&mut self, width: Pixels) {
        self.size.width = width;
    }

    pub fn set_size_height(&mut self, height: Pixels) {
        self.size.height = height;
    }

    pub fn set_data(&mut self, data: serde_json::Value) {
        self.data = data;
    }

    pub fn push_input(&mut self, id: PortId) {
        self.inputs.push(id);
    }

    pub fn push_output(&mut self, id: PortId) {
        self.outputs.push(id);
    }

    pub fn clear_port_refs(&mut self) {
        self.inputs.clear();
        self.outputs.clear();
    }

    pub fn point(&self) -> Point<Pixels> {
        Point::new(self.x, self.y)
    }

    pub fn bounds(&self) -> Bounds<Pixels> {
        Bounds::new(self.point(), self.size)
    }

    pub fn set_size(mut self, size: Size<Pixels>) -> Self {
        self.size = size;
        self
    }

    pub fn output(mut self, id: PortId) -> Self {
        self.outputs.push(id);
        self
    }

    pub fn input(mut self, id: PortId) -> Self {
        self.inputs.push(id);
        self
    }

    pub fn parent(&self) -> Option<NodeId> {
        self.parent
    }

    pub fn children(&self) -> &[NodeId] {
        &self.children
    }

    pub(super) fn set_parent(&mut self, parent: Option<NodeId>) {
        self.parent = parent;
    }

    pub(super) fn push_child(&mut self, child: NodeId) {
        if !self.children.contains(&child) {
            self.children.push(child);
        }
    }

    pub(super) fn remove_child_ref(&mut self, child: NodeId) {
        self.children
            .iter()
            .position(|id| *id == child)
            .map(|index| self.children.remove(index));
    }

    pub(super) fn clear_children(&mut self) {
        self.children.clear();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PortId(Uuid);

impl Display for PortId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Default for PortId {
    fn default() -> Self {
        Self::new()
    }
}

impl PortId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
    pub fn from_string(s: impl Into<String>) -> Option<Self> {
        let string = s.into();
        Uuid::from_str(&string).ok().map(Self)
    }
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PortKind {
    Input,
    Output,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum PortPosition {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PortType {
    Any,
    Bool,
    Int,
    Float,
    String,
    List(Box<PortType>),
    Map(Box<PortType>, Box<PortType>),
    Custom(String),
    Union(Vec<PortType>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PortScope {
    /// The port is only allowed to connect to ports on nodes with the same direct parent (siblings).
    #[default]
    Local,
    /// The port is allowed to connect to ports within any level of the node hierarchy.
    Boundary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Port {
    id: PortId,
    kind: PortKind,
    index: usize,
    node_id: NodeId,
    position: PortPosition,
    size: Size<Pixels>,
    port_type: PortType,
    scope: PortScope,
}

impl Port {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: PortId,
        kind: PortKind,
        index: usize,
        node_id: NodeId,
        position: PortPosition,
        size: Size<Pixels>,
        port_type: PortType,
    ) -> Self {
        Self {
            id,
            kind,
            index,
            node_id,
            position,
            size,
            port_type,
            scope: PortScope::default(),
        }
    }

    pub fn id(&self) -> PortId {
        self.id
    }

    pub fn kind(&self) -> PortKind {
        self.kind
    }

    pub fn index(&self) -> usize {
        self.index
    }

    pub fn node_id(&self) -> NodeId {
        self.node_id
    }

    pub fn position(&self) -> PortPosition {
        self.position
    }

    pub fn size_ref(&self) -> &Size<Pixels> {
        &self.size
    }

    pub fn port_type_ref(&self) -> &PortType {
        &self.port_type
    }

    pub fn scope(&self) -> PortScope {
        self.scope
    }

    pub fn set_scope(&mut self, scope: PortScope) {
        self.scope = scope;
    }

    pub fn port_type_mut(&mut self) -> &mut PortType {
        &mut self.port_type
    }

    pub fn set_size(&mut self, size: Size<Pixels>) {
        self.size = size;
    }

    pub fn set_index(&mut self, index: usize) {
        self.index = index;
    }

    pub fn set_position(&mut self, position: PortPosition) {
        self.position = position;
    }

    pub fn set_id(&mut self, id: PortId) {
        self.id = id;
    }

    pub fn set_node_id(&mut self, node_id: NodeId) {
        self.node_id = node_id;
    }
}

impl Display for PortKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PortKind::Input => write!(f, "input"),
            PortKind::Output => write!(f, "output"),
        }
    }
}

impl Display for PortPosition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PortPosition::Left => write!(f, "left"),
            PortPosition::Right => write!(f, "right"),
            PortPosition::Top => write!(f, "top"),
            PortPosition::Bottom => write!(f, "bottom"),
        }
    }
}

impl FromStr for PortPosition {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "right" => Ok(Self::Right),
            "top" => Ok(Self::Top),
            "bottom" => Ok(Self::Bottom),
            "left" => Ok(Self::Left),
            _ => Err(anyhow::anyhow!("Invalid port position: {}", s)),
        }
    }
}

pub type NodeBuilderInGraph<'a> = NodeBuilder<'a, Set<&'a mut Graph>>;

pub struct NodeBuilder<'a, G = Unset> {
    graph: G,
    _phantom: PhantomData<&'a ()>,

    node_type: String,
    execute_type: String,
    x: Pixels,
    y: Pixels,
    size: Size<Pixels>,
    inputs: Vec<PortSpec>,
    outputs: Vec<PortSpec>,
    data: serde_json::Value,
}

#[derive(Clone)]
pub struct PortSpec {
    position: PortPosition,
    size: Size<Pixels>,
    port_type: PortType,
    scope: PortScope,
}

impl PortSpec {
    pub fn input(position: PortPosition) -> Self {
        Self {
            position,
            size: DEFAULT_PORT_SIZE,
            port_type: PortType::Any,
            scope: PortScope::default(),
        }
    }

    pub fn output(position: PortPosition) -> Self {
        Self {
            position,
            size: DEFAULT_PORT_SIZE,
            port_type: PortType::Any,
            scope: PortScope::default(),
        }
    }

    pub fn with_size(mut self, size: Size<Pixels>) -> Self {
        self.size = size;
        self
    }

    pub fn with_type(mut self, port_type: PortType) -> Self {
        self.port_type = port_type;
        self
    }

    pub fn with_scope(mut self, scope: PortScope) -> Self {
        self.scope = scope;
        self
    }
}

const DEFAULT_PORT_SIZE: Size<Pixels> = Size {
    width: px(12.0),
    height: px(12.0),
};

pub struct PortBuilder {
    id: PortId,
    kind: PortKind,
    index: usize,
    node_id: NodeId,
    position: PortPosition,
    size: Size<Pixels>,
    port_type: PortType,
}

impl PortBuilder {
    pub fn new(id: PortId) -> Self {
        Self {
            id,
            kind: PortKind::Input,
            index: 0,
            node_id: NodeId::from_uuid(Uuid::nil()),
            position: PortPosition::Left,
            size: DEFAULT_PORT_SIZE,
            port_type: PortType::Any,
        }
    }

    pub fn kind(mut self, kind: PortKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn node_id(mut self, node_id: NodeId) -> Self {
        self.node_id = node_id;
        self
    }

    pub fn index(mut self, index: usize) -> Self {
        self.index = index;
        self
    }

    pub fn position(mut self, position: PortPosition) -> Self {
        self.position = position;
        self
    }

    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.size = Size::new(px(width), px(height));
        self
    }

    pub fn port_type(mut self, port_type: PortType) -> Self {
        self.port_type = port_type;
        self
    }

    pub fn build(self) -> Port {
        Port::new(
            self.id,
            self.kind,
            self.index,
            self.node_id,
            self.position,
            self.size,
            self.port_type,
        )
    }
}

impl<'a> NodeBuilder<'a, Unset> {
    pub fn new(renderer_key: impl Into<String>) -> Self {
        Self {
            graph: Unset,
            _phantom: PhantomData,
            node_type: renderer_key.into(),
            execute_type: String::new(),
            x: px(0.0),
            y: px(0.0),
            size: Size {
                width: DEFAULT_NODE_WIDTH,
                height: DEFAULT_NODE_HEIGHT,
            },
            inputs: vec![],
            outputs: vec![],
            data: json!({}),
        }
    }

    pub fn graph(self, graph: &'a mut Graph) -> NodeBuilderInGraph<'a> {
        NodeBuilder {
            graph: Set(graph),
            _phantom: PhantomData,
            node_type: self.node_type,
            execute_type: self.execute_type,
            x: self.x,
            y: self.y,
            size: self.size,
            inputs: self.inputs,
            outputs: self.outputs,
            data: self.data,
        }
    }
}

impl<'a, G> NodeBuilder<'a, G> {
    pub fn execute_type(mut self, execute_type: impl Into<String>) -> Self {
        self.execute_type = execute_type.into();
        self
    }

    pub fn position(mut self, x: f32, y: f32) -> Self {
        self.x = x.into();
        self.y = y.into();
        self
    }

    pub fn size(mut self, w: f32, h: f32) -> Self {
        self.size = Size {
            width: w.into(),
            height: h.into(),
        };
        self
    }

    fn push_input_spec(&mut self, spec: PortSpec) {
        self.inputs.push(spec);
    }

    fn push_output_spec(&mut self, spec: PortSpec) {
        self.outputs.push(spec);
    }

    pub fn input(mut self) -> Self {
        self.push_input_spec(PortSpec::input(PortPosition::Left));
        self
    }

    pub fn output(mut self) -> Self {
        self.push_output_spec(PortSpec::output(PortPosition::Right));
        self
    }

    pub fn input_at(mut self, pos: PortPosition) -> Self {
        self.push_input_spec(PortSpec::input(pos));
        self
    }

    pub fn output_at(mut self, pos: PortPosition) -> Self {
        self.push_output_spec(PortSpec::output(pos));
        self
    }

    pub fn input_with(mut self, pos: PortPosition, size: Size<Pixels>) -> Self {
        self.push_input_spec(PortSpec::input(pos).with_size(size));
        self
    }

    pub fn output_with(mut self, pos: PortPosition, size: Size<Pixels>) -> Self {
        self.push_output_spec(PortSpec::output(pos).with_size(size));
        self
    }

    pub fn input_port(mut self, spec: PortSpec) -> Self {
        self.push_input_spec(spec);
        self
    }

    pub fn output_port(mut self, spec: PortSpec) -> Self {
        self.push_output_spec(spec);
        self
    }

    pub fn data(mut self, data: serde_json::Value) -> Self {
        self.data = data;
        self
    }

    /// Like [`Self::build_raw`], but uses the given node id and input/output port id lists.
    /// Returns an empty port vector: port records are expected to be loaded separately
    /// (e.g. from persistence). Any [`PortSpec`]s on this builder are ignored.
    pub fn build_raw_with_port_ids(
        self,
        node_id: NodeId,
        input_ids: Vec<PortId>,
        output_ids: Vec<PortId>,
    ) -> Node {
        Node {
            id: node_id,
            node_type: self.node_type,
            execute_type: self.execute_type,
            x: self.x,
            y: self.y,
            size: self.size,
            inputs: input_ids,
            outputs: output_ids,
            data: self.data,
            parent: None,
            children: vec![],
        }
    }

    pub fn build_raw(self) -> (Node, Vec<Port>, G) {
        let node_id = NodeId::new();

        let mut inputs = Vec::new();
        let mut outputs = Vec::new();

        let mut input_counters: HashMap<PortPosition, usize> = HashMap::new();

        // Create input ports
        let mut ports = vec![];
        for spec in self.inputs {
            let port_id = PortId::new();

            let index = input_counters.entry(spec.position).or_insert(0);
            let current_index = *index;
            *index += 1;

            ports.push(Port {
                id: port_id,
                kind: PortKind::Input,
                index: current_index,
                node_id,
                position: spec.position,
                size: spec.size,
                port_type: spec.port_type,
                scope: spec.scope,
            });

            inputs.push(port_id);
        }

        let mut output_counters: HashMap<PortPosition, usize> = HashMap::new();

        // Create output ports
        for spec in self.outputs {
            let port_id = PortId::new();

            let index = output_counters.entry(spec.position).or_insert(0);
            let current_index = *index;
            *index += 1;

            ports.push(Port {
                id: port_id,
                kind: PortKind::Output,
                index: current_index,
                node_id,
                position: spec.position,
                size: spec.size,
                port_type: spec.port_type,
                scope: spec.scope,
            });

            outputs.push(port_id);
        }

        (
            Node {
                id: node_id,
                node_type: self.node_type,
                execute_type: self.execute_type,
                x: self.x,
                y: self.y,
                size: self.size,
                inputs,
                outputs,
                data: self.data,
                parent: None,
                children: vec![],
            },
            ports,
            self.graph,
        )
    }
}

impl<'a> NodeBuilderInGraph<'a> {
    pub fn build(self) -> NodeId {
        let (node, ports, graph) = self.build_raw();
        let id = node.id();
        graph.0.add_node(node);
        for port in ports {
            graph.0.add_port(port);
        }
        id
    }

    /// Like [`Self::build`], but returns input and output [`PortId`] slices (in port-spec order)
    /// so callers can wire [`crate::Graph::create_edge`] without [`crate::Graph::get_node`].
    pub fn build_with_ports(self) -> (NodeId, Vec<PortId>, Vec<PortId>) {
        let (node, ports, graph) = self.build_raw();
        let id = node.id();
        let inputs = node.inputs().to_vec();
        let outputs = node.outputs().to_vec();
        graph.0.add_node(node);
        for port in ports {
            graph.0.add_port(port);
        }
        (id, inputs, outputs)
    }
}
