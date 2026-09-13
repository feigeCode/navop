use crate::types::FieldType;
use db_value::{
    CellState, ColumnDescriptor, DbValue, Nullability, ResultBatch, ResultRow, ValueModelError,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// SQL 脚本来源
#[derive(Clone, Debug)]
pub enum SqlSource {
    /// 直接的 SQL 脚本字符串
    Script(String),
    /// SQL 文件路径
    File(PathBuf),
}

impl SqlSource {
    pub fn file_size(&self) -> Option<u64> {
        match self {
            SqlSource::Script(s) => Some(s.len() as u64),
            SqlSource::File(path) => std::fs::metadata(path).ok().map(|m| m.len()),
        }
    }

    pub fn is_file(&self) -> bool {
        matches!(self, SqlSource::File(_))
    }
}

/// Execution options for SQL script
#[derive(Debug, Clone)]
pub struct ExecOptions {
    /// Whether to stop execution when encountering an error
    pub stop_on_error: bool,
    /// Whether to wrap the entire script in a transaction
    pub transactional: bool,
    /// Maximum number of rows to return for query results
    pub max_rows: Option<usize>,
    /// 是否启用流式执行（逐条解析执行，适合大文件/大脚本）
    /// 默认 false，会先解析所有语句再执行
    pub streaming: bool,
}

impl Default for ExecOptions {
    fn default() -> Self {
        Self {
            stop_on_error: true,
            transactional: false,
            max_rows: Some(1000),
            streaming: false,
        }
    }
}

/// Result of a single SQL statement execution
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SqlResult {
    /// Query result (SELECT, SHOW, etc.)
    Query(QueryResult),
    /// Execution result (INSERT, UPDATE, DELETE, DDL, etc.)
    Exec(ExecResult),
    /// Error result
    Error(SqlErrorInfo),
}

impl SqlResult {
    pub fn is_error(&self) -> bool {
        matches!(self, SqlResult::Error(_))
    }

    /// Restore the parser-provided statement as result metadata after execution-time rewriting.
    pub fn with_original_sql(mut self, sql: impl Into<String>) -> Self {
        let sql = sql.into();
        match &mut self {
            SqlResult::Query(result) => result.sql = sql,
            SqlResult::Exec(result) => result.sql = sql,
            SqlResult::Error(result) => result.sql = sql,
        }
        self
    }
}

/// Column metadata for query results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryColumnMeta {
    /// Column name
    pub name: String,
    /// Original database type (e.g., "VARCHAR(255)", "INT")
    pub db_type: String,
    /// Abstract field type for UI rendering
    pub field_type: FieldType,
    /// Whether the column is nullable
    pub nullable: bool,
    /// Character set used for text bytes in this result column.
    ///
    /// This is runtime/wire metadata and may differ from the table column's
    /// declared charset when the server applies `character_set_results`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_charset: Option<String>,
    /// Collation reported for this result column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_collation: Option<String>,
    /// Raw MySQL result-column collation ID, retained for diagnostics and
    /// forward compatibility when the local collation table is older.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_collation_id: Option<u16>,
}

impl QueryColumnMeta {
    pub fn new(name: impl Into<String>, db_type: impl Into<String>) -> Self {
        let db_type_str = db_type.into();
        let field_type = FieldType::from_db_type(&db_type_str);
        Self {
            name: name.into(),
            db_type: db_type_str,
            field_type,
            nullable: true,
            result_charset: None,
            result_collation: None,
            result_collation_id: None,
        }
    }

    pub fn with_nullable(mut self, nullable: bool) -> Self {
        self.nullable = nullable;
        self
    }

    pub fn with_result_encoding(
        mut self,
        charset: Option<impl Into<String>>,
        collation: Option<impl Into<String>>,
        collation_id: Option<u16>,
    ) -> Self {
        self.result_charset = charset.map(Into::into);
        self.result_collation = collation.map(Into::into);
        self.result_collation_id = collation_id;
        self
    }
}

/// Query result with data
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BinaryCell {
    /// Zero-based row index in [`QueryResult::rows`].
    pub row_index: usize,
    /// Zero-based column index in [`QueryResult::columns`].
    pub column_index: usize,
    /// Exact bytes returned by the database driver.
    pub bytes: Vec<u8>,
}

/// Query result with data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    /// Original SQL statement
    pub sql: String,
    /// Column names
    pub columns: Vec<String>,
    /// Column metadata with type information
    pub column_meta: Vec<QueryColumnMeta>,
    /// Row data (each row is a vector of optional strings)
    pub rows: Vec<Vec<Option<String>>>,
    /// Lossless binary values keyed by their row and column coordinates.
    ///
    /// `rows` remains string-based for compatibility with existing consumers; UI clients should
    /// prefer this sidecar whenever a matching cell exists.
    #[serde(default)]
    pub binary_cells: Vec<BinaryCell>,
    /// Execution time in milliseconds
    #[serde(with = "elapsed_ms_serde")]
    pub elapsed_ms: u128,
    /// 类型权威的 typed batch,进程内承载 [`db_value::ResultBatch`]。
    ///
    /// 只存在于内存中,不随 IPC/序列化传输;`rows`/`binary_cells` 仍是其
    /// legacy 兼容投影。通过 [`QueryResult::from_typed_batch`] 构造时填充。
    #[serde(skip)]
    pub typed_batch: Option<Arc<ResultBatch>>,
}

impl Default for QueryResult {
    fn default() -> Self {
        Self {
            sql: String::new(),
            columns: Vec::new(),
            column_meta: Vec::new(),
            rows: Vec::new(),
            binary_cells: Vec::new(),
            elapsed_ms: 0,
            typed_batch: None,
        }
    }
}

/// 把带类型的 batch 投影成 legacy 的 `rows` + `binary_cells`。
///
/// 这是 executor 与 IPC 共用的唯一 legacy 投影实现,避免两处漂移。`DateTime`
/// 归一到空格分隔,`Binary` 统一使用 [`db_value::format_binary_preview`] 的大写、
/// 有界 hex 预览并附带精确字节 sidecar。
/// `Undecoded`/`DecodeError` 无法无损投影时显式失败。
pub(crate) fn project_batch_to_legacy(
    batch: &ResultBatch,
) -> Result<(Vec<Vec<Option<String>>>, Vec<BinaryCell>), QueryResultValueError> {
    let mut rows = Vec::with_capacity(batch.rows.len());
    let mut binary_cells = Vec::new();

    for (row_index, row) in batch.rows.iter().enumerate() {
        let mut legacy_row = Vec::with_capacity(row.cells.len());
        for (column_index, cell) in row.cells.iter().enumerate() {
            match cell {
                CellState::Decoded(value) => {
                    if let DbValue::Binary(bytes) = value {
                        binary_cells.push(BinaryCell {
                            row_index,
                            column_index,
                            bytes: bytes.clone(),
                        });
                    }
                    legacy_row.push(legacy_text(value));
                }
                CellState::Undecoded { .. } | CellState::DecodeError { .. } => {
                    return Err(QueryResultValueError::UnsupportedCell {
                        row_index,
                        column_index,
                    });
                }
            }
        }
        rows.push(legacy_row);
    }

    Ok((rows, binary_cells))
}

fn legacy_text(value: &DbValue) -> Option<String> {
    match value {
        DbValue::Null => None,
        DbValue::DateTime(value) => Some(format_ipc_datetime(value)),
        DbValue::Binary(bytes) => Some(db_value::format_binary_preview(bytes)),
        other => Some(db_value::ResultBatch::display_value(other)),
    }
}

/// legacy 兼容投影保留的 datetime 展示:去掉 `T`/时区,秒后小数按需保留。
fn format_ipc_datetime(value: &str) -> String {
    if let Ok(datetime) = chrono::DateTime::parse_from_rfc3339(value) {
        return format_naive_datetime(datetime.naive_local());
    }

    let parsed = chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f"));
    match parsed {
        Ok(datetime) => format_naive_datetime(datetime),
        Err(_) => value.to_string(),
    }
}

fn format_naive_datetime(datetime: chrono::NaiveDateTime) -> String {
    let micros = datetime.and_utc().timestamp_subsec_micros();
    if micros == 0 {
        datetime.format("%Y-%m-%d %H:%M:%S").to_string()
    } else if micros % 1_000 == 0 {
        format!(
            "{}.{:03}",
            datetime.format("%Y-%m-%d %H:%M:%S"),
            micros / 1_000
        )
    } else {
        format!("{}.{:06}", datetime.format("%Y-%m-%d %H:%M:%S"), micros)
    }
}

impl QueryResult {
    /// Convert the compatibility representation into the typed value model.
    ///
    /// A legacy string without authoritative type metadata remains `LegacyText`.
    /// This method never parses hex previews or numeric strings.
    pub fn to_result_batch(&self) -> Result<ResultBatch, QueryResultValueError> {
        let view = self.typed_view().map_err(QueryResultValueError::Shape)?;
        let columns = self
            .columns
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let metadata = self.column_meta.get(index);
                ColumnDescriptor {
                    id: format!("column:{index}"),
                    label: name.clone(),
                    native_type: metadata
                        .map(|metadata| metadata.db_type.clone())
                        .unwrap_or_else(|| "UNKNOWN".to_string()),
                    logical_type: metadata
                        .map(|metadata| format!("{:?}", metadata.field_type))
                        .unwrap_or_else(|| "Unknown".to_string()),
                    nullable: metadata
                        .map(|metadata| {
                            if metadata.nullable {
                                Nullability::Yes
                            } else {
                                Nullability::No
                            }
                        })
                        .unwrap_or(Nullability::Unknown),
                    charset: metadata.and_then(|metadata| metadata.result_charset.clone()),
                    collation: metadata.and_then(|metadata| metadata.result_collation.clone()),
                    precision: None,
                    scale: None,
                }
            })
            .collect::<Vec<_>>();

        let rows = (0..self.rows.len())
            .map(|row_index| {
                let cells = (0..self.columns.len())
                    .map(|column_index| match view.cell(row_index, column_index) {
                        Some(QueryCellRef::Null) => CellState::Decoded(DbValue::Null),
                        Some(QueryCellRef::Binary(bytes)) => {
                            CellState::Decoded(DbValue::Binary(bytes.to_vec()))
                        }
                        Some(QueryCellRef::Text(text)) => {
                            CellState::Decoded(DbValue::LegacyText(text.to_string()))
                        }
                        None => CellState::Decoded(DbValue::Null),
                    })
                    .collect();
                ResultRow {
                    id: row_index as u64,
                    cells,
                }
            })
            .collect();

        ResultBatch::try_new(0, columns, rows, true).map_err(QueryResultValueError::Model)
    }

    /// Build the legacy projection consumed by older UI and serialization code.
    ///
    /// 投影不填充 [`Self::typed_batch`];这只是一种从 typed batch 重建 legacy
    /// 表示的兼容入口。
    pub fn from_result_batch(
        sql: String,
        batch: &ResultBatch,
        elapsed_ms: u128,
    ) -> Result<Self, QueryResultValueError> {
        let columns = batch
            .columns
            .iter()
            .map(|column| column.label.clone())
            .collect();
        let column_meta = batch
            .columns
            .iter()
            .map(|column| QueryColumnMeta::new(&column.label, &column.native_type))
            .collect();
        let (rows, binary_cells) = project_batch_to_legacy(batch)?;

        Ok(Self {
            sql,
            columns,
            column_meta,
            rows,
            binary_cells,
            elapsed_ms,
            typed_batch: None,
        })
    }

    /// 由 typed batch 承载结果:填充 legacy 投影并保留 [`Self::typed_batch`]。
    ///
    /// `batch` 是类型权威;`columns`/`column_meta`/`rows`/`binary_cells` 由它派生。
    pub fn from_typed_batch(
        sql: String,
        batch: ResultBatch,
        elapsed_ms: u128,
    ) -> Result<Self, QueryResultValueError> {
        let columns = batch
            .columns
            .iter()
            .map(|column| column.label.clone())
            .collect::<Vec<_>>();
        let column_meta = batch
            .columns
            .iter()
            .map(|column| {
                QueryColumnMeta::new(&column.label, &column.native_type)
                    .with_result_encoding(column.charset.clone(), column.collation.clone(), None)
                    .with_nullable(column.nullable == Nullability::Yes)
            })
            .collect::<Vec<_>>();
        let (rows, binary_cells) = project_batch_to_legacy(&batch)?;

        Ok(Self {
            sql,
            columns,
            column_meta,
            rows,
            binary_cells,
            elapsed_ms,
            typed_batch: Some(Arc::new(batch)),
        })
    }

    /// 访问进程内承载的 typed batch(如有)。
    pub fn typed_batch(&self) -> Option<&ResultBatch> {
        self.typed_batch.as_deref()
    }

    /// 丢弃进程内承载的 typed batch,仅保留 legacy 投影。
    pub fn invalidate_typed_batch(&mut self) {
        self.typed_batch = None;
    }
}

#[derive(Debug, thiserror::Error)]
pub enum QueryResultValueError {
    #[error("invalid legacy query result: {0}")]
    Shape(#[from] QueryResultError),
    #[error("invalid typed result batch: {0}")]
    Model(#[from] ValueModelError),
    #[error("cell ({row_index}, {column_index}) cannot be projected to the legacy result")]
    UnsupportedCell {
        row_index: usize,
        column_index: usize,
    },
}

#[cfg(test)]
mod typed_value_tests {
    use super::*;
    use db_value::{CellState, DbValue};

    fn result(rows: Vec<Vec<Option<&str>>>, binary_cells: Vec<BinaryCell>) -> QueryResult {
        QueryResult {
            sql: "select value from example".to_string(),
            columns: vec!["value".to_string()],
            column_meta: vec![QueryColumnMeta::new("value", "TEXT")],
            rows: rows
                .into_iter()
                .map(|row| {
                    row.into_iter()
                        .map(|value| value.map(str::to_string))
                        .collect()
                })
                .collect(),
            binary_cells,
            elapsed_ms: 0,
            ..Default::default()
        }
    }

    #[test]
    fn legacy_adapter_does_not_guess_text_or_numeric_types() {
        let result = result(vec![vec![Some("123")]], Vec::new());
        let batch = result.to_result_batch().unwrap();

        assert_eq!(
            batch.rows[0].cells[0],
            CellState::Decoded(DbValue::LegacyText("123".to_string()))
        );
    }

    #[test]
    fn legacy_adapter_uses_binary_sidecar_as_authority() {
        let result = result(
            vec![vec![Some("0x68656C6C6F")]],
            vec![BinaryCell {
                row_index: 0,
                column_index: 0,
                bytes: b"hello".to_vec(),
            }],
        );
        let batch = result.to_result_batch().unwrap();

        assert_eq!(
            batch.rows[0].cells[0],
            CellState::Decoded(DbValue::Binary(b"hello".to_vec()))
        );
    }

    #[test]
    fn typed_projection_rejects_undecoded_values_in_legacy_result() {
        let batch = ResultBatch::try_new(
            1,
            vec![db_value::ColumnDescriptor {
                id: "value".to_string(),
                label: "value".to_string(),
                native_type: "CUSTOM".to_string(),
                logical_type: "unknown".to_string(),
                nullable: db_value::Nullability::Unknown,
                charset: None,
                collation: None,
                precision: None,
                scale: None,
            }],
            vec![db_value::ResultRow {
                id: 0,
                cells: vec![CellState::Undecoded {
                    native_type: "CUSTOM".to_string(),
                    raw: None,
                    reason: "codec unavailable".to_string(),
                }],
            }],
            true,
        )
        .unwrap();

        assert!(matches!(
            QueryResult::from_result_batch("select".to_string(), &batch, 0),
            Err(QueryResultValueError::UnsupportedCell { .. })
        ));
    }
}

/// A validated, typed view over one query result cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryCellRef<'a> {
    Null,
    Text(&'a str),
    Binary(&'a [u8]),
}

/// Structural errors that make a [`QueryResult`] ambiguous or unsafe to consume.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum QueryResultError {
    #[error("query result column metadata has length {actual}, expected {expected}")]
    ColumnMetaWidth { actual: usize, expected: usize },
    #[error("query result row {row_index} has width {actual}, expected {expected}")]
    RowWidth {
        row_index: usize,
        actual: usize,
        expected: usize,
    },
    #[error("binary cell ({row_index}, {column_index}) is out of bounds")]
    BinaryCellOutOfBounds {
        row_index: usize,
        column_index: usize,
    },
    #[error("duplicate binary cell ({row_index}, {column_index})")]
    DuplicateBinaryCell {
        row_index: usize,
        column_index: usize,
    },
}

/// Validated typed access to a [`QueryResult`].
///
/// Binary sidecar values are authoritative over the string display value stored in `rows`.
pub struct QueryResultView<'a> {
    result: &'a QueryResult,
    binary_by_cell: HashMap<(usize, usize), &'a [u8]>,
}

impl QueryResult {
    /// Validate the result shape once and build an indexed typed view for lossless cell access.
    pub fn typed_view(&self) -> Result<QueryResultView<'_>, QueryResultError> {
        let column_count = self.columns.len();
        if !self.column_meta.is_empty() && self.column_meta.len() != column_count {
            return Err(QueryResultError::ColumnMetaWidth {
                actual: self.column_meta.len(),
                expected: column_count,
            });
        }

        for (row_index, row) in self.rows.iter().enumerate() {
            if row.len() != column_count {
                return Err(QueryResultError::RowWidth {
                    row_index,
                    actual: row.len(),
                    expected: column_count,
                });
            }
        }

        let mut binary_by_cell = HashMap::with_capacity(self.binary_cells.len());
        for cell in &self.binary_cells {
            if cell.row_index >= self.rows.len() || cell.column_index >= column_count {
                return Err(QueryResultError::BinaryCellOutOfBounds {
                    row_index: cell.row_index,
                    column_index: cell.column_index,
                });
            }
            if binary_by_cell
                .insert((cell.row_index, cell.column_index), cell.bytes.as_slice())
                .is_some()
            {
                return Err(QueryResultError::DuplicateBinaryCell {
                    row_index: cell.row_index,
                    column_index: cell.column_index,
                });
            }
        }

        Ok(QueryResultView {
            result: self,
            binary_by_cell,
        })
    }
}

impl<'a> QueryResultView<'a> {
    pub fn cell(&self, row_index: usize, column_index: usize) -> Option<QueryCellRef<'a>> {
        if let Some(bytes) = self.binary_by_cell.get(&(row_index, column_index)) {
            return Some(QueryCellRef::Binary(bytes));
        }

        self.result
            .rows
            .get(row_index)?
            .get(column_index)?
            .as_deref()
            .map_or(Some(QueryCellRef::Null), |text| {
                Some(QueryCellRef::Text(text))
            })
    }
}

/// Execution result for non-query statements
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecResult {
    /// Original SQL statement
    pub sql: String,
    /// Number of rows affected
    pub rows_affected: u64,
    /// Execution time in milliseconds
    #[serde(with = "elapsed_ms_serde")]
    pub elapsed_ms: u128,
    /// Optional message
    pub message: Option<String>,
}

/// Error information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqlErrorInfo {
    /// Original SQL statement
    pub sql: String,
    /// Error message
    pub message: String,
}

mod elapsed_ms_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &u128, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64((*value).try_into().unwrap_or(u64::MAX))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<u128, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(u64::deserialize(deserializer)? as u128)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_result_deserializes_legacy_payload_without_binary_cells() {
        let result: QueryResult = serde_json::from_value(serde_json::json!({
            "sql": "select payload from files",
            "columns": ["payload"],
            "column_meta": [{
                "name": "payload",
                "db_type": "BLOB",
                "field_type": "Binary",
                "nullable": true
            }],
            "rows": [["0x010203"]],
            "elapsed_ms": 1
        }))
        .expect("legacy query result should remain compatible");

        assert!(result.binary_cells.is_empty());
        assert!(result.column_meta[0].result_charset.is_none());
        assert!(result.column_meta[0].result_collation.is_none());
        assert!(result.column_meta[0].result_collation_id.is_none());
    }

    #[test]
    fn query_result_typed_batch_is_skipped_on_serialize_and_deserializes_to_none() {
        let result = QueryResult {
            sql: "select payload from files".to_string(),
            columns: vec!["payload".to_string()],
            column_meta: vec![QueryColumnMeta::new("payload", "BLOB")],
            rows: vec![vec![Some("0x010203".to_string())]],
            binary_cells: vec![],
            elapsed_ms: 1,
            typed_batch: Some(Arc::new(
                ResultBatch::try_new(
                    0,
                    vec![ColumnDescriptor {
                        id: "column:0".to_string(),
                        label: "payload".to_string(),
                        native_type: "BLOB".to_string(),
                        logical_type: "Binary".to_string(),
                        nullable: Nullability::Yes,
                        charset: None,
                        collation: None,
                        precision: None,
                        scale: None,
                    }],
                    vec![ResultRow {
                        id: 0,
                        cells: vec![CellState::Decoded(DbValue::Binary(vec![1, 2, 3]))],
                    }],
                    true,
                )
                .unwrap(),
            )),
        };

        let encoded = serde_json::to_value(&result).expect("query result should serialize");
        assert!(encoded.get("typed_batch").is_none());

        let decoded: QueryResult =
            serde_json::from_value(encoded).expect("query result should deserialize");
        assert!(decoded.typed_batch.is_none());
    }

    #[test]
    fn from_typed_batch_carries_typed_batch_and_invalidates() {
        let batch = ResultBatch::try_new(
            0,
            vec![ColumnDescriptor {
                id: "column:0".to_string(),
                label: "value".to_string(),
                native_type: "TEXT".to_string(),
                logical_type: "Text".to_string(),
                nullable: Nullability::Yes,
                charset: None,
                collation: None,
                precision: None,
                scale: None,
            }],
            vec![ResultRow {
                id: 0,
                cells: vec![CellState::Decoded(DbValue::Text("hello".to_string()))],
            }],
            true,
        )
        .unwrap();

        let mut result = QueryResult::from_typed_batch("select value".to_string(), batch, 7)
            .expect("from_typed_batch should succeed");

        assert!(result.typed_batch().is_some());
        assert_eq!(result.rows, vec![vec![Some("hello".to_string())]]);
        assert_eq!(result.columns, vec!["value".to_string()]);
        assert_eq!(result.column_meta[0].db_type, "TEXT");

        result.invalidate_typed_batch();
        assert!(result.typed_batch.is_none());
        assert!(result.typed_batch().is_none());
    }

    #[test]
    fn query_column_meta_result_encoding_is_optional_and_roundtrips() {
        let meta = QueryColumnMeta::new("payload", "MYSQL_TYPE_LONG_BLOB").with_result_encoding(
            Some("gbk"),
            Some("gbk_chinese_ci"),
            Some(28),
        );

        let value = serde_json::to_value(&meta).expect("column metadata should serialize");
        assert_eq!(value["result_charset"], "gbk");
        assert_eq!(value["result_collation"], "gbk_chinese_ci");
        assert_eq!(value["result_collation_id"], 28);

        let decoded: QueryColumnMeta =
            serde_json::from_value(value).expect("column metadata should deserialize");
        assert_eq!(decoded.result_charset.as_deref(), Some("gbk"));
        assert_eq!(decoded.result_collation.as_deref(), Some("gbk_chinese_ci"));
        assert_eq!(decoded.result_collation_id, Some(28));
    }

    #[test]
    fn query_column_meta_omits_missing_result_encoding() {
        let value = serde_json::to_value(QueryColumnMeta::new("id", "BIGINT"))
            .expect("column metadata should serialize");

        assert!(value.get("result_charset").is_none());
        assert!(value.get("result_collation").is_none());
        assert!(value.get("result_collation_id").is_none());
    }

    #[test]
    fn binary_cell_roundtrips_exact_bytes() {
        let cell = BinaryCell {
            row_index: 2,
            column_index: 3,
            bytes: vec![0, 1, 2, 0xff],
        };

        let encoded = serde_json::to_string(&cell).expect("binary cell should serialize");
        let decoded: BinaryCell =
            serde_json::from_str(&encoded).expect("binary cell should deserialize");

        assert_eq!(decoded, cell);
    }

    fn typed_result() -> QueryResult {
        QueryResult {
            sql: "select a, b, c".to_string(),
            columns: vec!["a".to_string(), "b".to_string(), "c".to_string()],
            column_meta: vec![],
            rows: vec![vec![
                None,
                Some(String::new()),
                Some("binary display".to_string()),
            ]],
            binary_cells: vec![BinaryCell {
                row_index: 0,
                column_index: 2,
                bytes: vec![0, 0xff],
            }],
            elapsed_ms: 0,
            ..Default::default()
        }
    }

    #[test]
    fn typed_view_distinguishes_null_empty_text_and_binary() {
        let result = typed_result();
        let view = result.typed_view().expect("result should be valid");

        assert_eq!(view.cell(0, 0), Some(QueryCellRef::Null));
        assert_eq!(view.cell(0, 1), Some(QueryCellRef::Text("")));
        assert_eq!(view.cell(0, 2), Some(QueryCellRef::Binary(&[0_u8, 0xff])));
        assert_eq!(view.cell(1, 0), None);
        assert_eq!(view.cell(0, 3), None);
    }

    #[test]
    fn typed_view_rejects_duplicate_binary_coordinates() {
        let mut result = typed_result();
        result.binary_cells.push(BinaryCell {
            row_index: 0,
            column_index: 2,
            bytes: vec![1],
        });

        assert_eq!(
            result.typed_view().err(),
            Some(QueryResultError::DuplicateBinaryCell {
                row_index: 0,
                column_index: 2,
            })
        );
    }

    #[test]
    fn typed_view_rejects_out_of_bounds_binary_coordinates() {
        for (row_index, column_index) in [(1, 0), (0, 3)] {
            let mut result = typed_result();
            result.binary_cells[0].row_index = row_index;
            result.binary_cells[0].column_index = column_index;

            assert_eq!(
                result.typed_view().err(),
                Some(QueryResultError::BinaryCellOutOfBounds {
                    row_index,
                    column_index,
                })
            );
        }
    }

    #[test]
    fn typed_view_rejects_inconsistent_row_width() {
        let mut result = typed_result();
        result.rows[0].pop();

        assert_eq!(
            result.typed_view().err(),
            Some(QueryResultError::RowWidth {
                row_index: 0,
                actual: 2,
                expected: 3,
            })
        );
    }

    #[test]
    fn typed_view_rejects_inconsistent_column_metadata_width() {
        let mut result = typed_result();
        result.column_meta = vec![QueryColumnMeta::new("a", "TEXT")];

        assert_eq!(
            result.typed_view().err(),
            Some(QueryResultError::ColumnMetaWidth {
                actual: 1,
                expected: 3,
            })
        );
    }

    #[test]
    fn sql_result_restores_original_statement_sql() {
        let original_sql = "select * from users";
        let rewritten_sql = "select * from users LIMIT 1000";
        let results = [
            SqlResult::Query(QueryResult {
                sql: rewritten_sql.to_string(),
                columns: vec![],
                column_meta: vec![],
                rows: vec![],
                binary_cells: vec![],
                elapsed_ms: 0,
                ..Default::default()
            }),
            SqlResult::Exec(ExecResult {
                sql: rewritten_sql.to_string(),
                rows_affected: 0,
                elapsed_ms: 0,
                message: None,
            }),
            SqlResult::Error(SqlErrorInfo {
                sql: rewritten_sql.to_string(),
                message: "failure".to_string(),
            }),
        ];

        for result in results {
            let result = result.with_original_sql(original_sql);
            let actual_sql = match result {
                SqlResult::Query(result) => result.sql,
                SqlResult::Exec(result) => result.sql,
                SqlResult::Error(result) => result.sql,
            };
            assert_eq!(original_sql, actual_sql);
        }
    }
}

pub fn format_message(sql: &str, rows_affected: u64) -> String {
    let trimmed = sql.trim().to_uppercase();

    if trimmed.starts_with("INSERT") {
        format!("Inserted {} row(s)", rows_affected)
    } else if trimmed.starts_with("UPDATE") {
        format!("Updated {} row(s)", rows_affected)
    } else if trimmed.starts_with("DELETE") {
        format!("Deleted {} row(s)", rows_affected)
    } else if trimmed.starts_with("REPLACE") {
        format!("Replaced {} row(s)", rows_affected)
    } else if trimmed.starts_with("CREATE") {
        "Object created successfully".to_string()
    } else if trimmed.starts_with("ALTER") {
        "Object altered successfully".to_string()
    } else if trimmed.starts_with("DROP") {
        "Object dropped successfully".to_string()
    } else if trimmed.starts_with("TRUNCATE") {
        "Table truncated successfully".to_string()
    } else if trimmed.starts_with("RENAME") {
        "Object renamed successfully".to_string()
    } else if trimmed.starts_with("USE") {
        "Database changed successfully".to_string()
    } else if trimmed.starts_with("SET") {
        "Variable set successfully".to_string()
    } else if trimmed.starts_with("BEGIN") || trimmed.starts_with("START TRANSACTION") {
        "Transaction started".to_string()
    } else if trimmed.starts_with("COMMIT") {
        "Transaction committed".to_string()
    } else if trimmed.starts_with("ROLLBACK") {
        "Transaction rolled back".to_string()
    } else {
        format!(
            "Query executed successfully, {} row(s) affected",
            rows_affected
        )
    }
}

/// Statement type classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatementType {
    /// Query statement (SELECT, SHOW, etc.)
    Query,
    /// Data manipulation (INSERT, UPDATE, DELETE)
    Dml,
    /// Data definition (CREATE, ALTER, DROP)
    Ddl,
    /// Transaction control (BEGIN, COMMIT, ROLLBACK)
    Transaction,
    /// Database commands (USE, SET)
    Command,
    /// Other execution statements
    Exec,
}
