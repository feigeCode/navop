use std::{collections::HashMap, fmt::Display, str::FromStr};

use gpui::{Bounds, Pixels, Point, Size, px};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::Graph;

pub const DEFAULT_NODE_WIDTH: Pixels = px(120.0);
pub const DEFAULT_NODE_HEIGHT: Pixels = px(60.0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(Uuid);

impl Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
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
    pub id: NodeId,
    pub node_type: String,
    pub execute_type: String,
    pub x: Pixels,
    pub y: Pixels,
    pub size: Size<Pixels>,

    pub inputs: Vec<PortId>,
    pub outputs: Vec<PortId>,
    pub data: serde_json::Value,
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
        }
    }

    pub fn node_type(mut self, ty: impl Into<String>) -> Self {
        self.node_type = ty.into();
        self
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PortId(Uuid);

impl Display for PortId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Port {
    pub id: PortId,
    pub kind: PortKind,
    pub index: usize,
    pub node_id: NodeId,
    pub position: PortPosition,
    pub size: Size<Pixels>,
}

impl ToString for PortKind {
    fn to_string(&self) -> String {
        match self {
            PortKind::Input => "input".into(),
            PortKind::Output => "output".into(),
        }
    }
}
impl ToString for PortPosition {
    fn to_string(&self) -> String {
        match self {
            PortPosition::Left => "left".into(),
            PortPosition::Right => "right".into(),
            PortPosition::Top => "top".into(),
            PortPosition::Bottom => "bottom".into(),
        }
    }
}

impl PortPosition {
    pub fn from_str(str: &str) -> Option<Self> {
        match str {
            "right" => Some(Self::Right),
            "top" => Some(Self::Top),
            "bottom" => Some(Self::Bottom),
            "left" => Some(Self::Left),
            _ => None,
        }
    }
}

pub struct NodeBuilder {
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
struct PortSpec {
    position: PortPosition,
    size: Size<Pixels>,
}

const DEFAULT_PORT_SIZE: Size<Pixels> = Size {
    width: px(12.0),
    height: px(12.0),
};

impl NodeBuilder {
    pub fn new(node_type: impl Into<String>) -> Self {
        Self {
            node_type: node_type.into(),
            execute_type: String::new(),
            x: px(0.0),
            y: px(0.0),
            size: Size {
                width: DEFAULT_NODE_WIDTH,
                height: DEFAULT_NODE_HEIGHT,
            },
            inputs: vec![],
            outputs: vec![],
            data: serde_json::Value::Null,
        }
    }

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

    pub fn input(mut self) -> Self {
        self.inputs.push(PortSpec {
            position: PortPosition::Left,
            size: DEFAULT_PORT_SIZE,
        });
        self
    }

    pub fn output(mut self) -> Self {
        self.outputs.push(PortSpec {
            position: PortPosition::Right,
            size: DEFAULT_PORT_SIZE,
        });
        self
    }

    pub fn input_at(mut self, pos: PortPosition) -> Self {
        self.inputs.push(PortSpec {
            position: pos,
            size: DEFAULT_PORT_SIZE,
        });
        self
    }

    pub fn output_at(mut self, pos: PortPosition) -> Self {
        self.outputs.push(PortSpec {
            position: pos,
            size: DEFAULT_PORT_SIZE,
        });
        self
    }

    pub fn input_with(mut self, pos: PortPosition, size: Size<Pixels>) -> Self {
        self.inputs.push(PortSpec {
            position: pos,
            size,
        });
        self
    }

    pub fn output_with(mut self, pos: PortPosition, size: Size<Pixels>) -> Self {
        self.outputs.push(PortSpec {
            position: pos,
            size,
        });
        self
    }

    pub fn data(mut self, data: serde_json::Value) -> Self {
        self.data = data;
        self
    }

    pub fn only_build(self, graph: &Graph) -> (Node, Vec<Port>) {
        let node_id = graph.next_node_id();

        let mut inputs = Vec::new();
        let mut outputs = Vec::new();

        let mut input_counters: HashMap<PortPosition, usize> = HashMap::new();

        // Create input ports
        let mut ports = vec![];
        for spec in self.inputs {
            let port_id = graph.next_port_id();

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
            });

            inputs.push(port_id);
        }

        let mut output_counters: HashMap<PortPosition, usize> = HashMap::new();

        // Create output ports
        for spec in self.outputs {
            let port_id = graph.next_port_id();

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
            },
            ports,
        )
    }

    pub fn build(self, graph: &mut Graph) -> NodeId {
        let (node, ports) = self.only_build(graph);
        graph.nodes.insert(node.id, node.clone());
        graph.node_order_mut().push(node.id);
        for port in ports {
            graph.ports.insert(port.id, port);
        }
        node.id
    }
}
