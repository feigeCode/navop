use std::{fmt::Display, str::FromStr as _};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Graph, PortId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EdgeId(Uuid);

impl Display for EdgeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Default for EdgeId {
    fn default() -> Self {
        Self::new()
    }
}

impl EdgeId {
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
pub struct Edge {
    pub id: EdgeId,
    pub source_port: PortId,

    pub target_port: PortId,
}

impl Default for Edge {
    fn default() -> Self {
        Self::new()
    }
}

impl Edge {
    pub fn new() -> Self {
        Self {
            id: EdgeId::new(),
            source_port: PortId::new(),
            target_port: PortId::new(),
        }
    }
    pub fn source(mut self, port: PortId) -> Self {
        self.source_port = port;
        self
    }
    pub fn target(mut self, port: PortId) -> Self {
        self.target_port = port;
        self
    }
}

pub struct EdgeBuilder<'a> {
    graph: Option<&'a mut Graph>,
    source: Option<PortId>,
    target: Option<PortId>,
}

impl<'a> Default for EdgeBuilder<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> EdgeBuilder<'a> {
    pub fn new() -> Self {
        Self {
            graph: None,
            source: None,
            target: None,
        }
    }

    pub fn graph(mut self, graph: &'a mut Graph) -> Self {
        self.graph = Some(graph);
        self
    }

    pub fn source(mut self, port: PortId) -> Self {
        self.source = Some(port);
        self
    }

    pub fn target(mut self, port: PortId) -> Self {
        self.target = Some(port);
        self
    }

    pub fn build(self) -> Option<EdgeId> {
        let graph = self.graph?;
        let source = self.source?;
        let target = self.target?;

        let edge_id = graph.next_edge_id();

        graph.add_edge(Edge {
            id: edge_id,
            source_port: source,
            target_port: target,
        });

        Some(edge_id)
    }
}
