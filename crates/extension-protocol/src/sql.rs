//! SQL 工具 (`sql/*`)、编辑器辅助 (`completion/*` / `lint/*`)。
//!
//! 这里仅定义 wire 数据结构,不包含实际解析/格式化逻辑——那些由扩展进程实现,
//! 宿主只是路由消息。
//!
//! 详见 [`docs/design/extensions/api-database.md`] §10-§11。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::conn::ConnId;

// ============================================================================
// sql/parse
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqlParseParams {
    pub sql: String,
    /// 方言提示(`mysql` / `postgres` / `oracle`),空则用驱动默认。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dialect: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqlParseResult {
    pub statements: Vec<ParsedStatement>,
    #[serde(default)]
    pub errors: Vec<ParseError>,
}

/// 一条 statement 的粗粒度解析输出。具体字段因 kind 而异,放 `extra`。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ParsedStatement {
    /// `select` / `insert` / `update` / `delete` / `ddl` / `dml` / `tcl` / `dcl` / `unknown`。
    pub kind: String,
    /// 涉及的表(简单分析,不一定精确)。
    #[serde(default)]
    pub tables: Vec<String>,
    /// 涉及的列。
    #[serde(default)]
    pub columns: Vec<String>,
    /// 该 statement 在原文中的范围(字节偏移)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_offset: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_offset: Option<u32>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub extra: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseError {
    pub message: String,
    pub start_offset: u32,
    pub end_offset: u32,
}

// ============================================================================
// sql/format
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqlFormatParams {
    pub sql: String,
    #[serde(default)]
    pub options: SqlFormatOptions,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SqlFormatOptions {
    /// 缩进空格数。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indent: Option<u32>,
    /// 关键字大写。
    #[serde(default)]
    pub uppercase: bool,
    /// 单行最大长度(超长换行)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_length: Option<u32>,
    /// 是否保留注释。
    #[serde(default = "default_true_field")]
    pub preserve_comments: bool,
    /// 方言提示。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dialect: Option<String>,
}

fn default_true_field() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqlFormatResult {
    pub formatted: String,
}

// ============================================================================
// sql/explain
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqlExplainParams {
    pub conn_id: ConnId,
    pub sql: String,
    /// `text` / `json` / `xml` / `yaml`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// 是否运行真实执行(EXPLAIN ANALYZE)。
    #[serde(default)]
    pub analyze: bool,
    /// 是否输出 buffers / costs / verbose 等附加信息。
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub options: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqlExplainResult {
    /// 实际格式(可能与请求不同——驱动按支持情况降级)。
    pub format: String,
    /// 文本格式的完整输出。
    pub content: String,
    /// 结构化 plan(可选,format 是 json/structured 时填充)。
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub plan: Value,
}

// ============================================================================
// sql/build
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SqlBuildOp {
    TableChanges,
    CopyInsert,
    CopyInsertWithComments,
    CopyUpdate,
    CopyDelete,
    InsertStatement,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqlBuildParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conn_id: Option<ConnId>,
    pub op: SqlBuildOp,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SqlBuildResult {
    pub sql: String,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[cfg(test)]
mod build_tests {
    use super::*;

    #[test]
    fn sql_build_op_serializes_as_snake_case() {
        let params = SqlBuildParams {
            conn_id: None,
            op: SqlBuildOp::CopyInsertWithComments,
            payload: serde_json::json!({"table": "users"}),
        };

        let json = serde_json::to_value(params).unwrap();

        assert_eq!(json["op"], "copy_insert_with_comments");
        assert_eq!(json["payload"]["table"], "users");
    }
}

// ============================================================================
// completion/provide
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionParams {
    pub conn_id: ConnId,
    pub sql: String,
    /// 光标在 sql 中的字节偏移。
    pub cursor_offset: u32,
    #[serde(default)]
    pub context: CompletionContext,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompletionContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_database: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_schema: Option<String>,
    /// 触发字符(`.` / 空格等),便于扩展判断「点之后」补全场景。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResult {
    pub items: Vec<CompletionItem>,
    /// 候选还未完整(列表很长时,提示客户端可以再请求)。
    #[serde(default)]
    pub is_incomplete: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompletionItem {
    pub label: String,
    /// `keyword` / `table` / `column` / `function` / `schema` / `database` /
    /// `index` / `view` / `type` / `procedure` / `trigger` / `sequence` /
    /// `snippet` / `value` / `unknown`。
    pub kind: String,
    /// 实际插入的文本(可能比 label 多 / 少东西,比如带括号、引号)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub insert_text: Option<String>,
    /// 排序提示,小的在前。建议格式:`NNNN` 数字串。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_text: Option<String>,
    /// 详情(显示在右侧),例如 `table in db1`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// 文档(悬浮卡片)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub extra: Value,
}

// ============================================================================
// lint/analyze
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LintParams {
    pub sql: String,
    #[serde(default)]
    pub context: LintContext,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LintContext {
    /// 在已知连接的语境下分析(可访问 schema)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conn_id: Option<ConnId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dialect: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LintResult {
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
    Hint,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub range: DiagnosticRange,
    pub severity: DiagnosticSeverity,
    pub message: String,
    /// 规则码(`implicit_join` / `missing_index` 等)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// 修复建议(可有多个)。
    #[serde(default)]
    pub fixes: Vec<DiagnosticFix>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticRange {
    pub start_offset: u32,
    pub end_offset: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticFix {
    pub title: String,
    /// 替换的字节范围。
    pub range: DiagnosticRange,
    /// 替换文本。
    pub new_text: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sql_parse_params_with_dialect() {
        let p = SqlParseParams {
            sql: "SELECT 1".into(),
            dialect: Some("postgres".into()),
        };
        let j = serde_json::to_string(&p).unwrap();
        assert!(j.contains(r#""dialect":"postgres""#));
    }

    #[test]
    fn sql_parse_params_skip_none_dialect() {
        let p = SqlParseParams {
            sql: "SELECT 1".into(),
            dialect: None,
        };
        let j = serde_json::to_string(&p).unwrap();
        assert!(!j.contains("dialect"));
    }

    #[test]
    fn sql_parse_result_with_errors() {
        let r = SqlParseResult {
            statements: vec![ParsedStatement {
                kind: "select".into(),
                tables: vec!["users".into()],
                columns: vec!["*".into()],
                start_offset: Some(0),
                end_offset: Some(19),
                extra: Value::Null,
            }],
            errors: vec![ParseError {
                message: "unexpected token".into(),
                start_offset: 19,
                end_offset: 20,
            }],
        };
        let j = serde_json::to_string(&r).unwrap();
        let parsed: SqlParseResult = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.statements.len(), 1);
        assert_eq!(parsed.errors.len(), 1);
        assert_eq!(parsed.statements[0].kind, "select");
    }

    #[test]
    fn sql_format_options_preserve_comments_default_true() {
        let o: SqlFormatOptions = serde_json::from_str("{}").unwrap();
        assert!(o.preserve_comments);
        assert!(!o.uppercase);
    }

    #[test]
    fn sql_format_options_round_trip() {
        let o = SqlFormatOptions {
            indent: Some(2),
            uppercase: true,
            line_length: Some(80),
            preserve_comments: false,
            dialect: Some("mysql".into()),
        };
        let j = serde_json::to_string(&o).unwrap();
        let parsed: SqlFormatOptions = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.indent, Some(2));
        assert!(parsed.uppercase);
        assert_eq!(parsed.line_length, Some(80));
        assert!(!parsed.preserve_comments);
        assert_eq!(parsed.dialect.as_deref(), Some("mysql"));
    }

    #[test]
    fn sql_format_result_round_trip() {
        let r = SqlFormatResult {
            formatted: "SELECT *\nFROM users".into(),
        };
        let j = serde_json::to_string(&r).unwrap();
        let parsed: SqlFormatResult = serde_json::from_str(&j).unwrap();
        assert!(parsed.formatted.contains('\n'));
    }

    #[test]
    fn sql_explain_params_default_analyze_false() {
        let p: SqlExplainParams = serde_json::from_str(r#"{"conn_id":1,"sql":"x"}"#).unwrap();
        assert!(!p.analyze);
        assert!(p.format.is_none());
    }

    #[test]
    fn sql_explain_result_with_plan() {
        let r = SqlExplainResult {
            format: "json".into(),
            content: "{\"Plan\":{}}".into(),
            plan: serde_json::json!({"Plan": {}}),
        };
        let j = serde_json::to_string(&r).unwrap();
        let parsed: SqlExplainResult = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.format, "json");
        assert_eq!(parsed.plan, serde_json::json!({"Plan": {}}));
    }

    #[test]
    fn completion_params_round_trip() {
        let p = CompletionParams {
            conn_id: 17,
            sql: "SELECT * FRO".into(),
            cursor_offset: 13,
            context: CompletionContext {
                current_database: Some("db1".into()),
                current_schema: None,
                trigger: Some(" ".into()),
            },
        };
        let j = serde_json::to_string(&p).unwrap();
        let parsed: CompletionParams = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.cursor_offset, 13);
        assert_eq!(parsed.context.current_database.as_deref(), Some("db1"));
        assert_eq!(parsed.context.trigger.as_deref(), Some(" "));
    }

    #[test]
    fn completion_result_items() {
        let r = CompletionResult {
            items: vec![
                CompletionItem {
                    label: "FROM".into(),
                    kind: "keyword".into(),
                    insert_text: Some("FROM ".into()),
                    sort_text: Some("0001".into()),
                    ..Default::default()
                },
                CompletionItem {
                    label: "users".into(),
                    kind: "table".into(),
                    detail: Some("table in db1".into()),
                    insert_text: Some("users".into()),
                    ..Default::default()
                },
            ],
            is_incomplete: false,
        };
        let j = serde_json::to_string(&r).unwrap();
        let parsed: CompletionResult = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.items.len(), 2);
        assert_eq!(parsed.items[0].kind, "keyword");
        assert_eq!(parsed.items[1].kind, "table");
        assert_eq!(parsed.items[1].detail.as_deref(), Some("table in db1"));
    }

    #[test]
    fn diagnostic_severity_serde_lowercase() {
        assert_eq!(
            serde_json::to_string(&DiagnosticSeverity::Warning).unwrap(),
            r#""warning""#
        );
        let parsed: DiagnosticSeverity = serde_json::from_str(r#""error""#).unwrap();
        assert_eq!(parsed, DiagnosticSeverity::Error);
    }

    #[test]
    fn diagnostic_with_fixes_round_trip() {
        let d = Diagnostic {
            range: DiagnosticRange {
                start_offset: 7,
                end_offset: 12,
            },
            severity: DiagnosticSeverity::Warning,
            message: "Implicit join is deprecated".into(),
            code: Some("implicit_join".into()),
            fixes: vec![DiagnosticFix {
                title: "Use explicit JOIN".into(),
                range: DiagnosticRange {
                    start_offset: 7,
                    end_offset: 12,
                },
                new_text: "INNER JOIN users ON ...".into(),
            }],
        };
        let j = serde_json::to_string(&d).unwrap();
        let parsed: Diagnostic = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.range.start_offset, 7);
        assert_eq!(parsed.severity, DiagnosticSeverity::Warning);
        assert_eq!(parsed.code.as_deref(), Some("implicit_join"));
        assert_eq!(parsed.fixes.len(), 1);
    }

    #[test]
    fn lint_result_aggregate() {
        let r = LintResult {
            diagnostics: vec![
                Diagnostic {
                    range: DiagnosticRange {
                        start_offset: 0,
                        end_offset: 5,
                    },
                    severity: DiagnosticSeverity::Error,
                    message: "syntax".into(),
                    code: None,
                    fixes: vec![],
                },
                Diagnostic {
                    range: DiagnosticRange {
                        start_offset: 10,
                        end_offset: 15,
                    },
                    severity: DiagnosticSeverity::Info,
                    message: "consider index".into(),
                    code: Some("missing_index".into()),
                    fixes: vec![],
                },
            ],
        };
        let j = serde_json::to_string(&r).unwrap();
        let parsed: LintResult = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.diagnostics.len(), 2);
    }
}
