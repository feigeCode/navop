use gpui::{AnyElement, MouseMoveEvent, MouseUpEvent};

use crate::plugin::{PluginContext, RenderContext};

pub struct InteractionState {
    pub(crate) handler: Option<Box<dyn Interaction>>,
}

impl InteractionState {
    pub(crate) fn new() -> Self {
        Self { handler: None }
    }

    pub fn add(&mut self, handler: impl Interaction + 'static) {
        self.handler = Some(Box::new(handler));
    }

    pub fn clear(&mut self) {
        self.handler = None;
    }

    pub fn is_some(&self) -> bool {
        self.handler.is_some()
    }
}

pub trait Interaction {
    fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        ctx: &mut PluginContext,
    ) -> InteractionResult;

    fn on_mouse_up(&mut self, event: &MouseUpEvent, ctx: &mut PluginContext) -> InteractionResult;

    fn render(&self, _ctx: &mut RenderContext) -> Option<AnyElement> {
        None
    }
}

pub enum InteractionResult {
    Continue,
    End,
    Replace(Box<dyn Interaction>),
}

impl InteractionResult {
    pub fn replace(new_handler: impl Interaction + 'static) -> Self {
        Self::Replace(Box::new(new_handler))
    }
}
