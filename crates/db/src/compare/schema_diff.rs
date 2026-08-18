use super::{
    ColumnDiff, ColumnSchema, DiffStatus, ForeignKeyDiff, ForeignKeySchema, IndexDiff, IndexSchema,
    SchemaCompareError, SchemaCompareOptions, SchemaCompareResult, SchemaObjectType,
    SchemaTypeMappingContext, TableDiff, TableSchema, column_types_equivalent,
};
use std::collections::{HashMap, HashSet};

/// 比较源端和目标端的表结构
pub fn compare_schemas(
    source_tables: Vec<TableSchema>,
    target_tables: Vec<TableSchema>,
    options: SchemaCompareOptions,
) -> Result<SchemaCompareResult, SchemaCompareError> {
    compare_schemas_with_type_mapping(source_tables, target_tables, options, None)
}

/// 比较源端和目标端的表结构，并在跨数据库时按字段类型语义判断等价性。
pub fn compare_schemas_with_type_mapping(
    source_tables: Vec<TableSchema>,
    target_tables: Vec<TableSchema>,
    options: SchemaCompareOptions,
    type_mapping: Option<SchemaTypeMappingContext<'_>>,
) -> Result<SchemaCompareResult, SchemaCompareError> {
    validate_schema_identifiers(&source_tables, &options)?;
    validate_schema_identifiers(&target_tables, &options)?;

    let source_map: HashMap<String, TableSchema> = source_tables
        .into_iter()
        .map(|t| (identifier_key(&t.name, &options), t))
        .collect();
    let target_map: HashMap<String, TableSchema> = target_tables
        .into_iter()
        .map(|t| (identifier_key(&t.name, &options), t))
        .collect();

    let source_names = source_map.keys().cloned().collect::<HashSet<_>>();
    let target_names = target_map.keys().cloned().collect::<HashSet<_>>();

    let mut table_diffs = Vec::new();

    // 源端独有的表（新增）
    for name in source_names.difference(&target_names) {
        let source = source_map.get(name).cloned();
        table_diffs.push(TableDiff {
            name: source
                .as_ref()
                .map(|table| table.name.clone())
                .unwrap_or_default(),
            status: DiffStatus::Added,
            object_type: source
                .as_ref()
                .map(|table| table.object_type)
                .unwrap_or_default(),
            changes: vec![],
            source,
            target: None,
            column_diffs: vec![],
            index_diffs: vec![],
            foreign_key_diffs: vec![],
            comment_changed: false,
            table_options_changed: false,
        });
    }

    // 目标端独有的表（删除）
    for name in target_names.difference(&source_names) {
        let target = target_map.get(name).cloned();
        table_diffs.push(TableDiff {
            name: target
                .as_ref()
                .map(|table| table.name.clone())
                .unwrap_or_default(),
            status: DiffStatus::Removed,
            object_type: target
                .as_ref()
                .map(|table| table.object_type)
                .unwrap_or_default(),
            changes: vec![],
            source: None,
            target,
            column_diffs: vec![],
            index_diffs: vec![],
            foreign_key_diffs: vec![],
            comment_changed: false,
            table_options_changed: false,
        });
    }

    // 共同的表（可能修改）
    for name in source_names.intersection(&target_names) {
        let source_table = &source_map[name];
        let target_table = &target_map[name];

        if let Some(diff) =
            compare_table(source_table, target_table, &options, type_mapping.clone())
        {
            table_diffs.push(diff);
        }
    }

    table_diffs.sort_by(|left, right| left.name.cmp(&right.name));

    let mut result = SchemaCompareResult {
        table_diffs,
        ..Default::default()
    };
    result.refresh_counts();
    Ok(result)
}

/// 比较单个表
fn compare_table(
    source: &TableSchema,
    target: &TableSchema,
    options: &SchemaCompareOptions,
    type_mapping: Option<SchemaTypeMappingContext<'_>>,
) -> Option<TableDiff> {
    let column_diffs =
        compare_columns_with_options(&source.columns, &target.columns, options, type_mapping);
    let index_diffs = if options.compare_indexes {
        compare_indexes_with_options(&source.indexes, &target.indexes, options)
    } else {
        Vec::new()
    };
    let foreign_key_diffs = if options.compare_foreign_keys {
        compare_foreign_keys_with_options(&source.foreign_keys, &target.foreign_keys, options)
    } else {
        Vec::new()
    };

    let comment_changed = if options.ignore_comments {
        false
    } else {
        source.comment != target.comment
    };
    let table_options_changed = if options.ignore_table_options {
        false
    } else {
        normalized_metadata(source.engine.as_deref())
            != normalized_metadata(target.engine.as_deref())
            || normalized_metadata(source.charset.as_deref())
                != normalized_metadata(target.charset.as_deref())
            || normalized_metadata(source.collation.as_deref())
                != normalized_metadata(target.collation.as_deref())
    };
    let object_type_changed = source.object_type != target.object_type;
    let column_order_change = column_order_change(source, target, options);

    let has_changes = !column_diffs.is_empty()
        || !index_diffs.is_empty()
        || !foreign_key_diffs.is_empty()
        || comment_changed
        || table_options_changed
        || object_type_changed
        || column_order_change.is_some();

    if has_changes {
        let mut changes = table_changes(
            source,
            target,
            &column_diffs,
            &index_diffs,
            &foreign_key_diffs,
            comment_changed,
            table_options_changed,
        );
        if let Some(change) = column_order_change {
            changes.push(change);
        }
        if object_type_changed {
            changes.insert(
                0,
                format!(
                    "object_type: {} → {}",
                    object_type_label(target.object_type),
                    object_type_label(source.object_type)
                ),
            );
        }
        Some(TableDiff {
            name: source.name.clone(),
            status: DiffStatus::Modified,
            object_type: source.object_type,
            changes,
            source: Some(source.clone()),
            target: Some(target.clone()),
            column_diffs,
            index_diffs,
            foreign_key_diffs,
            comment_changed,
            table_options_changed,
        })
    } else {
        None
    }
}

fn column_order_change(
    source: &TableSchema,
    target: &TableSchema,
    options: &SchemaCompareOptions,
) -> Option<String> {
    if !options.compare_column_order {
        return None;
    }

    let source_order = source
        .columns
        .iter()
        .map(|column| identifier_key(&column.name, options))
        .collect::<Vec<_>>();
    let target_order = target
        .columns
        .iter()
        .map(|column| identifier_key(&column.name, options))
        .collect::<Vec<_>>();

    if source_order == target_order || source_order.len() != target_order.len() {
        return None;
    }

    let mut source_names = source_order.clone();
    let mut target_names = target_order.clone();
    source_names.sort();
    target_names.sort();
    if source_names != target_names {
        return None;
    }

    Some(format!(
        "column order: {} → {}",
        target_order.join(", "),
        source_order.join(", ")
    ))
}

/// 比较列
#[cfg(test)]
fn compare_columns(source: &[ColumnSchema], target: &[ColumnSchema]) -> Vec<ColumnDiff> {
    compare_columns_with_options(source, target, &SchemaCompareOptions::default(), None)
}

fn compare_columns_with_options(
    source: &[ColumnSchema],
    target: &[ColumnSchema],
    options: &SchemaCompareOptions,
    type_mapping: Option<SchemaTypeMappingContext<'_>>,
) -> Vec<ColumnDiff> {
    let source_map = source
        .iter()
        .map(|c| (identifier_key(&c.name, options), c))
        .collect::<HashMap<_, _>>();
    let target_map = target
        .iter()
        .map(|c| (identifier_key(&c.name, options), c))
        .collect::<HashMap<_, _>>();

    let source_names = source_map.keys().cloned().collect::<HashSet<_>>();
    let target_names = target_map.keys().cloned().collect::<HashSet<_>>();

    let mut diffs = Vec::new();

    // 新增列
    for name in source_names.difference(&target_names) {
        let source = (*source_map[name]).clone();
        diffs.push(ColumnDiff {
            name: source.name.clone(),
            status: DiffStatus::Added,
            changes: vec!["column added".to_string()],
            source: Some(source),
            target: None,
        });
    }

    // 删除列
    for name in target_names.difference(&source_names) {
        let target = (*target_map[name]).clone();
        diffs.push(ColumnDiff {
            name: target.name.clone(),
            status: DiffStatus::Removed,
            changes: vec!["column removed".to_string()],
            source: None,
            target: Some(target),
        });
    }

    // 修改列
    for name in source_names.intersection(&target_names) {
        let src = source_map[name];
        let tgt = target_map[name];

        if !column_eq(src, tgt, options, type_mapping.clone()) {
            diffs.push(ColumnDiff {
                name: src.name.clone(),
                status: DiffStatus::Modified,
                changes: column_changes(src, tgt, options, type_mapping.clone()),
                source: Some((*src).clone()),
                target: Some((*tgt).clone()),
            });
        }
    }

    diffs.sort_by(|left, right| left.name.cmp(&right.name));
    diffs
}

fn compare_indexes_with_options(
    source: &[IndexSchema],
    target: &[IndexSchema],
    options: &SchemaCompareOptions,
) -> Vec<IndexDiff> {
    let source_map = source
        .iter()
        .map(|index| (identifier_key(&index.name, options), index))
        .collect::<HashMap<_, _>>();
    let target_map = target
        .iter()
        .map(|index| (identifier_key(&index.name, options), index))
        .collect::<HashMap<_, _>>();

    let source_names = source_map.keys().cloned().collect::<HashSet<_>>();
    let target_names = target_map.keys().cloned().collect::<HashSet<_>>();

    let mut diffs = Vec::new();

    for name in source_names.difference(&target_names) {
        let source = (*source_map[name]).clone();
        diffs.push(IndexDiff {
            name: source.name.clone(),
            status: DiffStatus::Added,
            changes: vec!["index added".to_string()],
            source: Some(source),
            target: None,
        });
    }

    for name in target_names.difference(&source_names) {
        let target = (*target_map[name]).clone();
        diffs.push(IndexDiff {
            name: target.name.clone(),
            status: DiffStatus::Removed,
            changes: vec!["index removed".to_string()],
            source: None,
            target: Some(target),
        });
    }

    for name in source_names.intersection(&target_names) {
        let source_index = source_map[name];
        let target_index = target_map[name];
        if !index_eq(source_index, target_index, options) {
            diffs.push(IndexDiff {
                name: source_index.name.clone(),
                status: DiffStatus::Modified,
                changes: index_changes(source_index, target_index),
                source: Some((*source_index).clone()),
                target: Some((*target_index).clone()),
            });
        }
    }

    diffs.sort_by(|left, right| left.name.cmp(&right.name));
    diffs
}

fn compare_foreign_keys_with_options(
    source: &[ForeignKeySchema],
    target: &[ForeignKeySchema],
    options: &SchemaCompareOptions,
) -> Vec<ForeignKeyDiff> {
    let source_map = source
        .iter()
        .map(|fk| (identifier_key(&fk.name, options), fk))
        .collect::<HashMap<_, _>>();
    let target_map = target
        .iter()
        .map(|fk| (identifier_key(&fk.name, options), fk))
        .collect::<HashMap<_, _>>();

    let source_names = source_map.keys().cloned().collect::<HashSet<_>>();
    let target_names = target_map.keys().cloned().collect::<HashSet<_>>();

    let mut diffs = Vec::new();

    for name in source_names.difference(&target_names) {
        let source = (*source_map[name]).clone();
        diffs.push(ForeignKeyDiff {
            name: source.name.clone(),
            status: DiffStatus::Added,
            changes: vec!["foreign key added".to_string()],
            source: Some(source),
            target: None,
        });
    }

    for name in target_names.difference(&source_names) {
        let target = (*target_map[name]).clone();
        diffs.push(ForeignKeyDiff {
            name: target.name.clone(),
            status: DiffStatus::Removed,
            changes: vec!["foreign key removed".to_string()],
            source: None,
            target: Some(target),
        });
    }

    for name in source_names.intersection(&target_names) {
        let source_fk = source_map[name];
        let target_fk = target_map[name];
        if !foreign_key_eq(source_fk, target_fk, options) {
            diffs.push(ForeignKeyDiff {
                name: source_fk.name.clone(),
                status: DiffStatus::Modified,
                changes: foreign_key_changes(source_fk, target_fk),
                source: Some((*source_fk).clone()),
                target: Some((*target_fk).clone()),
            });
        }
    }

    diffs.sort_by(|left, right| left.name.cmp(&right.name));
    diffs
}

fn identifier_key(value: &str, options: &SchemaCompareOptions) -> String {
    if options.case_sensitive_identifiers {
        value.trim().to_string()
    } else {
        value.trim().to_lowercase()
    }
}

fn column_eq(
    left: &ColumnSchema,
    right: &ColumnSchema,
    options: &SchemaCompareOptions,
    type_mapping: Option<SchemaTypeMappingContext<'_>>,
) -> bool {
    data_type_eq(&left.data_type, &right.data_type, options, type_mapping)
        && left.nullable == right.nullable
        && left.default_value == right.default_value
        && (options.ignore_comments || left.comment == right.comment)
        && (options.ignore_charset_collation
            || (normalized_metadata(left.charset.as_deref())
                == normalized_metadata(right.charset.as_deref())
                && normalized_metadata(left.collation.as_deref())
                    == normalized_metadata(right.collation.as_deref())))
}

fn data_type_eq(
    left: &str,
    right: &str,
    options: &SchemaCompareOptions,
    type_mapping: Option<SchemaTypeMappingContext<'_>>,
) -> bool {
    let left = normalized_data_type(left, options);
    let right = normalized_data_type(right, options);
    type_mapping.map_or_else(
        || left.eq_ignore_ascii_case(&right),
        |context| column_types_equivalent(&left, &right, context),
    )
}

fn normalized_data_type(value: &str, options: &SchemaCompareOptions) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if options.ignore_auto_increment {
        normalized
            .split_whitespace()
            .filter(|part| !part.eq_ignore_ascii_case("AUTO_INCREMENT"))
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        normalized
    }
}

fn normalized_metadata(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_ascii_lowercase())
    }
}

fn index_eq(left: &IndexSchema, right: &IndexSchema, options: &SchemaCompareOptions) -> bool {
    left.unique == right.unique
        && identifier_list_eq(&left.columns, &right.columns, options)
        && identifier_key(&left.name, options) == identifier_key(&right.name, options)
}

fn foreign_key_eq(
    left: &ForeignKeySchema,
    right: &ForeignKeySchema,
    options: &SchemaCompareOptions,
) -> bool {
    identifier_key(&left.name, options) == identifier_key(&right.name, options)
        && identifier_list_eq(&left.columns, &right.columns, options)
        && normalized_metadata(left.ref_schema.as_deref())
            == normalized_metadata(right.ref_schema.as_deref())
        && identifier_key(&left.ref_table, options) == identifier_key(&right.ref_table, options)
        && identifier_list_eq(&left.ref_columns, &right.ref_columns, options)
        && foreign_key_action_eq(left.on_delete.as_deref(), right.on_delete.as_deref())
        && foreign_key_action_eq(left.on_update.as_deref(), right.on_update.as_deref())
}

fn foreign_key_action_eq(left: Option<&str>, right: Option<&str>) -> bool {
    normalized_foreign_key_action(left) == normalized_foreign_key_action(right)
}

fn normalized_foreign_key_action(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    Some(
        value
            .split_whitespace()
            .map(str::to_ascii_uppercase)
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn identifier_list_eq(left: &[String], right: &[String], options: &SchemaCompareOptions) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| identifier_key(left, options) == identifier_key(right, options))
}

fn column_changes(
    source: &ColumnSchema,
    target: &ColumnSchema,
    options: &SchemaCompareOptions,
    type_mapping: Option<SchemaTypeMappingContext<'_>>,
) -> Vec<String> {
    let mut changes = Vec::new();
    if !data_type_eq(&source.data_type, &target.data_type, options, type_mapping) {
        changes.push(format!("type: {} → {}", target.data_type, source.data_type));
    }
    if source.nullable != target.nullable {
        changes.push(format!(
            "nullable: {} → {}",
            target.nullable, source.nullable
        ));
    }
    if source.default_value != target.default_value {
        changes.push(format!(
            "default: {} → {}",
            display_optional(target.default_value.as_deref()),
            display_optional(source.default_value.as_deref())
        ));
    }
    if !options.ignore_comments && source.comment != target.comment {
        changes.push(format!(
            "comment: {} → {}",
            display_optional(target.comment.as_deref()),
            display_optional(source.comment.as_deref())
        ));
    }
    if !options.ignore_charset_collation {
        if normalized_metadata(source.charset.as_deref())
            != normalized_metadata(target.charset.as_deref())
        {
            changes.push(format!(
                "charset: {} → {}",
                display_optional(target.charset.as_deref()),
                display_optional(source.charset.as_deref())
            ));
        }
        if normalized_metadata(source.collation.as_deref())
            != normalized_metadata(target.collation.as_deref())
        {
            changes.push(format!(
                "collation: {} → {}",
                display_optional(target.collation.as_deref()),
                display_optional(source.collation.as_deref())
            ));
        }
    }
    changes
}

fn index_changes(source: &IndexSchema, target: &IndexSchema) -> Vec<String> {
    let mut changes = Vec::new();
    if source.unique != target.unique {
        changes.push(format!("unique: {} → {}", target.unique, source.unique));
    }
    if source.columns != target.columns {
        changes.push(format!(
            "columns: {} → {}",
            target.columns.join(", "),
            source.columns.join(", ")
        ));
    }
    changes
}

fn foreign_key_changes(source: &ForeignKeySchema, target: &ForeignKeySchema) -> Vec<String> {
    let mut changes = Vec::new();
    if source.columns != target.columns {
        changes.push(format!(
            "columns: {} → {}",
            target.columns.join(", "),
            source.columns.join(", ")
        ));
    }
    if source.ref_schema != target.ref_schema {
        changes.push(format!(
            "referenced schema: {} → {}",
            display_optional(target.ref_schema.as_deref()),
            display_optional(source.ref_schema.as_deref())
        ));
    }
    if source.ref_table != target.ref_table {
        changes.push(format!(
            "referenced table: {} → {}",
            target.ref_table, source.ref_table
        ));
    }
    if source.ref_columns != target.ref_columns {
        changes.push(format!(
            "referenced columns: {} → {}",
            target.ref_columns.join(", "),
            source.ref_columns.join(", ")
        ));
    }
    if !foreign_key_action_eq(source.on_delete.as_deref(), target.on_delete.as_deref()) {
        changes.push(format!(
            "on delete: {} → {}",
            display_optional(target.on_delete.as_deref()),
            display_optional(source.on_delete.as_deref())
        ));
    }
    if !foreign_key_action_eq(source.on_update.as_deref(), target.on_update.as_deref()) {
        changes.push(format!(
            "on update: {} → {}",
            display_optional(target.on_update.as_deref()),
            display_optional(source.on_update.as_deref())
        ));
    }
    changes
}

fn table_changes(
    source: &TableSchema,
    target: &TableSchema,
    column_diffs: &[ColumnDiff],
    index_diffs: &[IndexDiff],
    foreign_key_diffs: &[ForeignKeyDiff],
    comment_changed: bool,
    table_options_changed: bool,
) -> Vec<String> {
    let mut changes = Vec::new();
    changes.extend(
        column_diffs
            .iter()
            .flat_map(|diff| prefixed_changes("column", &diff.name, &diff.changes)),
    );
    changes.extend(
        index_diffs
            .iter()
            .flat_map(|diff| prefixed_changes("index", &diff.name, &diff.changes)),
    );
    changes.extend(
        foreign_key_diffs
            .iter()
            .flat_map(|diff| prefixed_changes("foreign key", &diff.name, &diff.changes)),
    );
    if comment_changed {
        changes.push(format!(
            "comment: {} → {}",
            display_optional(target.comment.as_deref()),
            display_optional(source.comment.as_deref())
        ));
    }
    if table_options_changed {
        if normalized_metadata(source.engine.as_deref())
            != normalized_metadata(target.engine.as_deref())
        {
            changes.push(format!(
                "engine: {} → {}",
                display_optional(target.engine.as_deref()),
                display_optional(source.engine.as_deref())
            ));
        }
        if normalized_metadata(source.charset.as_deref())
            != normalized_metadata(target.charset.as_deref())
        {
            changes.push(format!(
                "charset: {} → {}",
                display_optional(target.charset.as_deref()),
                display_optional(source.charset.as_deref())
            ));
        }
        if normalized_metadata(source.collation.as_deref())
            != normalized_metadata(target.collation.as_deref())
        {
            changes.push(format!(
                "collation: {} → {}",
                display_optional(target.collation.as_deref()),
                display_optional(source.collation.as_deref())
            ));
        }
    }
    changes
}

fn prefixed_changes<'a>(
    kind: &'a str,
    name: &'a str,
    changes: &'a [String],
) -> impl Iterator<Item = String> + 'a {
    changes
        .iter()
        .map(move |change| format!("{kind} `{name}`: {change}"))
}

fn display_optional(value: Option<&str>) -> String {
    value
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("NULL")
        .to_string()
}

fn object_type_label(object_type: SchemaObjectType) -> &'static str {
    match object_type {
        SchemaObjectType::Table => "table",
        SchemaObjectType::View => "view",
    }
}

fn validate_schema_identifiers(
    tables: &[TableSchema],
    options: &SchemaCompareOptions,
) -> Result<(), SchemaCompareError> {
    validate_unique_names(
        "table",
        tables.iter().map(|table| table.name.as_str()),
        options,
    )?;
    for table in tables {
        validate_unique_names(
            &format!("table `{}` column", table.name),
            table.columns.iter().map(|column| column.name.as_str()),
            options,
        )?;
        validate_unique_names(
            &format!("table `{}` index", table.name),
            table.indexes.iter().map(|index| index.name.as_str()),
            options,
        )?;
        validate_unique_names(
            &format!("table `{}` foreign key", table.name),
            table
                .foreign_keys
                .iter()
                .map(|foreign_key| foreign_key.name.as_str()),
            options,
        )?;
    }
    Ok(())
}

fn validate_unique_names<'a>(
    scope: &str,
    names: impl IntoIterator<Item = &'a str>,
    options: &SchemaCompareOptions,
) -> Result<(), SchemaCompareError> {
    let mut seen = HashMap::new();
    for name in names {
        let key = identifier_key(name, options);
        if let Some(previous) = seen.insert(key, name.to_string()) {
            return Err(SchemaCompareError::DuplicateIdentifier(format!(
                "{scope}: `{previous}` and `{name}`"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_core::storage::DatabaseType;

    fn column(name: &str, data_type: &str, nullable: bool) -> ColumnSchema {
        ColumnSchema {
            name: name.to_string(),
            data_type: data_type.to_string(),
            nullable,
            default_value: None,
            comment: None,
            ..Default::default()
        }
    }

    fn table(name: &str, columns: Vec<ColumnSchema>) -> TableSchema {
        TableSchema {
            name: name.to_string(),
            columns,
            indexes: vec![],
            foreign_keys: vec![],
            comment: None,
            ..Default::default()
        }
    }

    #[test]
    fn test_compare_schemas_detects_added_removed_modified() {
        let source = vec![
            TableSchema {
                name: "users".to_string(),
                columns: vec![
                    column("id", "int", false),
                    column("name", "varchar(64)", false),
                ],
                indexes: vec![],
                foreign_keys: vec![],
                comment: None,
                ..Default::default()
            },
            TableSchema {
                name: "orders".to_string(),
                columns: vec![column("id", "int", false)],
                indexes: vec![],
                foreign_keys: vec![],
                comment: None,
                ..Default::default()
            },
        ];

        let target = vec![
            TableSchema {
                name: "users".to_string(),
                columns: vec![
                    column("id", "int", false),
                    column("name", "varchar(32)", true),
                ],
                indexes: vec![],
                foreign_keys: vec![],
                comment: None,
                ..Default::default()
            },
            TableSchema {
                name: "audit".to_string(),
                columns: vec![column("id", "int", false)],
                indexes: vec![],
                foreign_keys: vec![],
                comment: None,
                ..Default::default()
            },
        ];

        let result = compare_schemas(source, target, SchemaCompareOptions::default()).unwrap();

        assert_eq!(result.added_count, 1);
        assert_eq!(result.removed_count, 1);
        assert_eq!(result.modified_count, 1);

        let users_diff = result
            .table_diffs
            .iter()
            .find(|d| d.name == "users")
            .unwrap();
        assert_eq!(users_diff.status, DiffStatus::Modified);
        assert_eq!(users_diff.column_diffs.len(), 1);
    }

    #[test]
    fn test_compare_schemas_matches_identifiers_case_insensitively_by_default() {
        let source = vec![table(
            "Users",
            vec![column("ID", "int", false), column("Name", "varchar", true)],
        )];
        let target = vec![table(
            "users",
            vec![column("id", "int", false), column("name", "varchar", true)],
        )];

        let result = compare_schemas(source, target, SchemaCompareOptions::default()).unwrap();

        assert!(result.table_diffs.is_empty());
        assert_eq!(result.added_count, 0);
        assert_eq!(result.removed_count, 0);
        assert_eq!(result.modified_count, 0);
    }

    #[test]
    fn compare_schemas_ignores_column_order_by_default() {
        let source = vec![table(
            "users",
            vec![column("id", "int", false), column("name", "varchar", true)],
        )];
        let target = vec![table(
            "users",
            vec![column("name", "varchar", true), column("id", "int", false)],
        )];

        let result = compare_schemas(source, target, SchemaCompareOptions::default()).unwrap();

        assert!(result.table_diffs.is_empty());
    }

    #[test]
    fn compare_schemas_reports_column_order_when_enabled() {
        let source = vec![table(
            "users",
            vec![column("id", "int", false), column("name", "varchar", true)],
        )];
        let target = vec![table(
            "users",
            vec![column("name", "varchar", true), column("id", "int", false)],
        )];

        let result = compare_schemas(
            source,
            target,
            SchemaCompareOptions {
                compare_column_order: true,
                ..SchemaCompareOptions::default()
            },
        )
        .unwrap();

        let diff = &result.table_diffs[0];
        assert!(diff.column_diffs.is_empty());
        assert_eq!(
            diff.changes,
            vec!["column order: name, id → id, name".to_string()]
        );
    }

    #[test]
    fn compare_schemas_does_not_report_column_order_with_added_or_removed_columns() {
        let source = vec![table(
            "users",
            vec![column("id", "int", false), column("name", "varchar", true)],
        )];
        let target = vec![table(
            "users",
            vec![
                column("name", "varchar", true),
                column("legacy", "text", true),
            ],
        )];

        let result = compare_schemas(
            source,
            target,
            SchemaCompareOptions {
                compare_column_order: true,
                ..SchemaCompareOptions::default()
            },
        )
        .unwrap();

        let diff = &result.table_diffs[0];
        assert!(
            !diff
                .changes
                .iter()
                .any(|change| change.starts_with("column order:"))
        );
    }

    #[test]
    fn compare_schemas_uses_normalized_names_for_column_order() {
        let source = vec![table(
            "users",
            vec![column("ID", "int", false), column("Name", "varchar", true)],
        )];
        let target = vec![table(
            "users",
            vec![column("name", "varchar", true), column("id", "int", false)],
        )];

        let result = compare_schemas(
            source,
            target,
            SchemaCompareOptions {
                compare_column_order: true,
                ..SchemaCompareOptions::default()
            },
        )
        .unwrap();

        assert_eq!(
            result.table_diffs[0].changes,
            vec!["column order: name, id → id, name".to_string()]
        );
    }

    #[test]
    fn test_compare_schemas_rejects_duplicate_case_insensitive_table_names() {
        let source = vec![
            table("Users", vec![column("ID", "int", false)]),
            table("users", vec![column("id", "int", false)]),
        ];
        let target = vec![];

        let result = compare_schemas(source, target, SchemaCompareOptions::default());

        assert!(result.is_err());
    }

    #[test]
    fn test_compare_schemas_rejects_duplicate_case_insensitive_column_names() {
        let source = vec![table(
            "users",
            vec![column("ID", "int", false), column("id", "int", false)],
        )];
        let target = vec![table("users", vec![column("id", "int", false)])];

        let result = compare_schemas(source, target, SchemaCompareOptions::default());

        assert!(result.is_err());
    }

    #[test]
    fn compare_schemas_can_ignore_table_option_noise() {
        let mut source = table("users", vec![column("id", "int", false)]);
        source.engine = Some("InnoDB".to_string());
        source.charset = Some("utf8mb4".to_string());
        source.collation = Some("utf8mb4_0900_ai_ci".to_string());
        source.comment = Some("source comment".to_string());

        let mut target = table("users", vec![column("id", "int", false)]);
        target.engine = Some("MyISAM".to_string());
        target.charset = Some("utf8".to_string());
        target.collation = Some("utf8_general_ci".to_string());
        target.comment = Some("target comment".to_string());

        let result = compare_schemas(
            vec![source],
            vec![target],
            SchemaCompareOptions {
                ignore_comments: true,
                ignore_table_options: true,
                ..SchemaCompareOptions::default()
            },
        )
        .unwrap();

        assert!(result.table_diffs.is_empty());
    }

    #[test]
    fn compare_schemas_can_ignore_column_metadata_noise() {
        let mut source_column = column("id", "int auto_increment", false);
        source_column.charset = Some("utf8mb4".to_string());
        source_column.collation = Some("utf8mb4_0900_ai_ci".to_string());
        source_column.comment = Some("source id".to_string());

        let mut target_column = column("id", "int", false);
        target_column.charset = Some("utf8".to_string());
        target_column.collation = Some("utf8_general_ci".to_string());
        target_column.comment = Some("target id".to_string());

        let result = compare_schemas(
            vec![table("users", vec![source_column])],
            vec![table("users", vec![target_column])],
            SchemaCompareOptions {
                ignore_comments: true,
                ignore_auto_increment: true,
                ignore_charset_collation: true,
                ..SchemaCompareOptions::default()
            },
        )
        .unwrap();

        assert!(result.table_diffs.is_empty());
    }

    #[test]
    fn compare_schemas_can_ignore_index_and_foreign_key_objects() {
        let mut source = table("orders", vec![column("id", "int", false)]);
        source.indexes = vec![IndexSchema {
            name: "idx_orders_user".to_string(),
            columns: vec!["user_id".to_string()],
            unique: false,
        }];
        source.foreign_keys = vec![ForeignKeySchema {
            name: "fk_orders_user".to_string(),
            columns: vec!["user_id".to_string()],
            ref_table: "users".to_string(),
            ref_schema: None,
            ref_columns: vec!["id".to_string()],
            on_delete: None,
            on_update: None,
        }];

        let target = table("orders", vec![column("id", "int", false)]);

        let result = compare_schemas(
            vec![source],
            vec![target],
            SchemaCompareOptions {
                compare_indexes: false,
                compare_foreign_keys: false,
                ..SchemaCompareOptions::default()
            },
        )
        .unwrap();

        assert!(result.table_diffs.is_empty());
    }

    #[test]
    fn test_column_comparison() {
        let source = vec![
            column("id", "int", false),
            column("name", "varchar(64)", false),
            column("email", "text", true),
        ];
        let target = vec![
            column("id", "int", false),
            column("name", "varchar(32)", false),
            column("phone", "text", true),
        ];

        let diffs = compare_columns(&source, &target);

        assert_eq!(diffs.len(), 3);
        assert!(
            diffs
                .iter()
                .any(|d| d.name == "email" && d.status == DiffStatus::Added)
        );
        assert!(
            diffs
                .iter()
                .any(|d| d.name == "phone" && d.status == DiffStatus::Removed)
        );
        assert!(
            diffs
                .iter()
                .any(|d| d.name == "name" && d.status == DiffStatus::Modified)
        );
    }

    #[test]
    fn test_added_table_diff_keeps_source_schema() {
        let source = vec![table("users", vec![column("id", "int", false)])];

        let result = compare_schemas(source, vec![], SchemaCompareOptions::default()).unwrap();

        let diff = result
            .table_diffs
            .iter()
            .find(|d| d.name == "users")
            .unwrap();
        assert_eq!(diff.status, DiffStatus::Added);
        assert_eq!(
            diff.source.as_ref().unwrap().columns[0].name,
            "id".to_string()
        );
        assert!(diff.target.is_none());
    }

    #[test]
    fn test_index_and_foreign_key_property_changes_are_modified() {
        let mut source = table("orders", vec![column("id", "int", false)]);
        source.indexes = vec![IndexSchema {
            name: "idx_orders_user".to_string(),
            columns: vec!["user_id".to_string()],
            unique: true,
        }];
        source.foreign_keys = vec![ForeignKeySchema {
            name: "fk_orders_user".to_string(),
            columns: vec!["user_id".to_string()],
            ref_table: "users".to_string(),
            ref_schema: None,
            ref_columns: vec!["id".to_string()],
            on_delete: Some("CASCADE".to_string()),
            on_update: Some("NO ACTION".to_string()),
        }];

        let mut target = source.clone();
        target.indexes[0].unique = false;
        target.foreign_keys[0].ref_columns = vec!["legacy_id".to_string()];

        let result =
            compare_schemas(vec![source], vec![target], SchemaCompareOptions::default()).unwrap();

        let diff = result
            .table_diffs
            .iter()
            .find(|d| d.name == "orders")
            .unwrap();
        assert!(
            diff.index_diffs
                .iter()
                .any(|d| d.name == "idx_orders_user" && d.status == DiffStatus::Modified)
        );
        assert!(
            diff.foreign_key_diffs
                .iter()
                .any(|d| d.name == "fk_orders_user" && d.status == DiffStatus::Modified)
        );
    }

    #[test]
    fn test_foreign_key_referenced_schema_changes_are_modified() {
        let mut source = table("orders", vec![column("id", "int", false)]);
        source.foreign_keys = vec![ForeignKeySchema {
            name: "fk_orders_user".to_string(),
            columns: vec!["user_id".to_string()],
            ref_table: "users".to_string(),
            ref_schema: Some("audit".to_string()),
            ref_columns: vec!["id".to_string()],
            on_delete: None,
            on_update: None,
        }];
        let mut target = source.clone();
        target.foreign_keys[0].ref_schema = Some("public".to_string());

        let result =
            compare_schemas(vec![source], vec![target], SchemaCompareOptions::default()).unwrap();

        let diff = &result.table_diffs[0].foreign_key_diffs[0];
        assert_eq!(diff.status, DiffStatus::Modified);
        assert!(
            diff.changes
                .iter()
                .any(|change| change.contains("referenced schema"))
        );
    }

    #[test]
    fn test_foreign_key_action_changes_are_modified() {
        let mut source = table("orders", vec![column("id", "int", false)]);
        source.foreign_keys = vec![ForeignKeySchema {
            name: "fk_orders_user".to_string(),
            columns: vec!["user_id".to_string()],
            ref_table: "users".to_string(),
            ref_schema: None,
            ref_columns: vec!["id".to_string()],
            on_delete: Some("CASCADE".to_string()),
            on_update: Some("NO ACTION".to_string()),
        }];

        let mut target = source.clone();
        target.foreign_keys[0].on_delete = Some("RESTRICT".to_string());

        let result =
            compare_schemas(vec![source], vec![target], SchemaCompareOptions::default()).unwrap();

        let diff = result
            .table_diffs
            .iter()
            .find(|d| d.name == "orders")
            .unwrap();
        assert!(
            diff.foreign_key_diffs
                .iter()
                .any(|d| d.name == "fk_orders_user" && d.status == DiffStatus::Modified)
        );
    }

    #[test]
    fn test_primary_key_column_changes_are_modified_indexes() {
        let mut source = table("orders", vec![column("id", "int", false)]);
        source.indexes = vec![IndexSchema {
            name: "PRIMARY".to_string(),
            columns: vec!["id".to_string()],
            unique: true,
        }];

        let mut target = table("orders", vec![column("id", "int", false)]);
        target.indexes = vec![IndexSchema {
            name: "PRIMARY".to_string(),
            columns: vec!["legacy_id".to_string()],
            unique: true,
        }];

        let result =
            compare_schemas(vec![source], vec![target], SchemaCompareOptions::default()).unwrap();

        let diff = result
            .table_diffs
            .iter()
            .find(|d| d.name == "orders")
            .unwrap();
        assert!(
            diff.index_diffs
                .iter()
                .any(|d| d.name == "PRIMARY" && d.status == DiffStatus::Modified)
        );
    }

    #[test]
    fn modified_column_describes_target_to_source_changes() {
        let mut source_column = column("name", "varchar(64)", true);
        source_column.default_value = Some("'anonymous'".to_string());
        let target_column = column("name", "varchar(32)", false);

        let result = compare_schemas(
            vec![table("users", vec![source_column])],
            vec![table("users", vec![target_column])],
            SchemaCompareOptions::default(),
        )
        .unwrap();

        let table_diff = &result.table_diffs[0];
        let column_diff = &table_diff.column_diffs[0];
        assert_eq!(
            column_diff.changes,
            vec![
                "type: varchar(32) → varchar(64)",
                "nullable: false → true",
                "default: NULL → 'anonymous'",
            ]
        );
        assert_eq!(
            table_diff.changes,
            vec![
                "column `name`: type: varchar(32) → varchar(64)",
                "column `name`: nullable: false → true",
                "column `name`: default: NULL → 'anonymous'",
            ]
        );
    }

    #[test]
    fn table_diff_preserves_table_option_change_details() {
        let mut source = table("users", vec![column("id", "int", false)]);
        source.engine = Some("InnoDB".to_string());
        source.charset = Some("utf8mb4".to_string());
        source.collation = Some("utf8mb4_0900_ai_ci".to_string());

        let mut target = source.clone();
        target.engine = Some("MyISAM".to_string());
        target.charset = Some("utf8".to_string());
        target.collation = Some("utf8_general_ci".to_string());

        let result =
            compare_schemas(vec![source], vec![target], SchemaCompareOptions::default()).unwrap();

        let table_diff = &result.table_diffs[0];
        assert!(table_diff.table_options_changed);
        assert_eq!(
            table_diff.changes,
            vec![
                "engine: MyISAM → InnoDB",
                "charset: utf8 → utf8mb4",
                "collation: utf8_general_ci → utf8mb4_0900_ai_ci",
            ]
        );
    }

    #[test]
    fn object_type_changes_and_added_removed_objects_keep_their_kind() {
        let mut source_common = table("common", vec![column("id", "int", false)]);
        source_common.object_type = SchemaObjectType::Table;
        let mut target_common = source_common.clone();
        target_common.object_type = SchemaObjectType::View;

        let mut source_view = table("source_view", vec![]);
        source_view.object_type = SchemaObjectType::View;
        let mut target_view = table("target_view", vec![]);
        target_view.object_type = SchemaObjectType::View;

        let result = compare_schemas(
            vec![source_view, source_common],
            vec![target_view, target_common],
            SchemaCompareOptions::default(),
        )
        .unwrap();

        let common = result
            .table_diffs
            .iter()
            .find(|diff| diff.name == "common")
            .unwrap();
        assert_eq!(common.object_type, SchemaObjectType::Table);
        assert_eq!(common.changes, vec!["object_type: view → table"]);

        let added = result
            .table_diffs
            .iter()
            .find(|diff| diff.name == "source_view")
            .unwrap();
        assert_eq!(added.status, DiffStatus::Added);
        assert_eq!(added.object_type, SchemaObjectType::View);
        assert!(added.changes.is_empty());

        let removed = result
            .table_diffs
            .iter()
            .find(|diff| diff.name == "target_view")
            .unwrap();
        assert_eq!(removed.status, DiffStatus::Removed);
        assert_eq!(removed.object_type, SchemaObjectType::View);
        assert!(removed.changes.is_empty());
    }

    #[test]
    fn schema_diff_order_is_deterministic() {
        let source = vec![
            table(
                "zeta",
                vec![
                    column("z_column", "text", false),
                    column("a_column", "text", false),
                ],
            ),
            table("alpha", vec![column("id", "int", false)]),
        ];
        let target = vec![
            table("zeta", vec![column("m_column", "text", false)]),
            table("middle", vec![column("id", "int", false)]),
        ];

        let result = compare_schemas(source, target, SchemaCompareOptions::default()).unwrap();

        assert_eq!(
            result
                .table_diffs
                .iter()
                .map(|diff| diff.name.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "middle", "zeta"]
        );
        let zeta = result
            .table_diffs
            .iter()
            .find(|diff| diff.name == "zeta")
            .unwrap();
        assert_eq!(
            zeta.column_diffs
                .iter()
                .map(|diff| diff.name.as_str())
                .collect::<Vec<_>>(),
            vec!["a_column", "m_column", "z_column"]
        );
    }

    #[test]
    fn cross_database_compare_treats_mapped_column_types_as_equivalent() {
        let source_database_type = DatabaseType::MySQL;
        let target_database_type = DatabaseType::PostgreSQL;
        let result = compare_schemas_with_type_mapping(
            vec![table(
                "users",
                vec![
                    column("id", "INT", false),
                    column("balance", "DECIMAL(10,2)", false),
                    column("name", "VARCHAR(255)", false),
                ],
            )],
            vec![table(
                "users",
                vec![
                    column("id", "INTEGER", false),
                    column("balance", "NUMERIC(10,2)", false),
                    column("name", "CHARACTER VARYING(255)", false),
                ],
            )],
            SchemaCompareOptions::default(),
            Some(SchemaTypeMappingContext::new(
                &source_database_type,
                &target_database_type,
            )),
        )
        .unwrap();

        assert!(result.table_diffs.is_empty());
    }

    #[test]
    fn cross_database_compare_keeps_precision_changes() {
        let source_database_type = DatabaseType::MySQL;
        let target_database_type = DatabaseType::PostgreSQL;
        let result = compare_schemas_with_type_mapping(
            vec![table(
                "accounts",
                vec![column("balance", "DECIMAL(10,2)", false)],
            )],
            vec![table(
                "accounts",
                vec![column("balance", "NUMERIC(12,2)", false)],
            )],
            SchemaCompareOptions::default(),
            Some(SchemaTypeMappingContext::new(
                &source_database_type,
                &target_database_type,
            )),
        )
        .unwrap();

        assert_eq!(result.modified_count, 1);
        assert_eq!(
            result.table_diffs[0].column_diffs[0].changes,
            vec!["type: NUMERIC(12,2) → DECIMAL(10,2)"]
        );
    }

    #[test]
    fn legacy_compare_does_not_apply_cross_database_type_aliases() {
        let result = compare_schemas(
            vec![table("users", vec![column("id", "INT", false)])],
            vec![table("users", vec![column("id", "INTEGER", false)])],
            SchemaCompareOptions::default(),
        )
        .unwrap();

        assert_eq!(result.modified_count, 1);
    }

    #[test]
    fn cross_database_compare_does_not_equate_sql_server_rowversion_with_timestamp() {
        let source_database_type = DatabaseType::MSSQL;
        let target_database_type = DatabaseType::PostgreSQL;
        let result = compare_schemas_with_type_mapping(
            vec![table("events", vec![column("version", "TIMESTAMP", false)])],
            vec![table("events", vec![column("version", "TIMESTAMP", false)])],
            SchemaCompareOptions::default(),
            Some(SchemaTypeMappingContext::new(
                &source_database_type,
                &target_database_type,
            )),
        )
        .unwrap();

        assert_eq!(result.modified_count, 1);
    }
}
