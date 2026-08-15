//! Custom [`FlowEvent`](crate::plugin::FlowEvent) payloads for primary (left-button) node dragging.
//! Emitted by flat ([`super::interaction::NodeDragInteraction`]) and nested
//! ([`crate::plugins::graph::drag::NestedNodeDragInteraction`]) drag interactions. Other plugins (e.g. snap guides) may
//! subscribe via [`FlowEvent::as_custom`](crate::plugin::FlowEvent::as_custom).
//!
//! [`NodeDragEvent::Tick`] carries [`std::sync::Arc`] so the emitter can share the same id list across
//! ticks without reallocating (custom events cannot borrow interaction state).

use std::sync::Arc;
use std::time::Duration;

use crate::NodeId;

/// Stored in [`crate::SharedState`] while a node drag interaction is active in
/// the dragging phase. [`super::NodePlugin`] and the interaction overlay use
/// [`super::node_ids_for_drag_overlay`] so dragged roots **and their descendants** render on the
/// interaction layer only (avoids the parent card covering children left on the static layer).
#[derive(Clone, Debug)]
pub struct ActiveNodeDrag(pub Arc<[NodeId]>);

/// Default throttle for [`NodeDragEvent::Tick`] ([`crate::plugins::NodeInteractionPlugin::new`]).
/// Use [`crate::plugins::NodeInteractionPlugin::with_drag_tick_interval`] to change it.
pub const NODE_DRAG_TICK_INTERVAL: Duration = Duration::from_millis(50);

/// Primary node drag lifecycle on the canvas (left-button drag from node drag plugins).
#[derive(Debug, Clone)]
pub enum NodeDragEvent {
    /// Throttled while dragging; [`crate::Graph`] already holds updated positions for these nodes.
    /// Same slice is reused for the whole drag (cheap [`Arc::clone`] per tick).
    Tick(Arc<[NodeId]>),
    /// Drag finished: click without move, or pointer released after a drag.
    End,
}
