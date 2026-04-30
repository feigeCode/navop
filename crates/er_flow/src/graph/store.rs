use gpui::{Pixels, Size};
use serde::{Deserialize, Serialize};

use crate::{Edge, EdgeId, Node, NodeId, Port, PortId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GraphOp {
    // --- Node ---
    AddNode(Node),

    RemoveNode { id: NodeId },

    MoveNode { id: NodeId, x: f32, y: f32 },

    ResizeNode { id: NodeId, size: Size<Pixels> },

    UpdateNodeData { id: NodeId, data: serde_json::Value },

    // --- node_order ---
    NodeOrderInsert { id: NodeId },
    NodeOrderRemove { index: usize },

    // --- Port ---
    AddPort(Port),

    RemovePort(PortId),

    // --- Edge ---
    AddEdge(Edge),

    RemoveEdge(EdgeId),

    Batch(Vec<GraphOp>),
}

#[derive(Debug, Clone)]
pub struct GraphChange {
    pub kind: GraphChangeKind,
    pub source: ChangeSource,
}

impl GraphChange {
    pub fn is_local(&self) -> bool {
        matches!(self.source, ChangeSource::Local)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeSource {
    Local,
    Remote,
    Undo,
    Redo,
}

#[derive(Debug, Clone)]
pub enum GraphChangeKind {
    // --- Node ---
    NodeAdded(Node),
    NodeRemoved {
        id: NodeId,
    },
    NodeMoved {
        id: NodeId,
        x: f32,
        y: f32,
    },
    NodeSetWidthed {
        id: NodeId,
        width: f32,
    },
    NodeSetHeighted {
        id: NodeId,
        height: f32,
    },
    NodeDataUpdated {
        id: NodeId,
        data: serde_json::Value,
    },

    // --- node_order ---
    NodeOrderUpdate(Vec<NodeId>),

    // --- Port ---
    PortAdded(Port),
    PortRemoved {
        id: PortId,
    },

    // --- Edge ---
    EdgeAdded(Edge),
    EdgeRemoved {
        id: EdgeId,
    },

    /// No graph mutation; used to request a frame repaint (e.g. after remote awareness updates).
    RedrawRequested,

    Batch(Vec<GraphChangeKind>),
}
