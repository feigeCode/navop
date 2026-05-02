use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{MouseButton, Pixels, Point, px};

use crate::{
    NodeId,
    canvas::{Interaction, InteractionResult},
    plugin::{EventResult, FlowEvent, InitPluginContext, InputEvent, Plugin, PluginContext},
    plugins::node::{
        ActiveNodeDrag, NODE_DRAG_TICK_INTERVAL, NodeDragEvent,
        command::{DragNodesCommand, SelecteNodeCommand},
    },
};

const DRAG_THRESHOLD: Pixels = px(2.0);
const DRAG_COMMAND_INTERVAL: Duration = Duration::from_millis(50);

/// Configures [`NodeDragInteraction`] sampling for [`NodeDragEvent::Tick`].
pub struct NodeInteractionPlugin {
    drag_tick_interval: Duration,
}

impl NodeInteractionPlugin {
    pub fn new() -> Self {
        Self {
            drag_tick_interval: NODE_DRAG_TICK_INTERVAL,
        }
    }

    /// Override the drag tick interval (e.g. lower for snappier alignment feedback, higher to reduce load).
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

    fn setup(&mut self, _ctx: &mut InitPluginContext) {}

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
            } else {
                ctx.clear_selected_node();
            }
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
    last_drag_command_at: Option<Instant>,
    last_node_drag_tick_at: Option<Instant>,
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
        /// Stable for this drag; cheap to [`Arc::clone`] into each [`NodeDragEvent::Tick`].
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
            last_drag_command_at: None,
            last_node_drag_tick_at: None,
        }
    }
}

impl Interaction for NodeDragInteraction {
    fn on_mouse_move(
        &mut self,
        ev: &gpui::MouseMoveEvent,
        ctx: &mut PluginContext,
    ) -> crate::canvas::InteractionResult {
        match &self.state {
            NodeDragState::Pending {
                node_id,
                start_mouse,
                ..
            } => {
                let delta = ctx.screen_to_world(ev.position) - *start_mouse;
                if delta.x.abs() > DRAG_THRESHOLD || delta.y.abs() > DRAG_THRESHOLD {
                    let mut nodes = vec![];

                    if ctx.graph.selected_node().contains(node_id) {
                        for id in ctx.graph.selected_node() {
                            if let Some(node) = ctx.nodes().get(id) {
                                nodes.push((*id, node.point()));
                            }
                        }
                    } else if let Some(node) = ctx.nodes().get(node_id) {
                        nodes.push((*node_id, node.point()));
                    }
                    let dragged_ids: Arc<[NodeId]> =
                        nodes.iter().map(|(id, _)| *id).collect::<Vec<_>>().into();
                    self.state = NodeDragState::Draging {
                        start_mouse: ev.position,
                        start_positions: nodes,
                        dragged_ids: Arc::clone(&dragged_ids),
                    };
                    ctx.shared_state.insert(ActiveNodeDrag(dragged_ids));

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

                let now = Instant::now();

                if ctx.has_sync_plugin() {
                    let should_command = self
                        .last_drag_command_at
                        .map(|t| now.duration_since(t) >= DRAG_COMMAND_INTERVAL)
                        .unwrap_or(true);
                    if should_command {
                        ctx.execute_command(DragNodesCommand::new(start_positions, ctx));
                        self.last_drag_command_at = Some(now);
                    }
                }

                let should_tick = self
                    .last_node_drag_tick_at
                    .map(|t| now.duration_since(t) >= self.drag_tick_interval)
                    .unwrap_or(true);
                if should_tick {
                    self.last_node_drag_tick_at = Some(now);
                    ctx.emit(FlowEvent::custom(NodeDragEvent::Tick(Arc::clone(
                        dragged_ids,
                    ))));
                } else {
                    ctx.notify();
                }
            }
        }
        InteractionResult::Continue
    }
    fn on_mouse_up(
        &mut self,
        _ev: &gpui::MouseUpEvent,
        ctx: &mut PluginContext,
    ) -> crate::canvas::InteractionResult {
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
            NodeDragState::Draging { dragged_ids, .. } => Some(super::render_node_cards(
                ctx,
                dragged_ids.as_ref(),
                "draging-node-cards",
            )),
            NodeDragState::Pending { .. } => None,
        }
    }
}
