//! Alignment guides while dragging nodes.
//!
//! Subscribes to [`NodeDragEvent`](crate::plugins::node::NodeDragEvent) from
//! [`crate::plugins::NodeInteractionPlugin`] and runs
//! [`compute_alignment_guides`] only here. [`SetAlignmentGuides`] remains for manual overrides.
//! This keeps [`crate::canvas::InteractionState`] free of overlay-specific fields.

use std::collections::HashSet;

use gpui::{AnyElement, Element, ParentElement, Pixels, Point, Styled, div, px, rgb};

use crate::{
    Graph, NodeId, Viewport,
    plugin::{EventResult, FlowEvent, Plugin, PluginContext, RenderContext, RenderLayer},
    plugins::node::NodeDragEvent,
    theme::FlowTheme,
};

/// Screen-space snap distance, converted to world units via `threshold / zoom`.
const SNAP_SCREEN_PX: f32 = 4.0;

/// World-space lines to draw as alignment guides (full width / height of the canvas view).
#[derive(Debug, Clone, Default)]
pub struct AlignmentGuides {
    pub vertical_x: Vec<Pixels>,
    pub horizontal_y: Vec<Pixels>,
}

/// Payload for [`FlowEvent::custom`]. Any code may emit `None` to clear guides.
#[derive(Debug, Clone)]
pub struct SetAlignmentGuides(pub Option<AlignmentGuides>);

fn union_drag_bounds(graph: &Graph, dragged_ids: &[NodeId]) -> Option<gpui::Bounds<Pixels>> {
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    let mut any = false;

    for id in dragged_ids.iter().copied() {
        let Some(n) = graph.get_node(&id) else {
            continue;
        };
        any = true;
        let b = n.bounds();
        let l: f32 = b.origin.x.into();
        let t: f32 = b.origin.y.into();
        let r: f32 = (b.origin.x + b.size.width).into();
        let bot: f32 = (b.origin.y + b.size.height).into();
        min_x = min_x.min(l);
        min_y = min_y.min(t);
        max_x = max_x.max(r);
        max_y = max_y.max(bot);
    }

    if !any {
        return None;
    }

    Some(gpui::Bounds::new(
        Point::new(min_x.into(), min_y.into()),
        gpui::Size::new((max_x - min_x).into(), (max_y - min_y).into()),
    ))
}

fn dedup_sorted_coords(mut v: Vec<Pixels>) -> Vec<Pixels> {
    v.sort_by(|a, b| f32::total_cmp(&(*a).into(), &(*b).into()));
    v.dedup_by(|a, b| {
        let af: f32 = (*a).into();
        let bf: f32 = (*b).into();
        af == bf
    });
    v
}

/// Computes alignment guides for the current drag. Skips nodes that are not on-screen and uses an
/// AABB broadphase so distant nodes are not considered.
pub(crate) fn compute_alignment_guides(
    graph: &Graph,
    viewport: &Viewport,
    dragged_ids: &[NodeId],
) -> Option<AlignmentGuides> {
    let dragged_set: HashSet<NodeId> = dragged_ids.iter().copied().collect();
    let thr = px(SNAP_SCREEN_PX) / viewport.zoom;
    let union = union_drag_bounds(graph, dragged_ids)?;
    let dl = union.origin.x;
    let dr = union.origin.x + union.size.width;
    let dcx = (dl + dr) * 0.5;
    let dt = union.origin.y;
    let db = union.origin.y + union.size.height;
    let dcy = (dt + db) * 0.5;

    let drag_x_lo = dl - thr;
    let drag_x_hi = dr + thr;
    let drag_y_lo = dt - thr;
    let drag_y_hi = db + thr;

    let drag_xs = [dl, dcx, dr];
    let drag_ys = [dt, dcy, db];

    let mut ref_x: Vec<Pixels> = Vec::new();
    let mut ref_y: Vec<Pixels> = Vec::new();

    for (id, node) in graph.nodes() {
        if dragged_set.contains(id) {
            continue;
        }
        if !viewport.is_node_visible(node) {
            continue;
        }

        let b = node.bounds();
        let rl = b.origin.x;
        let rr = rl + b.size.width;
        let rcx = (rl + rr) * 0.5;
        let rt = b.origin.y;
        let rb = rt + b.size.height;
        let rcy = (rt + rb) * 0.5;

        let can_vertical = !(rr < drag_x_lo || rl > drag_x_hi);
        let can_horizontal = !(rb < drag_y_lo || rt > drag_y_hi);
        if !can_vertical && !can_horizontal {
            continue;
        }

        if can_vertical {
            ref_x.extend([rl, rcx, rr]);
        }
        if can_horizontal {
            ref_y.extend([rt, rcy, rb]);
        }
    }

    let mut vertical_x = Vec::new();
    for rx in ref_x {
        if drag_xs.iter().any(|dx| (*dx - rx).abs() <= thr) {
            vertical_x.push(rx);
        }
    }

    let mut horizontal_y = Vec::new();
    for ry in ref_y {
        if drag_ys.iter().any(|dy| (*dy - ry).abs() <= thr) {
            horizontal_y.push(ry);
        }
    }

    vertical_x = dedup_sorted_coords(vertical_x);
    horizontal_y = dedup_sorted_coords(horizontal_y);

    if vertical_x.is_empty() && horizontal_y.is_empty() {
        return None;
    }

    Some(AlignmentGuides {
        vertical_x,
        horizontal_y,
    })
}

pub struct SnapGuidesPlugin {
    guides: Option<AlignmentGuides>,
}

impl SnapGuidesPlugin {
    pub fn new() -> Self {
        Self { guides: None }
    }
}

impl Plugin for SnapGuidesPlugin {
    fn name(&self) -> &'static str {
        "snap_guides"
    }

    fn on_event(&mut self, event: &FlowEvent, ctx: &mut PluginContext) -> EventResult {
        if let Some(sg) = event.as_custom::<SetAlignmentGuides>() {
            self.guides = sg.0.clone();
        }
        if let Some(evt) = event.as_custom::<NodeDragEvent>() {
            match evt {
                NodeDragEvent::Tick(ids) => {
                    self.guides = compute_alignment_guides(ctx.graph, ctx.viewport, ids.as_ref());
                    ctx.notify();
                }
                NodeDragEvent::End => {
                    self.guides = None;
                    ctx.notify();
                }
            }
        }
        EventResult::Continue
    }

    fn render(&mut self, ctx: &mut RenderContext) -> Option<AnyElement> {
        let guides = self.guides.as_ref()?;
        let wb = ctx.viewport.window_bounds?;
        let w = wb.size.width;
        let h = wb.size.height;
        let theme = ctx.theme;

        let vx = guides.vertical_x.iter().map(|wx| vline(*wx, h, ctx, theme));
        let hy = guides
            .horizontal_y
            .iter()
            .map(|wy| hline(*wy, w, ctx, theme));

        Some(
            div()
                .absolute()
                .size_full()
                .children(vx.chain(hy))
                .into_any(),
        )
    }

    fn priority(&self) -> i32 {
        118
    }

    fn render_layer(&self) -> RenderLayer {
        RenderLayer::Interaction
    }
}

fn vline(wx: Pixels, win_h: Pixels, ctx: &RenderContext<'_>, theme: &FlowTheme) -> AnyElement {
    let sx = ctx.viewport.world_to_screen(Point::new(wx, px(0.0))).x;
    div()
        .absolute()
        .left(sx)
        .top(px(0.0))
        .w(px(1.0))
        .h(win_h)
        .bg(rgb(theme.selection_rect_border))
        .into_any()
}

fn hline(wy: Pixels, win_w: Pixels, ctx: &RenderContext<'_>, theme: &FlowTheme) -> AnyElement {
    let sy = ctx.viewport.world_to_screen(Point::new(px(0.0), wy)).y;
    div()
        .absolute()
        .left(px(0.0))
        .top(sy)
        .w(win_w)
        .h(px(1.0))
        .bg(rgb(theme.selection_rect_border))
        .into_any()
}
