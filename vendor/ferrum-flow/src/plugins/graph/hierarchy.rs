//! Hierarchy scope helpers for [`super::GraphPlugin`].

use crate::{Graph, NodeId};

/// Paint-scope queries for nested graphs ([`super::GraphPlugin`] only; not used with
/// [`crate::plugins::NodePlugin`] / [`crate::plugins::EdgePlugin`] on the same canvas).
pub(super) trait GraphHierarchy {
    /// `true` when any node has a parent or children.
    fn has_node_hierarchy(&self) -> bool;

    /// Root-level group: has children and no ancestor is also a group ([`Graph::paint_order`] subtree).
    fn is_top_level_group_anchor(&self, id: NodeId) -> bool;
}

impl GraphHierarchy for Graph {
    fn has_node_hierarchy(&self) -> bool {
        self.roots().len() != self.nodes().len()
    }

    fn is_top_level_group_anchor(&self, id: NodeId) -> bool {
        is_top_level_group_anchor(self, id)
    }
}

fn is_top_level_group_anchor(graph: &Graph, id: NodeId) -> bool {
    if graph.children_of(id).is_empty() {
        return false;
    }
    graph.ancestors(id).all(|a| graph.children_of(a).is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_graph_skips_graph_scope_nodes() {
        let mut g = Graph::new();
        let _a = g.create_node("default").build();
        let _b = g.create_node("default").build();
        assert!(!g.has_node_hierarchy());
    }

    #[test]
    fn nested_graph_classifies_scope() {
        let mut g = Graph::new();
        let p = g.create_node("default").build();
        let c = g.create_node("default").build();
        g.add_child(p, c).unwrap();
        assert!(g.has_node_hierarchy());
        assert!(g.is_top_level_group_anchor(p));
        assert!(!g.is_top_level_group_anchor(c));
    }
}
