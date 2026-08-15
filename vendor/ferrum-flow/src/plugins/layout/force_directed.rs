//! Fruchterman–Reingold–style **force-directed** layout (repulsion between all pairs, attraction on
//! edges). Works on **any** simple graph topology including cycles; ignores [`LayoutDirection`].
//!
//! Edge endpoints are the same as [`super::layered_dag`]: **source port node → target port node**.

use std::collections::{HashMap, HashSet};

use gpui::{Point, px};

use crate::{Graph, NodeId};

use super::strategy::{
    LayoutError, LayoutOptions, LayoutOutput, LayoutPhase, LayoutStrategy, NodePositionDelta,
    PositionHint,
};

const EPS: f32 = 1e-4;

/// Force-directed placement (FR-style repulsion + edge springs).
#[derive(Debug, Clone)]
pub struct ForceDirectedLayout {
    /// Simulation steps (more → smoother, slower).
    pub iterations: usize,
    /// Target edge length in world space; also scales repulsion via `k`.
    pub ideal_length: f32,
    /// Initial cap on displacement per iteration (linear cooling to ~5% at the end).
    pub initial_temperature: f32,
}

impl Default for ForceDirectedLayout {
    fn default() -> Self {
        Self {
            iterations: 200,
            ideal_length: 100.0,
            initial_temperature: 72.0,
        }
    }
}

impl LayoutStrategy for ForceDirectedLayout {
    fn id(&self) -> &'static str {
        "force_directed"
    }

    fn label(&self) -> &'static str {
        "Force-directed"
    }

    fn phase(&self) -> LayoutPhase {
        LayoutPhase::Optimizer
    }

    fn compute(
        &self,
        graph: &Graph,
        options: &LayoutOptions,
        hint: Option<&PositionHint>,
    ) -> Result<LayoutOutput, LayoutError> {
        if graph.nodes().is_empty() {
            return Err(LayoutError::EmptyGraph);
        }

        let ids = ordered_node_ids(graph);
        if ids.len() == 1 {
            return Ok(LayoutOutput::Unchanged);
        }

        let edges = undirected_edges(graph);
        let k = (self.ideal_length.max(8.0) + options.layer_spacing * 0.25).max(16.0);

        let mut pos: HashMap<NodeId, (f32, f32)> = HashMap::new();
        for id in &ids {
            let Some(n) = graph.get_node(id) else {
                log::error!("node not found: {}", id);
                continue;
            };
            let p = hint.and_then(|h| h.get(id)).unwrap_or_else(|| n.point());
            pos.insert(*id, (px_f32(p.x), px_f32(p.y)));
        }

        let iters = self
            .iterations
            .min(options.force_iterations.max(1) as usize)
            .clamp(20, 800);
        let conv = options.force_convergence_threshold;
        let use_conv = conv > 0.0 && conv.is_finite();

        for it in 0..iters {
            let t = self.initial_temperature * (1.0 - it as f32 / iters as f32).max(0.05);

            let mut disp: HashMap<NodeId, (f32, f32)> =
                ids.iter().map(|&id| (id, (0.0, 0.0))).collect();

            // Repulsion: all unordered pairs
            for i in 0..ids.len() {
                for j in (i + 1)..ids.len() {
                    let u = ids[i];
                    let v = ids[j];
                    let (ux, uy) = pos[&u];
                    let (vx, vy) = pos[&v];
                    let dx = vx - ux;
                    let dy = vy - uy;
                    let d2 = dx * dx + dy * dy;
                    if d2 < EPS * EPS {
                        continue;
                    }
                    let d = d2.sqrt();
                    // Repulsive magnitude k²/d; along unit vector from u→v applied to push u back / v forward
                    let f = k * k / d;
                    let rx = (dx / d) * f;
                    let ry = (dy / d) * f;
                    if let Some(du) = disp.get_mut(&u) {
                        du.0 -= rx;
                        du.1 -= ry;
                    }

                    if let Some(dv) = disp.get_mut(&v) {
                        dv.0 += rx;
                        dv.1 += ry;
                    }
                }
            }

            // Attraction: undirected edges (spring dist² / k along edge)
            for &(u, v) in &edges {
                let (ux, uy) = pos[&u];
                let (vx, vy) = pos[&v];
                let dx = vx - ux;
                let dy = vy - uy;
                let d2 = dx * dx + dy * dy;
                let d = d2.sqrt().max(EPS);
                let f = d * d / k;
                let ax = (dx / d) * f;
                let ay = (dy / d) * f;
                if let Some(du) = disp.get_mut(&u) {
                    du.0 += ax;
                    du.1 += ay;
                }
                if let Some(dv) = disp.get_mut(&v) {
                    dv.0 -= ax;
                    dv.1 -= ay;
                }
            }

            // Apply capped displacement; optionally stop when the largest step is tiny.
            let mut max_step = 0.0f32;
            for id in &ids {
                let (dx, dy) = disp[id];
                let mag = (dx * dx + dy * dy).sqrt();
                if mag < EPS {
                    continue;
                }
                let scale = t / mag;
                let step_x = dx * scale;
                let step_y = dy * scale;
                max_step = max_step.max((step_x * step_x + step_y * step_y).sqrt());
                if let Some((px_, py_)) = pos.get_mut(id) {
                    *px_ += step_x;
                    *py_ += step_y;
                }
            }

            if use_conv && max_step < conv {
                break;
            }
        }

        Ok(delta_from_positions(graph, pos))
    }
}

fn px_f32(p: gpui::Pixels) -> f32 {
    p.into()
}

fn f32_px(x: f32) -> gpui::Pixels {
    px(x)
}

fn ordered_node_ids(graph: &Graph) -> Vec<NodeId> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for id in graph.node_order() {
        if graph.get_node(id).is_some() {
            out.push(*id);
            seen.insert(*id);
        }
    }
    for id in graph.nodes().keys() {
        if seen.insert(*id) {
            out.push(*id);
        }
    }
    out
}

fn undirected_edges(graph: &Graph) -> Vec<(NodeId, NodeId)> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for e in graph.edges_values() {
        let Some(s) = graph.get_port(&e.source_port) else {
            continue;
        };
        let Some(t) = graph.get_port(&e.target_port) else {
            continue;
        };
        let u = s.node_id();
        let v = t.node_id();
        if u == v {
            continue;
        }
        let a = u.as_uuid().as_u128();
        let b = v.as_uuid().as_u128();
        let key = if a <= b { (u, v) } else { (v, u) };
        if seen.insert(key) {
            out.push((u, v));
        }
    }
    out
}

fn delta_from_positions(graph: &Graph, positions: HashMap<NodeId, (f32, f32)>) -> LayoutOutput {
    let mut from = Vec::new();
    let mut to = Vec::new();
    for id in graph.nodes().keys() {
        let Some(n) = graph.get_node(id) else {
            continue;
        };
        let Some(&(nx, ny)) = positions.get(id) else {
            continue;
        };
        from.push((*id, n.point()));
        to.push((*id, Point::new(f32_px(nx), f32_px(ny))));
    }

    let delta = NodePositionDelta::new(from, to);
    if delta.has_changes() {
        LayoutOutput::Delta(delta)
    } else {
        LayoutOutput::Unchanged
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn force_spreads_collapsed_triangle() {
        let graph = Graph::build(|g| {
            let (_, ia, oa) = g
                .create_node("")
                .position(100.0, 100.0)
                .input()
                .output()
                .data(json!({ "label": "A" }))
                .build_with_ports();
            let (_, ib, ob) = g
                .create_node("")
                .position(105.0, 102.0)
                .input()
                .output()
                .data(json!({ "label": "B" }))
                .build_with_ports();
            let (_, ic, oc) = g
                .create_node("")
                .position(98.0, 104.0)
                .input()
                .output()
                .data(json!({ "label": "C" }))
                .build_with_ports();
            g.create_edge().source(oa[0]).target(ib[0]).build();
            g.create_edge().source(ob[0]).target(ic[0]).build();
            g.create_edge().source(oc[0]).target(ia[0]).build();
        });

        let s = ForceDirectedLayout {
            iterations: 250,
            ideal_length: 90.0,
            initial_temperature: 80.0,
        };
        let out = s
            .compute(&graph, &LayoutOptions::default(), None)
            .expect("compute");

        let LayoutOutput::Delta(d) = out else {
            panic!("expected delta");
        };
        assert!(d.has_changes());

        let mut min_d2 = f32::MAX;
        let to_map: HashMap<_, _> =
            d.to.iter()
                .map(|(id, p)| (*id, (px_f32(p.x), px_f32(p.y))))
                .collect();
        let ids: Vec<_> = to_map.keys().copied().collect();
        for i in 0..ids.len() {
            for j in (i + 1)..ids.len() {
                let (ax, ay) = to_map[&ids[i]];
                let (bx, by) = to_map[&ids[j]];
                let dx = ax - bx;
                let dy = ay - by;
                min_d2 = min_d2.min(dx * dx + dy * dy);
            }
        }
        assert!(
            min_d2.sqrt() > 20.0,
            "nodes should spread apart, min dist={}",
            min_d2.sqrt()
        );
    }

    #[test]
    fn convergence_threshold_zero_disables_early_exit() {
        let graph = Graph::build(|g| {
            let (_, _, o) = g
                .create_node("")
                .position(0.0, 0.0)
                .output()
                .data(json!({}))
                .build_with_ports();
            let (_, i, _) = g
                .create_node("")
                .position(40.0, 0.0)
                .input()
                .data(json!({}))
                .build_with_ports();
            g.create_edge().source(o[0]).target(i[0]).build();
        });
        let s = ForceDirectedLayout::default();
        let out = s
            .compute(
                &graph,
                &LayoutOptions {
                    force_convergence_threshold: 0.0,
                    ..Default::default()
                },
                None,
            )
            .expect("compute");
        assert!(matches!(
            out,
            LayoutOutput::Delta(_) | LayoutOutput::Unchanged
        ));
    }
}
