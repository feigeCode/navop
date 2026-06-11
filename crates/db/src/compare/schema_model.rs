use serde::{Deserialize, Serialize};

/// 列定义
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnSchema {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default_value: Option<String>,
    pub comment: Option<String>,
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
    pub ref_columns: Vec<String>,
}

/// 表结构
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableSchema {
    pub name: String,
    pub columns: Vec<ColumnSchema>,
    pub indexes: Vec<IndexSchema>,
    pub foreign_keys: Vec<ForeignKeySchema>,
    pub comment: Option<String>,
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
    pub source: Option<ColumnSchema>,
    pub target: Option<ColumnSchema>,
}

/// 索引差异
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexDiff {
    pub name: String,
    pub status: DiffStatus,
    pub source: Option<IndexSchema>,
    pub target: Option<IndexSchema>,
}

/// 外键差异
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForeignKeyDiff {
    pub name: String,
    pub status: DiffStatus,
    pub source: Option<ForeignKeySchema>,
    pub target: Option<ForeignKeySchema>,
}

/// 表差异
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableDiff {
    pub name: String,
    pub status: DiffStatus,
    pub source: Option<TableSchema>,
    pub target: Option<TableSchema>,
    pub column_diffs: Vec<ColumnDiff>,
    pub index_diffs: Vec<IndexDiff>,
    pub foreign_key_diffs: Vec<ForeignKeyDiff>,
    pub comment_changed: bool,
}

/// 结构比较结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaCompareResult {
    pub table_diffs: Vec<TableDiff>,
    pub added_count: usize,
    pub removed_count: usize,
    pub modified_count: usize,
}

/// 结构比较选项
#[derive(Debug, Clone)]
pub struct SchemaCompareOptions {
    pub ignore_comments: bool,
}

impl Default for SchemaCompareOptions {
    fn default() -> Self {
        Self {
            ignore_comments: false,
        }
    }
}

/// 结构比较错误
#[derive(Debug, thiserror::Error)]
pub enum SchemaCompareError {
    #[error("无效的表结构")]
    InvalidSchema,
}
