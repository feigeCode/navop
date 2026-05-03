use ferrum_flow::{Command, CommandContext, EventResult, FlowEvent, GraphOp, InputEvent, Interaction, InteractionResult, Plugin, PluginContext, RenderContext, RenderLayer};
use gpui::{div, px, rgb, Bounds, IntoElement, MouseButton, ParentElement, Pixels, Point, Size, Styled};
use crate::er_diagram::{ER_DIAGRAM_PAN_LABEL, ER_DIAGRAM_TOOL_GAP, ER_DIAGRAM_TOOL_MARGIN, ER_DIAGRAM_TOOL_SIZE};

pub(crate) struct ErDiagramPanModePlugin {
    last_bounds: Option<Bounds<Pixels>>,
}

impl ErDiagramPanModePlugin {
    pub fn new() -> Self {
        Self { last_bounds: None }
    }
}

struct ErDiagramPanning {
    start_mouse: Point<Pixels>,
    start_offset: Point<Pixels>,
}

impl Interaction for ErDiagramPanning {
    fn on_mouse_move(
        &mut self,
        ev: &gpui::MouseMoveEvent,
        ctx: &mut PluginContext,
    ) -> InteractionResult {
        let dx = ev.position.x - self.start_mouse.x;
        let dy = ev.position.y - self.start_mouse.y;
        ctx.set_offset(Point::new(
            self.start_offset.x + dx,
            self.start_offset.y + dy,
        ));
        ctx.notify();
        InteractionResult::Continue
    }

    fn on_mouse_up(
        &mut self,
        _event: &gpui::MouseUpEvent,
        ctx: &mut PluginContext,
    ) -> InteractionResult {
        ctx.execute_command(ErDiagramPanningCommand {
            from: self.start_offset,
            to: ctx.offset(),
        });
        ctx.cancel_interaction();
        InteractionResult::End
    }
}

struct ErDiagramPanningCommand {
    from: Point<Pixels>,
    to: Point<Pixels>,
}

impl Command for ErDiagramPanningCommand {
    fn name(&self) -> &'static str {
        "er_diagram_panning"
    }

    fn execute(&mut self, ctx: &mut CommandContext) {
        ctx.set_offset(self.to);
    }

    fn undo(&mut self, ctx: &mut CommandContext) {
        ctx.set_offset(self.from);
    }

    fn to_ops(&self, _ctx: &mut CommandContext) -> Vec<GraphOp> {
        vec![]
    }
}

impl Plugin for ErDiagramPanModePlugin {
    fn name(&self) -> &'static str {
        "er_diagram_pan_mode"
    }

    fn setup(&mut self, ctx: &mut ferrum_flow::InitPluginContext) {
        ctx.shared_state.insert(ErDiagramPanState::default());
    }

    fn on_event(&mut self, event: &FlowEvent, ctx: &mut PluginContext) -> EventResult {
        if let FlowEvent::Input(InputEvent::MouseDown(ev)) = event
            && ev.button == MouseButton::Left
        {
            if self
                .last_bounds
                .is_some_and(|bounds| bounds.contains(&ev.position))
            {
                let active = ctx
                    .shared_state
                    .get_mut::<ErDiagramPanState>()
                    .map(|state| {
                        state.active = !state.active;
                        state.active
                    })
                    .unwrap_or(false);
                if active {
                    ctx.cancel_interaction();
                }
                ctx.notify();
                return EventResult::Stop;
            }

            if ctx
                .shared_state
                .get::<ErDiagramPanState>()
                .is_some_and(|state| state.active)
            {
                ctx.start_interaction(ErDiagramPanning {
                    start_mouse: ev.position,
                    start_offset: ctx.offset(),
                });
                return EventResult::Stop;
            }
        }
        EventResult::Continue
    }

    fn render(&mut self, ctx: &mut RenderContext) -> Option<gpui::AnyElement> {
        let window_bounds = ctx.window_bounds().unwrap_or_else(|| {
            let viewport_size = ctx.window.viewport_size();
            Bounds::new(
                Point::new(px(0.0), px(0.0)),
                Size::new(viewport_size.width, viewport_size.height),
            )
        });
        let active = ctx
            .get_shared_state::<ErDiagramPanState>()
            .is_some_and(|state| state.active);
        let button_bounds = er_diagram_pan_button_bounds(window_bounds);
        self.last_bounds = Some(button_bounds);
        let theme = ctx.theme;
        let background = if active {
            theme.zoom_controls_text
        } else {
            theme.zoom_controls_background
        };
        let text = if active {
            theme.zoom_controls_background
        } else {
            theme.zoom_controls_text
        };

        Some(
            div()
                .absolute()
                .size_full()
                .child(
                    div()
                        .absolute()
                        .bottom(px(ER_DIAGRAM_TOOL_MARGIN))
                        .left(px(ER_DIAGRAM_TOOL_MARGIN
                            + 4.0 * ER_DIAGRAM_TOOL_SIZE
                            + 4.0 * ER_DIAGRAM_TOOL_GAP))
                        .w(px(ER_DIAGRAM_TOOL_SIZE))
                        .h(px(ER_DIAGRAM_TOOL_SIZE))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.0))
                        .bg(rgb(background))
                        .border_1()
                        .border_color(rgb(theme.zoom_controls_border))
                        .text_sm()
                        .text_color(rgb(text))
                        .child(ER_DIAGRAM_PAN_LABEL),
                )
                .into_any_element(),
        )
    }

    fn priority(&self) -> i32 {
        127
    }

    fn render_layer(&self) -> RenderLayer {
        RenderLayer::Overlay
    }
}

fn er_diagram_pan_button_bounds(window_bounds: Bounds<Pixels>) -> Bounds<Pixels> {
    let height: f32 = window_bounds.size.height.into();
    let x = ER_DIAGRAM_TOOL_MARGIN + 4.0 * ER_DIAGRAM_TOOL_SIZE + 4.0 * ER_DIAGRAM_TOOL_GAP;
    let y = height - ER_DIAGRAM_TOOL_MARGIN - ER_DIAGRAM_TOOL_SIZE;
    Bounds::new(
        Point::new(px(x), px(y)),
        Size::new(px(ER_DIAGRAM_TOOL_SIZE), px(ER_DIAGRAM_TOOL_SIZE)),
    )
}

#[derive(Default)]
pub struct ErDiagramPanState {
    active: bool,
}
