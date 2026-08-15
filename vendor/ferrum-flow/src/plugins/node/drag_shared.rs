//! Shared node-drag helpers (sync throttle, [`NodeDragEvent::Tick`]) for flat
//! [`super::interaction`] and nested [`crate::plugins::graph::NestedNodeDragPlugin`].

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{Pixels, Point, px};

use crate::{
    NodeId,
    plugin::{FlowEvent, PluginContext},
    plugins::node::{ActiveNodeDrag, NodeDragEvent, command::DragNodesCommand},
};

pub const DRAG_THRESHOLD: Pixels = px(2.0);
pub const DRAG_COMMAND_INTERVAL: Duration = Duration::from_millis(50);

/// Throttled sync command + [`NodeDragEvent::Tick`] while dragging.
#[derive(Default)]
pub struct DragSessionTimers {
    pub last_drag_command_at: Option<Instant>,
    pub last_node_drag_tick_at: Option<Instant>,
}

/// Nodes to move: selection if `hit` is selected, otherwise just `hit` (local origins).
pub fn collect_drag_nodes(ctx: &PluginContext, hit: NodeId) -> Vec<(NodeId, Point<Pixels>)> {
    let mut nodes = Vec::new();
    if ctx.graph.selected_node().contains(&hit) {
        for id in ctx.graph.selected_node() {
            if let Some(node) = ctx.nodes().get(id) {
                nodes.push((*id, node.point()));
            }
        }
    } else if let Some(node) = ctx.nodes().get(&hit) {
        nodes.push((hit, node.point()));
    }
    nodes
}

pub fn dragged_ids_from_nodes(nodes: &[(NodeId, Point<Pixels>)]) -> Arc<[NodeId]> {
    nodes.iter().map(|(id, _)| *id).collect::<Vec<_>>().into()
}

/// World-space movement since `start_world` exceeds the drag threshold.
pub fn exceeds_drag_threshold(
    ctx: &PluginContext,
    start_world: Point<Pixels>,
    current_screen: Point<Pixels>,
) -> bool {
    let delta = ctx.screen_to_world(current_screen) - start_world;
    delta.x.abs() > DRAG_THRESHOLD || delta.y.abs() > DRAG_THRESHOLD
}

/// Pointer delta in world units (screen origin → canvas).
pub fn screen_pointer_world_delta(
    ctx: &PluginContext,
    start_screen: Point<Pixels>,
    current_screen: Point<Pixels>,
) -> Point<Pixels> {
    let dx = ctx.screen_length_to_world(current_screen.x - start_screen.x);
    let dy = ctx.screen_length_to_world(current_screen.y - start_screen.y);
    Point::new(dx, dy)
}

pub fn start_world_positions(
    ctx: &PluginContext,
    locals: &[(NodeId, Point<Pixels>)],
) -> Vec<(NodeId, Point<Pixels>)> {
    locals
        .iter()
        .map(|(id, local)| (*id, ctx.graph.node_world_point(*id).unwrap_or(*local)))
        .collect()
}

pub fn insert_active_drag(ctx: &mut PluginContext, dragged_ids: Arc<[NodeId]>) {
    ctx.shared_state.insert(ActiveNodeDrag(dragged_ids));
}

pub fn clear_active_drag(ctx: &mut PluginContext) {
    ctx.shared_state.remove::<ActiveNodeDrag>();
}

pub fn run_drag_side_effects(
    ctx: &mut PluginContext,
    start_positions: &[(NodeId, Point<Pixels>)],
    dragged_ids: &Arc<[NodeId]>,
    timers: &mut DragSessionTimers,
    drag_tick_interval: Duration,
) {
    let now = Instant::now();

    if ctx.has_sync_plugin() {
        let should_command = timers
            .last_drag_command_at
            .map(|t| now.duration_since(t) >= DRAG_COMMAND_INTERVAL)
            .unwrap_or(true);
        if should_command {
            ctx.execute_command(DragNodesCommand::new(start_positions, ctx));
            timers.last_drag_command_at = Some(now);
        }
    }

    let should_tick = timers
        .last_node_drag_tick_at
        .map(|t| now.duration_since(t) >= drag_tick_interval)
        .unwrap_or(true);
    if should_tick {
        timers.last_node_drag_tick_at = Some(now);
        ctx.emit(FlowEvent::custom(NodeDragEvent::Tick(Arc::clone(
            dragged_ids,
        ))));
    } else {
        ctx.notify();
    }
}

/// Applies a world-space drag delta to one node's stored **local** position.
pub(crate) trait ApplyNodeDragDelta {
    fn apply(
        &self,
        ctx: &mut PluginContext,
        id: NodeId,
        start_local: Point<Pixels>,
        start_world: Point<Pixels>,
        world_delta: Point<Pixels>,
        dragged: &[NodeId],
    );
}

pub fn apply_drag_to_nodes(
    ctx: &mut PluginContext,
    start_locals: &[(NodeId, Point<Pixels>)],
    start_worlds: &[(NodeId, Point<Pixels>)],
    world_delta: Point<Pixels>,
    dragged: &[NodeId],
    applier: &dyn ApplyNodeDragDelta,
) {
    for (id, start_local) in start_locals {
        let start_world = start_worlds
            .iter()
            .find(|(nid, _)| nid == id)
            .map(|(_, w)| *w)
            .unwrap_or(*start_local);
        applier.apply(ctx, *id, *start_local, start_world, world_delta, dragged);
    }
}
