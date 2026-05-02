//! Bottom-left zoom controls (left to right: **+ − ↺ ⛶**): zoom in, zoom out, reset scale, fit entire graph.

use gpui::{
    Bounds, IntoElement as _, MouseButton, ParentElement as _, Pixels, Point, Size, Styled as _,
    div, px, rgb,
};

/// Unicode minus sign (not ASCII hyphen).
const LABEL_ZOOM_OUT: &str = "\u{2212}";
const LABEL_ZOOM_IN: &str = "+";
/// Anticlockwise open circle arrow — common “reset view” symbol.
const LABEL_RESET_ZOOM: &str = "\u{21BA}";
/// Square four corners — “frame / fit content” (same action as [`crate::plugins::FitAllGraphPlugin`]).
const LABEL_FIT_ENTIRE_GRAPH: &str = "\u{26F6}";

use crate::{
    canvas::{Command, CommandContext},
    plugin::{
        EventResult, FlowEvent, InputEvent, Plugin, PluginContext, RenderContext, RenderLayer,
    },
};

use super::fit_all::fit_entire_graph;
use super::viewport_frame::{ZOOM_MAX, ZOOM_MIN};

const MARGIN: f32 = 16.0;
/// Square control size (width = height).
const BTN: f32 = 36.0;
const GAP: f32 = 6.0;
/// Same step as [`crate::plugins::ViewportPlugin`] wheel zoom.
const ZOOM_STEP: f32 = 1.1;

struct ZoomControlsLayout {
    zoom_in: Bounds<Pixels>,
    zoom_out: Bounds<Pixels>,
    reset: Bounds<Pixels>,
    fit_entire_graph: Bounds<Pixels>,
}

impl ZoomControlsLayout {
    fn hit(&self, p: Point<Pixels>) -> Option<Hit> {
        if self.zoom_in.contains(&p) {
            Some(Hit::ZoomIn)
        } else if self.zoom_out.contains(&p) {
            Some(Hit::ZoomOut)
        } else if self.reset.contains(&p) {
            Some(Hit::ResetZoom)
        } else if self.fit_entire_graph.contains(&p) {
            Some(Hit::FitEntireGraph)
        } else {
            None
        }
    }
}

#[derive(Copy, Clone)]
enum Hit {
    ZoomIn,
    ZoomOut,
    ResetZoom,
    FitEntireGraph,
}

fn bar_outer_size() -> (f32, f32) {
    let w = 4.0 * BTN + 3.0 * GAP;
    (w, BTN)
}

fn build_layout(window_bounds: Bounds<Pixels>) -> ZoomControlsLayout {
    let wh: f32 = window_bounds.size.height.into();
    let (_, bar_h) = bar_outer_size();
    let s = px(BTN);
    let m = px(MARGIN);

    let y0 = px(wh - MARGIN - bar_h);
    let x0 = m;

    let zoom_in = Bounds::new(Point::new(x0, y0), Size::new(s, s));
    let zoom_out = Bounds::new(
        Point::new(px(f32::from(x0) + BTN + GAP), y0),
        Size::new(s, s),
    );
    let reset = Bounds::new(
        Point::new(px(f32::from(x0) + 2.0 * (BTN + GAP)), y0),
        Size::new(s, s),
    );
    let fit_entire_graph = Bounds::new(
        Point::new(px(f32::from(x0) + 3.0 * (BTN + GAP)), y0),
        Size::new(s, s),
    );

    ZoomControlsLayout {
        zoom_in,
        zoom_out,
        reset,
        fit_entire_graph,
    }
}

struct ViewportZoomCommand {
    from_zoom: f32,
    from_offset: Point<Pixels>,
    to_zoom: f32,
    to_offset: Point<Pixels>,
}

impl Command for ViewportZoomCommand {
    fn name(&self) -> &'static str {
        "viewport_zoom"
    }

    fn execute(&mut self, ctx: &mut CommandContext) {
        ctx.set_zoom(self.to_zoom);
        ctx.set_offset(self.to_offset);
    }

    fn undo(&mut self, ctx: &mut CommandContext) {
        ctx.set_zoom(self.from_zoom);
        ctx.set_offset(self.from_offset);
    }

    fn to_ops(&self, ctx: &mut crate::CommandContext) -> Vec<crate::GraphOp> {
        ctx.set_zoom(self.to_zoom);
        ctx.set_offset(self.to_offset);
        vec![]
    }
}

fn apply_zoom(ctx: &mut PluginContext, anchor_screen: Point<Pixels>, to_zoom: f32) {
    let to_zoom = to_zoom.clamp(ZOOM_MIN, ZOOM_MAX);
    let from_zoom = ctx.zoom();
    let from_offset = ctx.offset();
    if (from_zoom - to_zoom).abs() < 1e-5 {
        return;
    }
    let anchor_world = ctx.screen_to_world(anchor_screen);
    let wx: f32 = anchor_world.x.into();
    let wy: f32 = anchor_world.y.into();
    let ax: f32 = anchor_screen.x.into();
    let ay: f32 = anchor_screen.y.into();
    let to_offset = Point::new(px(ax - wx * to_zoom), px(ay - wy * to_zoom));
    ctx.execute_command(ViewportZoomCommand {
        from_zoom,
        from_offset,
        to_zoom,
        to_offset,
    });
}

fn window_center_screen(ctx: &PluginContext) -> Option<Point<Pixels>> {
    let wb = ctx.window_bounds()?;
    let cx: f32 = (wb.size.width / 2.0).into();
    let cy: f32 = (wb.size.height / 2.0).into();
    Some(Point::new(px(cx), px(cy)))
}

fn zoom_by_factor(ctx: &mut PluginContext, factor: f32) {
    let Some(center) = window_center_screen(ctx) else {
        return;
    };
    apply_zoom(ctx, center, ctx.zoom_scaled_by(factor));
}

fn reset_zoom(ctx: &mut PluginContext) {
    let Some(center) = window_center_screen(ctx) else {
        return;
    };
    apply_zoom(ctx, center, 1.0);
}

/// Bottom-left **+** / **−** / **↺** / **⛶** (fit all); priority **128** so clicks beat canvas selection.
pub struct ZoomControlsPlugin {
    last_layout: Option<ZoomControlsLayout>,
}

impl Default for ZoomControlsPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl ZoomControlsPlugin {
    pub fn new() -> Self {
        Self { last_layout: None }
    }
}

impl Plugin for ZoomControlsPlugin {
    fn name(&self) -> &'static str {
        "zoom_controls"
    }

    fn setup(&mut self, _ctx: &mut crate::plugin::InitPluginContext) {}

    fn on_event(&mut self, event: &FlowEvent, ctx: &mut PluginContext) -> EventResult {
        if let FlowEvent::Input(InputEvent::MouseDown(ev)) = event
            && ev.button == MouseButton::Left
            && let Some(ref layout) = self.last_layout
            && let Some(hit) = layout.hit(ev.position)
        {
            match hit {
                Hit::ZoomIn => zoom_by_factor(ctx, ZOOM_STEP),
                Hit::ZoomOut => zoom_by_factor(ctx, 1.0 / ZOOM_STEP),
                Hit::ResetZoom => reset_zoom(ctx),
                Hit::FitEntireGraph => fit_entire_graph(ctx),
            }
            ctx.notify();
            return EventResult::Stop;
        }
        EventResult::Continue
    }

    fn priority(&self) -> i32 {
        128
    }

    fn render_layer(&self) -> RenderLayer {
        RenderLayer::Overlay
    }

    fn render(&mut self, ctx: &mut RenderContext) -> Option<gpui::AnyElement> {
        let win = ctx.window_bounds().unwrap_or_else(|| {
            let vs = ctx.window.viewport_size();
            Bounds::new(Point::new(px(0.0), px(0.0)), Size::new(vs.width, vs.height))
        });
        let wh: f32 = win.size.height.into();
        let (bar_w, bar_h) = bar_outer_size();
        if wh < MARGIN + bar_h + 1.0 {
            self.last_layout = None;
            return None;
        }

        let layout = build_layout(win);
        self.last_layout = Some(layout);

        let bar_w_px = px(bar_w);

        let btn_bg = ctx.theme.zoom_controls_background;
        let btn_border = ctx.theme.zoom_controls_border;
        let btn_text = ctx.theme.zoom_controls_text;

        let mk_btn = move |label: &'static str| {
            div()
                .w(px(BTN))
                .h(px(BTN))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .bg(rgb(btn_bg))
                .border_1()
                .border_color(rgb(btn_border))
                .text_sm()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(rgb(btn_text))
                .child(label)
        };

        Some(
            div()
                .absolute()
                .size_full()
                .child(
                    div()
                        .absolute()
                        .bottom(px(MARGIN))
                        .left(px(MARGIN))
                        .w(bar_w_px)
                        .h(px(bar_h))
                        .flex()
                        .flex_row()
                        .gap(px(GAP))
                        .items_center()
                        .children(vec![
                            mk_btn(LABEL_ZOOM_IN),
                            mk_btn(LABEL_ZOOM_OUT),
                            mk_btn(LABEL_RESET_ZOOM),
                            mk_btn(LABEL_FIT_ENTIRE_GRAPH),
                        ]),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod command_interop_tests {
    use gpui::{Point, px};

    use crate::{Graph, command_interop::assert_command_interop};

    use super::ViewportZoomCommand;

    #[test]
    fn viewport_zoom_command_interop() {
        let base = Graph::new();
        let cmd = ViewportZoomCommand {
            from_zoom: 1.0,
            from_offset: Point::new(px(0.0), px(0.0)),
            to_zoom: 1.25,
            to_offset: Point::new(px(5.0), px(6.0)),
        };
        assert_command_interop(
            &base,
            || {
                Box::new(ViewportZoomCommand {
                    from_zoom: cmd.from_zoom,
                    from_offset: cmd.from_offset,
                    to_zoom: cmd.to_zoom,
                    to_offset: cmd.to_offset,
                })
            },
            "ViewportZoomCommand",
        );
    }
}
