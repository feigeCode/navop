//! Bottom-left zoom controls (left to right: **+ − ↺ ⛶**): zoom in, zoom out, reset scale, fit entire graph.

use gpui::{
    Bounds, InteractiveElement as _, IntoElement as _, MouseButton, ParentElement as _, Pixels,
    Point, Size, Styled as _, div, px, rgb,
};

/// Unicode minus sign (not ASCII hyphen).
const LABEL_ZOOM_OUT: &str = "\u{2212}";
const LABEL_ZOOM_IN: &str = "+";
/// Anticlockwise open circle arrow — common “reset view” symbol.
const LABEL_RESET_ZOOM: &str = "\u{21BA}";
/// Square four corners — “frame / fit content” (same action as [`crate::plugins::FitAllGraphPlugin`]).
const LABEL_FIT_ENTIRE_GRAPH: &str = "\u{26F6}";
const LABEL_SELECT_TOOL: &str = "\u{2196}";
const LABEL_PAN_TOOL: &str = "\u{270B}";

use crate::{
    canvas::{Command, CommandContext},
    plugin::{
        EventResult, FlowEvent, InputEvent, Plugin, PluginContext, RenderContext, RenderLayer,
    },
};

use super::fit_all::fit_entire_graph;
use super::tool_mode::{CanvasToolMode, current_tool_mode, set_tool_mode};
use super::viewport_frame::{ZOOM_MAX, ZOOM_MIN};

const MARGIN: f32 = 16.0;
/// Square control size (width = height).
const BTN: f32 = 36.0;
const GAP: f32 = 6.0;
const GROUP_GAP: f32 = 10.0;
/// Same step as [`crate::plugins::ViewportPlugin`] wheel zoom.
const ZOOM_STEP: f32 = 1.1;

struct ZoomControlsLayout {
    select_tool: Bounds<Pixels>,
    pan_tool: Bounds<Pixels>,
    zoom_in: Bounds<Pixels>,
    zoom_out: Bounds<Pixels>,
    reset: Bounds<Pixels>,
    fit_entire_graph: Bounds<Pixels>,
}

impl ZoomControlsLayout {
    fn hit(&self, p: Point<Pixels>) -> Option<Hit> {
        if self.select_tool.contains(&p) {
            Some(Hit::SelectTool)
        } else if self.pan_tool.contains(&p) {
            Some(Hit::PanTool)
        } else if self.zoom_in.contains(&p) {
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
    SelectTool,
    PanTool,
    ZoomIn,
    ZoomOut,
    ResetZoom,
    FitEntireGraph,
}

#[derive(Clone, Copy)]
struct ControlButtonColors {
    background: u32,
    border: u32,
    text: u32,
    active_background: u32,
    active_text: u32,
}

fn control_button(label: &'static str, active: bool, colors: ControlButtonColors) -> gpui::Div {
    div()
        .w(px(BTN))
        .h(px(BTN))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .bg(rgb(if active {
            colors.active_background
        } else {
            colors.background
        }))
        .border_1()
        .border_color(rgb(if active {
            colors.active_background
        } else {
            colors.border
        }))
        .text_sm()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(rgb(if active {
            colors.active_text
        } else {
            colors.text
        }))
        .child(label)
}

fn bar_outer_size() -> (f32, f32) {
    let w = 6.0 * BTN + 4.0 * GAP + GROUP_GAP;
    (w, BTN)
}

fn build_layout(window_bounds: Bounds<Pixels>) -> ZoomControlsLayout {
    let wh: f32 = window_bounds.size.height.into();
    let (_, bar_h) = bar_outer_size();
    let s = px(BTN);
    let m = px(MARGIN);

    let y0 = px(wh - MARGIN - bar_h);
    let x0 = m;

    let select_tool = Bounds::new(Point::new(x0, y0), Size::new(s, s));
    let pan_tool = Bounds::new(
        Point::new(px(f32::from(x0) + BTN + GAP), y0),
        Size::new(s, s),
    );
    let zoom_start = f32::from(x0) + 2.0 * BTN + GAP + GROUP_GAP;
    let zoom_in = Bounds::new(Point::new(px(zoom_start), y0), Size::new(s, s));
    let zoom_out = Bounds::new(Point::new(px(zoom_start + BTN + GAP), y0), Size::new(s, s));
    let reset = Bounds::new(
        Point::new(px(zoom_start + 2.0 * (BTN + GAP)), y0),
        Size::new(s, s),
    );
    let fit_entire_graph = Bounds::new(
        Point::new(px(zoom_start + 3.0 * (BTN + GAP)), y0),
        Size::new(s, s),
    );

    ZoomControlsLayout {
        select_tool,
        pan_tool,
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
        ctx.viewport.zoom = self.to_zoom;
        ctx.viewport.offset.x = self.to_offset.x;
        ctx.viewport.offset.y = self.to_offset.y;
    }

    fn undo(&mut self, ctx: &mut CommandContext) {
        ctx.viewport.zoom = self.from_zoom;
        ctx.viewport.offset.x = self.from_offset.x;
        ctx.viewport.offset.y = self.from_offset.y;
    }

    fn to_ops(&self, ctx: &mut crate::CommandContext) -> Vec<crate::GraphOp> {
        ctx.viewport.zoom = self.to_zoom;
        ctx.viewport.offset.x = self.to_offset.x;
        ctx.viewport.offset.y = self.to_offset.y;
        vec![]
    }
}

fn apply_zoom(ctx: &mut PluginContext, anchor_screen: Point<Pixels>, to_zoom: f32) {
    let to_zoom = to_zoom.clamp(ZOOM_MIN, ZOOM_MAX);
    let from_zoom = ctx.viewport.zoom;
    let from_offset = ctx.viewport.offset;
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
    let wb = ctx.viewport.window_bounds?;
    let cx: f32 = (wb.size.width / 2.0).into();
    let cy: f32 = (wb.size.height / 2.0).into();
    Some(Point::new(px(cx), px(cy)))
}

fn zoom_by_factor(ctx: &mut PluginContext, factor: f32) {
    let Some(center) = window_center_screen(ctx) else {
        return;
    };
    apply_zoom(ctx, center, ctx.viewport.zoom * factor);
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
        if let FlowEvent::Input(InputEvent::MouseDown(ev)) = event {
            if ev.button == MouseButton::Left {
                if let Some(ref layout) = self.last_layout {
                    if let Some(hit) = layout.hit(ev.position) {
                        match hit {
                            Hit::SelectTool => {
                                set_tool_mode(ctx.shared_state, CanvasToolMode::Select)
                            }
                            Hit::PanTool => set_tool_mode(ctx.shared_state, CanvasToolMode::Pan),
                            Hit::ZoomIn => zoom_by_factor(ctx, ZOOM_STEP),
                            Hit::ZoomOut => zoom_by_factor(ctx, 1.0 / ZOOM_STEP),
                            Hit::ResetZoom => reset_zoom(ctx),
                            Hit::FitEntireGraph => fit_entire_graph(ctx),
                        }
                        ctx.notify();
                        return EventResult::Stop;
                    }
                }
            }
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
        let win = ctx.viewport.window_bounds.unwrap_or_else(|| {
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
        let active_bg = ctx.theme.edge_stroke_selected;
        let active_text = 0x00ffffff;
        let active_mode = current_tool_mode(ctx.shared_state);
        let canvas_entity = ctx.canvas_entity.clone();
        let colors = ControlButtonColors {
            background: btn_bg,
            border: btn_border,
            text: btn_text,
            active_background: active_bg,
            active_text,
        };

        let mk_tool_btn = {
            let canvas_entity = canvas_entity.clone();
            move |label: &'static str, active: bool, mode: CanvasToolMode| {
                let canvas_entity = canvas_entity.clone();
                control_button(label, active, colors).on_mouse_down(
                    MouseButton::Left,
                    move |_, _, cx| {
                        cx.stop_propagation();
                        canvas_entity.update(cx, |canvas, cx| {
                            set_tool_mode(&mut canvas.shared_state, mode);
                            cx.notify();
                        });
                    },
                )
            }
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
                            mk_tool_btn(
                                LABEL_SELECT_TOOL,
                                active_mode == CanvasToolMode::Select,
                                CanvasToolMode::Select,
                            )
                            .into_any_element(),
                            mk_tool_btn(
                                LABEL_PAN_TOOL,
                                active_mode == CanvasToolMode::Pan,
                                CanvasToolMode::Pan,
                            )
                            .into_any_element(),
                            div().w(px(GROUP_GAP - GAP)).h(px(BTN)).into_any_element(),
                            control_button(LABEL_ZOOM_IN, false, colors).into_any_element(),
                            control_button(LABEL_ZOOM_OUT, false, colors).into_any_element(),
                            control_button(LABEL_RESET_ZOOM, false, colors).into_any_element(),
                            control_button(LABEL_FIT_ENTIRE_GRAPH, false, colors)
                                .into_any_element(),
                        ]),
                )
                .into_any_element(),
        )
    }
}
