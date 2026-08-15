//! Nested (parent/child) node dragging — register with [`super::GraphPlugin`], not [`crate::plugins::NodePlugin`].

mod apply;
mod boundary;
mod interaction;
mod policy;

use std::time::Duration;

use gpui::MouseButton;

use crate::{
    plugin::{EventResult, FlowEvent, InputEvent, Plugin, PluginContext},
    plugins::node::NODE_DRAG_TICK_INTERVAL,
};

use super::pointer::graph_edge_hit_at;

pub use interaction::NestedNodeDragInteraction;
pub use policy::BoundaryDragPolicy;

pub struct NestedNodeDragPlugin {
    drag_tick_interval: Duration,
    boundary_drag_policy: BoundaryDragPolicy,
}

impl NestedNodeDragPlugin {
    pub fn new() -> Self {
        Self {
            drag_tick_interval: NODE_DRAG_TICK_INTERVAL,
            boundary_drag_policy: BoundaryDragPolicy::default(),
        }
    }

    pub fn with_drag_tick_interval(interval: Duration) -> Self {
        Self {
            drag_tick_interval: interval,
            ..Self::new()
        }
    }

    pub fn with_boundary_drag_policy(policy: BoundaryDragPolicy) -> Self {
        Self {
            boundary_drag_policy: policy,
            ..Self::new()
        }
    }

    pub fn boundary_drag_policy(&self) -> BoundaryDragPolicy {
        self.boundary_drag_policy
    }
}

impl Default for NestedNodeDragPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for NestedNodeDragPlugin {
    fn name(&self) -> &'static str {
        "graph_node_drag"
    }

    fn on_event(&mut self, event: &FlowEvent, ctx: &mut PluginContext) -> EventResult {
        if let FlowEvent::Input(InputEvent::MouseDown(ev)) = event {
            if ev.button != MouseButton::Left {
                return EventResult::Continue;
            }

            // Edges (incl. cross-parent portal layer) win over parent group bounds / child cards.
            if graph_edge_hit_at(ev.position, ctx).is_some() {
                return EventResult::Continue;
            }

            let mouse_world = ctx.screen_to_world(ev.position);
            if let Some(node_id) = ctx.hit_node(mouse_world) {
                ctx.start_interaction(NestedNodeDragInteraction::start(
                    node_id,
                    mouse_world,
                    ev.modifiers.shift,
                    self.drag_tick_interval,
                    self.boundary_drag_policy,
                ));
                return EventResult::Stop;
            }
            ctx.clear_selected_node();
        }
        EventResult::Continue
    }

    fn priority(&self) -> i32 {
        121
    }
}
