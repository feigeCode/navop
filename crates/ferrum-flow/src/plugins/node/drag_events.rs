//! Custom [`FlowEvent`](crate::plugin::FlowEvent) payloads for primary (left-button) node dragging.
//! Emitted by [`super::interaction::NodeDragInteraction`]. Other plugins (e.g. snap guides) may
//! subscribe via [`FlowEvent::as_custom`](crate::plugin::FlowEvent::as_custom).
//!
//! [`NodeDragEvent::Tick`] carries [`std::sync::Arc`] so the emitter can share the same id list across
//! ticks without reallocating (custom events cannot borrow interaction state).

use std::sync::Arc;
use std::time::Duration;

use crate::NodeId;

/// Stored in [`crate::SharedState`] while [`super::interaction::NodeDragInteraction`] is active in
/// the dragging phase: these node ids are rendered on the interaction layer only; [`super::NodePlugin`]
/// skips them in the static nodes layer to cut work per frame.
#[derive(Clone, Debug)]
pub struct ActiveNodeDrag(pub Arc<[NodeId]>);

/// Default throttle for [`NodeDragEvent::Tick`] ([`crate::plugins::NodeInteractionPlugin::new`]).
/// Use [`crate::plugins::NodeInteractionPlugin::with_drag_tick_interval`] to change it.
pub const NODE_DRAG_TICK_INTERVAL: Duration = Duration::from_millis(50);

/// Primary node drag lifecycle on the canvas (left-button drag from [`super::NodeInteractionPlugin`]).
#[derive(Debug, Clone)]
pub enum NodeDragEvent {
    /// Throttled while dragging; [`crate::Graph`] already holds updated positions for these nodes.
    /// Same slice is reused for the whole drag (cheap [`Arc::clone`] per tick).
    Tick(Arc<[NodeId]>),
    /// Drag finished: click without move, or pointer released after a drag.
    End,
}
