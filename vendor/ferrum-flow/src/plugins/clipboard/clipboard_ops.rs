use std::collections::{HashMap, HashSet};

use gpui::{Pixels, Point, px};

use crate::{CompositeCommand, Edge, Graph, Node, NodeId, plugin::PluginContext};

use super::copied_subgraph::CopiedSubgraph;
use crate::plugins::{AttachChildCommand, CreateEdge, CreateNode, CreatePort};

#[derive(Clone)]
pub(crate) struct ClipboardShared(pub CopiedSubgraph);

pub(crate) fn set_clipboard_subgraph(ctx: &mut PluginContext, sub: CopiedSubgraph) {
    ctx.shared_state.insert(ClipboardShared(sub));
}

pub(crate) fn get_clipboard_subgraph(ctx: &PluginContext) -> Option<CopiedSubgraph> {
    ctx.shared_state
        .get::<ClipboardShared>()
        .map(|s| s.0.clone())
}

pub(crate) fn has_clipboard_subgraph(ctx: &PluginContext) -> bool {
    ctx.shared_state.contains::<ClipboardShared>()
}

/// Selected nodes plus every descendant (nested copy includes the whole subtree).
fn copy_node_closure(graph: &Graph) -> HashSet<NodeId> {
    let mut closure = HashSet::new();
    for &id in graph.selected_node() {
        closure.insert(id);
        closure.extend(graph.descendants(id));
    }
    closure
}

pub(crate) fn extract_subgraph(graph: &Graph) -> Option<CopiedSubgraph> {
    if graph.selected_node_is_empty() {
        return None;
    }
    let closure = copy_node_closure(graph);
    if closure.is_empty() {
        return None;
    }

    let mut port_ids = HashSet::new();
    let mut ports = Vec::new();
    let mut nodes = Vec::with_capacity(closure.len());
    for id in graph.paint_order_iter().filter(|id| closure.contains(id)) {
        let n = graph.get_node(&id)?;
        for pid in n.inputs().iter().chain(n.outputs().iter()) {
            port_ids.insert(*pid);
            if let Some(p) = graph.get_port(pid) {
                ports.push(p.clone());
            }
        }
        nodes.push(n.clone());
    }

    let edges = graph
        .edges_values()
        .filter(|e| port_ids.contains(&e.source_port) && port_ids.contains(&e.target_port))
        .cloned()
        .collect();

    Some(CopiedSubgraph {
        nodes,
        ports,
        edges,
    })
}

fn copied_node_ids(sub: &CopiedSubgraph) -> HashSet<NodeId> {
    sub.nodes.iter().map(|n| n.id()).collect()
}

/// True when `node` has no parent in the copied set (paste anchor applies in world space).
fn is_copy_subgraph_root(node: &Node, in_copy: &HashSet<NodeId>) -> bool {
    node.parent().is_none_or(|p| !in_copy.contains(&p))
}

/// World origins for nodes in a copied subgraph, using the same parent-chain rule as [`Graph::node_world_point`].
fn world_positions_in_sub(sub: &CopiedSubgraph) -> HashMap<NodeId, Point<Pixels>> {
    let in_copy = copied_node_ids(sub);
    let by_id: HashMap<_, _> = sub.nodes.iter().map(|n| (n.id(), n)).collect();
    let mut world = HashMap::with_capacity(sub.nodes.len());

    fn world_of(
        id: NodeId,
        by_id: &HashMap<NodeId, &Node>,
        in_copy: &HashSet<NodeId>,
        world: &mut HashMap<NodeId, Point<Pixels>>,
    ) -> Point<Pixels> {
        if let Some(&p) = world.get(&id) {
            return p;
        }
        let node = by_id[&id];
        let mut origin = node.point();
        if let Some(parent) = node.parent().filter(|p| in_copy.contains(p)) {
            let parent_world = world_of(parent, by_id, in_copy, world);
            origin.x += parent_world.x;
            origin.y += parent_world.y;
        }
        world.insert(id, origin);
        origin
    }

    for &id in in_copy.iter() {
        world_of(id, &by_id, &in_copy, &mut world);
    }
    world
}

/// Top-left of the axis-aligned bounding box of copied node positions (world space).
fn subgraph_bounds_top_left(sub: &CopiedSubgraph) -> Point<Pixels> {
    let worlds = world_positions_in_sub(sub);
    let (mut min_x, mut min_y) = (f32::INFINITY, f32::INFINITY);
    for p in worlds.values() {
        min_x = min_x.min(p.x.into());
        min_y = min_y.min(p.y.into());
    }
    Point::new(px(min_x), px(min_y))
}

/// Paste with the subgraph's bounding-box top-left placed at `anchor_world`.
pub(crate) fn paste_subgraph_at_world(
    ctx: &mut PluginContext,
    sub: &CopiedSubgraph,
    anchor_world: Point<Pixels>,
) {
    paste_subgraph_with_anchor(ctx, sub, anchor_world);
}

/// Paste offset from the copied layout (keyboard paste): bbox top-left moves by (40, 40) in world space.
pub(crate) fn paste_subgraph(ctx: &mut PluginContext, sub: &CopiedSubgraph) {
    const NUDGE: f32 = 40.0;
    let origin = subgraph_bounds_top_left(sub);
    let anchor = Point::new(origin.x + px(NUDGE), origin.y + px(NUDGE));
    paste_subgraph_with_anchor(ctx, sub, anchor);
}

fn paste_subgraph_with_anchor(
    ctx: &mut PluginContext,
    sub: &CopiedSubgraph,
    anchor_world: Point<Pixels>,
) {
    if sub.nodes.is_empty() {
        return;
    }

    let in_copy = copied_node_ids(sub);
    let origin = subgraph_bounds_top_left(sub);
    let worlds = world_positions_in_sub(sub);
    let ox: f32 = origin.x.into();
    let oy: f32 = origin.y.into();
    let ax: f32 = anchor_world.x.into();
    let ay: f32 = anchor_world.y.into();

    let mut node_map = HashMap::new();
    for n in &sub.nodes {
        node_map.insert(n.id(), ctx.graph.next_node_id());
    }
    let mut port_map = HashMap::new();
    for p in &sub.ports {
        port_map.insert(p.id(), ctx.graph.next_port_id());
    }

    let mut composite = CompositeCommand::new();

    let mut new_node_ids = Vec::new();

    for old in &sub.nodes {
        let new_id = node_map[&old.id()];
        let (nx, ny) = if is_copy_subgraph_root(old, &in_copy) {
            let world = worlds[&old.id()];
            let wx: f32 = world.x.into();
            let wy: f32 = world.y.into();
            (ax + wx - ox, ay + wy - oy)
        } else {
            let (x, y) = old.position();
            (f32::from(x), f32::from(y))
        };
        // Clone-then-patch avoids missing newly added fields on Node.
        let mut node = old.clone();
        node.set_id(new_id);
        node.set_position(nx.into(), ny.into());
        node.clear_port_refs();

        for pid in old.inputs() {
            node.push_input(port_map[pid]);
        }
        for pid in old.outputs() {
            node.push_output(port_map[pid]);
        }
        new_node_ids.push(new_id);
        composite.push(CreateNode::new(node));
    }

    for old in &sub.nodes {
        if let Some(old_parent) = old.parent().filter(|p| in_copy.contains(p)) {
            let parent = node_map[&old_parent];
            let child = node_map[&old.id()];
            composite.push(AttachChildCommand::link(parent, child));
        }
    }

    for old in &sub.ports {
        // Clone-then-patch avoids missing newly added fields on Port.
        let mut port = old.clone();
        port.set_id(port_map[&old.id()]);
        port.set_node_id(node_map[&old.node_id()]);
        composite.push(CreatePort::new(port));
    }

    for old in &sub.edges {
        let edge = Edge {
            id: ctx.graph.next_edge_id(),
            source_port: port_map[&old.source_port],
            target_port: port_map[&old.target_port],
        };
        composite.push(CreateEdge::new(edge));
    }

    let pasted_ids = sub.nodes.iter().map(|n| node_map[&n.id()]);

    ctx.execute_command(composite);
    ctx.clear_selected_edge();
    ctx.clear_selected_node();
    for nid in pasted_ids {
        ctx.add_selected_node(nid, true);
    }
    ctx.cache_port_offset_with_node(&new_node_ids);
}

#[cfg(test)]
mod tests {
    use gpui::{Point, px};
    use serde_json::{Value, json};

    use crate::plugin_testing::PluginTestHarness;

    use super::{extract_subgraph, paste_subgraph_at_world};

    fn node_for_compare(mut value: Value) -> Value {
        let obj = value
            .as_object_mut()
            .expect("serialized node should be JSON object");
        obj.remove("id");
        obj.remove("inputs");
        obj.remove("outputs");
        value
    }

    fn port_for_compare(mut value: Value) -> Value {
        let obj = value
            .as_object_mut()
            .expect("serialized port should be JSON object");
        obj.remove("id");
        obj.remove("node_id");
        value
    }

    #[test]
    fn paste_keeps_node_fields_except_ids_and_port_refs() {
        let mut harness = PluginTestHarness::default();

        let n1 = harness
            .graph
            .create_node("default")
            .execute_type("exec-a")
            .position(120.0, 80.0)
            .size(210.0, 90.0)
            .input()
            .output()
            .data(json!({"label":"A","tag":"node-a","nested":{"k":1}}))
            .build();
        let n2 = harness
            .graph
            .create_node("default")
            .execute_type("exec-b")
            .position(360.0, 220.0)
            .size(260.0, 110.0)
            .input()
            .output()
            .data(json!({"label":"B","tag":"node-b","nested":{"k":2}}))
            .build();

        harness.graph.add_selected_node(n1, false);
        harness.graph.add_selected_node(n2, true);

        let copied = extract_subgraph(&harness.graph).expect("copy should extract selected nodes");
        let old_a = harness.graph.get_node(&n1).expect("original node-a exists");
        let old_b = harness.graph.get_node(&n2).expect("original node-b exists");
        let (x1, y1) = old_a.position();
        let (x2, y2) = old_b.position();
        let min_x = f32::from(x1).min(f32::from(x2));
        let min_y = f32::from(y1).min(f32::from(y2));
        let anchor = Point::new(px(min_x), px(min_y));

        harness.with_plugin_context(|ctx| {
            paste_subgraph_at_world(ctx, &copied, anchor);
        });

        let pasted_ids: Vec<_> = harness.graph.selected_node().iter().copied().collect();
        assert_eq!(
            pasted_ids.len(),
            2,
            "paste should select exactly two new nodes"
        );

        let mut pasted_a = None;
        let mut pasted_b = None;
        for id in pasted_ids {
            let node = harness
                .graph
                .get_node(&id)
                .expect("pasted node should exist");
            match node.data_ref().get("tag").and_then(Value::as_str) {
                Some("node-a") => pasted_a = Some(node.clone()),
                Some("node-b") => pasted_b = Some(node.clone()),
                _ => {}
            }
        }

        let pasted_a = pasted_a.expect("pasted node-a should exist");
        let pasted_b = pasted_b.expect("pasted node-b should exist");
        let old_a = harness.graph.get_node(&n1).expect("original node-a exists");
        let old_b = harness.graph.get_node(&n2).expect("original node-b exists");

        assert_eq!(old_a.inputs().len(), pasted_a.inputs().len());
        assert_eq!(old_a.outputs().len(), pasted_a.outputs().len());
        assert_eq!(old_b.inputs().len(), pasted_b.inputs().len());
        assert_eq!(old_b.outputs().len(), pasted_b.outputs().len());

        for (old_pid, pasted_pid) in old_a.inputs().iter().zip(pasted_a.inputs().iter()) {
            let old_port = harness
                .graph
                .get_port(old_pid)
                .expect("old node-a input port should exist");
            let pasted_port = harness
                .graph
                .get_port(pasted_pid)
                .expect("pasted node-a input port should exist");
            assert_eq!(
                port_for_compare(
                    serde_json::to_value(old_port).expect("serialize old node-a input")
                ),
                port_for_compare(
                    serde_json::to_value(pasted_port).expect("serialize pasted node-a input")
                ),
                "node-a input port fields should remain consistent after copy/paste"
            );
        }
        for (old_pid, pasted_pid) in old_a.outputs().iter().zip(pasted_a.outputs().iter()) {
            let old_port = harness
                .graph
                .get_port(old_pid)
                .expect("old node-a output port should exist");
            let pasted_port = harness
                .graph
                .get_port(pasted_pid)
                .expect("pasted node-a output port should exist");
            assert_eq!(
                port_for_compare(
                    serde_json::to_value(old_port).expect("serialize old node-a output")
                ),
                port_for_compare(
                    serde_json::to_value(pasted_port).expect("serialize pasted node-a output")
                ),
                "node-a output port fields should remain consistent after copy/paste"
            );
        }
        for (old_pid, pasted_pid) in old_b.inputs().iter().zip(pasted_b.inputs().iter()) {
            let old_port = harness
                .graph
                .get_port(old_pid)
                .expect("old node-b input port should exist");
            let pasted_port = harness
                .graph
                .get_port(pasted_pid)
                .expect("pasted node-b input port should exist");
            assert_eq!(
                port_for_compare(
                    serde_json::to_value(old_port).expect("serialize old node-b input")
                ),
                port_for_compare(
                    serde_json::to_value(pasted_port).expect("serialize pasted node-b input")
                ),
                "node-b input port fields should remain consistent after copy/paste"
            );
        }
        for (old_pid, pasted_pid) in old_b.outputs().iter().zip(pasted_b.outputs().iter()) {
            let old_port = harness
                .graph
                .get_port(old_pid)
                .expect("old node-b output port should exist");
            let pasted_port = harness
                .graph
                .get_port(pasted_pid)
                .expect("pasted node-b output port should exist");
            assert_eq!(
                port_for_compare(
                    serde_json::to_value(old_port).expect("serialize old node-b output")
                ),
                port_for_compare(
                    serde_json::to_value(pasted_port).expect("serialize pasted node-b output")
                ),
                "node-b output port fields should remain consistent after copy/paste"
            );
        }

        assert_eq!(
            node_for_compare(serde_json::to_value(old_a).expect("serialize old node-a")),
            node_for_compare(serde_json::to_value(&pasted_a).expect("serialize pasted node-a")),
            "node-a fields should remain consistent after copy/paste"
        );
        assert_eq!(
            node_for_compare(serde_json::to_value(old_b).expect("serialize old node-b")),
            node_for_compare(serde_json::to_value(&pasted_b).expect("serialize pasted node-b")),
            "node-b fields should remain consistent after copy/paste"
        );
    }

    #[test]
    fn copy_parent_includes_children_and_paste_restores_hierarchy() {
        let mut harness = PluginTestHarness::default();

        let parent = harness
            .graph
            .create_node("default")
            .position(100.0, 100.0)
            .data(json!({"role":"parent"}))
            .build();
        let child = harness
            .graph
            .create_node("default")
            .position(10.0, 10.0)
            .data(json!({"role":"child"}))
            .build();
        harness.graph.add_child(parent, child).unwrap();

        harness.graph.add_selected_node(parent, false);

        let copied = extract_subgraph(&harness.graph).expect("copy should include subtree");
        assert_eq!(
            copied.nodes.len(),
            2,
            "copy should include parent and descendant"
        );

        let world_child_before = harness
            .graph
            .node_world_point(child)
            .expect("child world position");
        let anchor = Point::new(px(300.0), px(300.0));

        harness.with_plugin_context(|ctx| {
            paste_subgraph_at_world(ctx, &copied, anchor);
        });

        let pasted_parent = harness
            .graph
            .selected_node()
            .iter()
            .find_map(|id| {
                let node = harness.graph.get_node(id)?;
                (node.data_ref().get("role")? == "parent").then_some(*id)
            })
            .expect("pasted parent should be selected");
        let pasted_child = harness
            .graph
            .descendants(pasted_parent)
            .next()
            .expect("pasted parent should have the copied child");
        assert_eq!(
            harness.graph.get_node(&pasted_child).unwrap().parent(),
            Some(pasted_parent),
            "paste should restore parent/child link"
        );
        assert_eq!(
            harness.graph.get_node(&pasted_child).unwrap().point(),
            harness.graph.get_node(&child).unwrap().point(),
            "child local offset should be preserved under the new parent"
        );
        assert_eq!(
            harness.graph.node_world_point(pasted_child),
            Some(Point::new(
                world_child_before.x + px(200.0),
                world_child_before.y + px(200.0)
            )),
            "paste should move the subtree by the anchor offset in world space"
        );
    }
}
