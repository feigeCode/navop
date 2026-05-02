mod command;
mod drag_events;
mod interaction;

pub use command::DragNodesCommand;
pub use drag_events::{ActiveNodeDrag, NODE_DRAG_TICK_INTERVAL, NodeDragEvent};
use gpui::{Element as _, ElementId, InteractiveElement as _, ParentElement, div};
pub use interaction::NodeInteractionPlugin;

/// Renders the given nodes (and their ports) like [`NodePlugin`], for use on the interaction overlay.
pub(super) fn render_node_cards(
    ctx: &mut RenderContext,
    node_ids: &[crate::NodeId],
    id: &'static str,
) -> gpui::AnyElement {
    ctx.cache_port_offset_with_nodes(node_ids);
    let list = node_ids.iter().filter_map(|node_id| {
        let node = ctx.graph.nodes().get(node_id)?;
        let render = ctx.renderers.get(node.renderer_key());

        let node_render = render.render(node, ctx);

        let port_ids: Vec<crate::PortId> = ctx.cached_port_ids_for_node(node_id).collect();
        let ports = port_ids.iter().filter_map(|port_id| {
            let port = ctx.graph.get_port(port_id)?;
            render.port_render(node, port, ctx)
        });

        Some(
            div()
                .id(ElementId::Uuid(*node_id.as_uuid()))
                .child(node_render)
                .children(ports),
        )
    });

    div().id(id).children(list).into_any()
}

use std::sync::Arc;

use crate::NodeId;
use crate::plugin::{Plugin, RenderContext};
use crate::viewport::ViewportVisibilityCacheKey;

/// Invalidates [`NodePlugin::static_layer_node_ids`] when the viewport changes **or** the active
/// node-drag overlay set changes ([`ActiveNodeDrag`] `Arc` identity + length).
#[derive(Clone, Copy, Debug, PartialEq)]
struct NodeStaticLayerCacheKey {
    viewport: ViewportVisibilityCacheKey,
    nodes_len: usize,
    node_order_len: usize,
    node_order_tail: Option<u128>,
    /// `None` when not dragging; else [`Arc::as_ptr`] + len of the shared drag id list.
    drag_arc: Option<(usize, usize)>,
}

impl NodeStaticLayerCacheKey {
    fn from_render_ctx(ctx: &RenderContext) -> Self {
        let drag = ctx.get_shared_state::<ActiveNodeDrag>();
        Self {
            viewport: ctx.viewport().visibility_cache_key(),
            nodes_len: ctx.graph.nodes().len(),
            node_order_len: ctx.graph.node_order().len(),
            node_order_tail: ctx
                .graph
                .node_order()
                .last()
                .map(|id| id.as_uuid().as_u128()),
            drag_arc: drag.map(|d| {
                let p = Arc::as_ptr(&d.0);
                (p.cast::<NodeId>() as usize, d.0.len())
            }),
        }
    }
}

pub struct NodePlugin {
    static_layer_cache_key: Option<NodeStaticLayerCacheKey>,
    /// Viewport-visible nodes for the static [`RenderLayer::Nodes`] layer, already excluding
    /// [`ActiveNodeDrag`] ids (those render on the interaction overlay).
    static_layer_node_ids: Vec<NodeId>,
}

impl NodePlugin {
    pub fn new() -> Self {
        Self {
            static_layer_cache_key: None,
            static_layer_node_ids: Vec::new(),
        }
    }
}

impl Default for NodePlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for NodePlugin {
    fn name(&self) -> &'static str {
        "node"
    }
    fn priority(&self) -> i32 {
        60
    }
    fn render_layer(&self) -> crate::plugin::RenderLayer {
        crate::plugin::RenderLayer::Nodes
    }
    fn render(&mut self, ctx: &mut RenderContext) -> Option<gpui::AnyElement> {
        let key = NodeStaticLayerCacheKey::from_render_ctx(ctx);
        if self.static_layer_cache_key != Some(key) {
            self.static_layer_cache_key = Some(key);
            let active = ctx.get_shared_state::<ActiveNodeDrag>();
            self.static_layer_node_ids = ctx
                .graph
                .node_order()
                .iter()
                .filter(|node_id| ctx.is_node_visible(node_id))
                .filter(|node_id| !active.is_some_and(|d| d.0.contains(node_id)))
                .copied()
                .collect();
        }

        Some(render_node_cards(
            ctx,
            &self.static_layer_node_ids,
            "static-layer-node-cards",
        ))
    }
}
