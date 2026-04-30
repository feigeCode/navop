use super::{ErColumnModel, ErForeignKeyModel, ErTableModel, build_er_graph, infer_relationships};

#[test]
fn build_er_graph_maps_tables_columns_and_foreign_keys() {
    let graph = build_er_graph(vec![users_table_with_fk(), organizations_table()]);

    assert_eq!(2, graph.nodes().len());
    assert_eq!(1, graph.edges.len());

    let users = graph
        .nodes()
        .values()
        .find(|node| node.data["table"] == "users")
        .expect("users node should exist");
    assert_eq!("er.table", users.node_type);
    assert_eq!("public", users.data["schema"]);
    assert_eq!("id", users.data["columns"][0]["name"]);
    assert_eq!(true, users.data["columns"][0]["pk"]);
    assert_eq!(true, users.data["columns"][1]["fk"]);
    assert_eq!(false, users.data["columns"][1]["nullable"]);
    let users_height: f32 = users.size.height.into();
    assert_eq!(126.0, users_height);
}

#[test]
fn infer_relationships_links_user_id_to_users_id() {
    let relationships = infer_relationships(&[users_table(), orders_table()]);

    assert_eq!(1, relationships.len());
    assert_eq!("public.orders", relationships[0].source_table);
    assert_eq!(vec!["user_id"], relationships[0].source_columns);
    assert_eq!("public.users", relationships[0].target_table);
    assert!(relationships[0].inferred);
}

#[test]
fn build_er_graph_places_dependents_to_the_right_of_targets() {
    let graph = build_er_graph(vec![users_table(), orders_table()]);

    let users = graph
        .nodes()
        .values()
        .find(|node| node.data["table"] == "users")
        .expect("users node should exist");
    let orders = graph
        .nodes()
        .values()
        .find(|node| node.data["table"] == "orders")
        .expect("orders node should exist");

    assert!(orders.x > users.x);
    assert_eq!(users.y, orders.y);
}

#[test]
fn build_er_graph_connects_target_right_side_to_source_left_side() {
    let graph = build_er_graph(vec![users_table(), orders_table()]);
    let users = graph
        .nodes()
        .values()
        .find(|node| node.data["table"] == "users")
        .expect("users node should exist");
    let orders = graph
        .nodes()
        .values()
        .find(|node| node.data["table"] == "orders")
        .expect("orders node should exist");
    let edge = graph.edges.values().next().expect("edge should exist");

    assert_eq!(Some(&edge.source_port), users.outputs.first());
    assert_eq!(Some(&edge.target_port), orders.inputs.first());
}

#[test]
fn build_er_graph_stacks_unrelated_tables_without_overlap() {
    let graph = build_er_graph(vec![users_table(), organizations_table()]);
    let mut nodes: Vec<_> = graph.nodes().values().collect();
    nodes.sort_by(|a, b| {
        let ay: f32 = a.y.into();
        let by: f32 = b.y.into();
        f32::total_cmp(&ay, &by)
    });

    let first_bottom: f32 = (nodes[0].y + nodes[0].size.height).into();
    let second_top: f32 = nodes[1].y.into();
    assert!(second_top > first_bottom);
}

fn users_table_with_fk() -> ErTableModel {
    let mut table = users_table();
    table.columns.push(ErColumnModel {
        name: "org_id".to_string(),
        data_type: "bigint".to_string(),
        is_primary_key: false,
        is_nullable: false,
    });
    table.foreign_keys.push(ErForeignKeyModel {
        name: "fk_users_org".to_string(),
        columns: vec!["org_id".to_string()],
        ref_table: "organizations".to_string(),
        ref_columns: vec!["id".to_string()],
    });
    table
}

fn users_table() -> ErTableModel {
    ErTableModel {
        name: "users".to_string(),
        schema: Some("public".to_string()),
        columns: vec![id_column()],
        foreign_keys: vec![],
    }
}

fn organizations_table() -> ErTableModel {
    ErTableModel {
        name: "organizations".to_string(),
        schema: Some("public".to_string()),
        columns: vec![id_column()],
        foreign_keys: vec![],
    }
}

fn orders_table() -> ErTableModel {
    ErTableModel {
        name: "orders".to_string(),
        schema: Some("public".to_string()),
        columns: vec![
            id_column(),
            ErColumnModel {
                name: "user_id".to_string(),
                data_type: "bigint".to_string(),
                is_primary_key: false,
                is_nullable: false,
            },
        ],
        foreign_keys: vec![],
    }
}

fn id_column() -> ErColumnModel {
    ErColumnModel {
        name: "id".to_string(),
        data_type: "bigint".to_string(),
        is_primary_key: true,
        is_nullable: false,
    }
}
