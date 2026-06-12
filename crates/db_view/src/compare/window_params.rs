use crate::compare::{DataCompareParams, SchemaCompareParams};

#[derive(Debug, Clone)]
pub(super) struct DataCompareSelection {
    pub connection_id: String,
    pub database: String,
    pub schema: String,
    pub table: String,
}

#[derive(Debug, Clone)]
pub(super) struct SchemaCompareSelection {
    pub connection_id: String,
    pub database: String,
    pub schema: String,
}

pub(super) fn data_compare_params(
    source: DataCompareSelection,
    target: DataCompareSelection,
    key_columns: String,
) -> Result<DataCompareParams, &'static str> {
    if source.connection_id.trim().is_empty()
        || source.database.trim().is_empty()
        || source.table.trim().is_empty()
    {
        return Err("Source connection, database and table are required");
    }
    if target.connection_id.trim().is_empty()
        || target.database.trim().is_empty()
        || target.table.trim().is_empty()
    {
        return Err("Target connection, database and table are required");
    }

    Ok(DataCompareParams {
        source_connection_id: source.connection_id,
        source_database: source.database,
        source_schema: empty_to_none(source.schema),
        source_table: source.table,
        target_connection_id: target.connection_id,
        target_database: target.database,
        target_schema: empty_to_none(target.schema),
        target_table: target.table,
        key_columns: split_columns(key_columns),
    })
}

pub(super) fn schema_compare_params(
    source: SchemaCompareSelection,
    target: SchemaCompareSelection,
) -> Result<SchemaCompareParams, &'static str> {
    if source.connection_id.trim().is_empty() || source.database.trim().is_empty() {
        return Err("Source connection and database are required");
    }
    if target.connection_id.trim().is_empty() || target.database.trim().is_empty() {
        return Err("Target connection and database are required");
    }

    Ok(SchemaCompareParams {
        source_connection_id: source.connection_id,
        source_database: source.database,
        source_schema: empty_to_none(source.schema),
        target_connection_id: target.connection_id,
        target_database: target.database,
        target_schema: empty_to_none(target.schema),
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
