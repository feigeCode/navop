use db::DbNode;

use crate::compare::{DataCompareParams, SchemaCompareParams};

pub(super) fn data_compare_params(
    source_node: &DbNode,
    target_connection_id: String,
    target_database: String,
    target_schema: String,
    target_table: String,
    key_columns: String,
) -> Result<DataCompareParams, &'static str> {
    let Some(source_database) = source_node.get_database_name() else {
        return Err("Source database is required");
    };
    if target_connection_id.trim().is_empty()
        || target_database.trim().is_empty()
        || target_table.trim().is_empty()
    {
        return Err("Target connection, database and table are required");
    }

    Ok(DataCompareParams {
        source_connection_id: source_node.connection_id.clone(),
        source_database,
        source_schema: source_node.get_schema_name(),
        source_table: source_node.name.clone(),
        target_connection_id,
        target_database,
        target_schema: empty_to_none(target_schema),
        target_table,
        key_columns: split_columns(key_columns),
    })
}

pub(super) fn schema_compare_params(
    source_node: &DbNode,
    target_connection_id: String,
    target_database: String,
    target_schema: String,
) -> Result<SchemaCompareParams, &'static str> {
    let Some(source_database) = source_node.get_database_name() else {
        return Err("Source database is required");
    };
    if target_connection_id.trim().is_empty() || target_database.trim().is_empty() {
        return Err("Target connection and database are required");
    }

    Ok(SchemaCompareParams {
        source_connection_id: source_node.connection_id.clone(),
        source_database,
        source_schema: source_node.get_schema_name(),
        target_connection_id,
        target_database,
        target_schema: empty_to_none(target_schema),
    })
}

pub(super) fn split_columns(value: String) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|column| !column.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn empty_to_none(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
