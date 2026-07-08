use std::collections::{HashMap, HashSet};

use crate::compare::{DataCompareParams, DataCompareTablePair, SchemaCompareParams};

#[derive(Debug, Clone)]
pub(super) struct DataCompareSelection {
    pub connection_id: String,
    pub database: String,
    pub schema: String,
    pub tables: Vec<String>,
}

#[derive(Debug, Clone)]
pub(super) struct SchemaCompareSelection {
    pub connection_id: String,
    pub database: String,
    pub schema: String,
    pub tables: Vec<String>,
}

#[derive(Debug, Clone)]
pub(super) struct SchemaCompareSettings {
    pub case_sensitive_identifiers: bool,
    pub compare_indexes: bool,
    pub compare_foreign_keys: bool,
    pub ignore_comments: bool,
    pub ignore_auto_increment: bool,
    pub ignore_charset_collation: bool,
    pub ignore_table_options: bool,
    pub compare_column_order: bool,
}

impl Default for SchemaCompareSettings {
    fn default() -> Self {
        Self {
            case_sensitive_identifiers: false,
            compare_indexes: true,
            compare_foreign_keys: true,
            ignore_comments: false,
            ignore_auto_increment: false,
            ignore_charset_collation: false,
            ignore_table_options: false,
            compare_column_order: false,
        }
    }
}

pub(super) fn data_compare_params(
    source: DataCompareSelection,
    target: DataCompareSelection,
    key_columns: String,
    case_sensitive_identifiers: bool,
) -> Result<DataCompareParams, &'static str> {
    if source.connection_id.trim().is_empty() || source.database.trim().is_empty() {
        return Err("Source connection and database are required");
    }
    if target.connection_id.trim().is_empty() || target.database.trim().is_empty() {
        return Err("Target connection and database are required");
    }
    let table_pairs =
        data_compare_table_pairs(&source.tables, &target.tables, case_sensitive_identifiers)?;

    Ok(DataCompareParams {
        source_connection_id: source.connection_id,
        source_database: source.database,
        source_schema: empty_to_none(source.schema),
        target_connection_id: target.connection_id,
        target_database: target.database,
        target_schema: empty_to_none(target.schema),
        table_pairs,
        key_columns: split_columns(key_columns),
        case_sensitive_identifiers,
    })
}

fn data_compare_table_pairs(
    source_tables: &[String],
    target_tables: &[String],
    case_sensitive_identifiers: bool,
) -> Result<Vec<DataCompareTablePair>, &'static str> {
    let source_tables = normalized_table_list(source_tables, case_sensitive_identifiers)?;
    let target_tables = normalized_table_list(target_tables, case_sensitive_identifiers)?;
    if source_tables.is_empty() {
        return Err("Select at least one source table");
    }
    if target_tables.is_empty() {
        return Ok(source_tables
            .into_iter()
            .map(|source| DataCompareTablePair {
                target_table: source.clone(),
                source_table: source,
            })
            .collect());
    }
    if source_tables.len() == 1 && target_tables.len() == 1 {
        return Ok(vec![DataCompareTablePair {
            source_table: source_tables[0].clone(),
            target_table: target_tables[0].clone(),
        }]);
    }

    let target_by_normalized =
        table_map_by_identifier_key(&target_tables, case_sensitive_identifiers)?;
    Ok(source_tables
        .iter()
        .map(|source| {
            let target_table = target_by_normalized
                .get(&identifier_key(source, case_sensitive_identifiers))
                .cloned()
                .unwrap_or_else(|| source.clone());
            DataCompareTablePair {
                source_table: source.clone(),
                target_table,
            }
        })
        .collect())
}

fn normalized_table_list(
    tables: &[String],
    case_sensitive_identifiers: bool,
) -> Result<Vec<String>, &'static str> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for table in tables
        .iter()
        .map(|table| table.trim())
        .filter(|table| !table.is_empty())
    {
        if !seen.insert(identifier_key(table, case_sensitive_identifiers)) {
            return Err("Duplicate selected table names are not supported");
        }
        normalized.push(table.to_string());
    }
    Ok(normalized)
}

fn table_map_by_identifier_key(
    tables: &[String],
    case_sensitive_identifiers: bool,
) -> Result<HashMap<String, String>, &'static str> {
    let mut map = HashMap::new();
    for table in tables {
        if map
            .insert(
                identifier_key(table, case_sensitive_identifiers),
                table.clone(),
            )
            .is_some()
        {
            return Err("Duplicate selected table names are not supported");
        }
    }
    Ok(map)
}

pub(super) fn schema_compare_params(
    source: SchemaCompareSelection,
    target: SchemaCompareSelection,
    settings: SchemaCompareSettings,
) -> Result<SchemaCompareParams, &'static str> {
    if source.connection_id.trim().is_empty() || source.database.trim().is_empty() {
        return Err("Source connection and database are required");
    }
    if target.connection_id.trim().is_empty() || target.database.trim().is_empty() {
        return Err("Target connection and database are required");
    }
    let mut source_tables =
        normalized_table_list(&source.tables, settings.case_sensitive_identifiers)?;
    let mut target_tables =
        normalized_table_list(&target.tables, settings.case_sensitive_identifiers)?;
    if source_tables.is_empty() && !target_tables.is_empty() {
        source_tables = target_tables.clone();
    } else if target_tables.is_empty() && !source_tables.is_empty() {
        target_tables = source_tables.clone();
    }

    Ok(SchemaCompareParams {
        source_connection_id: source.connection_id,
        source_database: source.database,
        source_schema: empty_to_none(source.schema),
        source_tables,
        target_connection_id: target.connection_id,
        target_database: target.database,
        target_schema: empty_to_none(target.schema),
        target_tables,
        case_sensitive_identifiers: settings.case_sensitive_identifiers,
        compare_indexes: settings.compare_indexes,
        compare_foreign_keys: settings.compare_foreign_keys,
        ignore_comments: settings.ignore_comments,
        ignore_auto_increment: settings.ignore_auto_increment,
        ignore_charset_collation: settings.ignore_charset_collation,
        ignore_table_options: settings.ignore_table_options,
        compare_column_order: settings.compare_column_order,
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

fn identifier_key(value: &str, case_sensitive_identifiers: bool) -> String {
    if case_sensitive_identifiers {
        value.trim().to_string()
    } else {
        value.trim().to_lowercase()
    }
}
