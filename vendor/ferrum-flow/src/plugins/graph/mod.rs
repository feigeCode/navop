//! Standalone node + edge paint ([`GraphPlugin`]); replaces [`crate::plugins::NodePlugin`] and
//! [`crate::plugins::EdgePlugin`] on the same canvas (do not register those together).
//!
//! Flat graphs: one node layer + edge overlay. Nested graphs: group z-order (shell → intra edges →
//! children → ports) plus cross-parent edge overlay.
//!
//! Z-order per group anchor (product rules):
//! 1. Parent shell (background)
//! 2. Intra-parent edges (same direct parent; below children)
//! 3. Children (nested group or leaf cards)
//! 4. Parent ports
//!
//! Cross-parent edges are painted in a top overlay within this layer.
//!
//! Pointer: edge hit-test via [`pointer::graph_handle_edge_mouse_down`] (pre-caches ports; see
//! [`pointer`] module). Register **either** `EdgePlugin` or `GraphPlugin`, not both.

mod drag;
mod hierarchy;
mod plan;
mod pointer;

use std::collections::{HashMap, HashSet};

use gpui::{
    AnyElement, Element as _, ElementId, InteractiveElement as _, MouseButton, ParentElement as _,
    Styled as _, div,
};

use crate::EdgeId;
use crate::plugin::{
    EventResult, FlowEvent, InputEvent, Plugin, PluginContext, RenderContext, RenderLayer,
};
use crate::plugins::edge::{EdgeGeometry, edge_geometry2, edges_canvas_element};
use crate::plugins::node::render_node_card;
use crate::plugins::node::{
    ActiveNodeDrag, node_ids_for_drag_overlay, render_lod::NodeCardsLod, render_node_cards,
    render_node_ports, render_node_shell,
};
use pointer::graph_handle_edge_mouse_down;

pub use crate::plugins::node::{NodeRenderLod, NodeRenderLodConfig};
pub use drag::{BoundaryDragPolicy, NestedNodeDragPlugin};
use hierarchy::GraphHierarchy;
pub use plan::{EdgePaintKind, classify_edge};

pub struct GraphPlugin {
    lod_config: NodeRenderLodConfig,
}

impl GraphPlugin {
    pub fn new() -> Self {
        Self {
            lod_config: NodeRenderLodConfig::default(),
        }
    }

    pub fn with_lod_config(lod_config: NodeRenderLodConfig) -> Self {
        Self { lod_config }
    }

    pub fn lod_config(&self) -> &NodeRenderLodConfig {
        &self.lod_config
    }

    pub fn set_lod_config(&mut self, lod_config: NodeRenderLodConfig) {
        self.lod_config = lod_config;
    }
}

impl Default for GraphPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for GraphPlugin {
    fn name(&self) -> &'static str {
        "graph"
    }

    fn on_event(&mut self, event: &FlowEvent, ctx: &mut PluginContext) -> EventResult {
        if let FlowEvent::Input(InputEvent::MouseDown(ev)) = event {
            if ev.button != MouseButton::Left {
                return EventResult::Continue;
            }
            return graph_handle_edge_mouse_down(ev.position, ev.modifiers.shift, ctx);
        }
        EventResult::Continue
    }

    /// Same as [`crate::plugins::EdgePlugin`] for pointer dispatch (before [`crate::plugins::SelectionPlugin`] at 100).
    fn priority(&self) -> i32 {
        120
    }

    fn render_layer(&self) -> RenderLayer {
        RenderLayer::Nodes
    }

    fn render(&mut self, ctx: &mut RenderContext) -> Option<AnyElement> {
        let drag_overlay: HashSet<_> = ctx
            .get_shared_state::<ActiveNodeDrag>()
            .map(|d| {
                node_ids_for_drag_overlay(ctx.graph, d.0.as_ref())
                    .into_iter()
                    .collect()
            })
            .unwrap_or_default();

        let lod_ctx = NodeCardsLod {
            config: &self.lod_config,
            drag_overlay: &drag_overlay,
        };
        let lod = Some(&lod_ctx);

        if !ctx.graph.has_node_hierarchy() {
            return render_flat_graph(ctx, &drag_overlay, lod);
        }

        let stroke = ctx.theme.edge_stroke;
        let stroke_sel = ctx.theme.edge_stroke_selected;
        let selected = ctx.graph.selected_edge().clone();
        let (intra_by_parent, cross_edges) = collect_hierarchy_edges(ctx);

        let paint_order = ctx.graph.paint_order();
        let mut body_children: Vec<AnyElement> = Vec::new();
        let mut covered = HashSet::new();

        // DOM order follows `paint_order` (roots / siblings / click-to-front), not
        // "all groups first, then root leaves".
        for &id in &paint_order {
            if covered.contains(&id) || drag_overlay.contains(&id) {
                continue;
            }
            if !ctx.is_node_visible(&id) {
                continue;
            }

            if ctx.graph.is_top_level_group_anchor(id) {
                if let Some(el) = render_group_anchor(
                    ctx,
                    id,
                    &paint_order,
                    &drag_overlay,
                    GroupPaintPass::Static,
                    &intra_by_parent,
                    &selected,
                    stroke,
                    stroke_sel,
                    &mut covered,
                    lod,
                ) {
                    body_children.push(el);
                }
                covered.insert(id);
                for d in ctx.graph.descendants(id) {
                    covered.insert(d);
                }
                continue;
            }

            if ctx.graph.children_of(id).is_empty() {
                body_children.push(render_node_cards(ctx, &[id], "graph-root-leaf", lod));
                covered.insert(id);
            }
        }

        let cross_static = cross_edges_for_static_layer(&cross_edges, &drag_overlay, ctx);
        let cross_layer = if cross_static.is_empty() {
            None
        } else {
            Some(div().absolute().size_full().child(edges_canvas_element(
                cross_static,
                selected.clone(),
                stroke,
                stroke_sel,
            )))
        };

        Some(
            div()
                .id("graph-layer")
                .absolute()
                .size_full()
                .child(div().absolute().size_full().children(body_children))
                .children(cross_layer)
                .into_any(),
        )
    }
}

/// Flat canvas: all nodes in [`Graph::paint_order`], all visible edges on top (like legacy plugins).
fn render_flat_graph(
    ctx: &mut RenderContext,
    drag_overlay: &HashSet<crate::NodeId>,
    lod: Option<&NodeCardsLod<'_>>,
) -> Option<AnyElement> {
    let stroke = ctx.theme.edge_stroke;
    let stroke_sel = ctx.theme.edge_stroke_selected;
    let selected = ctx.graph.selected_edge().clone();

    let mut layer = div().id("graph-layer").absolute().size_full().child({
        let list = ctx.graph.paint_order_iter().filter_map(|node_id| {
            let node = ctx.graph.nodes().get(&node_id)?;
            if !ctx.is_node_visible_node(node) || drag_overlay.contains(&node_id) {
                return None;
            }
            Some(render_node_card(ctx, node_id, node, lod))
        });
        div().children(list).into_any()
    });

    let mut edges: Vec<(EdgeId, Option<EdgeGeometry>)> = Vec::new();
    for (_, edge) in ctx.graph.edges().iter() {
        if !plan::edge_is_visible(ctx, edge) {
            continue;
        }
        ctx.cache_port_offset_with_edge(&edge.id);
        edges.push((edge.id, edge_geometry2(edge, ctx)));
    }

    if !edges.is_empty() {
        layer = layer.child(
            div()
                .absolute()
                .size_full()
                .child(edges_canvas_element(edges, selected, stroke, stroke_sel)),
        );
    }
    Some(layer.into_any())
}

/// Flat graph: dragged nodes excluded from [`render_flat_graph`] static layer, redrawn here.
fn render_flat_drag_overlay(
    ctx: &mut RenderContext,
    overlay_ids: &[crate::NodeId],
) -> Option<AnyElement> {
    let drag_overlay: HashSet<_> = overlay_ids.iter().copied().collect();
    let node_ids: Vec<_> = ctx
        .graph
        .paint_order()
        .iter()
        .filter(|id| drag_overlay.contains(id))
        .filter(|id| ctx.is_node_visible(id))
        .copied()
        .collect();

    if node_ids.is_empty() {
        return None;
    }

    Some(
        div()
            .id("graph-drag-overlay")
            .absolute()
            .size_full()
            .child(render_node_cards(ctx, &node_ids, "graph-drag-flat", None))
            .into_any(),
    )
}

/// Interaction-layer paint while dragging ([`crate::plugins::graph::NestedNodeDragPlugin`]).
pub(crate) fn render_hierarchy_drag_overlay(
    ctx: &mut RenderContext,
    overlay_ids: &[crate::NodeId],
) -> Option<AnyElement> {
    if overlay_ids.is_empty() {
        return None;
    }

    if !ctx.graph.has_node_hierarchy() {
        return render_flat_drag_overlay(ctx, overlay_ids);
    }

    let drag_overlay: HashSet<_> = overlay_ids.iter().copied().collect();
    let stroke = ctx.theme.edge_stroke;
    let stroke_sel = ctx.theme.edge_stroke_selected;
    let selected = ctx.graph.selected_edge().clone();
    let (intra_by_parent, cross_edges) = collect_hierarchy_edges(ctx);

    let paint_order = ctx.graph.paint_order();
    let mut body_children: Vec<AnyElement> = Vec::new();
    let mut covered = HashSet::new();

    for &id in &paint_order {
        if !drag_overlay.contains(&id) || covered.contains(&id) {
            continue;
        }
        if !ctx.is_node_visible(&id) {
            continue;
        }

        // Any in-overlay group (L2+), not only top-level L1 — nested sub-groups are skipped by
        // `is_top_level_group_anchor` but must still paint when dragged without their parent.
        if !ctx.graph.children_of(id).is_empty() {
            if let Some(el) = render_group_anchor(
                ctx,
                id,
                &paint_order,
                &drag_overlay,
                GroupPaintPass::DragOverlay,
                &intra_by_parent,
                &selected,
                stroke,
                stroke_sel,
                &mut covered,
                None,
            ) {
                body_children.push(el);
            }
            covered.insert(id);
            for d in ctx.graph.descendants(id) {
                covered.insert(d);
            }
            continue;
        }

        body_children.push(render_node_cards(ctx, &[id], "graph-drag-leaf", None));
        covered.insert(id);
    }

    let cross_in_drag = cross_edges_for_drag_overlay(&cross_edges, &drag_overlay, ctx);

    if body_children.is_empty() && cross_in_drag.is_empty() {
        return None;
    }

    let cross_layer = if cross_in_drag.is_empty() {
        None
    } else {
        Some(div().absolute().size_full().child(edges_canvas_element(
            cross_in_drag,
            selected.clone(),
            stroke,
            stroke_sel,
        )))
    };

    Some(
        div()
            .id("graph-drag-overlay")
            .absolute()
            .size_full()
            .child(div().absolute().size_full().children(body_children))
            .children(cross_layer)
            .into_any(),
    )
}

type HierarchyEdges = (
    HashMap<crate::NodeId, Vec<(EdgeId, Option<EdgeGeometry>)>>,
    Vec<(EdgeId, Option<EdgeGeometry>)>,
);

fn collect_hierarchy_edges(ctx: &mut RenderContext) -> HierarchyEdges {
    let mut intra_by_parent: HashMap<crate::NodeId, Vec<(EdgeId, Option<EdgeGeometry>)>> =
        HashMap::new();
    let mut cross_edges: Vec<(EdgeId, Option<EdgeGeometry>)> = Vec::new();

    for (_, edge) in ctx.graph.edges().iter() {
        if !plan::edge_is_visible(ctx, edge) {
            continue;
        }
        ctx.cache_port_offset_with_edge(&edge.id);
        let geom = edge_geometry2(edge, ctx);
        match classify_edge(ctx.graph, edge) {
            EdgePaintKind::IntraParent { parent } => {
                intra_by_parent
                    .entry(parent)
                    .or_default()
                    .push((edge.id, geom));
            }
            EdgePaintKind::Cross => cross_edges.push((edge.id, geom)),
        }
    }

    (intra_by_parent, cross_edges)
}

fn edge_endpoint_node_ids(
    ctx: &RenderContext,
    edge: &crate::Edge,
) -> Option<(crate::NodeId, crate::NodeId)> {
    let sp = ctx.graph.get_port(&edge.source_port)?;
    let tp = ctx.graph.get_port(&edge.target_port)?;
    Some((sp.node_id(), tp.node_id()))
}

fn edge_any_endpoint_in_set(
    ctx: &RenderContext,
    edge: &crate::Edge,
    nodes: &HashSet<crate::NodeId>,
) -> bool {
    let Some((s, t)) = edge_endpoint_node_ids(ctx, edge) else {
        return false;
    };
    nodes.contains(&s) || nodes.contains(&t)
}

fn cross_edges_for_static_layer(
    cross_edges: &[(EdgeId, Option<EdgeGeometry>)],
    drag_overlay: &HashSet<crate::NodeId>,
    ctx: &RenderContext,
) -> Vec<(EdgeId, Option<EdgeGeometry>)> {
    if drag_overlay.is_empty() {
        return cross_edges.to_vec();
    }
    cross_edges
        .iter()
        .filter(|(id, _)| {
            ctx.graph
                .get_edge(id)
                .is_some_and(|e| !edge_any_endpoint_in_set(ctx, e, drag_overlay))
        })
        .map(|(id, geom)| (*id, geom.clone()))
        .collect()
}

fn cross_edges_for_drag_overlay(
    cross_edges: &[(EdgeId, Option<EdgeGeometry>)],
    drag_overlay: &HashSet<crate::NodeId>,
    ctx: &RenderContext,
) -> Vec<(EdgeId, Option<EdgeGeometry>)> {
    cross_edges
        .iter()
        .filter(|(id, _)| {
            ctx.graph
                .get_edge(id)
                .is_some_and(|e| edge_any_endpoint_in_set(ctx, e, drag_overlay))
        })
        .map(|(id, geom)| (*id, geom.clone()))
        .collect()
}

#[derive(Clone, Copy)]
enum GroupPaintPass {
    /// Main graph layer: omit nodes in [`ActiveNodeDrag`].
    Static,
    /// Interaction overlay: only nodes in the drag set.
    DragOverlay,
}

#[allow(clippy::too_many_arguments)]
fn render_group_anchor(
    ctx: &mut RenderContext,
    anchor: crate::NodeId,
    paint_order: &[crate::NodeId],
    drag_overlay: &HashSet<crate::NodeId>,
    pass: GroupPaintPass,
    intra_by_parent: &HashMap<crate::NodeId, Vec<(EdgeId, Option<EdgeGeometry>)>>,
    selected: &HashSet<EdgeId>,
    stroke: u32,
    stroke_sel: u32,
    covered: &mut HashSet<crate::NodeId>,
    lod: Option<&NodeCardsLod<'_>>,
) -> Option<AnyElement> {
    match pass {
        GroupPaintPass::Static if drag_overlay.contains(&anchor) => return None,
        GroupPaintPass::DragOverlay if !drag_overlay.contains(&anchor) => return None,
        _ => {}
    }

    let mut group = div().id(ElementId::Uuid(*anchor.as_uuid()));

    if let Some(shell) = render_node_shell(ctx, &anchor) {
        group = group.child(shell);
    }

    if let Some(edges) = intra_by_parent.get(&anchor)
        && !edges.is_empty()
    {
        group = group.child(
            div()
                .id(ElementId::Uuid(*anchor.as_uuid()))
                .absolute()
                .size_full()
                .child(edges_canvas_element(
                    edges.clone(),
                    selected.clone(),
                    stroke,
                    stroke_sel,
                )),
        );
    }

    let child_ids: Vec<_> = paint_order
        .iter()
        .filter(|id| {
            ctx.graph
                .get_node(id)
                .and_then(|n| n.parent())
                .is_some_and(|p| p == anchor)
        })
        .copied()
        .filter(|id| child_in_pass(*id, pass, drag_overlay))
        .filter(|id| ctx.is_node_visible(id))
        .collect();

    for child in child_ids {
        covered.insert(child);
        if !ctx.graph.children_of(child).is_empty() {
            if let Some(nested) = render_group_anchor(
                ctx,
                child,
                paint_order,
                drag_overlay,
                pass,
                intra_by_parent,
                selected,
                stroke,
                stroke_sel,
                covered,
                lod,
            ) {
                group = group.child(nested);
            }
        } else {
            group = group.child(render_node_cards(ctx, &[child], "graph-child", lod));
        }
    }

    if let Some(ports) = render_node_ports(ctx, &anchor) {
        group = group.child(ports);
    }

    Some(group.into_any())
}

fn child_in_pass(
    id: crate::NodeId,
    pass: GroupPaintPass,
    drag_overlay: &HashSet<crate::NodeId>,
) -> bool {
    match pass {
        GroupPaintPass::Static => !drag_overlay.contains(&id),
        GroupPaintPass::DragOverlay => drag_overlay.contains(&id),
    }
}
