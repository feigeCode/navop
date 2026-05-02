use futures::channel::mpsc::UnboundedSender;
use gpui::{AnyElement, Pixels, Point};

use crate::{FlowEvent, GraphChange, GraphOp, RenderContext, Viewport};

pub trait SyncPlugin {
    fn name(&self) -> &'static str;

    fn setup(&mut self, change_sender: UnboundedSender<GraphChange>);

    fn process_intent(&self, op: GraphOp);

    fn undo(&mut self);
    fn redo(&mut self);

    fn on_event(&mut self, _event: &FlowEvent, _ctx: &mut SyncPluginContext);

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
