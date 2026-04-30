use futures::channel::mpsc::UnboundedSender;
use gpui::{AnyElement, MouseMoveEvent, Pixels, Point};

use crate::{GraphChange, GraphOp, RenderContext};

pub trait SyncPlugin {
    fn name(&self) -> &'static str;

    fn setup(&mut self, change_sender: UnboundedSender<GraphChange>);

    fn process_intent(&self, op: GraphOp);

    fn undo(&mut self);
    fn redo(&mut self);

    /// `world` is the cursor in flow (graph) space, e.g. `viewport.screen_to_world(event.position)`.
    fn on_mouse_move(&mut self, event: &MouseMoveEvent, world: Point<Pixels>);

    /// Cursor left the canvas; clear shared presence so peers hide your cursor.
    fn on_mouse_leave(&mut self) {}

    fn render(&mut self, _ctx: &mut RenderContext) -> Vec<AnyElement> {
        vec![]
    }
}
