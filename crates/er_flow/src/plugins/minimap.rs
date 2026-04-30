//! Overview minimap: full-graph bounds in world space, current viewport indicator, click-to-center.

use gpui::{Bounds, Element, MouseButton, PathBuilder, Pixels, Point, Size, canvas, px, rgb};

use crate::{
    Graph, Viewport,
    canvas::{Command, CommandContext},
    plugin::{
        EventResult, FlowEvent, InputEvent, Plugin, PluginContext, RenderContext, RenderLayer,
    },
};

const MAP_W: f32 = 200.0;
const MAP_H: f32 = 140.0;
const OUTER_MARGIN: f32 = 16.0;
const INNER_INSET: f32 = 3.0;
const WORLD_PAD: f32 = 96.0;

/// Last-computed layout for hit-testing (updated each [`MinimapPlugin::render`]).
#[derive(Clone)]
struct MinimapLayout {
    chrome: Bounds<Pixels>,
    inner: Bounds<Pixels>,
    world_x0: f32,
    world_y0: f32,
    world_w: f32,
    world_h: f32,
}

impl MinimapLayout {
    fn contains_chrome(&self, p: Point<Pixels>) -> bool {
        self.chrome.contains(&p)
    }

    /// Maps a screen position inside the chrome to world coordinates (clamped to the mapped extent).
    fn screen_to_world(&self, screen: Point<Pixels>) -> Point<Pixels> {
        let ix: f32 = self.inner.origin.x.into();
        let iy: f32 = self.inner.origin.y.into();
        let iw: f32 = self.inner.size.width.into();
        let ih: f32 = self.inner.size.height.into();
        let sx: f32 = screen.x.into();
        let sy: f32 = screen.y.into();
        let u = ((sx - ix) / iw.max(1.0)).clamp(0.0, 1.0);
        let v = ((sy - iy) / ih.max(1.0)).clamp(0.0, 1.0);
        let wx = self.world_x0 + u * self.world_w;
        let wy = self.world_y0 + v * self.world_h;
        Point::new(px(wx), px(wy))
    }
}

fn graph_world_extent(graph: &Graph, viewport: &Viewport) -> (f32, f32, f32, f32) {
    let nodes: Vec<_> = graph
        .nodes()
        .values()
        .filter(|n| viewport.is_node_visible(n))
        .collect();
    if nodes.is_empty() {
        return (0.0, 0.0, 640.0, 480.0);
    }
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    for n in nodes {
        let x: f32 = n.x.into();
        let y: f32 = n.y.into();
        let w: f32 = n.size.width.into();
        let h: f32 = n.size.height.into();
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x + w);
        max_y = max_y.max(y + h);
    }
    let w = (max_x - min_x + 2.0 * WORLD_PAD).max(120.0);
    let h = (max_y - min_y + 2.0 * WORLD_PAD).max(120.0);
    (min_x - WORLD_PAD, min_y - WORLD_PAD, w, h)
}

fn visible_world_aabb(viewport: &Viewport, win: &Bounds<Pixels>) -> (f32, f32, f32, f32) {
    let w: f32 = win.size.width.into();
    let h: f32 = win.size.height.into();
    let corners = [
        viewport.screen_to_world(Point::new(px(0.0), px(0.0))),
        viewport.screen_to_world(Point::new(px(w), px(0.0))),
        viewport.screen_to_world(Point::new(px(w), px(h))),
        viewport.screen_to_world(Point::new(px(0.0), px(h))),
    ];
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    for c in corners {
        let x: f32 = c.x.into();
        let y: f32 = c.y.into();
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    (
        min_x,
        min_y,
        (max_x - min_x).max(1.0),
        (max_y - min_y).max(1.0),
    )
}

fn build_layout(viewport: &Viewport, graph: &Graph) -> Option<MinimapLayout> {
    let win = viewport.window_bounds?;
    let ww: f32 = win.size.width.into();
    let wh: f32 = win.size.height.into();
    if ww < MAP_W + OUTER_MARGIN || wh < MAP_H + OUTER_MARGIN {
        return None;
    }

    let map_w = px(MAP_W);
    let map_h = px(MAP_H);
    let ox = win.size.width - map_w - px(OUTER_MARGIN);
    let oy = win.size.height - map_h - px(OUTER_MARGIN);
    let chrome = Bounds::new(Point::new(ox, oy), Size::new(map_w, map_h));

    let inset = px(INNER_INSET);
    let inner = Bounds::new(
        chrome.origin + Point::new(inset, inset),
        Size::new(
            chrome.size.width - inset * 2.0,
            chrome.size.height - inset * 2.0,
        ),
    );

    let (wx0, wy0, ww, wh) = graph_world_extent(graph, viewport);

    Some(MinimapLayout {
        chrome,
        inner,
        world_x0: wx0,
        world_y0: wy0,
        world_w: ww.max(1.0),
        world_h: wh.max(1.0),
    })
}

fn world_to_inner_pt(wx: f32, wy: f32, layout: &MinimapLayout) -> Point<Pixels> {
    let u = ((wx - layout.world_x0) / layout.world_w).clamp(0.0, 1.0);
    let v = ((wy - layout.world_y0) / layout.world_h).clamp(0.0, 1.0);
    let ix: f32 = layout.inner.origin.x.into();
    let iy: f32 = layout.inner.origin.y.into();
    let iw: f32 = layout.inner.size.width.into();
    let ih: f32 = layout.inner.size.height.into();
    Point::new(px(ix + u * iw), px(iy + v * ih))
}

fn center_viewport_on_world(ctx: &mut PluginContext, world: Point<Pixels>) {
    let Some(wb) = ctx.viewport.window_bounds else {
        return;
    };
    let cx: f32 = (wb.size.width / 2.0).into();
    let cy: f32 = (wb.size.height / 2.0).into();
    let z = ctx.viewport.zoom;
    let wx: f32 = world.x.into();
    let wy: f32 = world.y.into();
    let from = ctx.viewport.offset;
    ctx.viewport.offset.x = px(cx - wx * z);
    ctx.viewport.offset.y = px(cy - wy * z);
    let to = ctx.viewport.offset;
    ctx.execute_command(MinimapPanCommand { from, to });
}

struct MinimapPanCommand {
    from: Point<Pixels>,
    to: Point<Pixels>,
}

impl Command for MinimapPanCommand {
    fn name(&self) -> &'static str {
        "minimap_pan"
    }

    fn execute(&mut self, ctx: &mut CommandContext) {
        ctx.viewport.offset.x = self.to.x;
        ctx.viewport.offset.y = self.to.y;
    }

    fn undo(&mut self, ctx: &mut CommandContext) {
        ctx.viewport.offset.x = self.from.x;
        ctx.viewport.offset.y = self.from.y;
    }

    fn to_ops(&self, _ctx: &mut crate::CommandContext) -> Vec<crate::GraphOp> {
        vec![]
    }
}

/// Renders a bottom-right overview map and pans the viewport when the user clicks it.
///
/// Uses priority **135** so clicks hit the minimap before [`crate::plugins::SelectionPlugin`] (100)
/// starts a canvas selection.
pub struct MinimapPlugin {
    last_layout: Option<MinimapLayout>,
}

impl MinimapPlugin {
    pub fn new() -> Self {
        Self { last_layout: None }
    }
}

impl Plugin for MinimapPlugin {
    fn name(&self) -> &'static str {
        "minimap"
    }

    fn setup(&mut self, _ctx: &mut crate::plugin::InitPluginContext) {}

    fn on_event(&mut self, event: &FlowEvent, ctx: &mut PluginContext) -> EventResult {
        if let FlowEvent::Input(InputEvent::MouseDown(ev)) = event {
            if let Some(ref layout) = self.last_layout {
                if layout.contains_chrome(ev.position) {
                    if ev.button == MouseButton::Right {
                        return EventResult::Stop;
                    }
                    if ev.button == MouseButton::Left {
                        let world = layout.screen_to_world(ev.position);
                        center_viewport_on_world(ctx, world);
                        ctx.notify();
                        return EventResult::Stop;
                    }
                }
            }
        }
        EventResult::Continue
    }

    fn priority(&self) -> i32 {
        135
    }

    fn render_layer(&self) -> RenderLayer {
        RenderLayer::Overlay
    }

    fn render(&mut self, ctx: &mut RenderContext) -> Option<gpui::AnyElement> {
        let layout = build_layout(ctx.viewport, ctx.graph)?;
        self.last_layout = Some(layout.clone());

        let inner = layout.inner;

        let nodes: Vec<_> = ctx
            .graph
            .nodes()
            .values()
            .filter(|n| ctx.is_node_visible(&n.id))
            .map(|n| {
                let x: f32 = n.x.into();
                let y: f32 = n.y.into();
                let w: f32 = n.size.width.into();
                let h: f32 = n.size.height.into();
                (x, y, w, h)
            })
            .collect();

        let edges: Vec<_> = ctx
            .graph
            .edges
            .values()
            .filter(|e| ctx.is_edge_visible(e))
            .filter_map(|e| {
                let s = ctx.graph.ports.get(&e.source_port)?;
                let t = ctx.graph.ports.get(&e.target_port)?;
                let sn = ctx.graph.nodes.get(&s.node_id)?;
                let tn = ctx.graph.nodes.get(&t.node_id)?;
                let sx: f32 = f32::from(sn.x) + f32::from(sn.size.width) * 0.5;
                let sy: f32 = f32::from(sn.y) + f32::from(sn.size.height) * 0.5;
                let tx: f32 = f32::from(tn.x) + f32::from(tn.size.width) * 0.5;
                let ty: f32 = f32::from(tn.y) + f32::from(tn.size.height) * 0.5;
                Some((sx, sy, tx, ty))
            })
            .collect();

        let win_bounds = ctx.viewport.window_bounds?;
        let (vx0, vy0, vw, vh) = visible_world_aabb(ctx.viewport, &win_bounds);
        let v_tl = world_to_inner_pt(vx0, vy0, &layout);
        let v_br = world_to_inner_pt(vx0 + vw, vy0 + vh, &layout);

        let minimap_background = ctx.theme.minimap_background;
        let minimap_border = ctx.theme.minimap_border;
        let minimap_edge = ctx.theme.minimap_edge;
        let minimap_node_fill = ctx.theme.minimap_node_fill;
        let minimap_node_stroke = ctx.theme.minimap_node_stroke;
        let minimap_viewport_stroke = ctx.theme.minimap_viewport_stroke;

        Some(
            canvas(
                move |_, _, _| (),
                move |_, _, win, _| {
                    // Inner background
                    if let Ok(p) = rect_fill_path(inner) {
                        win.paint_path(p, rgb(minimap_background));
                    }
                    if let Ok(p) = rect_stroke_path(inner, px(1.0)) {
                        win.paint_path(p, rgb(minimap_border));
                    }

                    // Edges (straight segments between node centers)
                    for (sx, sy, tx, ty) in edges {
                        let a = world_to_inner_pt(sx, sy, &layout);
                        let b = world_to_inner_pt(tx, ty, &layout);
                        let mut line = PathBuilder::stroke(px(1.0));
                        line.move_to(a);
                        line.line_to(b);
                        if let Ok(p) = line.build() {
                            win.paint_path(p, rgb(minimap_edge));
                        }
                    }

                    for (x, y, nw, nh) in nodes {
                        let p0 = world_to_inner_pt(x, y, &layout);
                        let p1 = world_to_inner_pt(x + nw, y + nh, &layout);
                        let min_x = f32::min(f32::from(p0.x), f32::from(p1.x));
                        let max_x = f32::max(f32::from(p0.x), f32::from(p1.x));
                        let min_y = f32::min(f32::from(p0.y), f32::from(p1.y));
                        let max_y = f32::max(f32::from(p0.y), f32::from(p1.y));
                        let rw = (max_x - min_x).max(2.0);
                        let rh = (max_y - min_y).max(2.0);
                        let o = Point::new(px(min_x), px(min_y));
                        let s = Size::new(px(rw), px(rh));
                        if let Ok(p) = rect_fill_bounds(o, s) {
                            win.paint_path(p, rgb(minimap_node_fill));
                        }
                        if let Ok(p) = rect_stroke_bounds(o, s, px(1.0)) {
                            win.paint_path(p, rgb(minimap_node_stroke));
                        }
                    }

                    // Viewport frame
                    let min_x = f32::min(f32::from(v_tl.x), f32::from(v_br.x));
                    let max_x = f32::max(f32::from(v_tl.x), f32::from(v_br.x));
                    let min_y = f32::min(f32::from(v_tl.y), f32::from(v_br.y));
                    let max_y = f32::max(f32::from(v_tl.y), f32::from(v_br.y));
                    let vo = Point::new(px(min_x), px(min_y));
                    let vs = Size::new(px((max_x - min_x).max(2.0)), px((max_y - min_y).max(2.0)));
                    if let Ok(p) = rect_stroke_bounds(vo, vs, px(1.5)) {
                        win.paint_path(p, rgb(minimap_viewport_stroke));
                    }
                },
            )
            .into_any(),
        )
    }
}

fn rect_fill_path(b: Bounds<Pixels>) -> Result<gpui::Path<Pixels>, anyhow::Error> {
    rect_fill_bounds(b.origin, b.size)
}

fn rect_fill_bounds(
    o: Point<Pixels>,
    s: Size<Pixels>,
) -> Result<gpui::Path<Pixels>, anyhow::Error> {
    let x0: f32 = o.x.into();
    let y0: f32 = o.y.into();
    let w: f32 = s.width.into();
    let h: f32 = s.height.into();
    let pts = [
        Point::new(px(x0), px(y0)),
        Point::new(px(x0 + w), px(y0)),
        Point::new(px(x0 + w), px(y0 + h)),
        Point::new(px(x0), px(y0 + h)),
    ];
    let mut pb = PathBuilder::fill();
    pb.add_polygon(&pts, true);
    pb.build()
}

fn rect_stroke_path(b: Bounds<Pixels>, width: Pixels) -> Result<gpui::Path<Pixels>, anyhow::Error> {
    rect_stroke_bounds(b.origin, b.size, width)
}

fn rect_stroke_bounds(
    o: Point<Pixels>,
    s: Size<Pixels>,
    width: Pixels,
) -> Result<gpui::Path<Pixels>, anyhow::Error> {
    let x0: f32 = o.x.into();
    let y0: f32 = o.y.into();
    let w: f32 = s.width.into();
    let h: f32 = s.height.into();
    let mut line = PathBuilder::stroke(width);
    line.move_to(Point::new(px(x0), px(y0)));
    line.line_to(Point::new(px(x0 + w), px(y0)));
    line.line_to(Point::new(px(x0 + w), px(y0 + h)));
    line.line_to(Point::new(px(x0), px(y0 + h)));
    line.close();
    line.build()
}
