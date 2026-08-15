//! When [`LayoutOptions::pack_isolated_nodes`] is true, moves **isolated** nodes (no undirected
//! graph edge to another node) into a horizontal row **below** the axis-aligned bounds of all
//! connected nodes. Uses the same top-left convention as [`super::layered_dag`] /
//! [`super::force_directed`] ([`crate::Node::point`] + size).

use std::collections::{HashMap, HashSet};

use gpui::{Point, px};

use crate::{Graph, NodeId};

use super::strategy::{
    LayoutError, LayoutOptions, LayoutOutput, LayoutPhase, LayoutStrategy, NodePositionDelta,
    PositionHint,
};

/// Packs nodes with degree 0 in the port-edge graph into a strip under the rest of the graph.
#[derive(Debug, Default, Clone, Copy)]
pub struct PackIsolatedNodesLayout;

impl LayoutStrategy for PackIsolatedNodesLayout {
    fn id(&self) -> &'static str {
        "pack_isolated_nodes"
    }

    fn label(&self) -> &'static str {
        "Pack isolated nodes"
    }

    fn phase(&self) -> LayoutPhase {
        LayoutPhase::PostProcessor
    }

    fn compute(
        &self,
        graph: &Graph,
        options: &LayoutOptions,
        hint: Option<&PositionHint>,
    ) -> Result<LayoutOutput, LayoutError> {
        if !options.pack_isolated_nodes {
            return Ok(LayoutOutput::Unchanged);
        }
        if graph.nodes().is_empty() {
            return Err(LayoutError::EmptyGraph);
        }

        let edges = undirected_edges(graph);
        let mut deg: HashMap<NodeId, usize> = graph.nodes().keys().map(|id| (*id, 0)).collect();
        for &(u, v) in &edges {
            *deg.entry(u).or_insert(0) += 1;
            *deg.entry(v).or_insert(0) += 1;
        }

        let isolated: HashSet<NodeId> = graph
            .nodes()
            .keys()
            .copied()
            .filter(|id| *deg.get(id).unwrap_or(&0) == 0)
            .collect();

        if isolated.is_empty() {
            return Ok(LayoutOutput::Unchanged);
        }

        let mut pos: HashMap<NodeId, (f32, f32)> = HashMap::new();
        for id in graph.nodes().keys() {
            let Some(n) = graph.get_node(id) else {
                continue;
            };
            let p = hint.and_then(|h| h.get(id)).unwrap_or_else(|| n.point());
            pos.insert(*id, (px_f32(p.x), px_f32(p.y)));
        }

        let bbox_connected = union_bbox_connected(graph, &pos, &deg);

        let gap = options.sibling_spacing.max(8.0);
        let ordered = ordered_node_ids(graph);
        let isolated_ordered: Vec<NodeId> = ordered
            .into_iter()
            .filter(|id| isolated.contains(id))
            .collect();

        if let Some((min_x, _min_y, _max_x, max_y)) = bbox_connected {
            let row_top = max_y + gap;
            let mut x = min_x;
            for id in &isolated_ordered {
                let Some(n) = graph.get_node(id) else {
                    continue;
                };
                let w = px_f32(n.size_ref().width);
                pos.insert(*id, (x, row_top));
                x += w + gap;
            }
        } else {
            // Every node is isolated: single row from origin.
            let mut x = 0.0f32;
            let y = 0.0f32;
            for id in &isolated_ordered {
                let Some(n) = graph.get_node(id) else {
                    continue;
                };
                let w = px_f32(n.size_ref().width);
                pos.insert(*id, (x, y));
                x += w + gap;
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

fn union_bbox_connected(
    graph: &Graph,
    pos: &HashMap<NodeId, (f32, f32)>,
    deg: &HashMap<NodeId, usize>,
) -> Option<(f32, f32, f32, f32)> {
    let mut bbox: Option<(f32, f32, f32, f32)> = None;
    for id in graph.nodes().keys() {
        if *deg.get(id).unwrap_or(&0) == 0 {
            continue;
        }
        let Some(n) = graph.get_node(id) else {
            continue;
        };
        let Some(&(x, y)) = pos.get(id) else {
            continue;
        };
        let w = px_f32(n.size_ref().width);
        let h = px_f32(n.size_ref().height);
        let ax1 = x + w;
        let ay1 = y + h;
        bbox = Some(match bbox {
            None => (x, y, ax1, ay1),
            Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(ax1), y1.max(ay1)),
        });
    }
    bbox
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
    fn disabled_when_flag_off() {
        let graph = Graph::build(|g| {
            let (_, _, o) = g
                .create_node("")
                .position(0.0, 0.0)
                .output()
                .data(json!({}))
                .build_with_ports();
            let (_, i, _) = g
                .create_node("")
                .position(400.0, 0.0)
                .input()
                .data(json!({}))
                .build_with_ports();
            g.create_edge().source(o[0]).target(i[0]).build();
        });
        let s = PackIsolatedNodesLayout;
        let out = s
            .compute(
                &graph,
                &LayoutOptions {
                    pack_isolated_nodes: false,
                    ..Default::default()
                },
                None,
            )
            .expect("compute");
        assert!(matches!(out, LayoutOutput::Unchanged));
    }

    #[test]
    fn moves_isolate_below_connected_block() {
        let graph = Graph::build(|g| {
            let (_, _, o_a) = g
                .create_node("")
                .position(0.0, 0.0)
                .size(80.0, 40.0)
                .output()
                .data(json!({ "label": "A" }))
                .build_with_ports();
            let (_, i_b, _o_b) = g
                .create_node("")
                .position(120.0, 0.0)
                .size(80.0, 40.0)
                .input()
                .output()
                .data(json!({ "label": "B" }))
                .build_with_ports();
            g.create_edge().source(o_a[0]).target(i_b[0]).build();
            g.create_node("")
                .position(500.0, 10.0)
                .size(60.0, 40.0)
                .data(json!({ "label": "lonely" }))
                .build();
        });

        let s = PackIsolatedNodesLayout;
        let out = s
            .compute(
                &graph,
                &LayoutOptions {
                    pack_isolated_nodes: true,
                    sibling_spacing: 16.0,
                    ..Default::default()
                },
                None,
            )
            .expect("compute");

        let LayoutOutput::Delta(d) = out else {
            panic!("expected delta");
        };
        assert!(d.has_changes());

        let lonely_y =
            d.to.iter()
                .find_map(|(id, p)| {
                    let n = graph.get_node(id)?;
                    (n.data_ref().get("label") == Some(&json!("lonely"))).then_some(px_f32(p.y))
                })
                .expect("lonely node");
        assert!(
            lonely_y >= 56.0,
            "isolated node should sit below connected bbox, y={lonely_y}"
        );
    }
}
