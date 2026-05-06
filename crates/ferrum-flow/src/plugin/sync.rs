//! Collaboration / replication hooks for the canvas graph.
//!
//! A [`SyncPlugin`] sits **beside** the local [`crate::Graph`]: the canvas still applies
//! [`crate::GraphOp`] through commands and history, while the plugin mirrors those intents into a
//! shared model (CRDT, document store, network sync, etc.) and pushes updates back through
//! [`GraphChange`] so the UI stays consistent with peers and with undo/redo semantics.
//!
//! Implementations typically:
//! - In [`SyncPlugin::setup`], subscribe to the shared model and forward diffs on
//!   [`UnboundedSender<GraphChange>`], setting [`crate::ChangeSource`] (`Local`, `Remote`, `Undo`,
//!   …) so the host can tell operator-driven edits from replay or remote merges.
//! - In [`SyncPlugin::process_intent`], apply each [`GraphOp`] produced locally (after a command
//!   runs or history replays) into that model, using whatever metadata your stack needs to avoid
//!   mis-classifying those writes when your subscription fires again.
//! - In [`SyncPlugin::undo`] / [`SyncPlugin::redo`], advance **your** backend undo manager if the
//!   sync layer owns a stack separate from the canvas history.
//!
//! The concrete backend (Yjs, operational transform, file append, etc.) is up to the plugin; this
//! trait only defines the integration surface with the canvas.

use futures::channel::mpsc::UnboundedSender;
use gpui::{AnyElement, Pixels, Point};

use crate::{FlowEvent, GraphChange, GraphOp, RenderContext, Viewport};

/// Bridges local graph edits to a replicated or external graph model, and streams model changes
/// back into the canvas.
///
/// **Data flow (intended pattern)**  
/// 1. User action → canvas runs a command → [`GraphOp`]s are applied to the local graph.  
/// 2. The host forwards those ops to [`SyncPlugin::process_intent`] so the plugin updates its
///    shared state.  
/// 3. Shared state emits updates (local echo, remote peer, or undo replay) → plugin sends
///    [`GraphChange`] on the channel passed to [`SyncPlugin::setup`].  
/// 4. Canvas applies those changes and refreshes; [`GraphChange::source`] distinguishes how each
///    change should be treated (e.g. skip re-broadcasting remote edits).
///
/// Keep [`process_intent`](SyncPlugin::process_intent) idempotent with respect to your own
/// observers where possible: the same logical op may be reflected back through your subscription;
/// tagging “local intent” vs “remote” vs “undo” origins is the usual way to stay consistent.
pub trait SyncPlugin {
    fn name(&self) -> &'static str;

    /// One-time wiring: subscribe to the shared model, retain subscriptions for the plugin
    /// lifetime, and send [`GraphChange`] values on `change_sender` whenever the model moves.
    ///
    /// The host owns the receiver; do not block the UI thread on long-running I/O—spawn a task or
    /// use non-blocking channels as appropriate.
    fn setup(&mut self, change_sender: UnboundedSender<GraphChange>);

    /// Apply a single local [`GraphOp`] (or a batch already decomposed by the host) into your
    /// backend. This is invoked for operator-driven edits after they hit the local graph, not as
    /// a replacement for the canvas command pipeline.
    fn process_intent(&self, op: GraphOp);

    /// Step the sync-layer undo stack backward, if your backend maintains one in addition to (or
    /// instead of) mirroring canvas history.
    fn undo(&mut self);
    /// Step the sync-layer undo stack forward.
    fn redo(&mut self);

    /// Optional: handle canvas [`FlowEvent`]s for awareness, presence, or other non-[`GraphOp`]
    /// signals. Use [`SyncPluginContext`] for coordinate transforms when needed.
    fn on_event(&mut self, _event: &FlowEvent, _ctx: &mut SyncPluginContext);

    /// Optional overlay (e.g. remote pointers) drawn with normal canvas [`RenderContext`].
    fn render(&mut self, _ctx: &mut RenderContext) -> Vec<AnyElement> {
        vec![]
    }
}

pub struct SyncPluginContext<'a> {
    viewport: &'a Viewport,
}

impl<'a> SyncPluginContext<'a> {
    pub(crate) fn new(viewport: &'a Viewport) -> Self {
        Self { viewport }
    }

    pub fn screen_to_world(&self, screen: Point<Pixels>) -> Point<Pixels> {
        self.viewport.screen_to_world(screen)
    }

    pub fn world_to_screen(&self, world: Point<Pixels>) -> Point<Pixels> {
        self.viewport.world_to_screen(world)
    }
}
