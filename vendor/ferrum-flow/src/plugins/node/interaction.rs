use std::sync::Arc;
use std::time::Duration;

use gpui::{MouseButton, Pixels, Point};

use super::{DragSessionTimers, exceeds_drag_threshold, run_drag_side_effects};
use crate::{
    NodeId,
    canvas::{Interaction, InteractionResult},
    plugin::{EventResult, FlowEvent, InputEvent, Plugin, PluginContext},
    plugins::node::{
        ActiveNodeDrag, NODE_DRAG_TICK_INTERVAL, NodeDragEvent, collect_drag_nodes,
        command::{DragNodesCommand, SelecteNodeCommand},
        dragged_ids_from_nodes, insert_active_drag,
    },
};

/// Flat canvas node drag. Nested graphs use [`crate::plugins::graph::NestedNodeDragPlugin`] instead.
pub struct NodeInteractionPlugin {
    drag_tick_interval: Duration,
}

impl NodeInteractionPlugin {
    pub fn new() -> Self {
        Self {
            drag_tick_interval: NODE_DRAG_TICK_INTERVAL,
        }
    }

    pub fn with_drag_tick_interval(interval: Duration) -> Self {
        Self {
            drag_tick_interval: interval,
        }
    }
}

impl Default for NodeInteractionPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for NodeInteractionPlugin {
    fn name(&self) -> &'static str {
        "node_interaction"
    }

    fn on_event(&mut self, event: &FlowEvent, ctx: &mut PluginContext) -> EventResult {
        if let FlowEvent::Input(InputEvent::MouseDown(ev)) = event {
            if ev.button != MouseButton::Left {
                return EventResult::Continue;
            }
            let mouse_world = ctx.screen_to_world(ev.position);
            if let Some(node_id) = ctx.hit_node(mouse_world) {
                ctx.start_interaction(NodeDragInteraction::start(
                    node_id,
                    mouse_world,
                    ev.modifiers.shift,
                    self.drag_tick_interval,
                ));
                return EventResult::Stop;
            }
            ctx.clear_selected_node();
        }
        EventResult::Continue
    }

    fn priority(&self) -> i32 {
        120
    }
}

pub struct NodeDragInteraction {
    state: NodeDragState,
    drag_tick_interval: Duration,
    timers: DragSessionTimers,
}

enum NodeDragState {
    Pending {
        node_id: NodeId,
        start_mouse: Point<Pixels>,
        shift: bool,
    },
    Draging {
        start_mouse: Point<Pixels>,
        start_positions: Vec<(NodeId, Point<Pixels>)>,
        dragged_ids: Arc<[NodeId]>,
    },
}

impl NodeDragInteraction {
    fn start(
        node_id: NodeId,
        start_mouse: Point<Pixels>,
        shift: bool,
        drag_tick_interval: Duration,
    ) -> Self {
        Self {
            state: NodeDragState::Pending {
                node_id,
                start_mouse,
                shift,
            },
            drag_tick_interval,
            timers: DragSessionTimers::default(),
        }
    }
}

impl Interaction for NodeDragInteraction {
    fn on_mouse_move(
        &mut self,
        ev: &gpui::MouseMoveEvent,
        ctx: &mut PluginContext,
    ) -> InteractionResult {
        match &self.state {
            NodeDragState::Pending {
                node_id,
                start_mouse,
                ..
            } => {
                if exceeds_drag_threshold(ctx, *start_mouse, ev.position) {
                    let nodes = collect_drag_nodes(ctx, *node_id);
                    let dragged_ids: Arc<[NodeId]> = dragged_ids_from_nodes(&nodes);
                    insert_active_drag(ctx, Arc::clone(&dragged_ids));
                    self.state = NodeDragState::Draging {
                        start_mouse: ev.position,
                        start_positions: nodes,
                        dragged_ids,
                    };

                    ctx.notify();
                }
            }
            NodeDragState::Draging {
                start_mouse,
                start_positions,
                dragged_ids,
            } => {
                let dx = ctx.screen_length_to_world(ev.position.x - start_mouse.x);
                let dy = ctx.screen_length_to_world(ev.position.y - start_mouse.y);
                for (id, point) in start_positions.iter() {
                    if let Some(node) = ctx.get_node_mut(id) {
                        node.set_position(point.x + dx, point.y + dy);
                    }
                }

                run_drag_side_effects(
                    ctx,
                    start_positions,
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
        ctx.shared_state.remove::<ActiveNodeDrag>();
        match &self.state {
            NodeDragState::Pending { node_id, shift, .. } => {
                ctx.emit(FlowEvent::custom(NodeDragEvent::End));
                ctx.execute_command(SelecteNodeCommand::new(*node_id, *shift, ctx));
                InteractionResult::End
            }
            NodeDragState::Draging {
                start_positions, ..
            } => {
                ctx.emit(FlowEvent::custom(NodeDragEvent::End));
                ctx.execute_command(DragNodesCommand::new(start_positions, ctx));
                InteractionResult::End
            }
        }
    }

    fn render(&self, ctx: &mut crate::plugin::RenderContext) -> Option<gpui::AnyElement> {
        match &self.state {
            NodeDragState::Draging { dragged_ids, .. } => {
                let overlay_ids = super::node_ids_for_drag_overlay(ctx.graph, dragged_ids.as_ref());
                Some(super::render_node_cards(
                    ctx,
                    &overlay_ids,
                    "draging-node-cards",
                    None,
                ))
            }
            NodeDragState::Pending { .. } => None,
        }
    }
}
