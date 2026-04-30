use std::collections::{BTreeMap, HashMap, HashSet};

use er_flow::{Graph, PortPosition};
use serde_json::json;

const NODE_TYPE_TABLE: &str = "er.table";
const TABLE_WIDTH: f32 = 280.0;
const TABLE_HEADER_HEIGHT: f32 = 58.0;
const COLUMN_ROW_HEIGHT: f32 = 34.0;
const TABLE_MIN_HEIGHT: f32 = 96.0;
const GRID_X_START: f32 = 40.0;
const GRID_Y_START: f32 = 40.0;
const LAYER_X_GAP: f32 = 400.0;
const LAYER_Y_GAP: f32 = 56.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ErColumnModel {
    pub name: String,
    pub data_type: String,
    pub is_primary_key: bool,
    pub is_nullable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ErForeignKeyModel {
    pub name: String,
    pub columns: Vec<String>,
    pub ref_table: String,
    pub ref_columns: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ErTableModel {
    pub name: String,
    pub schema: Option<String>,
    pub columns: Vec<ErColumnModel>,
    pub foreign_keys: Vec<ErForeignKeyModel>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ErRelationship {
    pub source_table: String,
    pub source_columns: Vec<String>,
    pub target_table: String,
    pub target_columns: Vec<String>,
    pub inferred: bool,
}

pub(crate) fn build_er_graph(tables: Vec<ErTableModel>) -> Graph {
    let mut graph = Graph::new();
    let relationships = collect_relationships(&tables);
    let relationship_columns = relationship_column_lookup(&relationships);
    let positions = relationship_layout(&tables, &relationships);
    let mut node_ports = HashMap::new();

    for table in &tables {
        let (x, y) = positions
            .get(&table_key(table))
            .copied()
            .unwrap_or((GRID_X_START, GRID_Y_START));
        let height = table_height(table.columns.len());
        let node_id = graph
            .create_node(NODE_TYPE_TABLE)
            .position(x, y)
            .size(TABLE_WIDTH, height)
            .input_at(PortPosition::Left)
            .output_at(PortPosition::Right)
            .data(table_data(table, &relationship_columns))
            .build(&mut graph);
        if let Some(node) = graph.get_node(&node_id) {
            node_ports.insert(table_key(table), (node.inputs[0], node.outputs[0]));
        }
    }

    for relationship in relationships {
        let Some((source_in, _)) = node_ports.get(&relationship.source_table) else {
            continue;
        };
        let Some((_, target_out)) = node_ports.get(&relationship.target_table) else {
            continue;
        };
        graph
            .create_edge()
            .source(*target_out)
            .target(*source_in)
            .build(&mut graph);
    }

    graph
}

pub(crate) fn infer_relationships(tables: &[ErTableModel]) -> Vec<ErRelationship> {
    let table_keys = table_lookup(tables);
    let mut relationships = Vec::new();

    for table in tables {
        for column in &table.columns {
            if let Some(prefix) = infer_reference_prefix(&column.name) {
                if let Some(target) = find_target_table(&prefix, table, tables, &table_keys) {
                    relationships.push(ErRelationship {
                        source_table: table_key(table),
                        source_columns: vec![column.name.clone()],
                        target_table: table_key(target),
                        target_columns: vec!["id".to_string()],
                        inferred: true,
                    });
                }
            }
        }
    }

    relationships
}

fn collect_relationships(tables: &[ErTableModel]) -> Vec<ErRelationship> {
    let explicit = explicit_relationships(tables);
    let explicit_keys: HashSet<_> = explicit.iter().map(relationship_key).collect();
    let inferred = infer_relationships(tables)
        .into_iter()
        .filter(|relationship| !explicit_keys.contains(&relationship_key(relationship)));
    explicit.into_iter().chain(inferred).collect()
}

fn explicit_relationships(tables: &[ErTableModel]) -> Vec<ErRelationship> {
    let table_keys = table_lookup(tables);
    tables
        .iter()
        .flat_map(|table| explicit_for_table(table, &table_keys))
        .collect()
}

fn explicit_for_table(
    table: &ErTableModel,
    table_keys: &HashMap<String, String>,
) -> Vec<ErRelationship> {
    table
        .foreign_keys
        .iter()
        .filter_map(|fk| {
            let target = normalize_ref_table(&fk.ref_table, table, table_keys)?;
            Some(ErRelationship {
                source_table: table_key(table),
                source_columns: fk.columns.clone(),
                target_table: target,
                target_columns: fk.ref_columns.clone(),
                inferred: false,
            })
        })
        .collect()
}

fn find_target_table<'a>(
    prefix: &str,
    source: &ErTableModel,
    tables: &'a [ErTableModel],
    table_keys: &HashMap<String, String>,
) -> Option<&'a ErTableModel> {
    candidate_names(prefix)
        .iter()
        .filter_map(|candidate| normalize_ref_table(candidate, source, table_keys))
        .filter(|target_key| target_key != &table_key(source))
        .find_map(|target_key| tables.iter().find(|table| table_key(table) == target_key))
        .filter(|table| has_id_target_column(table))
}

fn candidate_names(prefix: &str) -> Vec<String> {
    let mut candidates = vec![prefix.to_string(), format!("{prefix}s")];
    if let Some(stem) = prefix.strip_suffix('y') {
        candidates.push(format!("{stem}ies"));
    }
    candidates
}

fn infer_reference_prefix(column_name: &str) -> Option<String> {
    let lower = column_name.to_lowercase();
    lower
        .strip_suffix("_id")
        .filter(|prefix| !prefix.is_empty())
        .map(str::to_string)
}

fn normalize_ref_table(
    ref_table: &str,
    source: &ErTableModel,
    table_keys: &HashMap<String, String>,
) -> Option<String> {
    let lower = ref_table.to_lowercase();
    if let Some(key) = table_keys.get(&lower) {
        return Some(key.clone());
    }
    source.schema.as_ref().and_then(|schema| {
        let key = format!("{}.{}", schema.to_lowercase(), lower);
        table_keys.get(&key).cloned()
    })
}

fn table_lookup(tables: &[ErTableModel]) -> HashMap<String, String> {
    let mut lookup = HashMap::new();
    for table in tables {
        let key = table_key(table);
        lookup.insert(table.name.to_lowercase(), key.clone());
        lookup.insert(key.to_lowercase(), key);
    }
    lookup
}

fn has_id_target_column(table: &ErTableModel) -> bool {
    table
        .columns
        .iter()
        .any(|column| column.name.eq_ignore_ascii_case("id") || column.is_primary_key)
}

fn relationship_key(relationship: &ErRelationship) -> (String, Vec<String>, String) {
    (
        relationship.source_table.clone(),
        relationship.source_columns.clone(),
        relationship.target_table.clone(),
    )
}

fn table_key(table: &ErTableModel) -> String {
    match &table.schema {
        Some(schema) if !schema.is_empty() => format!("{}.{}", schema, table.name),
        _ => table.name.clone(),
    }
}

fn table_height(column_count: usize) -> f32 {
    (TABLE_HEADER_HEIGHT + COLUMN_ROW_HEIGHT * column_count as f32).max(TABLE_MIN_HEIGHT)
}

fn relationship_layout(
    tables: &[ErTableModel],
    relationships: &[ErRelationship],
) -> HashMap<String, (f32, f32)> {
    let mut levels: HashMap<String, usize> = tables
        .iter()
        .map(|table| (table_key(table), 0usize))
        .collect();

    for _ in 0..tables.len() {
        for relationship in relationships {
            let target_level = levels.get(&relationship.target_table).copied().unwrap_or(0);
            let source_level = levels.entry(relationship.source_table.clone()).or_insert(0);
            *source_level = (*source_level).max(target_level + 1);
        }
    }

    layered_positions(tables, &levels)
}

fn layered_positions(
    tables: &[ErTableModel],
    levels: &HashMap<String, usize>,
) -> HashMap<String, (f32, f32)> {
    let mut layers: BTreeMap<usize, Vec<&ErTableModel>> = BTreeMap::new();
    for table in tables {
        let key = table_key(table);
        layers
            .entry(*levels.get(&key).unwrap_or(&0))
            .or_default()
            .push(table);
    }

    let mut positions = HashMap::new();
    for (level, layer_tables) in layers {
        let mut y = GRID_Y_START;
        for table in layer_tables {
            let key = table_key(table);
            positions.insert(key, (GRID_X_START + level as f32 * LAYER_X_GAP, y));
            y += table_height(table.columns.len()) + LAYER_Y_GAP;
        }
    }
    positions
}

fn relationship_column_lookup(relationships: &[ErRelationship]) -> HashSet<(String, String)> {
    relationships
        .iter()
        .flat_map(|relationship| {
            relationship
                .source_columns
                .iter()
                .map(|column| (relationship.source_table.clone(), column.to_lowercase()))
        })
        .collect()
}

fn table_data(
    table: &ErTableModel,
    relationship_columns: &HashSet<(String, String)>,
) -> serde_json::Value {
    let key = table_key(table);
    json!({
        "schema": table.schema,
        "table": table.name,
        "label": key,
        "columns": table.columns.iter().map(|column| {
            column_data(&key, column, relationship_columns)
        }).collect::<Vec<_>>()
    })
}

fn column_data(
    table_key: &str,
    column: &ErColumnModel,
    relationship_columns: &HashSet<(String, String)>,
) -> serde_json::Value {
    let is_fk = relationship_columns.contains(&(table_key.to_string(), column.name.to_lowercase()));
    json!({
        "name": column.name,
        "ty": column.data_type,
        "pk": column.is_primary_key,
        "fk": is_fk,
        "nullable": column.is_nullable
    })
}
