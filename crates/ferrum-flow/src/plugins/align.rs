use gpui::{Pixels, Point, px};

use crate::{
    NodeId,
    plugin::{FlowEvent, Plugin, PluginContext, primary_platform_modifier},
    plugins::node::DragNodesCommand,
};

/// Align selected nodes to their shared bounding box (⌘⇧L/R/T/B/H/V or Ctrl⇧…).
pub struct AlignPlugin;

#[derive(Clone, Copy)]
enum AlignKind {
    Left,
    Right,
    Top,
    Bottom,
    CenterH,
    CenterV,
}

type NodePositions = Vec<(NodeId, Point<Pixels>)>;
type AlignFromTo = (NodePositions, NodePositions);

impl AlignPlugin {
    pub fn new() -> Self {
        Self
    }
}

impl Default for AlignPlugin {
    fn default() -> Self {
        Self::new()
    }
}

fn align_shortcut(ev: &gpui::KeyDownEvent) -> bool {
    primary_platform_modifier(ev) && ev.keystroke.modifiers.shift
}

fn px_to_f32(p: Pixels) -> f32 {
    p.into()
}

fn f32_neq(a: f32, b: f32) -> bool {
    (a - b).abs() > 0.01
}

fn selected_nodes_ordered(ctx: &PluginContext) -> Vec<NodeId> {
    ctx.graph
        .node_order()
        .iter()
        .filter(|id| ctx.graph.selected_node().contains(id))
        .copied()
        .collect()
}

fn build_aligned_positions(ctx: &PluginContext, kind: AlignKind) -> Option<AlignFromTo> {
    let ids = selected_nodes_ordered(ctx);
    if ids.len() < 2 {
        return None;
    }

    let mut min_left = f32::INFINITY;
    let mut max_right = f32::NEG_INFINITY;
    let mut min_top = f32::INFINITY;
    let mut max_bottom = f32::NEG_INFINITY;

    for id in &ids {
        let n = ctx.get_node(id)?;
        let (nx, ny) = n.position();
        let size = *n.size_ref();
        let x = px_to_f32(nx);
        let y = px_to_f32(ny);
        let w = px_to_f32(size.width);
        let h = px_to_f32(size.height);
        min_left = min_left.min(x);
        max_right = max_right.max(x + w);
        min_top = min_top.min(y);
        max_bottom = max_bottom.max(y + h);
    }

    let center_x = (min_left + max_right) / 2.0;
    let center_y = (min_top + max_bottom) / 2.0;

    let mut from = Vec::with_capacity(ids.len());
    let mut to = Vec::with_capacity(ids.len());

    for id in ids {
        let n = ctx.get_node(&id)?;
        let p = n.point();
        from.push((id, p));

        let (nx, ny) = n.position();
        let size = *n.size_ref();
        let x = px_to_f32(nx);
        let y = px_to_f32(ny);
        let w = px_to_f32(size.width);
        let h = px_to_f32(size.height);

        let (nx, ny) = match kind {
            AlignKind::Left => (min_left, y),
            AlignKind::Right => (max_right - w, y),
            AlignKind::Top => (x, min_top),
            AlignKind::Bottom => (x, max_bottom - h),
            AlignKind::CenterH => (center_x - w / 2.0, y),
            AlignKind::CenterV => (x, center_y - h / 2.0),
        };
        to.push((id, Point::new(px(nx), px(ny))));
    }

    let changed = from.iter().zip(to.iter()).any(|((_, pf), (_, pt))| {
        f32_neq(px_to_f32(pf.x), px_to_f32(pt.x)) || f32_neq(px_to_f32(pf.y), px_to_f32(pt.y))
    });
    if !changed {
        return None;
    }

    Some((from, to))
}

fn apply_align(ctx: &mut PluginContext, kind: AlignKind) {
    let Some((from, to)) = build_aligned_positions(ctx, kind) else {
        return;
    };
    ctx.execute_command(DragNodesCommand::from_positions(from, to));
    ctx.cache_all_node_port_offset();
}

impl Plugin for AlignPlugin {
    fn name(&self) -> &'static str {
        "align"
    }

    fn setup(&mut self, _ctx: &mut crate::plugin::InitPluginContext) {}

    fn priority(&self) -> i32 {
        91
    }

    fn on_event(
        &mut self,
        event: &FlowEvent,
        ctx: &mut PluginContext,
    ) -> crate::plugin::EventResult {
        if let FlowEvent::Input(crate::plugin::InputEvent::KeyDown(ev)) = event {
            if !align_shortcut(ev) {
                return crate::plugin::EventResult::Continue;
            }
            let kind = match ev.keystroke.key.as_str() {
                "l" => Some(AlignKind::Left),
                "r" => Some(AlignKind::Right),
                "t" => Some(AlignKind::Top),
                "b" => Some(AlignKind::Bottom),
                "h" => Some(AlignKind::CenterH),
                "v" => Some(AlignKind::CenterV),
                _ => None,
            };
            if let Some(kind) = kind {
                apply_align(ctx, kind);
                return crate::plugin::EventResult::Stop;
            }
        }
        crate::plugin::EventResult::Continue
    }
}
