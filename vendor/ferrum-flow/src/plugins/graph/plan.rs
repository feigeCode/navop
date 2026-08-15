use crate::{Edge, Graph, NodeId, plugin::RenderContext};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgePaintKind {
    /// Both endpoints are direct children of `parent`.
    IntraParent { parent: NodeId },
    /// Cross-subtree / root / different parents — top overlay in [`super::GraphPlugin`].
    Cross,
}

pub fn classify_edge(graph: &Graph, edge: &Edge) -> EdgePaintKind {
    let Some(source_port) = graph.get_port(&edge.source_port) else {
        return EdgePaintKind::Cross;
    };
    let Some(target_port) = graph.get_port(&edge.target_port) else {
        return EdgePaintKind::Cross;
    };
    let Some(source) = graph.get_node(&source_port.node_id()) else {
        return EdgePaintKind::Cross;
    };
    let Some(target) = graph.get_node(&target_port.node_id()) else {
        return EdgePaintKind::Cross;
    };
    match (source.parent(), target.parent()) {
        (Some(p), Some(q)) if p == q => EdgePaintKind::IntraParent { parent: p },
        _ => EdgePaintKind::Cross,
    }
}

/// Same visibility rule as [`crate::plugins::EdgePlugin`]: either endpoint's node is on-screen.
pub fn edge_is_visible(ctx: &RenderContext, edge: &Edge) -> bool {
    let Some(source_port) = ctx.graph.get_port(&edge.source_port) else {
        return false;
    };
    let Some(target_port) = ctx.graph.get_port(&edge.target_port) else {
        return false;
    };
    ctx.is_node_visible(&source_port.node_id()) || ctx.is_node_visible(&target_port.node_id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Graph;

    #[test]
    fn intra_parent_same_direct_parent() {
        let mut g = Graph::new();
        let p = g.create_node("default").build();
        let (a, _, outs_a) = g.create_node("default").output().build_with_ports();
        let (b, ins_b, _) = g.create_node("default").input().build_with_ports();
        g.add_child(p, a).unwrap();
        g.add_child(p, b).unwrap();
        let e = g.create_edge().source(outs_a[0]).target(ins_b[0]).build();
        let edge = g.get_edge(&e).unwrap();
        assert_eq!(
            classify_edge(&g, edge),
            EdgePaintKind::IntraParent { parent: p }
        );
    }

    #[test]
    fn cross_when_different_parents() {
        let mut g = Graph::new();
        let p1 = g.create_node("default").build();
        let p2 = g.create_node("default").build();
        let (a, _, outs_a) = g.create_node("default").output().build_with_ports();
        let (b, ins_b, _) = g.create_node("default").input().build_with_ports();
        g.add_child(p1, a).unwrap();
        g.add_child(p2, b).unwrap();
        let e = g.create_edge().source(outs_a[0]).target(ins_b[0]).build();
        let edge = g.get_edge(&e).unwrap();
        assert_eq!(classify_edge(&g, edge), EdgePaintKind::Cross);
    }
}
