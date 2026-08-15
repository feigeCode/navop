//! Left-to-right (or top-to-bottom) **layered** layout for directed acyclic graphs.
//!
//! Edges follow **source port → target port** (producer node → consumer node). If the graph has a
//! directed cycle (at the node level), placement falls back to a simple grid so the user still
//! gets a readable stack.

use std::collections::{HashMap, HashSet, VecDeque};

use gpui::{Point, px};

use crate::{Graph, NodeId};

use super::strategy::{
    LayoutDirection, LayoutError, LayoutOptions, LayoutOutput, LayoutPhase, LayoutStrategy,
    NodePositionDelta, PositionHint,
};

/// Longest-path layering; cycles fall back to a fixed grid ordered by [`Graph::node_order`].
#[derive(Debug, Default, Clone, Copy)]
pub struct LayeredDagLayout;

impl LayoutStrategy for LayeredDagLayout {
    fn id(&self) -> &'static str {
        "layered_dag"
    }

    fn label(&self) -> &'static str {
        "Layered DAG"
    }

    fn phase(&self) -> LayoutPhase {
        LayoutPhase::Initializer
    }

    fn compute(
        &self,
        graph: &Graph,
        options: &LayoutOptions,
        _hint: Option<&PositionHint>,
    ) -> Result<LayoutOutput, LayoutError> {
        if graph.nodes().is_empty() {
            return Err(LayoutError::EmptyGraph);
        }

        let ordered = ordered_node_ids(graph);
        let edges = node_edges(graph);

        let (layers, cyclic) = assign_layers(&ordered, &edges);

        let positions = if cyclic {
            grid_positions(graph, &ordered, options)
        } else {
            layered_positions(graph, &ordered, &layers, options)
        };

        Ok(delta_from_positions(graph, positions))
    }
}

fn px_f32(p: gpui::Pixels) -> f32 {
    p.into()
}

fn f32_px(x: f32) -> gpui::Pixels {
    px(x)
}

/// `node_order` first, then any stray nodes.
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

/// Directed edges u → v (data from output node to input node).
fn node_edges(graph: &Graph) -> Vec<(NodeId, NodeId)> {
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
        if u != v {
            out.push((u, v));
        }
    }
    out
}

/// Returns `(node -> layer, true if cyclic at node level)`.
fn assign_layers(ordered: &[NodeId], edges: &[(NodeId, NodeId)]) -> (HashMap<NodeId, usize>, bool) {
    let n = ordered.len();
    let mut in_deg: HashMap<NodeId, usize> = HashMap::new();
    let mut adj: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for &id in ordered {
        in_deg.insert(id, 0);
    }
    for &(u, v) in edges {
        *in_deg.entry(v).or_insert(0) += 1;
        adj.entry(u).or_default().push(v);
    }

    let mut q: VecDeque<NodeId> = in_deg
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(id, _)| *id)
        .collect();

    let mut topo = Vec::new();
    let mut tmp = in_deg.clone();
    while let Some(u) = q.pop_front() {
        topo.push(u);
        for v in adj.get(&u).into_iter().flatten() {
            let Some(d) = tmp.get_mut(v) else {
                log::error!("in_deg not found for node: {}", v);
                continue;
            };
            *d -= 1;
            if *d == 0 {
                q.push_back(*v);
            }
        }
    }

    let cyclic = topo.len() != n;

    let mut layer: HashMap<NodeId, usize> = ordered.iter().map(|&id| (id, 0)).collect();
    if !cyclic {
        for &u in &topo {
            let lu = layer[&u];
            for v in adj.get(&u).into_iter().flatten() {
                let lv = layer.entry(*v).or_insert(0);
                *lv = (*lv).max(lu + 1);
            }
        }
    }

    (layer, cyclic)
}

fn max_node_width(graph: &Graph, ids: &[NodeId]) -> f32 {
    ids.iter()
        .filter_map(|id| graph.get_node(id))
        .map(|n| px_f32(n.size_ref().width))
        .fold(0.0f32, f32::max)
        .max(1.0)
}

fn layered_positions(
    graph: &Graph,
    ordered: &[NodeId],
    layers: &HashMap<NodeId, usize>,
    options: &LayoutOptions,
) -> HashMap<NodeId, (f32, f32)> {
    let max_w = max_node_width(graph, ordered).max(1.0);
    let col_pitch = max_w + options.layer_spacing.max(8.0);
    let row_gap = options.sibling_spacing.max(8.0);

    let order_index: HashMap<NodeId, usize> =
        ordered.iter().enumerate().map(|(i, id)| (*id, i)).collect();

    let max_layer = layers.values().copied().max().unwrap_or(0);
    let mut by_layer: Vec<Vec<NodeId>> = vec![Vec::new(); max_layer + 1];
    for &id in ordered {
        let l = *layers.get(&id).unwrap_or(&0);
        if l < by_layer.len() {
            by_layer[l].push(id);
        }
    }
    for col in &mut by_layer {
        col.sort_by_key(|id| order_index.get(id).copied().unwrap_or(usize::MAX));
    }

    let mut out = HashMap::new();
    match options.direction {
        LayoutDirection::LeftToRight => {
            for (layer_idx, col_nodes) in by_layer.iter().enumerate() {
                let x = layer_idx as f32 * col_pitch;
                let mut y = 0.0f32;
                for id in col_nodes {
                    let Some(n) = graph.get_node(id) else {
                        continue;
                    };
                    let h = px_f32(n.size_ref().height);
                    out.insert(*id, (x, y));
                    y += h + row_gap;
                }
            }
        }
        LayoutDirection::TopToBottom => {
            for (layer_idx, row_nodes) in by_layer.iter().enumerate() {
                let y = layer_idx as f32 * col_pitch;
                let mut x = 0.0f32;
                for id in row_nodes {
                    let Some(n) = graph.get_node(id) else {
                        continue;
                    };
                    let w = px_f32(n.size_ref().width);
                    out.insert(*id, (x, y));
                    x += w + row_gap;
                }
            }
        }
    }
    out
}

/// Fallback when the node graph is not a DAG.
fn grid_positions(
    graph: &Graph,
    ordered: &[NodeId],
    options: &LayoutOptions,
) -> HashMap<NodeId, (f32, f32)> {
    let gap_x = options.layer_spacing.max(24.0);
    let gap_y = options.sibling_spacing.max(24.0);
    let cols = (ordered.len() as f32).sqrt().ceil().max(1.0) as usize;

    let mut out = HashMap::new();
    for (i, id) in ordered.iter().enumerate() {
        let Some(n) = graph.get_node(id) else {
            continue;
        };
        let w = px_f32(n.size_ref().width);
        let h = px_f32(n.size_ref().height);
        let col = i % cols;
        let row = i / cols;
        let x = col as f32 * (w + gap_x);
        let y = row as f32 * (h + gap_y);
        out.insert(*id, (x, y));
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
    use crate::Graph;
    use serde_json::json;

    #[test]
    fn layered_chain_three_nodes() {
        let graph = Graph::build(|g| {
            let (_, _, o_a) = g
                .create_node("")
                .position(0.0, 0.0)
                .output()
                .data(json!({}))
                .build_with_ports();
            let (_, i_b, o_b) = g
                .create_node("")
                .position(500.0, 500.0)
                .input()
                .output()
                .data(json!({}))
                .build_with_ports();
            let (_, i_c, _) = g
                .create_node("")
                .position(900.0, 0.0)
                .input()
                .data(json!({}))
                .build_with_ports();
            g.create_edge().source(o_a[0]).target(i_b[0]).build();
            g.create_edge().source(o_b[0]).target(i_c[0]).build();
        });

        let s = LayeredDagLayout;
        let out = s
            .compute(
                &graph,
                &LayoutOptions {
                    layer_spacing: 80.0,
                    sibling_spacing: 32.0,
                    direction: LayoutDirection::LeftToRight,
                    ..Default::default()
                },
                None,
            )
            .expect("compute");

        let LayoutOutput::Delta(d) = out else {
            panic!("expected delta");
        };
        assert!(d.has_changes());
        // Three distinct layers → x order increasing along chain
        let mut xs: Vec<f32> = d.to.iter().map(|(_, p)| px_f32(p.x)).collect();
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert!(xs[0] < xs[1] && xs[1] < xs[2], "xs={xs:?}");
    }
}
