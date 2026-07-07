use super::{
    ColumnDiff, ColumnSchema, DiffStatus, ForeignKeyDiff, ForeignKeySchema, IndexDiff, IndexSchema,
    SchemaCompareError, SchemaCompareOptions, SchemaCompareResult, TableDiff, TableSchema,
};
use std::collections::{HashMap, HashSet};

/// 比较源端和目标端的表结构
pub fn compare_schemas(
    source_tables: Vec<TableSchema>,
    target_tables: Vec<TableSchema>,
    options: SchemaCompareOptions,
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
            source,
            target: None,
            column_diffs: vec![],
            index_diffs: vec![],
            foreign_key_diffs: vec![],
            comment_changed: false,
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
            source: None,
            target,
            column_diffs: vec![],
            index_diffs: vec![],
            foreign_key_diffs: vec![],
            comment_changed: false,
        });
    }

    // 共同的表（可能修改）
    for name in source_names.intersection(&target_names) {
        let source_table = &source_map[name];
        let target_table = &target_map[name];

        if let Some(diff) = compare_table(source_table, target_table, &options) {
            table_diffs.push(diff);
        }
    }

    let added_count = table_diffs
        .iter()
        .filter(|d| matches!(d.status, DiffStatus::Added))
        .count();
    let removed_count = table_diffs
        .iter()
        .filter(|d| matches!(d.status, DiffStatus::Removed))
        .count();
    let modified_count = table_diffs
        .iter()
        .filter(|d| matches!(d.status, DiffStatus::Modified))
        .count();

    Ok(SchemaCompareResult {
        table_diffs,
        added_count,
        removed_count,
        modified_count,
    })
}

/// 比较单个表
fn compare_table(
    source: &TableSchema,
    target: &TableSchema,
    options: &SchemaCompareOptions,
) -> Option<TableDiff> {
    let column_diffs = compare_columns_with_options(&source.columns, &target.columns, options);
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

    let has_changes = !column_diffs.is_empty()
        || !index_diffs.is_empty()
        || !foreign_key_diffs.is_empty()
        || comment_changed
        || table_options_changed;

    if has_changes {
        Some(TableDiff {
            name: source.name.clone(),
            status: DiffStatus::Modified,
            source: Some(source.clone()),
            target: Some(target.clone()),
            column_diffs,
            index_diffs,
            foreign_key_diffs,
            comment_changed,
        })
    } else {
        None
    }
}

/// 比较列
#[cfg(test)]
fn compare_columns(source: &[ColumnSchema], target: &[ColumnSchema]) -> Vec<ColumnDiff> {
    compare_columns_with_options(source, target, &SchemaCompareOptions::default())
}

fn compare_columns_with_options(
    source: &[ColumnSchema],
    target: &[ColumnSchema],
    options: &SchemaCompareOptions,
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
            source: None,
            target: Some(target),
        });
    }

    // 修改列
    for name in source_names.intersection(&target_names) {
        let src = source_map[name];
        let tgt = target_map[name];

        if !column_eq(src, tgt, options) {
            diffs.push(ColumnDiff {
                name: src.name.clone(),
                status: DiffStatus::Modified,
                source: Some((*src).clone()),
                target: Some((*tgt).clone()),
            });
        }
    }

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
            source: Some(source),
            target: None,
        });
    }

    for name in target_names.difference(&source_names) {
        let target = (*target_map[name]).clone();
        diffs.push(IndexDiff {
            name: target.name.clone(),
            status: DiffStatus::Removed,
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
                source: Some((*source_index).clone()),
                target: Some((*target_index).clone()),
            });
        }
    }

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
            source: Some(source),
            target: None,
        });
    }

    for name in target_names.difference(&source_names) {
        let target = (*target_map[name]).clone();
        diffs.push(ForeignKeyDiff {
            name: target.name.clone(),
            status: DiffStatus::Removed,
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
                source: Some((*source_fk).clone()),
                target: Some((*target_fk).clone()),
            });
        }
    }

    diffs
}

fn identifier_key(value: &str, options: &SchemaCompareOptions) -> String {
    if options.case_sensitive_identifiers {
        value.trim().to_string()
    } else {
        value.trim().to_lowercase()
    }
}

fn column_eq(left: &ColumnSchema, right: &ColumnSchema, options: &SchemaCompareOptions) -> bool {
    data_type_eq(&left.data_type, &right.data_type, options)
        && left.nullable == right.nullable
        && left.default_value == right.default_value
        && (options.ignore_comments || left.comment == right.comment)
        && (options.ignore_charset_collation
            || (normalized_metadata(left.charset.as_deref())
                == normalized_metadata(right.charset.as_deref())
                && normalized_metadata(left.collation.as_deref())
                    == normalized_metadata(right.collation.as_deref())))
}

fn data_type_eq(left: &str, right: &str, options: &SchemaCompareOptions) -> bool {
    normalized_data_type(left, options).eq_ignore_ascii_case(&normalized_data_type(right, options))
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
    fn test_foreign_key_action_changes_are_modified() {
        let mut source = table("orders", vec![column("id", "int", false)]);
        source.foreign_keys = vec![ForeignKeySchema {
            name: "fk_orders_user".to_string(),
            columns: vec!["user_id".to_string()],
            ref_table: "users".to_string(),
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
}
