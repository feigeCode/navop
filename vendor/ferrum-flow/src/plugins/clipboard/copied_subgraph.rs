use crate::{Edge, Node, Port};

#[derive(Clone)]
pub struct CopiedSubgraph {
    pub(crate) nodes: Vec<Node>,
    pub(crate) ports: Vec<Port>,
    pub(crate) edges: Vec<Edge>,
}
