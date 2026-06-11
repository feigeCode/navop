use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 数据比较的单元格值
pub type CellValue = serde_json::Value;

/// 数据比较的行数据（列名 -> 值）
pub type RowData = HashMap<String, CellValue>;

/// 数据比较的键值映射（键列名 -> 值）
pub type KeyValues = HashMap<String, CellValue>;

/// 数据比较结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataCompareResult {
    /// 源表名
    pub source_table: String,
    /// 目标表名
    pub target_table: String,
    /// 键列名列表
    pub key_columns: Vec<String>,
    /// 比较的列名列表
    pub columns: Vec<String>,
    /// 新增行（源端存在、目标端不存在）
    pub added: Vec<RowData>,
    /// 删除行（目标端存在、源端不存在）
    pub removed: Vec<RowData>,
    /// 修改行（两端键相同但非键列值不同）
    pub modified: Vec<DataCompareModifiedRow>,
    /// 源端是否被截断（未全量比较）
    pub source_truncated: bool,
    /// 目标端是否被截断（未全量比较）
    pub target_truncated: bool,
}

/// 修改行的详细信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataCompareModifiedRow {
    /// 键列值（用于定位行）
    pub key_values: KeyValues,
    /// 源端整行数据
    pub source_values: RowData,
    /// 目标端整行数据
    pub target_values: RowData,
    /// 发生变化的列（列名 -> (源值, 目标值)）
    pub changes: HashMap<String, (CellValue, CellValue)>,
}

/// 数据比较错误
#[derive(Debug, thiserror::Error)]
pub enum DataCompareError {
    #[error("键列不能为空")]
    EmptyKeyColumns,

    #[error("键列 {0} 不存在于比较列中")]
    KeyColumnNotFound(String),

    #[error("源端存在重复键: {0}")]
    DuplicateSourceKey(String),

    #[error("目标端存在重复键: {0}")]
    DuplicateTargetKey(String),

    #[error("无共同列可比较")]
    NoCommonColumns,

    #[error("查询失败: {0}")]
    QueryError(String),

    #[error("数据转换失败: {0}")]
    ConversionError(String),
}
