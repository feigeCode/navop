use serde::{Deserialize, Serialize};

use super::{CompareSchemaSide, RoutineDiff, TriggerDiff};

/// Schema object kind supported by the compare model.
///
/// The metadata layer may expose views alongside tables. Keeping the kind in
/// the compare model prevents a view from silently being treated as a table
/// and receiving destructive table DDL.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchemaObjectType {
    #[default]
    Table,
    View,
}

/// 列定义
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnSchema {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default_value: Option<String>,
    pub comment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collation: Option<String>,
}

/// 索引定义
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexSchema {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
}

/// 外键定义
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForeignKeySchema {
    pub name: String,
    pub columns: Vec<String>,
    pub ref_table: String,
    /// Schema containing the referenced table, when it differs from the target table schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ref_schema: Option<String>,
    pub ref_columns: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_delete: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_update: Option<String>,
}

/// 表结构
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableSchema {
    pub name: String,
    #[serde(default)]
    pub object_type: SchemaObjectType,
    pub columns: Vec<ColumnSchema>,
    pub indexes: Vec<IndexSchema>,
    pub foreign_keys: Vec<ForeignKeySchema>,
    pub comment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collation: Option<String>,
}

/// 差异状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffStatus {
    Added,
    Removed,
    Modified,
}

/// 列差异
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnDiff {
    pub name: String,
    pub status: DiffStatus,
    #[serde(default)]
    pub changes: Vec<String>,
    pub source: Option<ColumnSchema>,
    pub target: Option<ColumnSchema>,
}

/// 索引差异
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexDiff {
    pub name: String,
    pub status: DiffStatus,
    #[serde(default)]
    pub changes: Vec<String>,
    pub source: Option<IndexSchema>,
    pub target: Option<IndexSchema>,
}

/// 外键差异
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForeignKeyDiff {
    pub name: String,
    pub status: DiffStatus,
    #[serde(default)]
    pub changes: Vec<String>,
    pub source: Option<ForeignKeySchema>,
    pub target: Option<ForeignKeySchema>,
}

/// 表差异
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableDiff {
    pub name: String,
    pub status: DiffStatus,
    #[serde(default)]
    pub object_type: SchemaObjectType,
    #[serde(default)]
    pub changes: Vec<String>,
    pub source: Option<TableSchema>,
    pub target: Option<TableSchema>,
    pub column_diffs: Vec<ColumnDiff>,
    pub index_diffs: Vec<IndexDiff>,
    pub foreign_key_diffs: Vec<ForeignKeyDiff>,
    pub comment_changed: bool,
    #[serde(default)]
    pub table_options_changed: bool,
}

/// 结构比较结果
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SchemaCompareResult {
    #[serde(default)]
    pub table_diffs: Vec<TableDiff>,
    /// Function/procedure differences are read-only. The sync planner must
    /// explicitly skip them until routine DDL generation is implemented.
    #[serde(default)]
    pub routine_diffs: Vec<RoutineDiff>,
    /// Trigger differences are read-only. The sync planner must explicitly
    /// skip them until trigger DDL generation is implemented.
    #[serde(default)]
    pub trigger_diffs: Vec<TriggerDiff>,
    /// 单表元数据读取失败。存在失败时，差异结果仅供查看，不能安全生成同步 SQL。
    #[serde(default)]
    pub table_failures: Vec<SchemaCompareTableFailure>,
    #[serde(default)]
    pub added_count: usize,
    #[serde(default)]
    pub removed_count: usize,
    #[serde(default)]
    pub modified_count: usize,
}

impl SchemaCompareResult {
    pub fn has_failed_tables(&self) -> bool {
        !self.table_failures.is_empty()
    }

    pub fn total_diff_count(&self) -> usize {
        self.table_diffs.len() + self.routine_diffs.len() + self.trigger_diffs.len()
    }

    pub fn refresh_counts(&mut self) {
        self.added_count = self
            .table_diffs
            .iter()
            .map(|diff| diff.status)
            .chain(self.routine_diffs.iter().map(|diff| diff.status))
            .chain(self.trigger_diffs.iter().map(|diff| diff.status))
            .filter(|status| *status == DiffStatus::Added)
            .count();
        self.removed_count = self
            .table_diffs
            .iter()
            .map(|diff| diff.status)
            .chain(self.routine_diffs.iter().map(|diff| diff.status))
            .chain(self.trigger_diffs.iter().map(|diff| diff.status))
            .filter(|status| *status == DiffStatus::Removed)
            .count();
        self.modified_count = self
            .table_diffs
            .iter()
            .map(|diff| diff.status)
            .chain(self.routine_diffs.iter().map(|diff| diff.status))
            .chain(self.trigger_diffs.iter().map(|diff| diff.status))
            .filter(|status| *status == DiffStatus::Modified)
            .count();
    }
}

#[cfg(test)]
mod result_tests {
    use super::*;
    use crate::compare::{RoutineKind, RoutineSchema, TriggerSchema};

    #[test]
    fn schema_compare_result_counts_all_supported_object_kinds() {
        let mut result = SchemaCompareResult {
            table_diffs: vec![TableDiff {
                name: "users".to_string(),
                status: DiffStatus::Added,
                object_type: SchemaObjectType::Table,
                changes: Vec::new(),
                source: None,
                target: None,
                column_diffs: Vec::new(),
                index_diffs: Vec::new(),
                foreign_key_diffs: Vec::new(),
                comment_changed: false,
                table_options_changed: false,
            }],
            routine_diffs: vec![RoutineDiff {
                name: "calculate".to_string(),
                kind: RoutineKind::Function,
                status: DiffStatus::Modified,
                changes: vec!["definition changed".to_string()],
                source: Some(RoutineSchema::default()),
                target: Some(RoutineSchema::default()),
            }],
            trigger_diffs: vec![TriggerDiff {
                name: "audit".to_string(),
                status: DiffStatus::Removed,
                changes: Vec::new(),
                source: None,
                target: Some(TriggerSchema::default()),
            }],
            ..Default::default()
        };

        result.refresh_counts();

        assert_eq!(result.total_diff_count(), 3);
        assert_eq!(result.added_count, 1);
        assert_eq!(result.removed_count, 1);
        assert_eq!(result.modified_count, 1);
    }
}

/// 结构比较中的单表元数据读取失败。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaCompareTableFailure {
    pub side: CompareSchemaSide,
    pub table: String,
    pub error: String,
}

/// 结构比较选项
#[derive(Debug, Clone)]
pub struct SchemaCompareOptions {
    pub ignore_comments: bool,
    pub case_sensitive_identifiers: bool,
    pub ignore_auto_increment: bool,
    pub ignore_charset_collation: bool,
    pub ignore_table_options: bool,
    pub compare_indexes: bool,
    pub compare_foreign_keys: bool,
    pub compare_column_order: bool,
}

impl Default for SchemaCompareOptions {
    fn default() -> Self {
        Self {
            ignore_comments: false,
            case_sensitive_identifiers: false,
            ignore_auto_increment: false,
            ignore_charset_collation: false,
            ignore_table_options: false,
            compare_indexes: true,
            compare_foreign_keys: true,
            compare_column_order: false,
        }
    }
}

/// 结构比较错误
#[derive(Debug, thiserror::Error)]
pub enum SchemaCompareError {
    #[error("无效的表结构")]
    InvalidSchema,

    #[error("存在重复标识符: {0}")]
    DuplicateIdentifier(String),
}
