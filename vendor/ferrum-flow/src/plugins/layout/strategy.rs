//! Pluggable graph layout: [`LayoutStrategy`], [`LayoutPhase`], [`PositionHint`], and options.
//!
//! Implementations live in sibling modules (e.g. layered DAG, force). [`super::LayoutPipeline`]
//! runs several strategies in order, threading [`PositionHint`] between stages.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use gpui::{Pixels, Point};

use crate::{Graph, NodeId};

/// Role of a strategy inside a multi-stage layout pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LayoutPhase {
    /// Coarse placement from topology (layering, tree, …).
    #[default]
    Initializer,
    /// Refinement using positions from earlier stages (`hint`).
    Optimizer,
    /// Cosmetic pass (snap, pack isolates, …).
    PostProcessor,
}

/// Warm-start positions between pipeline stages. Cheap to clone ([`Arc`]).
#[derive(Debug, Clone, Default)]
pub struct PositionHint {
    positions: Arc<HashMap<NodeId, Point<Pixels>>>,
}

impl PositionHint {
    /// Current node centers from `graph` (world space).
    pub fn from_graph(graph: &Graph) -> Self {
        let mut m = HashMap::new();
        for id in graph.nodes().keys() {
            if let Some(n) = graph.get_node(id) {
                m.insert(*id, n.point());
            }
        }
        Self {
            positions: Arc::new(m),
        }
    }

    pub fn from_positions(positions: HashMap<NodeId, Point<Pixels>>) -> Self {
        Self {
            positions: Arc::new(positions),
        }
    }

    /// Take proposed centers from a previous stage’s [`LayoutOutput::Delta`].
    pub fn from_delta_to(delta: &NodePositionDelta) -> Self {
        Self::from_positions(delta.to.iter().map(|(id, p)| (*id, *p)).collect())
    }

    pub fn positions(&self) -> &HashMap<NodeId, Point<Pixels>> {
        &self.positions
    }

    pub fn get(&self, id: &NodeId) -> Option<Point<Pixels>> {
        self.positions.get(id).copied()
    }

    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }
}

/// Tunable knobs for layout passes. Marked [`non_exhaustive`] so new fields can be added without
/// breaking downstream struct literals.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct LayoutOptions {
    /// World-space gap between adjacent layers (e.g. left-to-right DAG).
    pub layer_spacing: f32,
    /// World-space gap between nodes in the same layer.
    pub sibling_spacing: f32,
    /// Primary flow direction for layered layouts.
    pub direction: LayoutDirection,

    /// Force-directed phase: iteration budget (algorithms may clamp).
    pub force_iterations: u32,
    /// Force-directed phase: when **> 0** and finite, stop once the largest capped displacement in
    /// an iteration falls **strictly below** this value (world pixels). Use `0.0` to disable early
    /// exit (always run up to the iteration budget).
    pub force_convergence_threshold: f32,
    /// When true, [`crate::plugins::layout::PackIsolatedNodesLayout`] moves degree-0 nodes into a strip under the connected subgraph.
    pub pack_isolated_nodes: bool,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self {
            layer_spacing: 120.0,
            sibling_spacing: 48.0,
            direction: LayoutDirection::LeftToRight,
            force_iterations: 200,
            force_convergence_threshold: 1.0,
            pack_isolated_nodes: false,
        }
    }
}

/// Primary axis along which “layers” advance for layered algorithms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LayoutDirection {
    #[default]
    LeftToRight,
    TopToBottom,
}

/// Before / after node positions for one layout pass (same length, paired by index).
#[derive(Debug, Clone)]
pub struct NodePositionDelta {
    pub(crate) from: Vec<(NodeId, Point<Pixels>)>,
    pub(crate) to: Vec<(NodeId, Point<Pixels>)>,
}

impl NodePositionDelta {
    pub fn new(from: Vec<(NodeId, Point<Pixels>)>, to: Vec<(NodeId, Point<Pixels>)>) -> Self {
        Self { from, to }
    }

    /// Returns `true` if any node position changed (using raw pixel comparison).
    pub fn has_changes(&self) -> bool {
        self.from.len() == self.to.len()
            && self
                .from
                .iter()
                .zip(self.to.iter())
                .any(|((_, a), (_, b))| a != b)
    }
}

/// Result of a successful layout computation (no fatal error).
#[derive(Debug, Clone)]
pub enum LayoutOutput {
    /// No moves to apply (empty graph, filtered set, or already satisfied).
    Unchanged,
    /// Apply with [`crate::plugins::node::DragNodesCommand::from_positions`].
    Delta(NodePositionDelta),
}

/// Recoverable layout failure (graph shape, missing data, unsupported case).
#[derive(Debug, Clone)]
pub enum LayoutError {
    /// Graph has no nodes to lay out.
    EmptyGraph,
    /// Directed cycle prevents a pure layered ordering (caller may choose another strategy).
    CycleInGraph,
    /// User-visible or loggable detail.
    Message(String),
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyGraph => write!(f, "empty graph"),
            Self::CycleInGraph => write!(f, "graph contains a directed cycle"),
            Self::Message(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for LayoutError {}

/// One layout algorithm (layered DAG, force, grid, …) or a composite pipeline.
///
/// Object-safe: use `&Graph` and owned output; keep impls [`Send`] + [`Sync`] so the canvas can
/// hold `Arc<dyn LayoutStrategy>` across threads if needed.
pub trait LayoutStrategy: Send + Sync {
    /// Stable id for menus, config, and logging (e.g. `"layered_dag"`).
    fn id(&self) -> &'static str;

    /// Short human-readable label (optional UI).
    fn label(&self) -> &'static str {
        self.id()
    }

    /// Pipeline role; used to document ordering and future UI grouping.
    fn phase(&self) -> LayoutPhase;

    /// If `false`, a pipeline may skip this stage for the current graph.
    fn can_apply(&self, graph: &Graph) -> bool {
        let _ = graph;
        true
    }

    /// Compute new positions. Must not mutate `graph`; callers apply [`LayoutOutput::Delta`].
    ///
    /// `hint` carries centers from earlier pipeline stages; [`None`] for the first stage.
    fn compute(
        &self,
        graph: &Graph,
        options: &LayoutOptions,
        hint: Option<&PositionHint>,
    ) -> Result<LayoutOutput, LayoutError>;
}
