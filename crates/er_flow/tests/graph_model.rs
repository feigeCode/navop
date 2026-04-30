use er_flow::{Graph, PortPosition};
use gpui::{Point, px};
use serde_json::json;

#[test]
fn graph_builder_creates_nodes_ports_edges_and_round_trips_json() {
    let mut graph = Graph::new();

    let users = graph
        .create_node("er.table")
        .position(10.0, 20.0)
        .size(260.0, 180.0)
        .input_at(PortPosition::Left)
        .output_at(PortPosition::Right)
        .data(json!({
            "schema": "public",
            "table": "users",
            "columns": [
                { "name": "id", "ty": "bigint", "pk": true, "nullable": false },
                { "name": "org_id", "ty": "bigint", "fk": "organizations.id", "nullable": false }
            ]
        }))
        .build(&mut graph);

    let organizations = graph
        .create_node("er.table")
        .position(360.0, 20.0)
        .size(260.0, 140.0)
        .input_at(PortPosition::Left)
        .output_at(PortPosition::Right)
        .data(json!({
            "schema": "public",
            "table": "organizations",
            "columns": [
                { "name": "id", "ty": "bigint", "pk": true, "nullable": false }
            ]
        }))
        .build(&mut graph);

    let users_out = graph.get_node(&users).unwrap().outputs[0];
    let organizations_in = graph.get_node(&organizations).unwrap().inputs[0];
    let edge = graph
        .create_edge()
        .source(users_out)
        .target(organizations_in)
        .build(&mut graph);

    assert!(edge.is_some());
    assert_eq!(graph.nodes().len(), 2);
    assert_eq!(graph.ports.len(), 4);
    assert_eq!(graph.edges.len(), 1);

    let json = graph.to_json().unwrap();
    let restored = Graph::from_json(&json).unwrap();

    assert_eq!(restored.nodes().len(), 2);
    assert_eq!(restored.ports.len(), 4);
    assert_eq!(restored.edges.len(), 1);
}

#[test]
fn hit_node_uses_visual_stack_order() {
    let mut graph = Graph::new();

    let back = graph
        .create_node("table")
        .position(0.0, 0.0)
        .size(120.0, 120.0)
        .build(&mut graph);
    let front = graph
        .create_node("table")
        .position(40.0, 40.0)
        .size(120.0, 120.0)
        .build(&mut graph);

    assert_eq!(Some(front), graph.hit_node(Point::new(px(60.0), px(60.0))));

    graph.bring_node_to_front(back);

    assert_eq!(Some(back), graph.hit_node(Point::new(px(60.0), px(60.0))));
    assert_eq!(None, graph.hit_node(Point::new(px(200.0), px(200.0))));
}
