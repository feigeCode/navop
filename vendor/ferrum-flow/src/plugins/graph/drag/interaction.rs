use std::sync::Arc;
use std::time::Duration;

use gpui::{Pixels, Point};

use crate::{
    NodeId,
    canvas::{Interaction, InteractionResult},
    plugin::PluginContext,
    plugins::node::{
        DragNodesCommand, DragSessionTimers, NodeDragEvent, SelecteNodeCommand,
        apply_drag_to_nodes, clear_active_drag, collect_drag_nodes, dragged_ids_from_nodes,
        exceeds_drag_threshold, insert_active_drag, node_ids_for_drag_overlay,
        run_drag_side_effects, screen_pointer_world_delta, start_world_positions,
    },
};

use super::super::render_hierarchy_drag_overlay;
use super::{apply::HierarchyDragDelta, policy::BoundaryDragPolicy};

pub struct NestedNodeDragInteraction {
    state: NestedDragState,
    drag_tick_interval: Duration,
    boundary_drag_policy: BoundaryDragPolicy,
    timers: DragSessionTimers,
}

enum NestedDragState {
    Pending {
        node_id: NodeId,
        start_mouse_world: Point<Pixels>,
        shift: bool,
    },
    Dragging {
        start_screen: Point<Pixels>,
        start_locals: Vec<(NodeId, Point<Pixels>)>,
        start_worlds: Vec<(NodeId, Point<Pixels>)>,
        dragged_ids: Arc<[NodeId]>,
    },
}

impl NestedNodeDragInteraction {
    pub(crate) fn start(
        node_id: NodeId,
        start_mouse_world: Point<Pixels>,
        shift: bool,
        drag_tick_interval: Duration,
        boundary_drag_policy: BoundaryDragPolicy,
    ) -> Self {
        Self {
            state: NestedDragState::Pending {
                node_id,
                start_mouse_world,
                shift,
            },
            drag_tick_interval,
            boundary_drag_policy,
            timers: DragSessionTimers::default(),
        }
    }
}

impl Interaction for NestedNodeDragInteraction {
    fn on_mouse_move(
        &mut self,
        ev: &gpui::MouseMoveEvent,
        ctx: &mut PluginContext,
    ) -> InteractionResult {
        match &self.state {
            NestedDragState::Pending {
                node_id,
                start_mouse_world,
                ..
            } => {
                if exceeds_drag_threshold(ctx, *start_mouse_world, ev.position) {
                    let start_locals = collect_drag_nodes(ctx, *node_id);
                    let start_worlds = start_world_positions(ctx, &start_locals);
                    let dragged_ids = dragged_ids_from_nodes(&start_locals);
                    insert_active_drag(ctx, Arc::clone(&dragged_ids));
                    self.state = NestedDragState::Dragging {
                        start_screen: ev.position,
                        start_locals,
                        start_worlds,
                        dragged_ids,
                    };
                    ctx.notify();
                }
            }
            NestedDragState::Dragging {
                start_screen,
                start_locals,
                start_worlds,
                dragged_ids,
            } => {
                let world_delta = screen_pointer_world_delta(ctx, *start_screen, ev.position);
                let applier = HierarchyDragDelta {
                    policy: self.boundary_drag_policy,
                };
                apply_drag_to_nodes(
                    ctx,
                    start_locals,
                    start_worlds,
                    world_delta,
                    dragged_ids.as_ref(),
                    &applier,
                );
                run_drag_side_effects(
                    ctx,
                    start_locals,
                    dragged_ids,
                    &mut self.timers,
                    self.drag_tick_interval,
                );
            }
        }
        InteractionResult::Continue
    }

    fn on_mouse_up(
        &mut self,
        _ev: &gpui::MouseUpEvent,
        ctx: &mut PluginContext,
    ) -> InteractionResult {
        clear_active_drag(ctx);
        match &self.state {
            NestedDragState::Pending { node_id, shift, .. } => {
                ctx.emit(crate::plugin::FlowEvent::custom(NodeDragEvent::End));
                ctx.execute_command(SelecteNodeCommand::new(*node_id, *shift, ctx));
                InteractionResult::End
            }
            NestedDragState::Dragging { start_locals, .. } => {
                ctx.emit(crate::plugin::FlowEvent::custom(NodeDragEvent::End));
                ctx.execute_command(DragNodesCommand::new(start_locals, ctx));
                InteractionResult::End
            }
        }
    }

    fn render(&self, ctx: &mut crate::plugin::RenderContext) -> Option<gpui::AnyElement> {
        let NestedDragState::Dragging { dragged_ids, .. } = &self.state else {
            return None;
        };
        let overlay_ids = node_ids_for_drag_overlay(ctx.graph, dragged_ids.as_ref());
        render_hierarchy_drag_overlay(ctx, &overlay_ids)
    }
}
