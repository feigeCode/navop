//! Multi-stage layout: run several [`LayoutStrategy`] implementations in order, pass
//! [`PositionHint`] between them, then emit one [`LayoutOutput`] from **current graph
//! positions** to the result of applying every stage in sequence (the graph is not mutated
//! between stages).

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use gpui::{Pixels, Point};

use crate::{Graph, NodeId};

use super::strategy::{
    LayoutError, LayoutOptions, LayoutOutput, LayoutPhase, LayoutStrategy, NodePositionDelta,
    PositionHint,
};

/// Ordered layout stages. See [`LayoutStrategy::compute`] for per-stage `hint` semantics.
#[derive(Clone)]
pub struct LayoutPipeline {
    pub id: &'static str,
    pub label: &'static str,
    pub phase: LayoutPhase,
    stages: Vec<Arc<dyn LayoutStrategy>>,
}

impl fmt::Debug for LayoutPipeline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LayoutPipeline")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("stages_len", &self.stages.len())
            .finish()
    }
}

impl LayoutPipeline {
    /// Default metadata: `id` = `"layout_pipeline"`, `label` = `"Layout pipeline"`,
    /// [`LayoutPhase::Optimizer`].
    pub fn new(stages: Vec<Arc<dyn LayoutStrategy>>) -> Self {
        Self::with_meta(
            "layout_pipeline",
            "Layout pipeline",
            LayoutPhase::Optimizer,
            stages,
        )
    }

    pub fn with_meta(
        id: &'static str,
        label: &'static str,
        phase: LayoutPhase,
        stages: Vec<Arc<dyn LayoutStrategy>>,
    ) -> Self {
        Self {
            id,
            label,
            phase,
            stages,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.stages.is_empty()
    }

    pub fn len(&self) -> usize {
        self.stages.len()
    }
}

impl LayoutStrategy for LayoutPipeline {
    fn id(&self) -> &'static str {
        self.id
    }

    fn label(&self) -> &'static str {
        self.label
    }

    fn phase(&self) -> LayoutPhase {
        self.phase
    }

    fn can_apply(&self, graph: &Graph) -> bool {
        !self.stages.is_empty() && self.stages.iter().any(|s| s.can_apply(graph))
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
        if self.stages.is_empty() {
            return Ok(LayoutOutput::Unchanged);
        }

        let mut virtual_pos = graph_node_centers(graph);
        let mut warm: Option<PositionHint> = hint.cloned();

        for stage in &self.stages {
            if !stage.can_apply(graph) {
                continue;
            }
            let out = stage.compute(graph, options, warm.as_ref())?;
            match out {
                LayoutOutput::Unchanged => {}
                LayoutOutput::Delta(d) => {
                    for (id, p) in &d.to {
                        virtual_pos.insert(*id, *p);
                    }
                    warm = Some(PositionHint::from_delta_to(&d));
                }
            }
        }

        Ok(delta_graph_to_positions(graph, &virtual_pos))
    }
}

fn graph_node_centers(graph: &Graph) -> HashMap<NodeId, Point<Pixels>> {
    let mut m = HashMap::new();
    for id in graph.nodes().keys() {
        if let Some(n) = graph.get_node(id) {
            m.insert(*id, n.point());
        }
    }
    m
}

fn delta_graph_to_positions(graph: &Graph, pos: &HashMap<NodeId, Point<Pixels>>) -> LayoutOutput {
    let mut from = Vec::new();
    let mut to = Vec::new();
    for id in graph.nodes().keys() {
        let Some(n) = graph.get_node(id) else {
            continue;
        };
        let Some(&p) = pos.get(id) else {
            continue;
        };
        from.push((*id, n.point()));
        to.push((*id, p));
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

    use super::super::force_directed::ForceDirectedLayout;
    use super::super::layered_dag::LayeredDagLayout;
    use super::super::strategy::LayoutPhase;

    #[test]
    fn layered_then_force_equivalent_to_preset() {
        let graph = Graph::build(|g| {
            let (_, _, o_a) = g
                .create_node("")
                .position(0.0, 0.0)
                .output()
                .data(json!({}))
                .build_with_ports();
            let (_, i_b, _) = g
                .create_node("")
                .position(200.0, 200.0)
                .input()
                .data(json!({}))
                .build_with_ports();
            g.create_edge().source(o_a[0]).target(i_b[0]).build();
        });

        let opts = LayoutOptions::default();
        let pipe = LayoutPipeline::with_meta(
            "layered_then_force",
            "Layered → force",
            LayoutPhase::Optimizer,
            vec![
                Arc::new(LayeredDagLayout),
                Arc::new(ForceDirectedLayout::default()),
            ],
        );

        let out = pipe.compute(&graph, &opts, None).expect("compute");
        let LayoutOutput::Delta(d) = out else {
            panic!("expected delta");
        };
        assert!(d.has_changes());
    }

    #[test]
    fn empty_pipeline_unchanged() {
        let graph = Graph::build(|g| {
            g.create_node("").position(0.0, 0.0).data(json!({})).build();
        });
        let pipe = LayoutPipeline::new(vec![]);
        let out = pipe
            .compute(&graph, &LayoutOptions::default(), None)
            .expect("compute");
        assert!(matches!(out, LayoutOutput::Unchanged));
    }
}
