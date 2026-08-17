use serde::{Deserialize, Serialize};

/// 比较任务事件
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CompareTaskEvent {
    /// 任务开始
    Started {
        task_id: String,
        total_tables: usize,
    },
    /// 表开始比较
    TableStarted {
        table: String,
        table_index: usize,
        total_tables: usize,
    },
    /// 加载元数据
    LoadingMetadata { table: Option<String> },
    /// 加载用于同步排序的表依赖元数据
    LoadingDependencyMetadata { table: Option<String> },
    /// 加载结构比较一侧的表列表
    LoadingTableList { side: CompareSchemaSide },
    /// 加载结构比较一侧的单表结构
    LoadingTableSchema {
        side: CompareSchemaSide,
        table: String,
        table_index: usize,
        total_tables: usize,
    },
    /// 比较已经加载的结构
    ComparingSchema,
    /// 计数行数
    CountingRows { table: String, side: CompareRowSide },
    /// 读取行
    FetchingRows {
        table: String,
        side: CompareRowSide,
        fetched_rows: usize,
        total_rows: Option<usize>,
    },
    /// 比较行
    ComparingRows { table: String },
    /// 生成同步计划
    PlanningSql { table: Option<String> },
    /// 表完成
    TableFinished {
        table: String,
        added: usize,
        removed: usize,
        modified: usize,
    },
    /// 错误
    Error {
        table: Option<String>,
        message: String,
    },
    /// 任务完成
    Finished { elapsed_ms: u64 },
}

/// 比较的源端或目标端
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareRowSide {
    Source,
    Target,
}

/// 结构比较的源端或目标端
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareSchemaSide {
    Source,
    Target,
}

/// 比较任务状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareTaskStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// 比较任务信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompareTaskInfo {
    pub id: String,
    pub status: CompareTaskStatus,
    pub created_at: u64,
    pub completed_at: Option<u64>,
}
