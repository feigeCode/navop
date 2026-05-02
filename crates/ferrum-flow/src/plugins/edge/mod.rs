use std::collections::HashSet;

use gpui::{Bounds, Element, MouseButton, PathBuilder, Pixels, Point, canvas, px, rgb};

use crate::{
    Edge, EdgeId, RenderContext,
    plugin::{FlowEvent, Plugin, PluginContext},
    plugins::edge::command::ClearEdgeCommand,
};

mod command;

use command::SelectEdgeCommand;

pub struct EdgePlugin {}

impl EdgePlugin {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for EdgePlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for EdgePlugin {
    fn name(&self) -> &'static str {
        "edge"
    }
    fn on_event(
        &mut self,
        event: &FlowEvent,
        ctx: &mut crate::plugin::PluginContext,
    ) -> crate::plugin::EventResult {
        if let FlowEvent::Input(crate::plugin::InputEvent::MouseDown(ev)) = event {
            if ev.button != MouseButton::Left {
                return crate::plugin::EventResult::Continue;
            }
            let shift = ev.modifiers.shift;
            if let Some(id) = hit_test_get_edge(ev.position, ctx) {
                ctx.cache_port_offset_with_edge(&id);
                ctx.execute_command(SelectEdgeCommand::new(id, shift, ctx));
                return crate::plugin::EventResult::Stop;
            } else if !shift {
                ctx.execute_command(ClearEdgeCommand::new(ctx));
            }
        }
        crate::plugin::EventResult::Continue
    }
    fn priority(&self) -> i32 {
        120
    }
    fn render_layer(&self) -> crate::plugin::RenderLayer {
        crate::plugin::RenderLayer::Edges
    }
    fn render(&mut self, ctx: &mut crate::RenderContext) -> Option<gpui::AnyElement> {
        let visible_nodes: HashSet<_> = ctx
            .graph
            .nodes()
            .iter()
            .filter(|(_, node)| ctx.is_node_visible_node(node))
            .map(|(id, _)| *id)
            .collect();

        let edges: Vec<_> = ctx
            .graph
            .edges()
            .iter()
            .filter(|(_, edge)| {
                let Some(source_port) = ctx.graph.get_port(&edge.source_port) else {
                    return false;
                };
                let Some(target_port) = ctx.graph.get_port(&edge.target_port) else {
                    return false;
                };

                visible_nodes.contains(&source_port.node_id())
                    || visible_nodes.contains(&target_port.node_id())
            })
            .map(|(k, v)| (*k, edge_geometry2(v, ctx)))
            .collect();

        let edge_ids = edges.iter().map(|(id, _)| *id);
        for edge_id in edge_ids {
            ctx.cache_port_offset_with_edge(&edge_id);
        }

        let selected_edges = ctx.graph.selected_edge().clone();
        let stroke = ctx.theme.edge_stroke;
        let stroke_sel = ctx.theme.edge_stroke_selected;

        Some(
            canvas(
                move |_, _, _| (edges, selected_edges, stroke, stroke_sel),
                move |_, (edges, selected_edges, stroke, stroke_sel), win, _| {
                    for (id, geometry) in edges.iter() {
                        let Some(EdgeGeometry { start, c1, c2, end }) = geometry else {
                            return;
                        };
                        let mut line = PathBuilder::stroke(px(1.0));
                        line.move_to(*start);
                        line.cubic_bezier_to(*end, *c1, *c2);

                        let selected = selected_edges.iter().any(|i| *i == *id);

                        if let Ok(line) = line.build() {
                            win.paint_path(line, rgb(if selected { stroke_sel } else { stroke }));
                        }
                    }
                },
            )
            .into_any(),
        )
    }
}

pub struct EdgeGeometry {
    pub start: Point<Pixels>,
    pub c1: Point<Pixels>,
    pub c2: Point<Pixels>,
    pub end: Point<Pixels>,
}

fn edge_geometry(edge: &Edge, ctx: &PluginContext) -> Option<EdgeGeometry> {
    let Edge {
        source_port: source_id,
        target_port: target_id,
        ..
    } = edge;

    let start = ctx.port_screen_center_by_port_id(*source_id)?;
    let end = ctx.port_screen_center_by_port_id(*target_id)?;

    let source_port = ctx.graph.get_port(source_id)?;
    let target_port = ctx.graph.get_port(target_id)?;

    let c1 = ctx.edge_control_point(start, source_port.position());
    let c2 = ctx.edge_control_point(end, target_port.position());

    Some(EdgeGeometry { start, c1, c2, end })
}

fn edge_geometry2(edge: &Edge, ctx: &RenderContext) -> Option<EdgeGeometry> {
    let Edge {
        source_port: source_id,
        target_port: target_id,
        ..
    } = edge;

    let start = ctx.port_screen_center_by_port_id(*source_id)?;
    let end = ctx.port_screen_center_by_port_id(*target_id)?;

    let source_port = ctx.graph.get_port(source_id)?;
    let target_port = ctx.graph.get_port(target_id)?;

    let c1 = ctx.edge_control_point(start, source_port.position());
    let c2 = ctx.edge_control_point(end, target_port.position());

    Some(EdgeGeometry { start, c1, c2, end })
}

fn hit_test_get_edge(mouse: Point<Pixels>, ctx: &PluginContext) -> Option<EdgeId> {
    let visible_nodes: HashSet<_> = ctx
        .graph
        .nodes()
        .iter()
        .filter(|(_, node)| ctx.is_node_visible_node(node))
        .map(|(id, _)| *id)
        .collect();

    let edges = ctx.graph.edges_values().filter(|edge| {
        let Some(source_port) = ctx.graph.get_port(&edge.source_port) else {
            return false;
        };
        let Some(target_port) = ctx.graph.get_port(&edge.target_port) else {
            return false;
        };

        visible_nodes.contains(&source_port.node_id())
            || visible_nodes.contains(&target_port.node_id())
    });
    for edge in edges {
        let Some(geom) = edge_geometry(edge, ctx) else {
            continue;
        };

        let bound = edge_bounds(&geom);
        if !bound.contains(&mouse) {
            continue;
        }

        if hit_test_edge(mouse, &geom) {
            return Some(edge.id);
        }
    }

    None
}

pub fn edge_bounds(geom: &EdgeGeometry) -> Bounds<Pixels> {
    let min_x = geom.start.x.min(geom.end.x).min(geom.c1.x).min(geom.c2.x);
    let max_x = geom.start.x.max(geom.end.x).max(geom.c1.x).max(geom.c2.x);

    let min_y = geom.start.y.min(geom.end.y).min(geom.c1.y).min(geom.c2.y);
    let max_y = geom.start.y.max(geom.end.y).max(geom.c1.y).max(geom.c2.y);

    Bounds::from_corners(
        Point::new(min_x - px(10.0), min_y - px(10.0)),
        Point::new(max_x + px(10.0), max_y + px(10.0)),
    )
}

fn hit_test_edge(mouse: Point<Pixels>, geom: &EdgeGeometry) -> bool {
    let points = sample_bezier(geom, 20);

    for segment in points.windows(2) {
        let d = distance_to_segment(mouse, segment[0], segment[1]);

        if d < 8.0 {
            return true;
        }
    }

    false
}

fn sample_bezier(geom: &EdgeGeometry, steps: usize) -> Vec<Point<Pixels>> {
    let mut points = Vec::new();

    for i in 0..=steps {
        let t = i as f32 / steps as f32;

        let x = (1.0 - t).powi(3) * geom.start.x
            + 3.0 * (1.0 - t).powi(2) * t * geom.c1.x
            + 3.0 * (1.0 - t) * t * t * geom.c2.x
            + t.powi(3) * geom.end.x;

        let y = (1.0 - t).powi(3) * geom.start.y
            + 3.0 * (1.0 - t).powi(2) * t * geom.c1.y
            + 3.0 * (1.0 - t) * t * t * geom.c2.y
            + t.powi(3) * geom.end.y;

        points.push(Point::new(x, y));
    }

    points
}
pub fn distance_to_segment(p: Point<Pixels>, a: Point<Pixels>, b: Point<Pixels>) -> f32 {
    let ap = vec_sub(p, a);
    let ab = vec_sub(b, a);

    let ab_len2 = ab.0 * ab.0 + ab.1 * ab.1;

    if ab_len2 == 0.0 {
        return vec_length(ap);
    }

    let t = (vec_dot(ap, ab) / ab_len2).clamp(0.0, 1.0);

    let closest = Point::new(f32::from(a.x) + ab.0 * t, f32::from(a.y) + ab.1 * t);

    let dx = f32::from(p.x) - closest.x;
    let dy = f32::from(p.y) - closest.y;

    (dx * dx + dy * dy).sqrt()
}

fn vec_sub(a: Point<Pixels>, b: Point<Pixels>) -> (f32, f32) {
    (f32::from(a.x - b.x), f32::from(a.y - b.y))
}

fn vec_dot(a: (f32, f32), b: (f32, f32)) -> f32 {
    a.0 * b.0 + a.1 * b.1
}

fn vec_length(v: (f32, f32)) -> f32 {
    (v.0 * v.0 + v.1 * v.1).sqrt()
}
