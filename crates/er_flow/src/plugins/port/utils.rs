use gpui::{Bounds, Path, PathBuilder, Pixels, Point, Size, px};

use crate::{PortId, PortPosition, RenderContext, Viewport, plugins::edge::get_control_point};

#[deprecated(note = "use `ctx.port_screen_center_by_port_id(port_id)`")]
#[allow(dead_code)] // kept for re-export; callers should migrate to `RenderContext` methods
pub fn port_screen_position(port_id: PortId, ctx: &RenderContext) -> Option<Point<Pixels>> {
    ctx.port_screen_center_by_port_id(port_id)
}

pub fn port_screen_bounds(
    port_id: PortId,
    ctx: &crate::plugin::PluginContext,
) -> Option<Bounds<Pixels>> {
    let port = &ctx.graph.ports.get(&port_id)?;
    let node = &ctx.nodes().get(&port.node_id)?;

    let node_pos = node.point();

    let offset = ctx.port_offset_cached(&port.node_id, &port_id)?;

    Some(Bounds::new(
        node_pos + offset - Point::new(port.size.width / 2.0, port.size.height / 2.0),
        port.size,
    ))
}

pub fn port_screen_big_bounds(
    port_id: PortId,
    ctx: &crate::plugin::PluginContext,
) -> Option<Bounds<Pixels>> {
    let mut bounds = port_screen_bounds(port_id, ctx)?;

    let offset_width = px(15.0) - bounds.size.width / 2.0;
    let offset_height = px(15.0) - bounds.size.height / 2.0;

    bounds.origin -= Point::new(offset_width, offset_height);

    bounds.size = Size {
        width: px(30.0),
        height: px(30.0),
    };

    Some(bounds)
}

/// Filled circle in screen space (for dangling-connection endpoint marker).
pub fn filled_disc_path(
    center: Point<Pixels>,
    radius: Pixels,
) -> Result<Path<Pixels>, anyhow::Error> {
    let r: f32 = radius.into();
    let cx: f32 = center.x.into();
    let cy: f32 = center.y.into();
    const SEGMENTS: usize = 28;
    let mut pts: Vec<Point<Pixels>> = Vec::with_capacity(SEGMENTS);
    for i in 0..SEGMENTS {
        let t = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        pts.push(Point::new(px(cx + r * t.cos()), px(cy + r * t.sin())));
    }
    let mut pb = PathBuilder::fill();
    pb.add_polygon(&pts, true);
    pb.build()
}

pub fn edge_bezier(
    start: Point<Pixels>,
    start_position: PortPosition,
    end_poisition: PortPosition,
    end: Point<Pixels>,
    viewport: &Viewport,
) -> Result<Path<Pixels>, anyhow::Error> {
    let control_a = get_control_point(start, start_position, viewport);
    let control_b = get_control_point(end, end_poisition, viewport);
    let mut line = PathBuilder::stroke(px(1.0));
    line.move_to(start);
    line.cubic_bezier_to(end, control_a, control_b);

    line.build()
}
