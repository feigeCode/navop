use std::collections::{HashMap, HashSet};

use anyhow::{Result, bail};
use ferrum_flow::{Graph, PortId};
use serde_json::json;

use crate::{ER_ENTITY_RENDERER_KEY, ErDataSource, ErDiagram, ErRelationship};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ErGraphOptions {
    pub columns: usize,
    pub node_width: f32,
    pub min_node_height: f32,
    pub row_gap: f32,
    pub column_gap: f32,
    pub origin_x: f32,
    pub origin_y: f32,
    pub field_row_height: f32,
}

impl Default for ErGraphOptions {
    fn default() -> Self {
        Self {
            columns: 3,
            node_width: 260.0,
            min_node_height: 140.0,
            row_gap: 80.0,
            column_gap: 80.0,
            origin_x: 80.0,
            origin_y: 80.0,
            field_row_height: 24.0,
        }
    }
}

pub fn graph_from_source<S: ErDataSource>(source: &S) -> Result<Graph> {
    let diagram = source.load_er_diagram()?;
    graph_from_diagram(&diagram)
}

pub fn graph_from_diagram(diagram: &ErDiagram) -> Result<Graph> {
    graph_from_diagram_with_options(diagram, ErGraphOptions::default())
}

pub fn graph_from_diagram_with_options(
    diagram: &ErDiagram,
    options: ErGraphOptions,
) -> Result<Graph> {
    validate_diagram(diagram)?;
    validate_options(options)?;

    let mut graph = Graph::new();
    let mut input_ports = HashMap::<String, PortId>::new();
    let mut output_ports = HashMap::<String, PortId>::new();
    let foreign_key_fields = foreign_key_fields(&diagram.relationships);
    let columns = options.columns.max(1);
    let mut row_heights = Vec::<f32>::new();

    for (index, entity) in diagram.entities.iter().enumerate() {
        let row = index / columns;
        let node_height = node_height(entity.fields.len(), options);
        if row_heights.len() <= row {
            row_heights.push(node_height);
        } else {
            row_heights[row] = row_heights[row].max(node_height);
        }
    }

    let row_offsets = row_offsets(&row_heights, options.row_gap);

    for (index, entity) in diagram.entities.iter().enumerate() {
        let row = index / columns;
        let column = index % columns;
        let x = options.origin_x + column as f32 * (options.node_width + options.column_gap);
        let y = options.origin_y + row_offsets[row];
        let node_height = node_height(entity.fields.len(), options);
        let fields = entity.fields.iter().map(|field| {
            json!({
                "name": field.name,
                "data_type": field.data_type,
                "nullable": field.nullable,
                "primary_key": field.primary_key,
                "unique": field.unique,
                "foreign_key": is_foreign_key(&foreign_key_fields, &entity.id, &field.name),
                "comment": field.comment,
            })
        });

        let node_id = graph
            .create_node(ER_ENTITY_RENDERER_KEY)
            .position(x, y)
            .size(options.node_width, node_height)
            .input()
            .output()
            .data(json!({
                "kind": "entity",
                "id": entity.id,
                "name": entity.name,
                "comment": entity.comment,
                "fields": fields.collect::<Vec<_>>(),
            }))
            .build()
            .ok_or_else(|| anyhow::anyhow!("failed to create node for entity `{}`", entity.id))?;

        let node = graph
            .get_node(&node_id)
            .ok_or_else(|| anyhow::anyhow!("created node for `{}` was not found", entity.id))?;
        let input =
            node.inputs().first().copied().ok_or_else(|| {
                anyhow::anyhow!("created node for `{}` has no input port", entity.id)
            })?;
        let output = node.outputs().first().copied().ok_or_else(|| {
            anyhow::anyhow!("created node for `{}` has no output port", entity.id)
        })?;

        input_ports.insert(entity.id.clone(), input);
        output_ports.insert(entity.id.clone(), output);
    }

    for relationship in &diagram.relationships {
        let source = output_ports[&relationship.from_entity];
        let target = input_ports[&relationship.to_entity];

        graph
            .create_edge()
            .source(source)
            .target(target)
            .build()
            .ok_or_else(|| {
                anyhow::anyhow!("failed to create relationship `{}`", relationship.id)
            })?;
    }

    Ok(graph)
}

fn validate_options(options: ErGraphOptions) -> Result<()> {
    if options.columns == 0 {
        bail!("ER graph layout columns must be greater than zero");
    }
    for (name, value) in [
        ("node_width", options.node_width),
        ("min_node_height", options.min_node_height),
        ("row_gap", options.row_gap),
        ("column_gap", options.column_gap),
        ("origin_x", options.origin_x),
        ("origin_y", options.origin_y),
        ("field_row_height", options.field_row_height),
    ] {
        if !value.is_finite() {
            bail!("ER graph layout option `{name}` must be finite");
        }
    }
    for (name, value) in [
        ("node_width", options.node_width),
        ("min_node_height", options.min_node_height),
        ("field_row_height", options.field_row_height),
    ] {
        if value <= 0.0 {
            bail!("ER graph layout option `{name}` must be greater than zero");
        }
    }
    Ok(())
}

fn validate_diagram(diagram: &ErDiagram) -> Result<()> {
    let mut entity_ids = HashSet::new();
    let mut fields_by_entity = HashMap::<&str, HashSet<&str>>::new();

    for entity in &diagram.entities {
        if entity.id.trim().is_empty() {
            bail!("ER entity id cannot be empty");
        }
        if !entity_ids.insert(entity.id.as_str()) {
            bail!("duplicate ER entity id `{}`", entity.id);
        }

        let mut field_names = HashSet::new();
        for field in &entity.fields {
            if field.name.trim().is_empty() {
                bail!("ER field name cannot be empty in entity `{}`", entity.id);
            }
            if !field_names.insert(field.name.as_str()) {
                bail!(
                    "duplicate ER field `{}` in entity `{}`",
                    field.name,
                    entity.id
                );
            }
        }
        fields_by_entity.insert(entity.id.as_str(), field_names);
    }

    let mut relationship_ids = HashSet::new();
    for relationship in &diagram.relationships {
        if relationship.id.trim().is_empty() {
            bail!("ER relationship id cannot be empty");
        }
        if !relationship_ids.insert(relationship.id.as_str()) {
            bail!("duplicate ER relationship id `{}`", relationship.id);
        }
        validate_relationship_endpoint(
            &fields_by_entity,
            &relationship.from_entity,
            &relationship.from_field,
            "source",
            relationship,
        )?;
        validate_relationship_endpoint(
            &fields_by_entity,
            &relationship.to_entity,
            &relationship.to_field,
            "target",
            relationship,
        )?;
    }

    Ok(())
}

fn validate_relationship_endpoint(
    fields_by_entity: &HashMap<&str, HashSet<&str>>,
    entity_id: &str,
    field_name: &str,
    endpoint: &str,
    relationship: &ErRelationship,
) -> Result<()> {
    let Some(fields) = fields_by_entity.get(entity_id) else {
        bail!(
            "ER relationship `{}` references unknown {endpoint} entity `{entity_id}`",
            relationship.id
        );
    };

    if !fields.contains(field_name) {
        bail!(
            "ER relationship `{}` references unknown {endpoint} field `{entity_id}.{field_name}`",
            relationship.id
        );
    }

    Ok(())
}

fn foreign_key_fields(relationships: &[ErRelationship]) -> HashSet<(&str, &str)> {
    relationships
        .iter()
        .map(|relationship| {
            (
                relationship.from_entity.as_str(),
                relationship.from_field.as_str(),
            )
        })
        .collect()
}

fn is_foreign_key(
    foreign_key_fields: &HashSet<(&str, &str)>,
    entity_id: &str,
    field_name: &str,
) -> bool {
    foreign_key_fields.contains(&(entity_id, field_name))
}

fn node_height(field_count: usize, options: ErGraphOptions) -> f32 {
    let field_height = field_count as f32 * options.field_row_height;
    options.min_node_height.max(72.0 + field_height)
}

fn row_offsets(row_heights: &[f32], row_gap: f32) -> Vec<f32> {
    let mut y = 0.0;
    row_heights
        .iter()
        .map(|height| {
            let current = y;
            y += height + row_gap;
            current
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ErEntity, ErField, ErRelationshipKind};

    fn field(name: &str, primary_key: bool) -> ErField {
        ErField {
            name: name.to_string(),
            data_type: "uuid".to_string(),
            nullable: false,
            primary_key,
            unique: primary_key,
            comment: None,
        }
    }

    fn users_orgs_diagram() -> ErDiagram {
        ErDiagram {
            entities: vec![
                ErEntity {
                    id: "users".to_string(),
                    name: "users".to_string(),
                    comment: None,
                    fields: vec![field("id", true), field("org_id", false)],
                },
                ErEntity {
                    id: "orgs".to_string(),
                    name: "organizations".to_string(),
                    comment: Some("customer organizations".to_string()),
                    fields: vec![field("id", true)],
                },
            ],
            relationships: vec![ErRelationship {
                id: "users_orgs".to_string(),
                from_entity: "users".to_string(),
                from_field: "org_id".to_string(),
                to_entity: "orgs".to_string(),
                to_field: "id".to_string(),
                kind: ErRelationshipKind::ManyToOne,
            }],
        }
    }

    #[test]
    fn converts_empty_diagram_to_empty_graph() {
        let graph = graph_from_diagram(&ErDiagram::default()).unwrap();

        assert!(graph.is_empty());
    }

    #[test]
    fn converts_entities_to_er_nodes() {
        let graph = graph_from_diagram(&users_orgs_diagram()).unwrap();

        assert_eq!(graph.nodes().len(), 2);
        assert!(
            graph
                .nodes()
                .values()
                .all(|node| node.renderer_key() == ER_ENTITY_RENDERER_KEY)
        );
    }

    #[test]
    fn stores_fields_in_node_data() {
        let graph = graph_from_diagram(&users_orgs_diagram()).unwrap();
        let users = graph
            .nodes()
            .values()
            .find(|node| node.data_ref()["id"] == "users")
            .unwrap();
        let fields = users.data_ref()["fields"].as_array().unwrap();

        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0]["name"], "id");
        assert_eq!(fields[1]["name"], "org_id");
        assert_eq!(fields[1]["foreign_key"], true);
    }

    #[test]
    fn converts_relationships_to_edges() {
        let graph = graph_from_diagram(&users_orgs_diagram()).unwrap();

        assert_eq!(graph.edges().len(), 1);
    }

    #[test]
    fn rejects_relationship_with_unknown_entity() {
        let mut diagram = users_orgs_diagram();
        diagram.relationships[0].to_entity = "missing".to_string();

        let error = graph_from_diagram(&diagram).unwrap_err();

        assert!(error.to_string().contains("unknown target entity"));
    }

    #[test]
    fn rejects_relationship_with_unknown_field() {
        let mut diagram = users_orgs_diagram();
        diagram.relationships[0].from_field = "missing".to_string();

        let error = graph_from_diagram(&diagram).unwrap_err();

        assert!(error.to_string().contains("unknown source field"));
    }

    #[test]
    fn rejects_duplicate_entity_ids() {
        let mut diagram = users_orgs_diagram();
        diagram.entities[1].id = "users".to_string();

        let error = graph_from_diagram(&diagram).unwrap_err();

        assert!(error.to_string().contains("duplicate ER entity id"));
    }

    #[test]
    fn rejects_invalid_layout_options() {
        let options = ErGraphOptions {
            node_width: f32::NAN,
            ..ErGraphOptions::default()
        };

        let error = graph_from_diagram_with_options(&users_orgs_diagram(), options).unwrap_err();

        assert!(error.to_string().contains("must be finite"));
    }
}
