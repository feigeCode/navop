use gpui::{Element, ParentElement, Styled, div, px, rgb};

use crate::plugin::Plugin;

pub struct BackgroundPlugin;

impl BackgroundPlugin {
    pub fn new() -> Self {
        Self {}
    }
}

impl Plugin for BackgroundPlugin {
    fn name(&self) -> &'static str {
        "background"
    }
    fn setup(&mut self, _ctx: &mut crate::plugin::InitPluginContext) {}
    fn on_event(
        &mut self,
        _event: &crate::plugin::FlowEvent,
        _context: &mut crate::plugin::PluginContext,
    ) -> crate::plugin::EventResult {
        crate::plugin::EventResult::Continue
    }
    fn priority(&self) -> i32 {
        0
    }
    fn render_layer(&self) -> crate::plugin::RenderLayer {
        crate::plugin::RenderLayer::Background
    }
    fn render(&mut self, ctx: &mut crate::plugin::RenderContext) -> Option<gpui::AnyElement> {
        let base_grid = 40.0;
        let zoom = ctx.viewport.zoom;

        let grid = base_grid * zoom;

        let offset = ctx.viewport.offset;

        let start_x = f32::from(offset.x) % grid;
        let start_y = f32::from(offset.y) % grid;

        let mut dots = Vec::new();

        let bounds = ctx.window.bounds();
        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);

        let mut x = start_x;

        while x < width {
            let mut y = start_y;

            while y < height {
                dots.push(
                    div()
                        .absolute()
                        .left(px(x))
                        .top(px(y))
                        .w(px(2.0))
                        .h(px(2.0))
                        .rounded_full()
                        .bg(rgb(ctx.theme.background_grid_dot)),
                );

                y += grid;
            }

            x += grid;
        }
        Some(
            div()
                .absolute()
                .w(px(width))
                .h(px(height))
                .bg(gpui::rgb(ctx.theme.background))
                .children(dots)
                .into_any(),
        )
    }
}
