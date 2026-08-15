//! Pointer helpers for [`super::GraphPlugin`] on nested / graph-only canvases.
//!
//! [`crate::plugins::EdgePlugin`] caches ports in its own `render`; here we pre-cache before hit-test
//! because edges are painted inside [`super::GraphPlugin`] instead.

use gpui::{Pixels, Point};

use crate::plugin::{EventResult, PluginContext};
use crate::plugins::edge::{edge_hit_at, handle_edge_mouse_down};

/// Cache port layout for edges that could be hit (same visibility rule as [`super::plan::edge_is_visible`]).
pub(crate) fn cache_visible_edge_ports_for_hit(ctx: &mut PluginContext) {
    let edge_ids: Vec<crate::EdgeId> = ctx.graph.edges().keys().copied().collect();
    for id in edge_ids {
        let Some(edge) = ctx.graph.get_edge(&id) else {
            continue;
        };
        let Some(source_port) = ctx.graph.get_port(&edge.source_port) else {
            continue;
        };
        let Some(target_port) = ctx.graph.get_port(&edge.target_port) else {
            continue;
        };
        if ctx.is_node_visible(&source_port.node_id())
            || ctx.is_node_visible(&target_port.node_id())
        {
            ctx.cache_port_offset_with_edge(&id);
        }
    }
}

/// Hit-test after port cache (nested graph paint does not run [`crate::plugins::EdgePlugin::render`]).
pub(crate) fn graph_edge_hit_at(
    mouse: Point<Pixels>,
    ctx: &mut PluginContext,
) -> Option<crate::EdgeId> {
    cache_visible_edge_ports_for_hit(ctx);
    edge_hit_at(mouse, ctx)
}

/// Edge select / clear for [`super::GraphPlugin::on_event`].
pub(crate) fn graph_handle_edge_mouse_down(
    position: Point<Pixels>,
    shift: bool,
    ctx: &mut PluginContext,
) -> EventResult {
    cache_visible_edge_ports_for_hit(ctx);
    handle_edge_mouse_down(position, shift, ctx)
}
