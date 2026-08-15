//! Level-of-detail policy for node painting at large graph sizes / low zoom.

use std::collections::HashSet;

use crate::{Graph, NodeId};

/// How much detail to paint for one node card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeRenderLod {
    /// Full [`NodeRenderer::render`](crate::NodeRenderer::render) body and ports.
    Full,
    /// Theme shell only ([`RenderContext::node_card_shell`](crate::RenderContext::node_card_shell)).
    ShellOnly,
}

/// Tunable thresholds for [`NodeRenderLod`]. Owned by [`GraphPlugin`](crate::plugins::GraphPlugin).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeRenderLodConfig {
    /// Master switch; when false, every node uses [`NodeRenderLod::Full`].
    pub enabled: bool,
    /// LOD may apply only when the graph has at least this many nodes.
    pub min_node_count: usize,
    /// At or below this zoom, nodes may downgrade (with a large enough graph).
    pub max_zoom_shell_only: f32,
    /// Screen-space width/height (px) below which a node may downgrade even if zoom is higher.
    pub min_screen_node_size: f32,
}

impl Default for NodeRenderLodConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_node_count: 500,
            max_zoom_shell_only: 0.2,
            min_screen_node_size: 12.0,
        }
    }
}

/// Optional LOD for [`super::render_node_cards`]; omit for full detail ([`NodePlugin`]).
pub(crate) struct NodeCardsLod<'a> {
    pub config: &'a NodeRenderLodConfig,
    pub drag_overlay: &'a HashSet<NodeId>,
}

/// Resolves LOD for one node given viewport zoom and interaction state.
pub fn resolve_node_render_lod(
    graph: &Graph,
    config: &NodeRenderLodConfig,
    zoom: f32,
    node_id: &NodeId,
    selected: &HashSet<NodeId>,
    drag_overlay: &HashSet<NodeId>,
) -> NodeRenderLod {
    if !config.enabled || graph.nodes().len() < config.min_node_count {
        return NodeRenderLod::Full;
    }
    if selected.contains(node_id) || drag_overlay.contains(node_id) {
        return NodeRenderLod::Full;
    }
    let Some(node) = graph.get_node(node_id) else {
        return NodeRenderLod::Full;
    };

    let screen_w: f32 = (node.size_ref().width * zoom).into();
    let screen_h: f32 = (node.size_ref().height * zoom).into();
    let screen_min = screen_w.min(screen_h);

    let zoom_small = zoom <= config.max_zoom_shell_only;
    let screen_tiny = screen_min < config.min_screen_node_size;

    if zoom_small || screen_tiny {
        NodeRenderLod::ShellOnly
    } else {
        NodeRenderLod::Full
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Graph;

    fn config() -> NodeRenderLodConfig {
        NodeRenderLodConfig {
            enabled: true,
            min_node_count: 2,
            max_zoom_shell_only: 0.2,
            min_screen_node_size: 12.0,
        }
    }

    #[test]
    fn disabled_or_small_graph_is_always_full() {
        let mut g = Graph::new();
        let a = g.create_node("default").size(100.0, 40.0).build();
        let cfg = config();
        assert_eq!(
            resolve_node_render_lod(&g, &cfg, 0.05, &a, &HashSet::new(), &HashSet::new()),
            NodeRenderLod::Full,
            "graph below min_node_count"
        );
        let mut disabled = cfg;
        disabled.enabled = false;
        let b = g.create_node("default").size(100.0, 40.0).build();
        assert_eq!(
            resolve_node_render_lod(&g, &disabled, 0.05, &b, &HashSet::new(), &HashSet::new()),
            NodeRenderLod::Full
        );
        let _ = b;
    }

    #[test]
    fn low_zoom_shell_only_when_graph_large_enough() {
        let mut g = Graph::new();
        let a = g.create_node("default").size(100.0, 40.0).build();
        let b = g.create_node("default").size(100.0, 40.0).build();
        let cfg = config();
        assert_eq!(
            resolve_node_render_lod(&g, &cfg, 0.15, &a, &HashSet::new(), &HashSet::new()),
            NodeRenderLod::ShellOnly
        );
        let _ = b;
    }

    #[test]
    fn selected_and_drag_overlay_stay_full() {
        let mut g = Graph::new();
        let a = g.create_node("default").size(100.0, 40.0).build();
        let b = g.create_node("default").size(100.0, 40.0).build();
        let cfg = config();
        let mut selected = HashSet::new();
        selected.insert(a);
        assert_eq!(
            resolve_node_render_lod(&g, &cfg, 0.05, &a, &selected, &HashSet::new()),
            NodeRenderLod::Full
        );
        let mut drag = HashSet::new();
        drag.insert(b);
        assert_eq!(
            resolve_node_render_lod(&g, &cfg, 0.05, &b, &HashSet::new(), &drag),
            NodeRenderLod::Full
        );
    }

    #[test]
    fn tiny_screen_size_triggers_shell_without_low_zoom() {
        let mut g = Graph::new();
        let a = g.create_node("default").size(20.0, 20.0).build();
        let b = g.create_node("default").size(20.0, 20.0).build();
        let cfg = config();
        // zoom 0.5 → screen 10px, below min_screen_node_size 12
        assert_eq!(
            resolve_node_render_lod(&g, &cfg, 0.5, &a, &HashSet::new(), &HashSet::new()),
            NodeRenderLod::ShellOnly
        );
        let _ = b;
    }
}
