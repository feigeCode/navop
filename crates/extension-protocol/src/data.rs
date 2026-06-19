//! 数据导入导出 (`data/*`) + 通用流式拉取 (`stream/*`)。
//!
//! 导出走流式协议:`data/export` 启动一个 stream,后续用 `stream/read` 拉块。
//! 导入分三步:`data/import_begin` → `data/import_chunk` * N → `data/import_commit`。
//!
//! 详见 [`docs/design/extensions/api-database.md`] §13。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::conn::ConnId;
use crate::row::Row;

// ============================================================================
// 格式与公共选项
// ============================================================================

/// 支持的导入导出格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataFormat {
    Csv,
    /// JSON 数组,适合小数据集。
    Json,
    /// 一行一个 JSON 对象,适合流式。
    Ndjson,
    /// 单表 SQL INSERT 语句(导出场景较多)。
    Sql,
    Parquet,
    /// Excel xlsx(只支持导出,导入需先转 csv)。
    Xlsx,
}

/// CSV / TSV 选项。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CsvOptions {
    /// 字段分隔符,默认 `,`。
    #[serde(default = "default_csv_delimiter")]
    pub delimiter: String,
    /// 是否含首行表头。
    #[serde(default = "default_true_field")]
    pub header: bool,
    /// 引号字符,默认 `"`。
    #[serde(default = "default_csv_quote")]
    pub quote: String,
    /// 跳过的行数(常见于 CSV 包含 BOM 或 metadata)。
    #[serde(default)]
    pub skip_rows: u32,
    /// 字符编码,例如 `utf-8` / `gbk`。
    #[serde(default = "default_csv_encoding")]
    pub encoding: String,
    /// NULL 字符串表示(导出时也用),空字符串表示用 `\N`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub null_string: Option<String>,
}

fn default_csv_delimiter() -> String {
    ",".into()
}
fn default_csv_quote() -> String {
    "\"".into()
}
fn default_csv_encoding() -> String {
    "utf-8".into()
}
fn default_true_field() -> bool {
    true
}

impl Default for CsvOptions {
    fn default() -> Self {
        Self {
            delimiter: default_csv_delimiter(),
            header: true,
            quote: default_csv_quote(),
            skip_rows: 0,
            encoding: default_csv_encoding(),
            null_string: None,
        }
    }
}

// ============================================================================
// data/export
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportParams {
    pub conn_id: ConnId,
    /// 整张表导出。互斥:不与 `sql` 同时填。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    /// 自定义 SQL 导出。互斥:不与 `table` 同时填。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sql: Option<String>,
    pub format: DataFormat,
    /// `WHERE` 子句(table 模式生效)。
    #[serde(rename = "where", default, skip_serializing_if = "Option::is_none")]
    pub where_clause: Option<String>,
    /// 仅导出指定列(空则全部)。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include_columns: Vec<String>,
    /// 排除某些列(在 `include_columns` 解析后再过滤)。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_columns: Vec<String>,
    /// 行数上限。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_rows: Option<u64>,
    /// 格式专属选项。
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub options: Value,
    /// 客户端预分配的 stream id(用于 stream/read)。
    pub stream_id: StreamId,
}

pub type StreamId = String;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExportResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_rows: Option<u64>,
    /// 格式实际使用的 metadata(列名 / 编码 / 分隔符等),host 可写到文件头。
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub metadata: Value,
}

// ============================================================================
// stream/read / stream/close
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamReadParams {
    pub stream_id: StreamId,
    /// 最多读多少字节(扩展可少于,但不能超过)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamReadResult {
    /// 数据 chunk,base64 编码(MessagePack 可换 bin)。
    #[serde(default)]
    pub data: String,
    #[serde(default)]
    pub done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamCloseParams {
    pub stream_id: StreamId,
}

// ============================================================================
// data/import_*
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportBeginParams {
    pub conn_id: ConnId,
    pub table: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    pub format: DataFormat,
    /// 显式指定列顺序(空则默认从表 schema 推)。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<String>,
    #[serde(default)]
    pub options: ImportOptions,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportOptions {
    /// 冲突时 upsert(否则报错)。
    #[serde(default)]
    pub upsert: bool,
    /// upsert 时的冲突列(默认是主键)。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_conflict_columns: Vec<String>,
    /// 一次提交的批大小(扩展自由控制,可忽略)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_size: Option<u32>,
    /// 失败行是否记录到 `failed` 列表(默认 true,大数据集建议关掉)。
    #[serde(default = "default_true_field")]
    pub track_failed_rows: bool,
    /// 失败超过阈值就 abort。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abort_on_failures: Option<u32>,
    /// 是否禁用触发器(若驱动支持)。
    #[serde(default)]
    pub disable_triggers: bool,
    /// 格式专属选项。
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub format_options: Value,
}

pub type ImportId = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportBeginResult {
    pub import_id: ImportId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportChunkParams {
    pub import_id: ImportId,
    /// 一批行,顺序对应 [`ImportBeginParams::columns`]。
    pub rows: Vec<Row>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportChunkResult {
    pub inserted: u64,
    /// 失败的行 + 错误(track_failed_rows 关时为空)。
    #[serde(default)]
    pub failed: Vec<FailedRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailedRow {
    /// 在整个 import 中的全局行号(0-based)。
    pub row_index: u64,
    pub message: String,
    pub code: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportCommitParams {
    pub import_id: ImportId,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportCommitResult {
    pub inserted: u64,
    pub updated: u64,
    pub deleted: u64,
    #[serde(default)]
    pub failed: Vec<FailedRow>,
    /// 整体耗时(毫秒)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportAbortParams {
    pub import_id: ImportId,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::row::CellValue;

    #[test]
    fn data_format_serde_snake_case() {
        assert_eq!(serde_json::to_string(&DataFormat::Csv).unwrap(), r#""csv""#);
        assert_eq!(
            serde_json::to_string(&DataFormat::Ndjson).unwrap(),
            r#""ndjson""#
        );
        assert_eq!(
            serde_json::to_string(&DataFormat::Parquet).unwrap(),
            r#""parquet""#
        );
        let parsed: DataFormat = serde_json::from_str(r#""xlsx""#).unwrap();
        assert_eq!(parsed, DataFormat::Xlsx);
    }

    #[test]
    fn csv_options_defaults() {
        let o: CsvOptions = serde_json::from_str("{}").unwrap();
        assert_eq!(o.delimiter, ",");
        assert!(o.header);
        assert_eq!(o.quote, "\"");
        assert_eq!(o.skip_rows, 0);
        assert_eq!(o.encoding, "utf-8");
        assert!(o.null_string.is_none());
    }

    #[test]
    fn csv_options_round_trip() {
        let o = CsvOptions {
            delimiter: "\t".into(),
            header: false,
            quote: "'".into(),
            skip_rows: 3,
            encoding: "gbk".into(),
            null_string: Some("\\N".into()),
        };
        let j = serde_json::to_string(&o).unwrap();
        let parsed: CsvOptions = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.delimiter, "\t");
        assert!(!parsed.header);
        assert_eq!(parsed.skip_rows, 3);
        assert_eq!(parsed.encoding, "gbk");
        assert_eq!(parsed.null_string.as_deref(), Some("\\N"));
    }

    #[test]
    fn export_params_with_table_mode() {
        let p = ExportParams {
            conn_id: 17,
            table: Some("users".into()),
            schema: None,
            database: None,
            sql: None,
            format: DataFormat::Csv,
            where_clause: Some("city = 'Beijing'".into()),
            include_columns: vec!["id".into(), "name".into()],
            exclude_columns: vec![],
            max_rows: Some(1000),
            options: serde_json::json!({"delimiter": ","}),
            stream_id: "s-1".into(),
        };
        let j = serde_json::to_string(&p).unwrap();
        assert!(j.contains(r#""where":"city = 'Beijing'""#));
        assert!(j.contains(r#""stream_id":"s-1""#));
        let parsed: ExportParams = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.format, DataFormat::Csv);
        assert_eq!(parsed.include_columns.len(), 2);
        assert!(parsed.exclude_columns.is_empty());
    }

    #[test]
    fn export_params_with_sql_mode() {
        let p = ExportParams {
            conn_id: 1,
            table: None,
            schema: None,
            database: None,
            sql: Some("SELECT a, b FROM t".into()),
            format: DataFormat::Ndjson,
            where_clause: None,
            include_columns: vec![],
            exclude_columns: vec![],
            max_rows: None,
            options: Value::Null,
            stream_id: "s-2".into(),
        };
        let j = serde_json::to_string(&p).unwrap();
        assert!(j.contains(r#""sql":"SELECT a, b FROM t""#));
        assert!(!j.contains("table"));
        assert!(!j.contains("where"));
        assert!(!j.contains("include_columns"));
        assert!(!j.contains("exclude_columns"));
    }

    #[test]
    fn export_result_round_trip() {
        let r = ExportResult {
            estimated_bytes: Some(12_345_678),
            estimated_rows: Some(1_000_000),
            metadata: serde_json::json!({"columns": ["a", "b"]}),
        };
        let j = serde_json::to_string(&r).unwrap();
        let parsed: ExportResult = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.estimated_bytes, Some(12_345_678));
        assert_eq!(parsed.estimated_rows, Some(1_000_000));
    }

    #[test]
    fn stream_read_result_with_done_flag() {
        let r = StreamReadResult {
            data: "AAECAw==".into(), // base64 of [0,1,2,3]
            done: false,
        };
        let j = serde_json::to_string(&r).unwrap();
        let parsed: StreamReadResult = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.data, "AAECAw==");
        assert!(!parsed.done);
    }

    #[test]
    fn stream_read_result_done_chunk_defaults_to_empty_data() {
        let parsed: StreamReadResult = serde_json::from_str(r#"{"done":true}"#).unwrap();

        assert_eq!(parsed.data, "");
        assert!(parsed.done);
    }

    #[test]
    fn stream_close_params_round_trip() {
        let p = StreamCloseParams {
            stream_id: "s-1".into(),
        };
        let j = serde_json::to_string(&p).unwrap();
        assert_eq!(j, r#"{"stream_id":"s-1"}"#);
    }

    #[test]
    fn import_begin_params_with_columns() {
        let p = ImportBeginParams {
            conn_id: 17,
            table: "users".into(),
            schema: None,
            database: None,
            format: DataFormat::Csv,
            columns: vec!["id".into(), "name".into()],
            options: ImportOptions {
                upsert: true,
                on_conflict_columns: vec!["id".into()],
                batch_size: Some(1000),
                track_failed_rows: true,
                abort_on_failures: Some(100),
                disable_triggers: false,
                format_options: serde_json::json!({"delimiter": ","}),
            },
        };
        let j = serde_json::to_string(&p).unwrap();
        let parsed: ImportBeginParams = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.columns, vec!["id".to_string(), "name".to_string()]);
        assert!(parsed.options.upsert);
        assert_eq!(parsed.options.abort_on_failures, Some(100));
    }

    #[test]
    fn import_options_defaults() {
        let o: ImportOptions = serde_json::from_str("{}").unwrap();
        assert!(!o.upsert);
        assert!(o.on_conflict_columns.is_empty());
        assert!(o.track_failed_rows);
        assert!(o.abort_on_failures.is_none());
        assert!(!o.disable_triggers);
    }

    #[test]
    fn import_chunk_params_with_rows() {
        let p = ImportChunkParams {
            import_id: "i-1".into(),
            rows: vec![
                vec![
                    CellValue::I64 { value: 1 },
                    CellValue::Text {
                        value: "alice".into(),
                    },
                ],
                vec![
                    CellValue::I64 { value: 2 },
                    CellValue::Text {
                        value: "bob".into(),
                    },
                ],
            ],
        };
        let j = serde_json::to_string(&p).unwrap();
        let parsed: ImportChunkParams = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.import_id, "i-1");
        assert_eq!(parsed.rows.len(), 2);
        assert_eq!(parsed.rows[0].len(), 2);
    }

    #[test]
    fn import_chunk_result_with_failed_rows() {
        let r = ImportChunkResult {
            inserted: 8,
            failed: vec![FailedRow {
                row_index: 3,
                message: "duplicate key".into(),
                code: -34011,
            }],
        };
        let j = serde_json::to_string(&r).unwrap();
        let parsed: ImportChunkResult = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.inserted, 8);
        assert_eq!(parsed.failed.len(), 1);
        assert_eq!(parsed.failed[0].row_index, 3);
        assert_eq!(parsed.failed[0].code, -34011);
    }

    #[test]
    fn import_commit_result_full() {
        let r = ImportCommitResult {
            inserted: 1234,
            updated: 56,
            deleted: 0,
            failed: vec![],
            elapsed_ms: Some(5_678),
        };
        let j = serde_json::to_string(&r).unwrap();
        let parsed: ImportCommitResult = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.inserted, 1234);
        assert_eq!(parsed.updated, 56);
        assert_eq!(parsed.elapsed_ms, Some(5_678));
    }

    #[test]
    fn import_abort_params_round_trip() {
        let p = ImportAbortParams {
            import_id: "i-1".into(),
        };
        let j = serde_json::to_string(&p).unwrap();
        assert_eq!(j, r#"{"import_id":"i-1"}"#);
    }
}
