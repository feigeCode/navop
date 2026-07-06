//! 查询执行(`query/*` / `cursor/*`)、非查询执行(`exec/*`)、事务(`tx/*`)。
//!
//! 查询走流式协议:`query/start` 返回 cursor + 元数据,后续 `cursor/fetch`
//! 多次拉取直到 `done == true`。`cursor/cancel` 停止取数据但保留 buffer,
//! `cursor/close` 彻底释放。
//!
//! 事务用 `tx_id` 与 conn 解耦:同一 conn 可以在协议层并行开多个 tx(实际能
//! 否并行由驱动决定)。
//!
//! 详见 [`docs/design/extensions/api-database.md`] §7-§9。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::conn::ConnId;
use crate::row::{CellValue, ColumnSpec, ParamValue, Row};

// ============================================================================
// query/start
// ============================================================================

/// `query/start` 请求参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryStartParams {
    pub conn_id: ConnId,
    pub sql: String,
    /// Query language hint. Defaults to SQL when absent for backward compatibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// 参数列表。位置参数 `?` / `$N` / `:name` 由驱动方言决定。
    #[serde(default)]
    pub params: Vec<ParamValue>,
    /// 一次 `cursor/fetch` 默认拉取多少行(驱动可上限)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetch_size: Option<u32>,
    /// 整体超时(毫秒),包含 buffer 阶段。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u32>,
    /// 上限行数,None 表示无限。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_rows: Option<u64>,
    /// 关联事务 id(若在事务内执行)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_id: Option<TxId>,
}

impl QueryStartParams {
    pub fn new(conn_id: ConnId, sql: impl Into<String>) -> Self {
        Self {
            conn_id,
            sql: sql.into(),
            language: None,
            params: Vec::new(),
            fetch_size: None,
            timeout_ms: None,
            max_rows: None,
            tx_id: None,
        }
    }

    pub fn with_params(mut self, params: Vec<ParamValue>) -> Self {
        self.params = params;
        self
    }

    pub fn fetch_size(mut self, n: u32) -> Self {
        self.fetch_size = Some(n);
        self
    }

    pub fn timeout_ms(mut self, ms: u32) -> Self {
        self.timeout_ms = Some(ms);
        self
    }

    pub fn in_tx(mut self, tx_id: impl Into<TxId>) -> Self {
        self.tx_id = Some(tx_id.into());
        self
    }
}

/// `query/start` 响应。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryStartResult {
    pub cursor_id: CursorId,
    pub columns: Vec<ColumnSpec>,
    /// 是否已知总行数(driver 在执行前能预估时为 true)。
    #[serde(default)]
    pub row_count_known: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_count_estimate: Option<u64>,
    /// 任意驱动私有的执行元数据(query plan id、cache hit 等)。
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub extra: Value,
}

/// Cursor id 由扩展分配,通常是字符串(便于带前缀如 `c-17-3`)。
pub type CursorId = String;

// ============================================================================
// cursor/fetch / cancel / close
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorFetchParams {
    pub cursor_id: CursorId,
    /// 最多拉多少行(可能少于 fetch_size,但不会多)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>,
    /// 上次 `cursor/fetch` 返回的 `next_token`(分页 key)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorFetchResult {
    pub rows: Vec<Row>,
    /// 是否已读完。`done == true` 之后再 fetch 应该返回空 rows。
    #[serde(default)]
    pub done: bool,
    /// 服务端分页 token(opaque),驱动可不用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorCancelParams {
    pub cursor_id: CursorId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorCloseParams {
    pub cursor_id: CursorId,
}

// ============================================================================
// exec/run / exec/batch
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecRunParams {
    pub conn_id: ConnId,
    pub sql: String,
    #[serde(default)]
    pub params: Vec<ParamValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_id: Option<TxId>,
}

impl ExecRunParams {
    pub fn new(conn_id: ConnId, sql: impl Into<String>) -> Self {
        Self {
            conn_id,
            sql: sql.into(),
            params: Vec::new(),
            timeout_ms: None,
            tx_id: None,
        }
    }

    pub fn with_params(mut self, params: Vec<ParamValue>) -> Self {
        self.params = params;
        self
    }
}

/// `exec/run` 响应。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecRunResult {
    pub affected_rows: u64,
    /// 自增 last id(MySQL / SQLite 等),`CellValue` 而非 i64 是为兼容 UUID 主键。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_insert_id: Option<CellValue>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecBatchParams {
    pub conn_id: ConnId,
    pub statements: Vec<String>,
    /// 第一个出错就停止 batch。
    #[serde(default = "default_true")]
    pub stop_on_error: bool,
    /// 整批包在事务里(驱动 best effort)。
    #[serde(default)]
    pub in_transaction: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u32>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecBatchResult {
    /// 与 `statements` 一一对应,失败的位置为对应 BatchError。
    pub results: Vec<ExecRunResult>,
    /// 失败的语句索引 + 错误信息(允许 `stop_on_error=false` 后多条失败)。
    #[serde(default)]
    pub errors: Vec<BatchError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchError {
    pub index: u32,
    pub code: i32,
    pub message: String,
}

// ============================================================================
// tx/*
// ============================================================================

pub type TxId = String;

/// 事务隔离级别(若驱动不支持则忽略,默认 `read_committed`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolationLevel {
    ReadUncommitted,
    ReadCommitted,
    RepeatableRead,
    Serializable,
    /// 由驱动决定默认级别。
    Default,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxBeginParams {
    pub conn_id: ConnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isolation: Option<IsolationLevel>,
    /// `true` 表示只读事务。
    #[serde(default)]
    pub read_only: bool,
    /// 是否声明 `DEFERRABLE`(PostgreSQL)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deferrable: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxBeginResult {
    pub tx_id: TxId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxCommitParams {
    pub tx_id: TxId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxRollbackParams {
    pub tx_id: TxId,
    /// 回滚到指定 savepoint(可选)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_savepoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxSavepointParams {
    pub tx_id: TxId,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxReleaseParams {
    pub tx_id: TxId,
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::row::ColumnTypeKind;

    #[test]
    fn query_start_params_builder() {
        let p = QueryStartParams::new(17, "SELECT * FROM users WHERE id = ?")
            .with_params(vec![CellValue::I64 { value: 42 }])
            .fetch_size(500)
            .timeout_ms(10_000)
            .in_tx("t-1");
        assert_eq!(p.conn_id, 17);
        assert_eq!(p.sql, "SELECT * FROM users WHERE id = ?");
        assert_eq!(p.params.len(), 1);
        assert_eq!(p.fetch_size, Some(500));
        assert_eq!(p.timeout_ms, Some(10_000));
        assert_eq!(p.tx_id.as_deref(), Some("t-1"));
    }

    #[test]
    fn query_start_params_serializes_optional_language() {
        let mut params = QueryStartParams::new(7, r#"{"query":{"match_all":{}}}"#);
        params.language = Some("elasticsearch_dsl".to_string());

        let value = serde_json::to_value(&params).expect("query params serialize");

        assert_eq!(value["conn_id"], 7);
        assert_eq!(value["language"], "elasticsearch_dsl");
        assert_eq!(value["sql"], r#"{"query":{"match_all":{}}}"#);
    }

    #[test]
    fn query_start_params_round_trip() {
        let p = QueryStartParams::new(17, "SELECT 1")
            .with_params(vec![CellValue::Null, CellValue::Text { value: "x".into() }]);
        let j = serde_json::to_string(&p).unwrap();
        let parsed: QueryStartParams = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.conn_id, 17);
        assert_eq!(parsed.sql, "SELECT 1");
        assert_eq!(parsed.params.len(), 2);
        assert!(matches!(parsed.params[0], CellValue::Null));
    }

    #[test]
    fn query_start_result_round_trip() {
        let r = QueryStartResult {
            cursor_id: "c-1".into(),
            columns: vec![ColumnSpec::new("id", "uuid", ColumnTypeKind::Uuid)],
            row_count_known: false,
            row_count_estimate: None,
            extra: Value::Null,
        };
        let j = serde_json::to_string(&r).unwrap();
        let parsed: QueryStartResult = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.cursor_id, "c-1");
        assert_eq!(parsed.columns.len(), 1);
    }

    #[test]
    fn cursor_fetch_params_skip_none() {
        let p = CursorFetchParams {
            cursor_id: "c-1".into(),
            n: Some(1000),
            next_token: None,
        };
        let j = serde_json::to_string(&p).unwrap();
        assert!(j.contains(r#""n":1000"#));
        assert!(!j.contains("next_token"));
    }

    #[test]
    fn cursor_fetch_result_with_next_token() {
        let r = CursorFetchResult {
            rows: vec![vec![CellValue::I64 { value: 1 }]],
            done: false,
            next_token: Some("opaque-key".into()),
        };
        let j = serde_json::to_string(&r).unwrap();
        let parsed: CursorFetchResult = serde_json::from_str(&j).unwrap();
        assert!(!parsed.done);
        assert_eq!(parsed.next_token.as_deref(), Some("opaque-key"));
        assert_eq!(parsed.rows.len(), 1);
    }

    #[test]
    fn cursor_fetch_result_done_empty() {
        let r = CursorFetchResult {
            rows: vec![],
            done: true,
            next_token: None,
        };
        let j = serde_json::to_string(&r).unwrap();
        assert!(j.contains(r#""done":true"#));
        let parsed: CursorFetchResult = serde_json::from_str(&j).unwrap();
        assert!(parsed.done);
        assert!(parsed.rows.is_empty());
    }

    #[test]
    fn cursor_cancel_close_round_trip() {
        let c = CursorCancelParams {
            cursor_id: "c-1".into(),
        };
        let cl = CursorCloseParams {
            cursor_id: "c-1".into(),
        };
        for j in [
            serde_json::to_string(&c).unwrap(),
            serde_json::to_string(&cl).unwrap(),
        ] {
            assert!(j.contains(r#""cursor_id":"c-1""#));
        }
    }

    #[test]
    fn exec_run_params_builder() {
        let p = ExecRunParams::new(17, "INSERT INTO t VALUES (?)")
            .with_params(vec![CellValue::Text { value: "x".into() }]);
        assert_eq!(p.conn_id, 17);
        assert_eq!(p.params.len(), 1);
    }

    #[test]
    fn exec_run_result_with_last_id() {
        let r = ExecRunResult {
            affected_rows: 1,
            last_insert_id: Some(CellValue::I64 { value: 42 }),
            warnings: vec![],
        };
        let j = serde_json::to_string(&r).unwrap();
        let parsed: ExecRunResult = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.affected_rows, 1);
        assert_eq!(parsed.last_insert_id, Some(CellValue::I64 { value: 42 }));
    }

    #[test]
    fn exec_run_result_with_uuid_last_id() {
        let r = ExecRunResult {
            affected_rows: 1,
            last_insert_id: Some(CellValue::Uuid {
                value: "550e8400-e29b-41d4-a716-446655440000".into(),
            }),
            warnings: vec![],
        };
        let j = serde_json::to_string(&r).unwrap();
        assert!(j.contains(r#""type":"uuid""#));
    }

    #[test]
    fn exec_run_result_skip_none_last_id() {
        let r = ExecRunResult {
            affected_rows: 0,
            last_insert_id: None,
            warnings: vec![],
        };
        let j = serde_json::to_string(&r).unwrap();
        assert!(!j.contains("last_insert_id"));
    }

    #[test]
    fn exec_batch_params_default_stop_on_error_true() {
        let p: ExecBatchParams =
            serde_json::from_str(r#"{"conn_id":1,"statements":["a","b"]}"#).unwrap();
        assert!(p.stop_on_error);
        assert!(!p.in_transaction);
    }

    #[test]
    fn exec_batch_params_round_trip() {
        let p = ExecBatchParams {
            conn_id: 17,
            statements: vec!["INSERT a".into(), "UPDATE b".into()],
            stop_on_error: false,
            in_transaction: true,
            timeout_ms: Some(5_000),
        };
        let j = serde_json::to_string(&p).unwrap();
        let parsed: ExecBatchParams = serde_json::from_str(&j).unwrap();
        assert!(!parsed.stop_on_error);
        assert!(parsed.in_transaction);
        assert_eq!(parsed.statements.len(), 2);
    }

    #[test]
    fn exec_batch_result_with_errors() {
        let r = ExecBatchResult {
            results: vec![
                ExecRunResult {
                    affected_rows: 1,
                    ..Default::default()
                },
                ExecRunResult::default(),
            ],
            errors: vec![BatchError {
                index: 1,
                code: -34001,
                message: "syntax error".into(),
            }],
        };
        let j = serde_json::to_string(&r).unwrap();
        let parsed: ExecBatchResult = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.results.len(), 2);
        assert_eq!(parsed.errors.len(), 1);
        assert_eq!(parsed.errors[0].index, 1);
    }

    #[test]
    fn isolation_level_serde_snake_case() {
        assert_eq!(
            serde_json::to_string(&IsolationLevel::ReadCommitted).unwrap(),
            r#""read_committed""#
        );
        assert_eq!(
            serde_json::to_string(&IsolationLevel::Serializable).unwrap(),
            r#""serializable""#
        );
        let parsed: IsolationLevel = serde_json::from_str(r#""repeatable_read""#).unwrap();
        assert_eq!(parsed, IsolationLevel::RepeatableRead);
    }

    #[test]
    fn tx_begin_params_default_read_only_false() {
        let p: TxBeginParams = serde_json::from_str(r#"{"conn_id":1}"#).unwrap();
        assert_eq!(p.conn_id, 1);
        assert!(!p.read_only);
        assert!(p.isolation.is_none());
    }

    #[test]
    fn tx_begin_params_full_round_trip() {
        let p = TxBeginParams {
            conn_id: 17,
            isolation: Some(IsolationLevel::Serializable),
            read_only: true,
            deferrable: Some(true),
        };
        let j = serde_json::to_string(&p).unwrap();
        let parsed: TxBeginParams = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.conn_id, 17);
        assert_eq!(parsed.isolation, Some(IsolationLevel::Serializable));
        assert!(parsed.read_only);
        assert_eq!(parsed.deferrable, Some(true));
    }

    #[test]
    fn tx_begin_result_round_trip() {
        let r = TxBeginResult {
            tx_id: "t-1".into(),
        };
        let j = serde_json::to_string(&r).unwrap();
        assert_eq!(j, r#"{"tx_id":"t-1"}"#);
    }

    #[test]
    fn tx_commit_rollback_savepoint_release() {
        let cm = TxCommitParams {
            tx_id: "t-1".into(),
        };
        let rb = TxRollbackParams {
            tx_id: "t-1".into(),
            to_savepoint: Some("sp1".into()),
        };
        let sp = TxSavepointParams {
            tx_id: "t-1".into(),
            name: "sp1".into(),
        };
        let rl = TxReleaseParams {
            tx_id: "t-1".into(),
            name: "sp1".into(),
        };
        let jcm = serde_json::to_string(&cm).unwrap();
        let jrb = serde_json::to_string(&rb).unwrap();
        let jsp = serde_json::to_string(&sp).unwrap();
        let jrl = serde_json::to_string(&rl).unwrap();
        assert_eq!(jcm, r#"{"tx_id":"t-1"}"#);
        assert!(jrb.contains(r#""to_savepoint":"sp1""#));
        assert!(jsp.contains(r#""name":"sp1""#));
        assert!(jrl.contains(r#""name":"sp1""#));
    }

    #[test]
    fn tx_rollback_without_savepoint_skips_field() {
        let rb = TxRollbackParams {
            tx_id: "t".into(),
            to_savepoint: None,
        };
        let j = serde_json::to_string(&rb).unwrap();
        assert!(!j.contains("to_savepoint"));
    }
}
