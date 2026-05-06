//! Helpers for verifying that [`Command`](crate::Command) implementations agree between
//! [`Command::execute`](crate::Command::execute), [`Command::undo`](crate::Command::undo), and
//! [`Command::to_ops`](crate::Command::to_ops).
//!
//! Enable the **`testing`** Cargo feature on `ferrum-flow` to use this module:
//!
//! ```toml
//! ferrum-flow = { version = "…", features = ["testing"] }
//! ```
//!
//! Run this crate’s built-in interop tests with:
//!
//! ```text
//! cargo test -p ferrum-flow --features testing
//! ```
//!
//! The public entry points are [`graph_snapshot`] and [`assert_command_interop`]. Example tests that
//! use them live next to each [`Command`](crate::Command) implementation under `plugins/` (and
//! `plugins/port/command.rs` for create commands).

use serde_json::{Value, json};

use crate::{
    Command, CommandContext, Graph, GraphOp, RendererRegistry, SharedState, Viewport,
    canvas::PortLayoutCache,
};

fn with_command_ctx<R>(graph: &mut Graph, f: impl FnOnce(&mut CommandContext) -> R) -> R {
    let mut port_offset_cache = PortLayoutCache::new();
    let mut viewport = Viewport::new();
    let mut renderers = RendererRegistry::new();
    let mut shared_state = SharedState::new();
    let mut notify = || {};
    let mut ctx = CommandContext::new(
        graph,
        &mut port_offset_cache,
        &mut viewport,
        &mut renderers,
        &mut shared_state,
        &mut notify,
    );
    f(&mut ctx)
}

fn apply_graph_op(graph: &mut Graph, op: GraphOp) {
    match op {
        GraphOp::AddNode(node) => {
            graph.add_node_without_order(node);
        }
        GraphOp::RemoveNode { id } => graph.remove_node(&id),
        GraphOp::MoveNode { id, x, y } => {
            if let Some(node) = graph.get_node_mut(&id) {
                node.set_position(x.into(), y.into());
            }
        }
        GraphOp::ResizeNode { id, size } => {
            if let Some(node) = graph.get_node_mut(&id) {
                node.set_size_mut(size);
            }
        }
        GraphOp::UpdateNodeData { id, data } => {
            if let Some(node) = graph.get_node_mut(&id) {
                node.set_data(data);
            }
        }
        GraphOp::NodeOrderInsert { id } => graph.node_order_mut().push(id),
        GraphOp::NodeOrderRemove { index } => {
            if index < graph.node_order().len() {
                graph.node_order_mut().remove(index);
            }
        }
        GraphOp::AddPort(port) => graph.add_port(port),
        GraphOp::RemovePort(id) => graph.remove_port(&id),
        GraphOp::AddEdge(edge) => graph.add_edge(edge),
        GraphOp::RemoveEdge(id) => graph.remove_edge(&id),
        GraphOp::Batch(ops) => {
            for op in ops {
                apply_graph_op(graph, op);
            }
        }
    }
}

/// Canonical JSON snapshot of a [`Graph`] for stable equality checks (sorted maps / sets).
pub fn graph_snapshot(graph: &Graph) -> Value {
    let mut nodes: Vec<_> = graph
        .nodes()
        .iter()
        .map(|(id, n)| {
            (
                id.to_string(),
                json!({
                    "x": f32::from(n.position().0),
                    "y": f32::from(n.position().1),
                    "w": f32::from(n.size_ref().width),
                    "h": f32::from(n.size_ref().height),
                    "inputs": n.inputs().iter().map(ToString::to_string).collect::<Vec<_>>(),
                    "outputs": n.outputs().iter().map(ToString::to_string).collect::<Vec<_>>(),
                }),
            )
        })
        .collect();
    nodes.sort_by(|a, b| a.0.cmp(&b.0));

    let mut ports: Vec<_> = graph
        .ports()
        .iter()
        .map(|(id, p)| {
            (
                id.to_string(),
                json!({
                    "node_id": p.node_id().to_string(),
                    "kind": p.kind().to_string(),
                    "position": p.position().to_string(),
                    "index": p.index(),
                    "w": f32::from(p.size_ref().width),
                    "h": f32::from(p.size_ref().height),
                }),
            )
        })
        .collect();
    ports.sort_by(|a, b| a.0.cmp(&b.0));

    let mut edges: Vec<_> = graph
        .edges()
        .iter()
        .map(|(id, e)| {
            (
                id.to_string(),
                json!({
                    "source": e.source_port.to_string(),
                    "target": e.target_port.to_string(),
                }),
            )
        })
        .collect();
    edges.sort_by(|a, b| a.0.cmp(&b.0));

    let mut selected_node: Vec<_> = graph
        .selected_node()
        .iter()
        .map(ToString::to_string)
        .collect();
    selected_node.sort();
    let mut selected_edge: Vec<_> = graph
        .selected_edge()
        .iter()
        .map(ToString::to_string)
        .collect();
    selected_edge.sort();

    json!({
        "nodes": nodes,
        "ports": ports,
        "edges": edges,
        "node_order": graph.node_order().iter().map(ToString::to_string).collect::<Vec<_>>(),
        "selected_node": selected_node,
        "selected_edge": selected_edge
    })
}

/// Asserts that `execute` + `undo` restores `base`, and that replaying `to_ops` matches `execute`.
pub fn assert_command_interop(
    base: &Graph,
    mut make: impl FnMut() -> Box<dyn Command>,
    case_name: &str,
) {
    let expected_after_execute = {
        let mut g = base.clone();
        with_command_ctx(&mut g, |ctx| {
            let mut cmd = make();
            cmd.execute(ctx);
        });
        g
    };

    let execute_then_undo = {
        let mut g = base.clone();
        with_command_ctx(&mut g, |ctx| {
            let mut cmd = make();
            cmd.execute(ctx);
            cmd.undo(ctx);
        });
        g
    };
    assert_eq!(
        graph_snapshot(&execute_then_undo),
        graph_snapshot(base),
        "execute+undo must restore original graph for {case_name}"
    );

    let via_ops = {
        let mut g = base.clone();
        with_command_ctx(&mut g, |ctx| {
            let cmd = make();
            let ops = cmd.to_ops(ctx);
            for op in ops {
                apply_graph_op(ctx.graph, op);
            }
        });
        g
    };
    assert_eq!(
        graph_snapshot(&via_ops),
        graph_snapshot(&expected_after_execute),
        "to_ops replay must match execute result for {case_name}"
    );
}
