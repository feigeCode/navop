use er_flow::{
    ErDiagram, ErEntity, ErField, ErRelationship, ErRelationshipKind, er_flow_theme,
    er_node_renderers, graph_from_diagram,
};
use ferrum_flow::FlowCanvas;
use gpui::{AppContext as _, Application, WindowOptions};

fn main() {
    Application::new().run(|cx| {
        let graph =
            graph_from_diagram(&sample_diagram()).expect("sample ER diagram should be valid");

        cx.open_window(WindowOptions::default(), |window, cx| {
            cx.new(|ctx| {
                FlowCanvas::builder(graph, ctx, window)
                    .theme(er_flow_theme())
                    .default_plugins()
                    .node_renderers(er_node_renderers())
                    .build()
            })
        })
        .unwrap();
    });
}

fn sample_diagram() -> ErDiagram {
    ErDiagram {
        entities: vec![users_entity(), organizations_entity(), orders_entity()],
        relationships: vec![
            ErRelationship {
                id: "users_organizations".to_string(),
                from_entity: "users".to_string(),
                from_field: "org_id".to_string(),
                to_entity: "organizations".to_string(),
                to_field: "id".to_string(),
                kind: ErRelationshipKind::ManyToOne,
            },
            ErRelationship {
                id: "orders_users".to_string(),
                from_entity: "orders".to_string(),
                from_field: "user_id".to_string(),
                to_entity: "users".to_string(),
                to_field: "id".to_string(),
                kind: ErRelationshipKind::ManyToOne,
            },
        ],
    }
}

fn users_entity() -> ErEntity {
    ErEntity {
        id: "users".to_string(),
        name: "users".to_string(),
        comment: Some("application users".to_string()),
        fields: vec![
            field("id", "uuid", false, true, true),
            field("org_id", "uuid", false, false, false),
            field("email", "text", false, false, true),
            field("created_at", "timestamp", false, false, false),
        ],
    }
}

fn organizations_entity() -> ErEntity {
    ErEntity {
        id: "organizations".to_string(),
        name: "organizations".to_string(),
        comment: Some("customer accounts".to_string()),
        fields: vec![
            field("id", "uuid", false, true, true),
            field("name", "text", false, false, true),
        ],
    }
}

fn orders_entity() -> ErEntity {
    ErEntity {
        id: "orders".to_string(),
        name: "orders".to_string(),
        comment: Some("purchase orders".to_string()),
        fields: vec![
            field("id", "uuid", false, true, true),
            field("user_id", "uuid", false, false, false),
            field("total_cents", "integer", false, false, false),
            field("paid_at", "timestamp", true, false, false),
        ],
    }
}

fn field(name: &str, data_type: &str, nullable: bool, primary_key: bool, unique: bool) -> ErField {
    ErField {
        name: name.to_string(),
        data_type: data_type.to_string(),
        nullable,
        primary_key,
        unique,
        comment: None,
    }
}
