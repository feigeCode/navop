use std::collections::{HashMap, HashSet};

use db::{ColumnInfo, ForeignKeyDefinition, GlobalDbState, TableInfo};
use gpui::AsyncApp;

use er_flow::{ErDiagram, ErEntity, ErField, ErRelationship, ErRelationshipKind};

pub(crate) async fn load_er_diagram(
    global_state: GlobalDbState,
    cx: &mut AsyncApp,
    connection_id: String,
    database: String,
    schema: Option<String>,
) -> anyhow::Result<ErDiagram> {
    let tables = global_state
        .list_tables(cx, connection_id.clone(), database.clone(), schema.clone())
        .await?;
    let mut entities = Vec::with_capacity(tables.len());
    let mut relationships = Vec::new();

    for table in tables {
        let (entity, table_relationships) = load_entity(
            &global_state,
            cx,
            &connection_id,
            &database,
            schema.clone(),
            table,
        )
        .await?;
        entities.push(entity);
        relationships.extend(table_relationships);
    }

    relationships = valid_relationships(&entities, relationships);
    infer_relationships(&entities, &mut relationships);
    Ok(ErDiagram {
        entities,
        relationships,
    })
}

async fn load_entity(
    global_state: &GlobalDbState,
    cx: &mut AsyncApp,
    connection_id: &str,
    database: &str,
    selected_schema: Option<String>,
    table: TableInfo,
) -> anyhow::Result<(ErEntity, Vec<ErRelationship>)> {
    let schema = table
        .schema
        .clone()
        .filter(|schema| !schema.is_empty())
        .or(selected_schema.filter(|schema| !schema.is_empty()));
    let entity_id = entity_id(schema.as_deref(), &table.name);
    let columns = global_state
        .list_columns(
            cx,
            connection_id.to_string(),
            database.to_string(),
            schema.clone(),
            table.name.clone(),
        )
        .await?;
    let foreign_keys = global_state
        .list_foreign_keys(
            cx,
            connection_id.to_string(),
            database.to_string(),
            schema.clone(),
            table.name.clone(),
        )
        .await
        .unwrap_or_default();

    let relationships = foreign_keys
        .into_iter()
        .flat_map(|fk| relationships_from_fk(&entity_id, schema.as_deref(), fk))
        .collect();

    Ok((
        ErEntity {
            id: entity_id,
            name: table.name,
            comment: None,
            fields: columns.into_iter().map(field_model).collect(),
        },
        relationships,
    ))
}

fn field_model(column: ColumnInfo) -> ErField {
    ErField {
        name: column.name,
        data_type: column.data_type,
        nullable: column.is_nullable,
        primary_key: column.is_primary_key,
        unique: false,
        comment: None,
    }
}

fn relationships_from_fk(
    from_entity: &str,
    source_schema: Option<&str>,
    fk: ForeignKeyDefinition,
) -> Vec<ErRelationship> {
    let to_entity = normalize_ref_entity(source_schema, &fk.ref_table);
    fk.columns
        .into_iter()
        .zip(fk.ref_columns)
        .map(|(from_field, to_field)| ErRelationship {
            id: format!("{}:{}:{}", fk.name, from_entity, from_field),
            from_entity: from_entity.to_string(),
            from_field,
            to_entity: to_entity.clone(),
            to_field,
            kind: ErRelationshipKind::ManyToOne,
        })
        .collect()
}

fn infer_relationships(entities: &[ErEntity], relationships: &mut Vec<ErRelationship>) {
    let lookup = entity_lookup(entities);
    let mut existing = relationship_keys(relationships);

    for entity in entities {
        for field in &entity.fields {
            let Some(prefix) = infer_reference_prefix(&field.name) else {
                continue;
            };
            let Some(target) = find_target_entity(&prefix, entity, &lookup) else {
                continue;
            };
            let Some(to_field) = target_id_field(target) else {
                continue;
            };
            let relationship = ErRelationship {
                id: format!("inferred:{}:{}", entity.id, field.name),
                from_entity: entity.id.clone(),
                from_field: field.name.clone(),
                to_entity: target.id.clone(),
                to_field: to_field.to_string(),
                kind: ErRelationshipKind::ManyToOne,
            };
            if existing.insert(relationship_key(&relationship)) {
                relationships.push(relationship);
            }
        }
    }
}

fn find_target_entity<'a>(
    prefix: &str,
    source: &ErEntity,
    lookup: &'a HashMap<String, &'a ErEntity>,
) -> Option<&'a ErEntity> {
    candidate_names(prefix)
        .iter()
        .filter_map(|candidate| lookup.get(&entity_ref_key(source, candidate)).copied())
        .filter(|entity| entity.id != source.id)
        .find(|entity| target_id_field(entity).is_some())
}

fn valid_relationships(
    entities: &[ErEntity],
    relationships: Vec<ErRelationship>,
) -> Vec<ErRelationship> {
    let fields = entity_fields(entities);
    relationships
        .into_iter()
        .filter(|relationship| {
            fields
                .get(relationship.from_entity.as_str())
                .is_some_and(|fields| fields.contains(relationship.from_field.as_str()))
                && fields
                    .get(relationship.to_entity.as_str())
                    .is_some_and(|fields| fields.contains(relationship.to_field.as_str()))
        })
        .collect()
}

fn entity_fields(entities: &[ErEntity]) -> HashMap<&str, HashSet<&str>> {
    entities
        .iter()
        .map(|entity| {
            (
                entity.id.as_str(),
                entity
                    .fields
                    .iter()
                    .map(|field| field.name.as_str())
                    .collect::<HashSet<_>>(),
            )
        })
        .collect()
}

fn relationship_keys(
    relationships: &[ErRelationship],
) -> HashSet<(String, String, String, String)> {
    relationships.iter().map(relationship_key).collect()
}

fn relationship_key(relationship: &ErRelationship) -> (String, String, String, String) {
    (
        relationship.from_entity.to_lowercase(),
        relationship.from_field.to_lowercase(),
        relationship.to_entity.to_lowercase(),
        relationship.to_field.to_lowercase(),
    )
}

fn target_id_field(entity: &ErEntity) -> Option<&str> {
    entity
        .fields
        .iter()
        .find(|field| field.name.eq_ignore_ascii_case("id"))
        .or_else(|| entity.fields.iter().find(|field| field.primary_key))
        .map(|field| field.name.as_str())
}

fn entity_lookup(entities: &[ErEntity]) -> HashMap<String, &ErEntity> {
    let mut lookup = HashMap::new();
    for entity in entities {
        lookup.insert(entity.name.to_lowercase(), entity);
        lookup.insert(entity.id.to_lowercase(), entity);
    }
    lookup
}

fn entity_ref_key(source: &ErEntity, ref_name: &str) -> String {
    if ref_name.contains('.') {
        return ref_name.to_lowercase();
    }
    match source.id.rsplit_once('.') {
        Some((schema, _)) => format!("{schema}.{ref_name}").to_lowercase(),
        None => ref_name.to_lowercase(),
    }
}

fn candidate_names(prefix: &str) -> Vec<String> {
    let mut candidates = vec![prefix.to_string(), format!("{prefix}s")];
    if let Some(stem) = prefix.strip_suffix('y') {
        candidates.push(format!("{stem}ies"));
    }
    candidates
}

fn infer_reference_prefix(column_name: &str) -> Option<String> {
    column_name
        .to_lowercase()
        .strip_suffix("_id")
        .filter(|prefix| !prefix.is_empty())
        .map(str::to_string)
}

fn normalize_ref_entity(source_schema: Option<&str>, ref_table: &str) -> String {
    if ref_table.contains('.') {
        return ref_table.to_string();
    }
    match source_schema {
        Some(schema) if !schema.is_empty() => format!("{schema}.{ref_table}"),
        _ => ref_table.to_string(),
    }
}

fn entity_id(schema: Option<&str>, table: &str) -> String {
    match schema {
        Some(schema) if !schema.is_empty() => format!("{schema}.{table}"),
        _ => table.to_string(),
    }
}
