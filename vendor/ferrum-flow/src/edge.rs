use std::{fmt::Display, marker::PhantomData, str::FromStr as _};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Graph;
use crate::builder_state::{Set, Unset};
use crate::node::PortId;

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

/// Edge construction with typestate over `graph`, `source`, and `target` ([`Unset`] / [`Set`]).
///
/// Start with [`EdgeBuilder::new`], or from [`Graph::create_edge`] which already binds the graph.
pub struct EdgeBuilder<'a, G = Unset, S = Unset, T = Unset> {
    graph: G,
    source: S,
    target: T,
    _phantom: PhantomData<&'a ()>,
}

/// [`EdgeBuilder`] after [`Graph::create_edge`] (graph field is [`Set`]).
pub type EdgeBuilderInGraph<'a> = EdgeBuilder<'a, Set<&'a mut Graph>, Unset, Unset>;

impl<'a> Default for EdgeBuilder<'a, Unset, Unset, Unset> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> EdgeBuilder<'a, Unset, Unset, Unset> {
    pub fn new() -> Self {
        Self {
            graph: Unset,
            source: Unset,
            target: Unset,
            _phantom: PhantomData,
        }
    }
}

impl<'a, S, T> EdgeBuilder<'a, Unset, S, T> {
    pub fn graph(self, graph: &'a mut Graph) -> EdgeBuilder<'a, Set<&'a mut Graph>, S, T> {
        EdgeBuilder {
            graph: Set(graph),
            source: self.source,
            target: self.target,
            _phantom: PhantomData,
        }
    }
}

impl<'a, G, T> EdgeBuilder<'a, G, Unset, T> {
    pub fn source(self, port: PortId) -> EdgeBuilder<'a, G, Set<PortId>, T> {
        EdgeBuilder {
            graph: self.graph,
            source: Set(port),
            target: self.target,
            _phantom: PhantomData,
        }
    }
}

impl<'a, G, S> EdgeBuilder<'a, G, S, Unset> {
    pub fn target(self, port: PortId) -> EdgeBuilder<'a, G, S, Set<PortId>> {
        EdgeBuilder {
            graph: self.graph,
            source: self.source,
            target: Set(port),
            _phantom: PhantomData,
        }
    }
}

impl<'a> EdgeBuilder<'a, Set<&'a mut Graph>, Set<PortId>, Set<PortId>> {
    /// Inserts the edge into the bound graph and returns its id.
    pub fn build(self) -> EdgeId {
        let graph = self.graph.0;
        let source = self.source.0;
        let target = self.target.0;

        let edge_id = graph.next_edge_id();

        graph.add_edge(Edge {
            id: edge_id,
            source_port: source,
            target_port: target,
        });

        edge_id
    }
}
