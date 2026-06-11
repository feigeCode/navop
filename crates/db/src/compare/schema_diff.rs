use super::{
    ColumnDiff, ColumnSchema, DiffStatus, ForeignKeyDiff, ForeignKeySchema, IndexDiff,
    IndexSchema, SchemaCompareError, SchemaCompareOptions, SchemaCompareResult, TableDiff,
    TableSchema,
};
use std::collections::{HashMap, HashSet};

/// 比较源端和目标端的表结构
pub fn compare_schemas(
    source_tables: Vec<TableSchema>,
    target_tables: Vec<TableSchema>,
    options: SchemaCompareOptions,
) -> Result<SchemaCompareResult, SchemaCompareError> {
    let source_map: HashMap<String, TableSchema> =
        source_tables.into_iter().map(|t| (t.name.clone(), t)).collect();
    let target_map: HashMap<String, TableSchema> =
        target_tables.into_iter().map(|t| (t.name.clone(), t)).collect();

    let source_names: HashSet<_> = source_map.keys().collect();
    let target_names: HashSet<_> = target_map.keys().collect();

    let mut table_diffs = Vec::new();

    // 源端独有的表（新增）
    for name in source_names.difference(&target_names) {
        table_diffs.push(TableDiff {
            name: (*name).clone(),
            status: DiffStatus::Added,
            column_diffs: vec![],
            index_diffs: vec![],
            foreign_key_diffs: vec![],
            comment_changed: false,
        });
    }

    // 目标端独有的表（删除）
    for name in target_names.difference(&source_names) {
        table_diffs.push(TableDiff {
            name: (*name).clone(),
            status: DiffStatus::Removed,
            column_diffs: vec![],
            index_diffs: vec![],
            foreign_key_diffs: vec![],
            comment_changed: false,
        });
    }

    // 共同的表（可能修改）
    for name in source_names.intersection(&target_names) {
        let source_table = &source_map[*name];
        let target_table = &target_map[*name];

        if let Some(diff) = compare_table(source_table, target_table, &options) {
            table_diffs.push(diff);
        }
    }

    let added_count = table_diffs.iter().filter(|d| matches!(d.status, DiffStatus::Added)).count();
    let removed_count = table_diffs.iter().filter(|d| matches!(d.status, DiffStatus::Removed)).count();
    let modified_count = table_diffs.iter().filter(|d| matches!(d.status, DiffStatus::Modified)).count();

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
    let column_diffs = compare_columns(&source.columns, &target.columns);
    let index_diffs = compare_indexes(&source.indexes, &target.indexes);
    let foreign_key_diffs = compare_foreign_keys(&source.foreign_keys, &target.foreign_keys);

    let comment_changed = if options.ignore_comments {
        false
    } else {
        source.comment != target.comment
    };

    let has_changes = !column_diffs.is_empty()
        || !index_diffs.is_empty()
        || !foreign_key_diffs.is_empty()
        || comment_changed;

    if has_changes {
        Some(TableDiff {
            name: source.name.clone(),
            status: DiffStatus::Modified,
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
fn compare_columns(source: &[ColumnSchema], target: &[ColumnSchema]) -> Vec<ColumnDiff> {
    let source_map: HashMap<_, _> = source.iter().map(|c| (&c.name, c)).collect();
    let target_map: HashMap<_, _> = target.iter().map(|c| (&c.name, c)).collect();

    let source_names: HashSet<_> = source_map.keys().collect();
    let target_names: HashSet<_> = target_map.keys().collect();

    let mut diffs = Vec::new();

    // 新增列
    for name in source_names.difference(&target_names) {
        diffs.push(ColumnDiff {
            name: (*name).to_string(),
            status: DiffStatus::Added,
            source: Some((*source_map[*name]).clone()),
            target: None,
        });
    }

    // 删除列
    for name in target_names.difference(&source_names) {
        diffs.push(ColumnDiff {
            name: (*name).to_string(),
            status: DiffStatus::Removed,
            source: None,
            target: Some((*target_map[*name]).clone()),
        });
    }

    // 修改列
    for name in source_names.intersection(&target_names) {
        let src = source_map[*name];
        let tgt = target_map[*name];

        if src.data_type != tgt.data_type
            || src.nullable != tgt.nullable
            || src.default_value != tgt.default_value
        {
            diffs.push(ColumnDiff {
                name: (*name).to_string(),
                status: DiffStatus::Modified,
                source: Some((*src).clone()),
                target: Some((*tgt).clone()),
            });
        }
    }

    diffs
}

/// 比较索引
fn compare_indexes(source: &[IndexSchema], target: &[IndexSchema]) -> Vec<IndexDiff> {
    let source_names: HashSet<_> = source.iter().map(|i| &i.name).collect();
    let target_names: HashSet<_> = target.iter().map(|i| &i.name).collect();

    let mut diffs = Vec::new();

    for name in source_names.difference(&target_names) {
        diffs.push(IndexDiff {
            name: (*name).to_string(),
            status: DiffStatus::Added,
        });
    }

    for name in target_names.difference(&source_names) {
        diffs.push(IndexDiff {
            name: (*name).to_string(),
            status: DiffStatus::Removed,
        });
    }

    diffs
}

/// 比较外键
fn compare_foreign_keys(
    source: &[ForeignKeySchema],
    target: &[ForeignKeySchema],
) -> Vec<ForeignKeyDiff> {
    let source_names: HashSet<_> = source.iter().map(|fk| &fk.name).collect();
    let target_names: HashSet<_> = target.iter().map(|fk| &fk.name).collect();

    let mut diffs = Vec::new();

    for name in source_names.difference(&target_names) {
        diffs.push(ForeignKeyDiff {
            name: (*name).to_string(),
            status: DiffStatus::Added,
        });
    }

    for name in target_names.difference(&source_names) {
        diffs.push(ForeignKeyDiff {
            name: (*name).to_string(),
            status: DiffStatus::Removed,
        });
    }

    diffs
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
        }
    }

    #[test]
    fn test_compare_schemas_detects_added_removed_modified() {
        let source = vec![
            TableSchema {
                name: "users".to_string(),
                columns: vec![column("id", "int", false), column("name", "varchar(64)", false)],
                indexes: vec![],
                foreign_keys: vec![],
                comment: None,
            },
            TableSchema {
                name: "orders".to_string(),
                columns: vec![column("id", "int", false)],
                indexes: vec![],
                foreign_keys: vec![],
                comment: None,
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
            },
            TableSchema {
                name: "audit".to_string(),
                columns: vec![column("id", "int", false)],
                indexes: vec![],
                foreign_keys: vec![],
                comment: None,
            },
        ];

        let result = compare_schemas(source, target, SchemaCompareOptions::default()).unwrap();

        assert_eq!(result.added_count, 1);
        assert_eq!(result.removed_count, 1);
        assert_eq!(result.modified_count, 1);

        let users_diff = result.table_diffs.iter().find(|d| d.name == "users").unwrap();
        assert_eq!(users_diff.status, DiffStatus::Modified);
        assert_eq!(users_diff.column_diffs.len(), 1);
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
        assert!(diffs.iter().any(|d| d.name == "email" && d.status == DiffStatus::Added));
        assert!(diffs.iter().any(|d| d.name == "phone" && d.status == DiffStatus::Removed));
        assert!(diffs.iter().any(|d| d.name == "name" && d.status == DiffStatus::Modified));
    }
}
