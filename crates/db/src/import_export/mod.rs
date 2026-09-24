use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::connection::DbConnection;

pub mod formats;
mod task;

pub use task::{ExportProgressRequest, ImportProgressRequest};

use crate::DatabasePlugin;

/// 数据格式枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataFormat {
    Sql,
    Json,
    Csv,
    Txt,
    Xml,
}

impl DataFormat {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "sql" => Some(Self::Sql),
            "json" => Some(Self::Json),
            "csv" => Some(Self::Csv),
            "txt" => Some(Self::Txt),
            "xml" => Some(Self::Xml),
            _ => None,
        }
    }

    pub fn extension(&self) -> &str {
        match self {
            Self::Sql => "sql",
            Self::Json => "json",
            Self::Csv => "csv",
            Self::Txt => "txt",
            Self::Xml => "xml",
        }
    }
}

/// CSV导入配置
#[derive(Debug, Clone)]
pub struct CsvImportConfig {
    pub field_delimiter: char,
    pub text_qualifier: Option<char>,
    pub has_header: bool,
    pub record_terminator: String,
    /// 未加引号时表示 SQL NULL 的字符串。
    pub null_string: String,
}

impl Default for CsvImportConfig {
    fn default() -> Self {
        Self {
            field_delimiter: ',',
            text_qualifier: Some('"'),
            has_header: true,
            record_terminator: "\n".to_string(),
            null_string: "\\N".to_string(),
        }
    }
}

/// CSV/TXT导出配置
#[derive(Debug, Clone)]
pub struct CsvExportConfig {
    pub field_delimiter: char,
    pub text_qualifier: Option<char>,
    pub include_header: bool,
    pub record_terminator: String,
    /// SQL NULL 的导出表示；空字符串仍使用带引号的空字段。
    pub null_string: String,
}

impl Default for CsvExportConfig {
    fn default() -> Self {
        Self {
            field_delimiter: ',',
            text_qualifier: Some('"'),
            include_header: true,
            record_terminator: "\n".to_string(),
            null_string: "\\N".to_string(),
        }
    }
}

/// SQL 导出时每一条 INSERT 语句合并的默认数据行数。
///
/// 取值与 Navicat「每条语句的数据行数」默认值一致；设为 `1` 即回到
/// 「一行一条 INSERT 语句」的形态。
pub const DEFAULT_ROWS_PER_INSERT_STATEMENT: usize = 100;

/// 每一条 INSERT 语句可合并的数据行数上限。
///
/// 批量导出时一页最多取一条语句所需的行数，因此这个上限同时约束了导出期间
/// 单页驻留内存（`max(SQL_EXPORT_PAGE_SIZE, 上限)` 行）。10 万行一条语句已经
/// 是几十 MB 级别的文本，再大对导出文件没有实际收益。
pub const MAX_ROWS_PER_INSERT_STATEMENT: usize = 100_000;

/// 把「每条语句的数据行数」规整到受支持的区间 `1..=MAX_ROWS_PER_INSERT_STATEMENT`。
pub fn normalize_rows_per_statement(rows_per_statement: usize) -> usize {
    rows_per_statement.clamp(1, MAX_ROWS_PER_INSERT_STATEMENT)
}

/// 导入配置
#[derive(Debug, Clone)]
pub struct ImportConfig {
    pub format: DataFormat,
    pub database: String,
    pub schema: Option<String>,
    pub table: Option<String>,
    pub stop_on_error: bool,
    pub use_transaction: bool,
    pub truncate_before_import: bool,
    pub csv_config: Option<CsvImportConfig>,
}

impl Default for ImportConfig {
    fn default() -> Self {
        Self {
            format: DataFormat::Sql,
            database: String::new(),
            schema: None,
            table: None,
            stop_on_error: true,
            use_transaction: true,
            truncate_before_import: false,
            csv_config: None,
        }
    }
}

/// 导出配置
#[derive(Debug, Clone)]
pub struct ExportConfig {
    pub format: DataFormat,
    pub database: String,
    pub schema: Option<String>,
    pub tables: Vec<String>,
    pub columns: Option<Vec<String>>,
    pub include_schema: bool,
    pub include_data: bool,
    pub where_clause: Option<String>,
    pub limit: Option<usize>,
    /// SQL 导出时每一条 INSERT 语句合并的数据行数，`1` 表示一行一条语句。
    ///
    /// 仅 [`DataFormat::Sql`] 使用；其他格式忽略该值。
    pub rows_per_statement: usize,
    pub csv_config: Option<CsvExportConfig>,
}

impl Default for ExportConfig {
    fn default() -> Self {
        Self {
            format: DataFormat::Sql,
            database: String::new(),
            schema: None,
            tables: Vec::new(),
            columns: None,
            include_schema: true,
            include_data: true,
            where_clause: None,
            limit: None,
            rows_per_statement: DEFAULT_ROWS_PER_INSERT_STATEMENT,
            csv_config: None,
        }
    }
}

/// 导入结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResult {
    pub success: bool,
    pub rows_imported: u64,
    pub errors: Vec<String>,
    pub elapsed_ms: u128,
}

/// 导出结果
#[derive(Debug, Clone)]
pub struct ExportResult {
    pub success: bool,
    pub output: String,
    pub rows_exported: u64,
    pub elapsed_ms: u128,
}

/// 导出进度事件
#[derive(Debug, Clone)]
pub enum ExportProgressEvent {
    HeaderExported {
        data: String,
    },
    TableStart {
        table: String,
        table_index: usize,
        total_tables: usize,
    },
    GettingStructure {
        table: String,
    },
    StructureExported {
        table: String,
        data: String,
    },
    FetchingData {
        table: String,
    },
    DataExported {
        table: String,
        rows: u64,
        data: String,
    },
    TableFinished {
        table: String,
    },
    Error {
        table: String,
        message: String,
    },
    Finished {
        total_rows: u64,
        elapsed_ms: u128,
    },
}

/// 导出进度发送器类型
pub type ExportProgressSender = mpsc::UnboundedSender<ExportProgressEvent>;

/// 导入进度事件
#[derive(Debug, Clone)]
pub enum ImportProgressEvent {
    FileStart {
        file: String,
        file_index: usize,
        total_files: usize,
    },
    ReadingFile {
        file: String,
    },
    ParsingFile {
        file: String,
    },
    ExecutingStatement {
        file: String,
        statement_index: usize,
        total_statements: usize,
    },
    StatementExecuted {
        file: String,
        rows_affected: u64,
    },
    FileFinished {
        file: String,
        rows_imported: u64,
    },
    Error {
        file: String,
        message: String,
    },
    Finished {
        total_rows: u64,
        elapsed_ms: u128,
    },
}

/// 导入进度发送器类型
pub type ImportProgressSender = mpsc::UnboundedSender<ImportProgressEvent>;

/// 格式处理器trait
#[async_trait]
pub trait FormatHandler: Send + Sync {
    /// 导入数据
    async fn import(
        &self,
        plugin: &dyn DatabasePlugin,
        connection: &dyn DbConnection,
        config: &ImportConfig,
        data: &str,
    ) -> Result<ImportResult>;

    /// 导入数据（带进度回调）
    async fn import_with_progress(
        &self,
        plugin: &dyn DatabasePlugin,
        connection: &dyn DbConnection,
        config: &ImportConfig,
        data: &str,
        file_name: &str,
        progress_tx: Option<ImportProgressSender>,
    ) -> Result<ImportResult> {
        let _ = (file_name, progress_tx);
        self.import(plugin, connection, config, data).await
    }

    /// 导出数据
    async fn export(
        &self,
        plugin: &dyn DatabasePlugin,
        connection: &dyn DbConnection,
        config: &ExportConfig,
    ) -> Result<ExportResult>;

    /// 导出数据（带进度回调）
    async fn export_with_progress(
        &self,
        plugin: &dyn DatabasePlugin,
        connection: &dyn DbConnection,
        config: &ExportConfig,
        progress_tx: Option<ExportProgressSender>,
    ) -> Result<ExportResult> {
        let _ = progress_tx;
        self.export(plugin, connection, config).await
    }
}
