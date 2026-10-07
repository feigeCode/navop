use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use crate::sql_editor_definition::DefaultSqlDefinitionProvider;
use crate::sql_editor_hover::DefaultSqlHoverProvider;
use crate::sql_editor_signature::DefaultSqlSignatureHelpProvider;
use crate::table_ddl::{TableDdlSection, TableDdlSources};
use anyhow::Result;
use db::plugin::SqlCompletionInfo;
use db::sql_editor::diagnostics::{
    SqlDiagnostic, SqlDiagnosticSeverity, SqlMetadataView, analyze_parser_diagnostics,
    analyze_semantic_diagnostics,
};
use db::sql_editor::sql_context_inferrer::{ContextInferrer, SqlContext as InferredSqlContext};
use db::sql_editor::sql_symbol_table::SymbolTable;
use db::sql_editor::sql_tokenizer::{SqlKeyword, SqlToken, SqlTokenKind, SqlTokenizer};
use db::sql_editor::statement_ranges::{SqlDialect, SqlStatementSnapshot};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext, AsyncApp, Context, Entity, Font, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Task, Window, div,
};
use gpui_component::WindowExt as _;
use gpui_component::highlighter::{Diagnostic, DiagnosticSeverity};
use gpui_component::input::{
    CodeActionProvider, CompletionProvider, Copy, Cut, DefinitionProvider, EditorState, GutterLane,
    GutterLaneOptions, GutterMarker, HoverProvider, Paste, SelectAll, ShowDocumentHandler, TabSize,
};
use gpui_component::native_menu::NativeMenu as PlatformNativeMenu;
use gpui_component::spinner::Spinner;
use gpui_component::tooltip::Tooltip;
use gpui_component::{Icon, Rope, RopeExt, Sizable as _, Size};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit,
    InlineCompletionContext, InlineCompletionItem, InlineCompletionResponse, InsertReplaceEdit,
    InsertTextFormat, Range as LspRange,
};
use one_assets::IconName;
use one_core::settings::{AppSettings, installed_grid_monospace_font};
use one_core::tab_container::TabContainer;
use one_ui::{ExtendedEditor, ExtendedEditorState, SignatureHelpProvider};
use rust_i18n::t;
use sum_tree::Bias;

gpui::actions!(
    sql_editor,
    [
        RunSelectedSql,
        RunCursorStatementSql,
        ShowHoverDetails,
        CopyHoverDdl
    ]
);

pub(crate) const SQL_GUTTER_IDLE: &str = "idle";
pub(crate) const SQL_GUTTER_RUNNING: &str = "running";
pub(crate) const SQL_GUTTER_SUCCEEDED: &str = "succeeded";
pub(crate) const SQL_GUTTER_FAILED: &str = "failed";
pub(crate) const SQL_GUTTER_CANCELLED: &str = "cancelled";

/// Kind of a table-like schema object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SqlObjectType {
    #[default]
    Table,
    View,
}

impl SqlObjectType {
    /// Stable display name used by hover content and DDL preview.
    pub fn as_str(&self) -> &'static str {
        match self {
            SqlObjectType::Table => "TABLE",
            SqlObjectType::View => "VIEW",
        }
    }
}

/// Detailed column metadata for hover and generated DDL preview.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SqlColumnDetail {
    pub name: String,
    pub data_type: String,
    pub is_nullable: bool,
    pub is_primary_key: bool,
    pub default_value: Option<String>,
    pub comment: Option<String>,
}

/// Detailed table-like object metadata for hover.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SqlTableDetail {
    pub object_type: SqlObjectType,
    pub schema: Option<String>,
    pub comment: Option<String>,
    pub engine: Option<String>,
    pub columns: Vec<SqlColumnDetail>,
}

/// 外部 database/schema（qualifier）的表/列元数据快照，用于跨库限定名补全。
///
/// 由视图层懒加载并按 qualifier 名（小写键）缓存在 [`SqlSchema::foreign_schemas`]。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ForeignSchema {
    /// qualifier 原始名称（保持大小写）。
    pub name: String,
    /// 拉取这份元数据时用的坐标：它才是表真正所在的 database/schema。
    /// 跨库限定名（`otherdb.users`）的建表 DDL 要靠它对上驱动查询。
    pub scope: ForeignSchemaScope,
    /// (表名, 说明)
    pub tables: Vec<(String, String)>,
    /// 表→列映射，每列为 (name, data_type, doc)
    pub columns_by_table: std::collections::HashMap<String, Vec<(String, String, String)>>,
    /// 表名→详细信息（供 hover 复用）。
    pub table_details: std::collections::HashMap<String, SqlTableDetail>,
}

/// 拉取外部 qualifier 元数据时使用的 (database, schema) 坐标。
///
/// database 型 qualifier（MySQL/ClickHouse）落在 database 上，schema 型 qualifier
/// （PG/MSSQL/Oracle）落在 schema 上，两者都由视图层在拉取时确定。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ForeignSchemaScope {
    pub database: String,
    pub schema: Option<String>,
}

/// Schema hints used by autocomplete and hover.
///
/// The snapshot is always scoped to one connection's currently selected
/// database/schema (see `current_database`/`current_schema`), which hover
/// resolution uses to reject cross-database bare-name references.
#[derive(Clone, Default)]
pub struct SqlSchema {
    pub tables: Vec<(String, String)>,    // (name, doc)
    pub columns: Vec<(String, String)>,   // global (name, doc)
    pub functions: Vec<(String, String)>, // (signature, doc)
    /// 表→列映射，每列包含 (name, data_type, doc)
    pub columns_by_table: std::collections::HashMap<String, Vec<(String, String, String)>>,
    /// 可用 database/schema（qualifier）列表，由视图层按方言填充。
    /// 当前 database/schema 由 `current_database` / `current_schema` 单独保存，
    /// completion 构建时与这里的外部 qualifier 合并。
    pub qualifiers: Vec<(String, String)>,
    /// 已懒加载的外部 qualifier 元数据，key 为 qualifier 小写名。
    pub foreign_schemas: std::collections::HashMap<String, ForeignSchema>,
    /// Database this snapshot was loaded for (scope guard for hover).
    pub current_database: Option<String>,
    /// Schema this snapshot was loaded for (scope guard for hover).
    pub current_schema: Option<String>,
    /// Detailed object info keyed by table name (as loaded), used by hover.
    pub table_details: std::collections::HashMap<String, SqlTableDetail>,
}

impl SqlSchema {
    pub fn with_tables(
        mut self,
        tables: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        self.tables = tables
            .into_iter()
            .map(|(n, d)| (n.into(), d.into()))
            .collect();
        self
    }
    pub fn with_columns(
        mut self,
        columns: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        self.columns = columns
            .into_iter()
            .map(|(n, d)| (n.into(), d.into()))
            .collect();
        self
    }
    pub fn with_functions(
        mut self,
        functions: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        self.functions = functions
            .into_iter()
            .map(|(n, d)| (n.into(), d.into()))
            .collect();
        self
    }
    /// 添加表的列信息（兼容旧 API，无类型信息）
    pub fn with_table_columns(
        mut self,
        table: impl Into<String>,
        columns: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        self.columns_by_table.insert(
            table.into(),
            columns
                .into_iter()
                .map(|(n, d)| (n.into(), String::new(), d.into()))
                .collect(),
        );
        self
    }
    /// 添加表的列信息（含类型信息）
    pub fn with_table_columns_typed(
        mut self,
        table: impl Into<String>,
        columns: impl IntoIterator<Item = (impl Into<String>, impl Into<String>, impl Into<String>)>,
    ) -> Self {
        self.columns_by_table.insert(
            table.into(),
            columns
                .into_iter()
                .map(|(n, t, d)| (n.into(), t.into(), d.into()))
                .collect(),
        );
        self
    }
    /// 记录本快照所属的 database/schema，供 hover 作用域校验使用。
    pub fn with_scope(mut self, database: Option<String>, schema: Option<String>) -> Self {
        self.current_database = database;
        self.current_schema = schema;
        self
    }
    /// 设置其他可用 database/schema（qualifier）候选列表。
    pub fn with_qualifiers(
        mut self,
        qualifiers: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        self.qualifiers = qualifiers
            .into_iter()
            .map(|(n, d)| (n.into(), d.into()))
            .collect();
        self
    }
    /// 缓存一个外部 qualifier 的元数据（key 归一化为小写）。
    pub fn with_foreign_schema(mut self, foreign: ForeignSchema) -> Self {
        self.foreign_schemas
            .insert(foreign.name.to_lowercase(), foreign);
        self
    }
    /// 添加表的详细元数据（用于 hover 与 DDL 预览）。
    pub fn with_table_detail(mut self, table: impl Into<String>, detail: SqlTableDetail) -> Self {
        self.table_details.insert(table.into(), detail);
        self
    }
}

/// SQL context for smarter completion suggestions
#[derive(Debug, Clone, PartialEq)]
pub enum SqlContext {
    /// Start of statement or unknown context
    Start,
    /// After SELECT keyword, expecting columns
    SelectColumns,
    /// After FROM/JOIN/INTO/UPDATE, expecting table name
    TableName,
    /// After WHERE/AND/OR/ON, expecting condition
    Condition,
    /// After ORDER BY/GROUP BY, expecting column
    OrderBy,
    /// After SET (in UPDATE), expecting column = value
    SetClause,
    /// After VALUES, expecting values
    Values,
    /// After CREATE TABLE, expecting table definition
    CreateTable,
    /// After dot (table.column), expecting column name
    DotColumn(String),
    /// After function name with open paren
    FunctionArgs,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SqlDotCompletionTarget {
    /// 当前 database/schema 的表列表。
    Tables,
    /// 当前库表名 → 列。
    Columns(String),
    /// 外部 database/schema → 表列表。
    ForeignTables(String),
    /// 外部 qualifier + 表名 → 列。
    ForeignColumns(String, String),
    None,
}

fn scope_matches(schema: &SqlSchema, qualifier: &str) -> bool {
    schema
        .current_database
        .as_deref()
        .is_some_and(|database| database.eq_ignore_ascii_case(qualifier))
        || schema
            .current_schema
            .as_deref()
            .is_some_and(|schema| schema.eq_ignore_ascii_case(qualifier))
}

/// 解析点号补全目标。`chain` 是光标前最后一个点号之前的限定链。
/// 返回值使用元数据中的规范名（大小写以元数据为准）。
pub(crate) fn sql_dot_completion_target_for_chain(
    schema: &SqlSchema,
    chain: &[String],
) -> SqlDotCompletionTarget {
    match chain {
        [name] => {
            if scope_matches(schema, name) {
                return SqlDotCompletionTarget::Tables;
            }
            if let Some(qualifier) = canonical_qualifier(schema, name) {
                return SqlDotCompletionTarget::ForeignTables(qualifier);
            }
            table_name_in_schema(schema, name)
                .map(SqlDotCompletionTarget::Columns)
                .unwrap_or(SqlDotCompletionTarget::None)
        }
        [qualifier, table] => {
            if let Some(canonical) = canonical_qualifier(schema, qualifier) {
                return SqlDotCompletionTarget::ForeignColumns(canonical, table.clone());
            }
            if scope_matches(schema, qualifier) {
                return table_name_in_schema(schema, table)
                    .map(SqlDotCompletionTarget::Columns)
                    .unwrap_or(SqlDotCompletionTarget::None);
            }
            SqlDotCompletionTarget::None
        }
        _ => SqlDotCompletionTarget::None,
    }
}

/// 返回元数据中的规范 qualifier 名（大小写不敏感匹配）。
fn canonical_qualifier(schema: &SqlSchema, name: &str) -> Option<String> {
    schema
        .qualifiers
        .iter()
        .find(|(qualifier, _)| qualifier.eq_ignore_ascii_case(name))
        .map(|(qualifier, _)| qualifier.clone())
}

/// 返回元数据中的规范表名（优先列映射键，其次表列表）。
fn table_name_in_schema(schema: &SqlSchema, name: &str) -> Option<String> {
    schema
        .columns_by_table
        .keys()
        .find(|table| table.eq_ignore_ascii_case(name))
        .cloned()
        .or_else(|| {
            schema
                .tables
                .iter()
                .find(|(table, _)| table.eq_ignore_ascii_case(name))
                .map(|(table, _)| table.clone())
        })
}

pub(crate) fn sql_dot_completion_target(
    schema: &SqlSchema,
    qualifier: &str,
) -> SqlDotCompletionTarget {
    sql_dot_completion_target_for_chain(schema, &[qualifier.to_string()])
}

/// 解析光标前最后一个点号之前的限定链：`db.tbl.` → ["db","tbl"]。
///
/// 只收集点号之前的标识符；正在输入的最后一个词（点号之后）不属于链。
pub(crate) fn dot_qualifier_chain(text: &str, offset: usize) -> Vec<String> {
    let mut tokenizer = SqlTokenizer::new(text);
    let tokens = tokenizer.tokenize();
    let meaningful: Vec<&SqlToken> = tokens
        .iter()
        .filter(|token| token.end <= offset && !token.is_whitespace() && !token.is_comment())
        .collect();
    let Some(mut index) = meaningful
        .iter()
        .rposition(|token| token.kind == SqlTokenKind::Dot)
    else {
        return Vec::new();
    };
    let mut chain = Vec::new();
    while index > 0 {
        let token = meaningful[index - 1];
        if !matches!(token.kind, SqlTokenKind::Ident | SqlTokenKind::QuotedIdent) {
            break;
        }
        chain.insert(0, completion_identifier_text(token));
        if index >= 2 && meaningful[index - 2].kind == SqlTokenKind::Dot {
            index -= 2;
        } else {
            break;
        }
    }
    chain
}

/// Priority scores for context-aware completion sorting.
/// Lower scores appear first in completion list (higher priority).
///
/// Default priority order (without context):
/// 1. Keywords (1000-1999)
/// 2. Tables (2000-2999)
/// 3. Columns (3000-3999)
/// 4. Functions (4000-4999)
/// 5. Snippets (5000+)
///
/// In specific contexts, relevant items get boosted to appear before keywords.
pub mod completion_priority {
    // Base priorities by item type (lower = higher priority)
    pub const KEYWORDS_BASE: i32 = 1000;
    pub const DATA_TYPES_BASE: i32 = 1500;
    pub const TABLES_BASE: i32 = 2000;
    /// 外部 database/schema（qualifier）名补全，排在当前库表之后、列之前。
    pub const QUALIFIERS_BASE: i32 = 2800;
    pub const COLUMNS_BASE: i32 = 3000;
    pub const SNIPPETS_BASE: i32 = 4000;
    pub const OPERATORS_BASE: i32 = 4500;
    pub const FUNCTIONS_BASE: i32 = 5000;

    // Context boost (subtract from base to increase priority)
    // Large boost to make context-relevant items appear before keywords
    pub const CONTEXT_BOOST: i32 = 2500;
    pub const PREFIX_MATCH_BOOST: i32 = 200;
    /// Boost for matches right after a word boundary (e.g. `user` in `admin_user`).
    /// Weaker than prefix match, stronger than plain substring match.
    pub const BOUNDARY_MATCH_BOOST: i32 = 100;

    use super::SqlContext;
    use lsp_types::CompletionItemKind;

    /// Calculate priority score for a completion item based on context.
    /// Lower scores appear first (higher priority).
    pub fn calculate_score(
        context: &SqlContext,
        item_kind: Option<CompletionItemKind>,
        matches_prefix: bool,
    ) -> i32 {
        let match_boost = if matches_prefix {
            PREFIX_MATCH_BOOST
        } else {
            0
        };
        calculate_score_with_match(context, item_kind, match_boost)
    }

    /// Calculate priority score with a fine-grained match boost.
    ///
    /// Use [`PREFIX_MATCH_BOOST`] for prefix matches, [`BOUNDARY_MATCH_BOOST`]
    /// for word-boundary matches and `0` for substring matches, so that
    /// prefix matches rank first, then boundary matches, then substrings.
    pub fn calculate_score_with_match(
        context: &SqlContext,
        item_kind: Option<CompletionItemKind>,
        match_boost: i32,
    ) -> i32 {
        // Determine base score by item type
        let base_score = match item_kind {
            Some(CompletionItemKind::KEYWORD) => KEYWORDS_BASE,
            Some(CompletionItemKind::TYPE_PARAMETER) => DATA_TYPES_BASE,
            Some(CompletionItemKind::STRUCT) => TABLES_BASE,
            Some(CompletionItemKind::FIELD) => COLUMNS_BASE,
            Some(CompletionItemKind::FUNCTION) => FUNCTIONS_BASE,
            Some(CompletionItemKind::OPERATOR) => OPERATORS_BASE,
            Some(CompletionItemKind::SNIPPET) => SNIPPETS_BASE,
            _ => COLUMNS_BASE, // Default to columns priority
        };

        // Apply context boost for relevant items
        let context_boost = match (context, item_kind) {
            // DotColumn: columns get boost
            (SqlContext::DotColumn(_), Some(CompletionItemKind::FIELD)) => CONTEXT_BOOST,

            // TableName: tables get boost
            (SqlContext::TableName, Some(CompletionItemKind::STRUCT)) => CONTEXT_BOOST,

            // SelectColumns: columns get boost
            (SqlContext::SelectColumns, Some(CompletionItemKind::FIELD)) => CONTEXT_BOOST,

            // Condition/OrderBy/SetClause: columns get boost
            (
                SqlContext::Condition | SqlContext::OrderBy | SqlContext::SetClause,
                Some(CompletionItemKind::FIELD),
            ) => CONTEXT_BOOST,

            // FunctionArgs: columns get boost
            (SqlContext::FunctionArgs, Some(CompletionItemKind::FIELD)) => CONTEXT_BOOST,

            // CreateTable: data types get boost (appear before keywords)
            (SqlContext::CreateTable, Some(CompletionItemKind::TYPE_PARAMETER)) => CONTEXT_BOOST,

            _ => 0,
        };

        // Lower score = higher priority
        base_score - context_boost - match_boost
    }

    /// Convert score to sort_text format.
    /// Lower scores appear first (higher priority).
    /// Format: "{score:05}_{label}" for stable sorting.
    pub fn score_to_sort_text(score: i32, label: &str) -> String {
        // Lower score = higher priority, so use score directly
        format!("{:05}_{}", score.clamp(0, 99999), label)
    }
}

/// Identifier match rank against the current word (case-insensitive).
///
/// Lower is better:
/// - `0`: prefix match (`user` matches `users`)
/// - `1`: word-boundary match, i.e. right after `_` (`user` matches `admin_user`)
/// - `2`: plain substring match (`ser` matches `users`)
/// - `None`: no match
///
/// `word_upper` must already be uppercased.
fn identifier_match_rank(label: &str, word_upper: &str) -> Option<i32> {
    if word_upper.is_empty() {
        return Some(0);
    }
    let upper = label.to_uppercase();
    if upper.starts_with(word_upper) {
        return Some(0);
    }
    let mut has_substring = false;
    for (pos, _) in upper.match_indices(word_upper) {
        if pos > 0 && upper.as_bytes()[pos - 1] == b'_' {
            return Some(1);
        }
        has_substring = true;
    }
    has_substring.then_some(2)
}

// Built-in SQL keywords and docs
pub(crate) const SQL_KEYWORDS: &[(&str, &str)] = &[
    ("SELECT", "Query rows from table(s)"),
    ("INSERT", "Insert new rows"),
    ("UPDATE", "Update existing rows"),
    ("DELETE", "Delete rows"),
    ("CREATE", "Create database object"),
    ("ALTER", "Modify database object"),
    ("DROP", "Remove database object"),
    ("TRUNCATE", "Remove all rows from table"),
    ("FROM", "Specify source table(s)"),
    ("WHERE", "Filter rows with predicates"),
    ("JOIN", "Combine rows from tables"),
    ("INNER JOIN", "Inner join tables"),
    ("LEFT JOIN", "Left outer join"),
    ("RIGHT JOIN", "Right outer join"),
    ("FULL JOIN", "Full outer join"),
    ("CROSS JOIN", "Cross product of tables"),
    ("ON", "Join condition"),
    ("USING", "Join using common columns"),
    ("GROUP BY", "Group rows for aggregation"),
    ("HAVING", "Filter grouped rows"),
    ("ORDER BY", "Sort result set"),
    ("ASC", "Ascending order"),
    ("DESC", "Descending order"),
    ("LIMIT", "Limit number of rows"),
    ("OFFSET", "Skip rows"),
    ("VALUES", "Specify values for INSERT"),
    ("INTO", "Target table for INSERT"),
    ("SET", "Set column values for UPDATE"),
    ("AND", "Logical AND"),
    ("OR", "Logical OR"),
    ("NOT", "Logical NOT"),
    ("IN", "Value in list"),
    ("EXISTS", "Subquery returns rows"),
    ("BETWEEN", "Value in range"),
    ("LIKE", "Pattern matching"),
    ("IS NULL", "Check for NULL"),
    ("IS NOT NULL", "Check for non-NULL"),
    ("AS", "Alias"),
    ("DISTINCT", "Remove duplicates"),
    ("ALL", "Include all rows"),
    ("UNION", "Combine result sets"),
    ("UNION ALL", "Combine without dedup"),
    ("INTERSECT", "Common rows"),
    ("EXCEPT", "Difference of sets"),
    ("CASE", "Conditional expression"),
    ("WHEN", "Condition in CASE"),
    ("THEN", "Result in CASE"),
    ("ELSE", "Default in CASE"),
    ("END", "End CASE expression"),
    ("WITH", "Common table expression"),
    ("TABLE", "Table keyword"),
    ("INDEX", "Index keyword"),
    ("VIEW", "View keyword"),
    ("PRIMARY KEY", "Primary key constraint"),
    ("FOREIGN KEY", "Foreign key constraint"),
    ("REFERENCES", "Reference constraint"),
    ("UNIQUE", "Unique constraint"),
    ("CHECK", "Check constraint"),
    ("DEFAULT", "Default value"),
    ("NOT NULL", "Not null constraint"),
    ("NULL", "NULL value"),
    ("TRUE", "Boolean true"),
    ("FALSE", "Boolean false"),
];

const SQL_FUNCTIONS: &[(&str, &str)] = &[
    ("COUNT(*)", "Count all rows"),
    ("COUNT(col)", "Count non-NULL values"),
    ("SUM(col)", "Sum of values"),
    ("AVG(col)", "Average value"),
    ("MIN(col)", "Minimum value"),
    ("MAX(col)", "Maximum value"),
    ("COALESCE(val1, val2, ...)", "First non-NULL value"),
    ("NULLIF(val1, val2)", "NULL if values equal"),
    ("CAST(expr AS type)", "Type conversion"),
    ("UPPER(str)", "Convert to uppercase"),
    ("LOWER(str)", "Convert to lowercase"),
    ("TRIM(str)", "Remove whitespace"),
    ("LENGTH(str)", "String length"),
    ("SUBSTRING(str, pos, len)", "Extract substring"),
    ("CONCAT(str1, str2)", "Concatenate strings"),
    ("REPLACE(str, from, to)", "Replace substring"),
    ("ABS(x)", "Absolute value"),
    ("ROUND(x, d)", "Round number"),
    ("FLOOR(x)", "Round down"),
    ("CEIL(x)", "Round up"),
    ("NOW()", "Current timestamp"),
    ("CURRENT_DATE", "Current date"),
    ("CURRENT_TIME", "Current time"),
];

/// 内置 SQL 数据类型（通用标准 SQL 类型）
pub(crate) const SQL_DATA_TYPES: &[(&str, &str)] = &[
    ("INT", "32-bit integer"),
    ("INTEGER", "32-bit integer"),
    ("BIGINT", "64-bit integer"),
    ("SMALLINT", "16-bit integer"),
    ("TINYINT", "8-bit integer"),
    ("FLOAT", "Floating-point number"),
    ("DOUBLE", "Double-precision floating-point"),
    ("DECIMAL", "Fixed-point number"),
    ("NUMERIC", "Fixed-point number"),
    ("REAL", "Floating-point number"),
    ("CHAR", "Fixed-length string"),
    ("VARCHAR", "Variable-length string"),
    ("TEXT", "Variable-length text"),
    ("NCHAR", "Fixed-length Unicode string"),
    ("NVARCHAR", "Variable-length Unicode string"),
    ("BOOLEAN", "Boolean value"),
    ("BOOL", "Boolean value"),
    ("DATE", "Date value"),
    ("TIME", "Time value"),
    ("DATETIME", "Date and time"),
    ("TIMESTAMP", "Timestamp value"),
    ("BLOB", "Binary large object"),
    ("CLOB", "Character large object"),
    ("BINARY", "Fixed-length binary"),
    ("VARBINARY", "Variable-length binary"),
    ("JSON", "JSON data"),
    ("XML", "XML data"),
    ("UUID", "UUID value"),
    ("SERIAL", "Auto-incrementing integer"),
];

#[derive(Clone)]
pub struct DefaultSqlCompletionProvider {
    sources: Rc<RefCell<SqlCompletionSources>>,
}

#[derive(Clone)]
pub(crate) struct SqlCompletionSources {
    pub(crate) schema: Arc<SqlSchema>,
    pub(crate) db_completion_info: Option<SqlCompletionInfo>,
}

impl Default for SqlCompletionSources {
    fn default() -> Self {
        Self {
            schema: Arc::new(SqlSchema::default()),
            db_completion_info: None,
        }
    }
}

impl DefaultSqlCompletionProvider {
    pub fn new(schema: SqlSchema) -> Self {
        Self {
            sources: Rc::new(RefCell::new(SqlCompletionSources {
                schema: Arc::new(schema),
                db_completion_info: None,
            })),
        }
    }

    pub fn with_db_completion_info(self, info: SqlCompletionInfo) -> Self {
        self.sources.borrow_mut().db_completion_info = Some(info);
        self
    }

    /// Atomically replace metadata while keeping this provider object alive.
    pub fn set_sources(&self, schema: SqlSchema, db_completion_info: SqlCompletionInfo) {
        *self.sources.borrow_mut() = SqlCompletionSources {
            schema: Arc::new(schema),
            db_completion_info: Some(db_completion_info),
        };
    }

    pub(crate) fn sources(&self) -> SqlCompletionSources {
        self.sources.borrow().clone()
    }

    /// Parse SQL text and return both context and symbol table.
    ///
    /// This method is used when we need the symbol table for DotColumn filtering.
    ///
    /// 符号表只由**当前语句**的 token 构建：文档里其他语句的 FROM/JOIN 表和别名
    /// 不能参与本语句的补全，否则会出现「当前表的字段列表里混入别的表的字段」
    /// （issue #329）。上下文推断同样只看当前语句的 token，避免解析到上一条语句
    /// 遗留的点号。
    fn parse_context_with_symbols(tokens: &[SqlToken], offset: usize) -> (SqlContext, SymbolTable) {
        let statement_tokens = current_statement_tokens(tokens, offset);
        let symbol_table = SymbolTable::build_from_tokens(statement_tokens);
        let inferred = ContextInferrer::infer(statement_tokens, offset, &symbol_table);
        (Self::convert_context(inferred), symbol_table)
    }

    /// Convert InferredSqlContext to local SqlContext enum.
    fn convert_context(inferred: InferredSqlContext) -> SqlContext {
        match inferred {
            InferredSqlContext::Start => SqlContext::Start,
            InferredSqlContext::SelectColumns => SqlContext::SelectColumns,
            InferredSqlContext::TableName => SqlContext::TableName,
            InferredSqlContext::Condition => SqlContext::Condition,
            InferredSqlContext::OrderBy => SqlContext::OrderBy,
            InferredSqlContext::SetClause => SqlContext::SetClause,
            InferredSqlContext::Values => SqlContext::Values,
            InferredSqlContext::CreateTable => SqlContext::CreateTable,
            InferredSqlContext::DotColumn(alias) => SqlContext::DotColumn(alias),
            InferredSqlContext::FunctionArgs => SqlContext::FunctionArgs,
        }
    }
}

pub(crate) fn clip_sql_offset(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset = offset.saturating_sub(1);
    }
    offset
}

pub(crate) fn cursor_is_in_sql_literal_or_comment(
    text: &str,
    tokens: &[SqlToken],
    offset: usize,
) -> bool {
    let offset = clip_sql_offset(text, offset);
    tokens.iter().any(|token| {
        if token.start >= offset || offset > token.end {
            return false;
        }

        match token.kind {
            SqlTokenKind::String => {
                offset < token.end
                    || (token.end == text.len()
                        && !token
                            .text
                            .strip_prefix('\'')
                            .is_some_and(|body| body.ends_with('\'') && !body.ends_with("''")))
            }
            SqlTokenKind::LineComment => offset <= token.end,
            SqlTokenKind::BlockComment => {
                offset < token.end
                    || (token.end == text.len() && !token.text.trim_end().ends_with("*/"))
            }
            _ => false,
        }
    })
}

/// 当前语句（以分号划分）覆盖的字节范围 `[start, end)`。
///
/// 起点是光标之前最后一个分号的末尾，终点是光标之后第一个分号的起点；光标之后
/// 没有分号时以 token 流末端（Eof token 的偏移）为界。分号划分对 `DELIMITER`
/// 之类的存储过程体并不精确，但这是旧的 `current_statement_has_from_keyword`
/// 已经在用的口径，收窄只会少给候选、不会给错候选。
fn current_statement_bounds(tokens: &[SqlToken], offset: usize) -> std::ops::Range<usize> {
    let start = tokens
        .iter()
        .filter(|token| token.kind == SqlTokenKind::Semicolon && token.end <= offset)
        .map(|token| token.end)
        .next_back()
        .unwrap_or(0);
    let end = tokens
        .iter()
        .find(|token| token.kind == SqlTokenKind::Semicolon && token.start >= offset)
        .map(|token| token.start)
        .unwrap_or_else(|| tokens.last().map_or(0, |token| token.end));

    start..end
}

/// 当前语句（以分号划分）的 token 子切片，含语句内的空白与注释；终止分号本身不计入。
///
/// 补全时用它来构建符号表和推断上下文，保证候选只来自光标所在的那条语句。
pub(crate) fn current_statement_tokens(tokens: &[SqlToken], offset: usize) -> &[SqlToken] {
    let bounds = current_statement_bounds(tokens, offset);
    let start = tokens
        .iter()
        .position(|token| token.end > bounds.start)
        .unwrap_or(tokens.len());
    let end = tokens
        .iter()
        .position(|token| token.start >= bounds.end)
        .unwrap_or(tokens.len());

    &tokens[start.min(end)..end]
}

pub(crate) fn current_statement_has_from_keyword(
    text: &str,
    tokens: &[SqlToken],
    offset: usize,
) -> bool {
    let offset = clip_sql_offset(text, offset);
    let bounds = current_statement_bounds(tokens, offset);

    tokens.iter().any(|token| {
        token.start >= bounds.start
            && token.end <= bounds.end
            && token.is_keyword_of(SqlKeyword::From)
    })
}

pub(crate) fn insert_column_target_table(text: &str, offset: usize) -> Option<String> {
    let meaningful = meaningful_tokens_before(text, offset);
    let into = meaningful
        .iter()
        .rposition(|token| token.is_keyword_of(SqlKeyword::Into))?;
    let (table, target_end) = qualified_table_after(&meaningful, into + 1)?;
    let mut stack = Vec::new();
    let mut values_before_open = false;
    for token in meaningful.iter().skip(target_end) {
        match token.kind {
            SqlTokenKind::LParen => stack.push(token),
            SqlTokenKind::RParen => {
                stack.pop();
            }
            SqlTokenKind::Keyword(SqlKeyword::Values) if stack.is_empty() => {
                values_before_open = true;
            }
            _ => {}
        }
    }
    (!values_before_open && !stack.is_empty()).then_some(table)
}

pub(crate) fn update_target_table(text: &str, offset: usize) -> Option<String> {
    let meaningful = meaningful_tokens_before(text, offset);
    let update = meaningful
        .iter()
        .rposition(|token| token.is_keyword_of(SqlKeyword::Update))?;
    qualified_table_after(&meaningful, update + 1).map(|(table, _)| table)
}

fn meaningful_tokens_before(text: &str, offset: usize) -> Vec<SqlToken> {
    let mut tokenizer = SqlTokenizer::new(text);
    tokenizer
        .tokenize()
        .into_iter()
        .filter(|token| {
            token.end <= offset
                && !token.is_whitespace()
                && !token.is_comment()
                && token.kind != SqlTokenKind::Eof
        })
        .collect()
}

fn qualified_table_after(tokens: &[SqlToken], start: usize) -> Option<(String, usize)> {
    let first = tokens.get(start)?;
    if !matches!(first.kind, SqlTokenKind::Ident | SqlTokenKind::QuotedIdent) {
        return None;
    }
    let mut table = completion_identifier_text(first);
    let mut index = start + 1;
    while tokens
        .get(index)
        .is_some_and(|token| token.kind == SqlTokenKind::Dot)
    {
        let part = tokens.get(index + 1)?;
        if !matches!(part.kind, SqlTokenKind::Ident | SqlTokenKind::QuotedIdent) {
            break;
        }
        table = completion_identifier_text(part);
        index += 2;
    }
    Some((table, index))
}

fn completion_identifier_text(token: &SqlToken) -> String {
    let text = &token.text;
    if let Some(body) = text
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    {
        body.replace("\"\"", "\"")
    } else if let Some(body) = text
        .strip_prefix('`')
        .and_then(|value| value.strip_suffix('`'))
    {
        body.replace("``", "`")
    } else if let Some(body) = text
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
    {
        body.replace("]]", "]")
    } else {
        text.clone()
    }
}

/// 查找已缓存的外部 qualifier 元数据（大小写不敏感）。
pub(crate) fn find_foreign_schema<'a>(
    schema: &'a SqlSchema,
    qualifier: &str,
) -> Option<&'a ForeignSchema> {
    schema
        .foreign_schemas
        .get(&qualifier.to_lowercase())
        .or_else(|| {
            schema
                .foreign_schemas
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(qualifier))
                .map(|(_, value)| value)
        })
}

/// 当前列查找：先当前库，再回退到外部 qualifier 缓存
/// （SymbolTable 对 `FROM db.tbl` 只保留表名，跨库表靠兜底解析）。
pub(crate) fn find_columns_with_foreign<'a>(
    schema: &'a SqlSchema,
    table: &str,
) -> Option<&'a Vec<(String, String, String)>> {
    find_schema_columns(schema, table).or_else(|| {
        schema.foreign_schemas.values().find_map(|foreign| {
            foreign.columns_by_table.get(table).or_else(|| {
                foreign
                    .columns_by_table
                    .iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case(table))
                    .map(|(_, value)| value)
            })
        })
    })
}

/// 补全项构建上下文，统一过滤、匹配加权和 text_edit 生成。
struct ItemBuildContext<'a> {
    context: &'a SqlContext,
    current_word: &'a str,
    replace_range: LspRange,
}

impl ItemBuildContext<'_> {
    fn matches(&self, label: &str) -> bool {
        identifier_match_rank(label, self.current_word).is_some()
    }

    fn match_boost(&self, label: &str) -> i32 {
        match identifier_match_rank(label, self.current_word) {
            Some(0) => completion_priority::PREFIX_MATCH_BOOST,
            Some(1) => completion_priority::BOUNDARY_MATCH_BOOST,
            _ => 0,
        }
    }

    fn matched_prefix(&self, label: &str) -> String {
        let upper = label.to_uppercase();
        if !self.current_word.is_empty() && upper.starts_with(self.current_word) {
            label
                .chars()
                .take(self.current_word.chars().count())
                .collect()
        } else {
            String::new()
        }
    }

    fn kind_base(kind: CompletionItemKind) -> i32 {
        match kind {
            CompletionItemKind::KEYWORD => completion_priority::KEYWORDS_BASE,
            CompletionItemKind::TYPE_PARAMETER => completion_priority::DATA_TYPES_BASE,
            CompletionItemKind::STRUCT => completion_priority::TABLES_BASE,
            CompletionItemKind::FIELD => completion_priority::COLUMNS_BASE,
            CompletionItemKind::FUNCTION => completion_priority::FUNCTIONS_BASE,
            CompletionItemKind::OPERATOR => completion_priority::OPERATORS_BASE,
            CompletionItemKind::SNIPPET => completion_priority::SNIPPETS_BASE,
            _ => completion_priority::COLUMNS_BASE,
        }
    }

    /// 以 `base` 作为类型基准分（默认类型基准之外的自定义基准，如 QUALIFIERS_BASE）。
    fn score(&self, label: &str, kind: CompletionItemKind, base: i32) -> i32 {
        completion_priority::calculate_score_with_match(
            self.context,
            Some(kind),
            self.match_boost(label),
        ) - Self::kind_base(kind)
            + base
    }

    #[allow(clippy::too_many_arguments)]
    fn push(
        &self,
        items: &mut Vec<CompletionItem>,
        label: String,
        new_text: String,
        kind: CompletionItemKind,
        base: i32,
        detail: Option<String>,
        doc: Option<String>,
    ) {
        let filter_text = self.matched_prefix(&label);
        let sort_text =
            completion_priority::score_to_sort_text(self.score(&label, kind, base), &label);
        items.push(CompletionItem {
            label,
            kind: Some(kind),
            detail,
            text_edit: Some(CompletionTextEdit::InsertAndReplace(InsertReplaceEdit {
                new_text,
                insert: self.replace_range,
                replace: self.replace_range,
            })),
            filter_text: Some(filter_text),
            documentation: doc.map(lsp_types::Documentation::String),
            sort_text: Some(sort_text),
            ..Default::default()
        });
    }
}

/// 外部 qualifier 的表列表补全项。
pub(crate) fn foreign_table_items(
    foreign: &ForeignSchema,
    context: &SqlContext,
    current_word: &str,
    replace_range: LspRange,
) -> Vec<CompletionItem> {
    let ctx = ItemBuildContext {
        context,
        current_word,
        replace_range,
    };
    let mut items = Vec::new();
    for (table, doc) in &foreign.tables {
        if !ctx.matches(table) {
            continue;
        }
        ctx.push(
            &mut items,
            table.clone(),
            table.clone(),
            CompletionItemKind::STRUCT,
            completion_priority::TABLES_BASE,
            Some("Table".to_string()),
            Some(doc.clone()),
        );
    }
    sort_and_truncate(items)
}

/// 外部 qualifier 内某张表的列补全项。
pub(crate) fn foreign_column_items(
    foreign: &ForeignSchema,
    table: &str,
    context: &SqlContext,
    current_word: &str,
    replace_range: LspRange,
) -> Vec<CompletionItem> {
    let Some(columns) = foreign.columns_by_table.get(table).or_else(|| {
        foreign
            .columns_by_table
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(table))
            .map(|(_, value)| value)
    }) else {
        return Vec::new();
    };
    column_list_items(columns, table, context, current_word, replace_range)
}

/// 列补全项（label 为列名，detail 展示类型或来源表）。
pub(crate) fn column_list_items(
    columns: &[(String, String, String)],
    table: &str,
    context: &SqlContext,
    current_word: &str,
    replace_range: LspRange,
) -> Vec<CompletionItem> {
    let ctx = ItemBuildContext {
        context,
        current_word,
        replace_range,
    };
    let mut items = Vec::new();
    for (column, data_type, doc) in columns {
        if !ctx.matches(column) {
            continue;
        }
        let detail = if data_type.is_empty() {
            format!("{table}.{column}")
        } else {
            format!("{column}: {data_type}")
        };
        ctx.push(
            &mut items,
            column.clone(),
            column.clone(),
            CompletionItemKind::FIELD,
            completion_priority::COLUMNS_BASE,
            Some(detail),
            (!doc.is_empty()).then(|| doc.clone()),
        );
    }
    sort_and_truncate(items)
}

/// database/schema（qualifier）名补全项，接受后插入 `name.` 触发表名补全。
pub(crate) fn qualifier_name_items(
    schema: &SqlSchema,
    context: &SqlContext,
    current_word: &str,
    replace_range: LspRange,
) -> Vec<CompletionItem> {
    let ctx = ItemBuildContext {
        context,
        current_word,
        replace_range,
    };
    let mut candidates = Vec::new();
    if let Some(database) = &schema.current_database {
        candidates.push((
            database.clone(),
            t!("SqlEditor.database_object").to_string(),
        ));
    }
    if let Some(current_schema) = &schema.current_schema {
        if !candidates
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case(current_schema))
        {
            candidates.push((
                current_schema.clone(),
                t!("SqlEditor.schema_object").to_string(),
            ));
        }
    }
    for (name, doc) in &schema.qualifiers {
        if !candidates
            .iter()
            .any(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
        {
            candidates.push((name.clone(), doc.clone()));
        }
    }

    let mut items = Vec::new();
    for (name, doc) in candidates {
        if !ctx.matches(&name) {
            continue;
        }
        ctx.push(
            &mut items,
            name.clone(),
            format!("{name}."),
            CompletionItemKind::MODULE,
            completion_priority::QUALIFIERS_BASE,
            Some(doc.clone()),
            (!doc.is_empty()).then_some(doc),
        );
    }
    sort_and_truncate(items)
}

fn sort_and_truncate(items: Vec<CompletionItem>) -> Vec<CompletionItem> {
    let mut items = items;
    items.sort_by(|a, b| {
        a.sort_text
            .as_ref()
            .unwrap_or(&a.label)
            .cmp(b.sort_text.as_ref().unwrap_or(&b.label))
    });
    items.truncate(50);
    items
}

/// 扫描文本中出现的、已知且尚未缓存的外部 qualifier（`q.` 模式），供懒加载触发。
pub(crate) fn pending_foreign_qualifiers(text: &str, schema: &SqlSchema) -> Vec<String> {
    if schema.qualifiers.is_empty() {
        return Vec::new();
    }
    let mut tokenizer = SqlTokenizer::new(text);
    let tokens = tokenizer.tokenize();
    let mut found: Vec<String> = Vec::new();
    for pair in tokens.windows(2) {
        if pair[1].kind != SqlTokenKind::Dot {
            continue;
        }
        if !matches!(
            pair[0].kind,
            SqlTokenKind::Ident | SqlTokenKind::QuotedIdent
        ) {
            continue;
        }
        let name = completion_identifier_text(&pair[0]);
        let Some((qualifier, _)) = schema
            .qualifiers
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(&name))
        else {
            continue;
        };
        let cached = schema
            .foreign_schemas
            .contains_key(&qualifier.to_lowercase());
        if !cached
            && !found
                .iter()
                .any(|item| item.eq_ignore_ascii_case(qualifier))
        {
            found.push(qualifier.clone());
        }
    }
    found
}

impl CompletionProvider for DefaultSqlCompletionProvider {
    fn completions(
        &self,
        rope: &Rope,
        offset: usize,
        _trigger: CompletionContext,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<CompletionResponse>> {
        let rope = rope.clone();
        let SqlCompletionSources {
            schema,
            db_completion_info: db_info,
        } = self.sources();

        cx.background_spawn(async move {
            let text = rope.to_string();
            let offset = rope.clip_offset(offset.min(rope.len()), Bias::Left);
            debug_assert!(text.is_char_boundary(offset));
            let before_cursor = &text[..offset];

            let mut tokenizer = SqlTokenizer::new(&text);
            let tokens = tokenizer.tokenize();
            if cursor_is_in_sql_literal_or_comment(&text, &tokens, offset) {
                return Ok(CompletionResponse::Array(vec![]));
            }

            // 分号后不显示补全（语句结束）
            if before_cursor.ends_with(';') {
                return Ok(CompletionResponse::Array(vec![]));
            }

            // Use tokenizer-based context parsing with symbol table
            let (context, symbol_table) = Self::parse_context_with_symbols(&tokens, offset);

            // Current word - find word start by scanning backwards from offset
            // Use clip_offset to ensure we're on a char boundary
            let mut start_offset = offset;
            while start_offset > 0 {
                let prev_offset = rope.clip_offset(start_offset.saturating_sub(1), Bias::Left);
                if prev_offset >= start_offset {
                    break;
                }
                let ch = rope.char(prev_offset);
                // 只将 ASCII 字母数字和下划线视为 SQL 标识符字符
                // 中文等非 ASCII 字符作为词边界处理，以便输入中文后仍能显示 SQL 补全
                if !(ch.is_ascii_alphanumeric() || ch == '_') {
                    break;
                }
                start_offset = prev_offset;
            }
            let current_word = rope.slice(start_offset..offset).to_string().to_uppercase();

            let start_pos = rope.offset_to_position(start_offset);
            let end_pos = rope.offset_to_position(offset);
            let replace_range = LspRange::new(start_pos, end_pos);

            let mut items = Vec::new();

            let matches_filter =
                |label: &str| -> bool { identifier_match_rank(label, &current_word).is_some() };

            // Match-quality boost: prefix > word-boundary > substring.
            let match_boost = |label: &str| -> i32 {
                match identifier_match_rank(label, &current_word) {
                    Some(0) => completion_priority::PREFIX_MATCH_BOOST,
                    Some(1) => completion_priority::BOUNDARY_MATCH_BOOST,
                    _ => 0,
                }
            };

            let matched_prefix = |label: &str| -> String {
                let lu = label.to_uppercase();
                if !current_word.is_empty() && lu.starts_with(&current_word) {
                    label.chars().take(current_word.chars().count()).collect()
                } else {
                    String::new()
                }
            };

            let target_table = insert_column_target_table(&text, offset).or_else(|| {
                matches!(context, SqlContext::SetClause)
                    .then(|| update_target_table(&text, offset))
                    .flatten()
            });
            if let Some(target_table) = target_table {
                if let Some(columns) = find_columns_with_foreign(&schema, &target_table) {
                    for (column, data_type, doc) in columns {
                        if !matches_filter(column) {
                            continue;
                        }
                        let score = completion_priority::calculate_score_with_match(
                            &SqlContext::SetClause,
                            Some(CompletionItemKind::FIELD),
                            match_boost(column),
                        );
                        items.push(CompletionItem {
                            label: column.clone(),
                            kind: Some(CompletionItemKind::FIELD),
                            detail: Some(if data_type.is_empty() {
                                format!("{target_table}.{column}")
                            } else {
                                format!("{column}: {data_type}")
                            }),
                            text_edit: Some(CompletionTextEdit::InsertAndReplace(
                                InsertReplaceEdit {
                                    new_text: column.clone(),
                                    insert: replace_range,
                                    replace: replace_range,
                                },
                            )),
                            filter_text: Some(matched_prefix(column)),
                            documentation: Some(lsp_types::Documentation::String(doc.clone())),
                            sort_text: Some(completion_priority::score_to_sort_text(score, column)),
                            ..Default::default()
                        });
                    }
                }
                items.sort_by(|a, b| a.sort_text.cmp(&b.sort_text));
                items.truncate(50);
                return Ok(CompletionResponse::Array(items));
            }

            // Handle dot context (table.column) - highest priority
            // Uses SymbolTable to resolve alias to actual table name
            if let SqlContext::DotColumn(alias_or_table) = &context {
                let chain = dot_qualifier_chain(&text, offset);
                let qualifier = chain
                    .last()
                    .cloned()
                    .unwrap_or_else(|| alias_or_table.clone());

                // 多级限定链（db.tbl. / schema.tbl.）直接按元数据解析，不走别名解析
                if chain.len() >= 2 {
                    let items = match sql_dot_completion_target_for_chain(&schema, &chain) {
                        SqlDotCompletionTarget::ForeignTables(name) => {
                            find_foreign_schema(&schema, &name)
                                .map(|foreign| {
                                    foreign_table_items(
                                        foreign,
                                        &SqlContext::TableName,
                                        &current_word,
                                        replace_range,
                                    )
                                })
                                .unwrap_or_default()
                        }
                        SqlDotCompletionTarget::ForeignColumns(qualifier, table) => {
                            find_foreign_schema(&schema, &qualifier)
                                .map(|foreign| {
                                    foreign_column_items(
                                        foreign,
                                        &table,
                                        &context,
                                        &current_word,
                                        replace_range,
                                    )
                                })
                                .unwrap_or_default()
                        }
                        SqlDotCompletionTarget::Columns(table) => {
                            find_columns_with_foreign(&schema, &table)
                                .map(|columns| {
                                    column_list_items(
                                        columns,
                                        &table,
                                        &context,
                                        &current_word,
                                        replace_range,
                                    )
                                })
                                .unwrap_or_default()
                        }
                        SqlDotCompletionTarget::Tables | SqlDotCompletionTarget::None => Vec::new(),
                    };
                    return Ok(CompletionResponse::Array(items));
                }

                // Resolve alias to table name using symbol table
                // If alias is found, use the resolved table name; otherwise use as-is
                let resolved_table = symbol_table
                    .resolve(&qualifier)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| alias_or_table.clone());

                let projected_columns = symbol_table
                    .projected_columns(&qualifier)
                    .or_else(|| symbol_table.projected_columns(&resolved_table));
                if let Some(columns) = projected_columns {
                    for column in columns {
                        if !matches_filter(column) {
                            continue;
                        }
                        let score = completion_priority::calculate_score_with_match(
                            &context,
                            Some(CompletionItemKind::FIELD),
                            match_boost(column),
                        );
                        items.push(CompletionItem {
                            label: column.clone(),
                            kind: Some(CompletionItemKind::FIELD),
                            detail: Some(format!("{qualifier}.{column}")),
                            text_edit: Some(CompletionTextEdit::InsertAndReplace(
                                InsertReplaceEdit {
                                    new_text: column.clone(),
                                    insert: replace_range,
                                    replace: replace_range,
                                },
                            )),
                            filter_text: Some(matched_prefix(column)),
                            sort_text: Some(completion_priority::score_to_sort_text(score, column)),
                            ..Default::default()
                        });
                    }
                    items.sort_by(|a, b| a.sort_text.cmp(&b.sort_text));
                    items.truncate(50);
                    return Ok(CompletionResponse::Array(items));
                }

                if sql_dot_completion_target(&schema, &resolved_table)
                    == SqlDotCompletionTarget::Tables
                {
                    for (table, doc) in &schema.tables {
                        if matches_filter(table) {
                            let score = completion_priority::calculate_score_with_match(
                                &SqlContext::TableName,
                                Some(CompletionItemKind::STRUCT),
                                match_boost(table),
                            );
                            items.push(CompletionItem {
                                label: table.clone(),
                                kind: Some(CompletionItemKind::STRUCT),
                                detail: Some("Table".to_string()),
                                text_edit: Some(CompletionTextEdit::InsertAndReplace(
                                    InsertReplaceEdit {
                                        new_text: table.clone(),
                                        insert: replace_range,
                                        replace: replace_range,
                                    },
                                )),
                                filter_text: Some(matched_prefix(table)),
                                documentation: Some(lsp_types::Documentation::String(doc.clone())),
                                sort_text: Some(completion_priority::score_to_sort_text(
                                    score, table,
                                )),
                                ..Default::default()
                            });
                        }
                    }
                    // 其他 database/schema 候选（接受后插入 `name.`）
                    items.extend(qualifier_name_items(
                        &schema,
                        &SqlContext::TableName,
                        &current_word,
                        replace_range,
                    ));
                    items.sort_by(|a, b| {
                        a.sort_text
                            .as_ref()
                            .unwrap_or(&a.label)
                            .cmp(b.sort_text.as_ref().unwrap_or(&b.label))
                    });
                    items.truncate(50);
                    return Ok(CompletionResponse::Array(items));
                }

                // 外部 database/schema 前缀（q.）→ 该库/schema 的表列表
                // 元数据未加载时返回空列表，视图层懒加载完成后会刷新补全。
                if let SqlDotCompletionTarget::ForeignTables(name) =
                    sql_dot_completion_target_for_chain(&schema, &chain)
                {
                    let items = find_foreign_schema(&schema, &name)
                        .map(|foreign| {
                            foreign_table_items(
                                foreign,
                                &SqlContext::TableName,
                                &current_word,
                                replace_range,
                            )
                        })
                        .unwrap_or_default();
                    return Ok(CompletionResponse::Array(items));
                }

                // 处理子查询别名：显示提示信息
                if resolved_table == "#subquery" {
                    items.push(CompletionItem {
                        label: "(subquery)".to_string(),
                        kind: Some(CompletionItemKind::TEXT),
                        detail: Some(t!("SqlEditor.subquery_column_uninferable").to_string()),
                        documentation: Some(lsp_types::Documentation::String(
                            t!("SqlEditor.subquery_doc", alias = alias_or_table).to_string(),
                        )),
                        ..Default::default()
                    });
                    return Ok(CompletionResponse::Array(items));
                }

                // 处理 CTE 别名：显示提示信息
                if resolved_table == "#cte" {
                    items.push(CompletionItem {
                        label: "(CTE)".to_string(),
                        kind: Some(CompletionItemKind::TEXT),
                        detail: Some(t!("SqlEditor.cte_column_uninferable").to_string()),
                        documentation: Some(lsp_types::Documentation::String(
                            t!("SqlEditor.cte_doc", alias = alias_or_table).to_string(),
                        )),
                        ..Default::default()
                    });
                    return Ok(CompletionResponse::Array(items));
                }

                // Try to find columns for the resolved table
                // First try exact match, then case-insensitive match
                let resolved_table = match sql_dot_completion_target(&schema, &resolved_table) {
                    SqlDotCompletionTarget::Columns(table) => table,
                    _ => return Ok(CompletionResponse::Array(items)),
                };
                // 当前库查不到时回退外部 qualifier 缓存（FROM db.tbl 只解析出裸表名）
                let columns = find_columns_with_foreign(&schema, &resolved_table);

                if let Some(cols) = columns {
                    for (column, data_type, doc) in cols {
                        if matches_filter(column) {
                            let boost = match_boost(column);
                            let score = completion_priority::calculate_score_with_match(
                                &context,
                                Some(CompletionItemKind::FIELD),
                                boost,
                            );
                            // 在 detail 中显示类型信息
                            let detail = if data_type.is_empty() {
                                format!("{}.{}", resolved_table, column)
                            } else {
                                format!("{}: {}", column, data_type)
                            };
                            items.push(CompletionItem {
                                label: column.clone(),
                                kind: Some(CompletionItemKind::FIELD),
                                detail: Some(detail),
                                text_edit: Some(CompletionTextEdit::InsertAndReplace(
                                    InsertReplaceEdit {
                                        new_text: column.clone(),
                                        insert: replace_range,
                                        replace: replace_range,
                                    },
                                )),
                                filter_text: Some(matched_prefix(column)),
                                documentation: Some(lsp_types::Documentation::String(doc.clone())),
                                sort_text: Some(completion_priority::score_to_sort_text(
                                    score, column,
                                )),
                                ..Default::default()
                            });
                        }
                    }
                }
                // Sort by score and truncate (Requirement 5.6: limit to 50 items)
                items.sort_by(|a, b| {
                    a.sort_text
                        .as_ref()
                        .unwrap_or(&a.label)
                        .cmp(b.sort_text.as_ref().unwrap_or(&b.label))
                });
                items.truncate(50);
                return Ok(CompletionResponse::Array(items));
            }

            // Handle CREATE TABLE context - special logic to distinguish different positions
            if context == SqlContext::CreateTable {
                let before_word = &text[..start_offset];

                // 检查是否在括号内
                let has_open_paren = before_word.contains('(');

                if !has_open_paren {
                    // 括号外：用户在输入表名，不显示补全
                    // 例如：CREATE TABLE users|
                    return Ok(CompletionResponse::Array(vec![]));
                }

                // 在括号内，检查光标前的 token 来判断用户在输入什么
                let prev_char = before_word.chars().rev().find(|c| !c.is_whitespace());

                match prev_char {
                    // 括号或逗号后：用户在输入字段名称，不显示补全
                    // 例如：(id INT, name|  或  (id|
                    Some('(') | Some(',') => {
                        return Ok(CompletionResponse::Array(vec![]));
                    }
                    // 右括号后：约束定义结束
                    Some(')') => {
                        if current_word.is_empty() {
                            return Ok(CompletionResponse::Array(vec![]));
                        }
                    }
                    // 其他情况（标识符或数据类型后）：显示数据类型和约束
                    // 例如：(id |  或  (id INT |
                    _ => {}
                }
            }

            // Context-aware completion priorities
            // (show_tables, show_columns, show_keywords, show_functions, show_types)
            let (show_tables, show_columns, show_keywords, show_functions, show_types) =
                match context {
                    SqlContext::TableName => (true, false, false, false, false),
                    SqlContext::SelectColumns => (false, true, true, true, false), // Allow keywords like FROM, AS, DISTINCT
                    SqlContext::OrderBy | SqlContext::SetClause => (false, true, true, true, false),
                    SqlContext::Condition => (false, true, true, true, false),
                    SqlContext::FunctionArgs => (false, true, false, true, false),
                    SqlContext::CreateTable => (false, false, true, false, true),
                    SqlContext::Values => (false, false, false, true, false),
                    SqlContext::Start => (true, false, true, false, false), // 语句开始，没有 FROM，不显示字段
                    SqlContext::DotColumn(_) => (false, true, false, false, false), // Only show columns for table.column
                };

            // Tables - priority based on context (Requirement 5.2)
            if show_tables {
                for (table, doc) in &schema.tables {
                    if matches_filter(table) {
                        let boost = match_boost(table);
                        let score = completion_priority::calculate_score_with_match(
                            &context,
                            Some(CompletionItemKind::STRUCT),
                            boost,
                        );
                        items.push(CompletionItem {
                            label: table.clone(),
                            kind: Some(CompletionItemKind::STRUCT),
                            detail: Some("Table".to_string()),
                            text_edit: Some(CompletionTextEdit::InsertAndReplace(
                                InsertReplaceEdit {
                                    new_text: table.clone(),
                                    insert: replace_range,
                                    replace: replace_range,
                                },
                            )),
                            filter_text: Some(matched_prefix(table)),
                            documentation: Some(lsp_types::Documentation::String(doc.clone())),
                            sort_text: Some(completion_priority::score_to_sort_text(score, table)),
                            ..Default::default()
                        });
                    }
                }
                // 其他 database/schema 候选（接受后插入 `name.`）
                items.extend(qualifier_name_items(
                    &schema,
                    &context,
                    &current_word,
                    replace_range,
                ));
            }

            // 限定引用在列位置同样合法（SELECT test2.tbl. / WHERE test2.tbl.col = ...），
            // 因此列上下文也要提供其他 database/schema 候选。
            if show_columns {
                items.extend(qualifier_name_items(
                    &schema,
                    &context,
                    &current_word,
                    replace_range,
                ));
            }

            // Columns - priority based on context (Requirements 5.3, 5.4)
            if show_columns {
                // In contexts where we have table information (SelectColumns, Condition, OrderBy, SetClause),
                // show columns from tables in FROM/JOIN clauses
                let use_table_columns = matches!(
                    context,
                    SqlContext::SelectColumns
                        | SqlContext::Condition
                        | SqlContext::OrderBy
                        | SqlContext::SetClause
                );

                if use_table_columns {
                    // 检查完整当前语句（包括光标后的部分），并忽略字符串/注释中的 FROM。
                    let has_from = current_statement_has_from_keyword(&text, &tokens, offset);

                    if !has_from {
                        // 当前语句没有 FROM，跳过列显示
                    } else {
                        // Get all tables from symbol table
                        let tables: Vec<String> = symbol_table
                            .all_aliases()
                            .map(|(_, table)| table.to_string())
                            .collect();

                        // Deduplicate tables (in case of multiple aliases for same table)
                        let mut seen_tables = std::collections::HashSet::new();
                        let unique_tables: Vec<String> = tables
                            .into_iter()
                            .filter(|t| seen_tables.insert(t.to_lowercase()))
                            .collect();

                        // 收集所有列及其来源表，用于检测重复列名
                        let mut all_columns: Vec<(String, String, String, String)> = Vec::new(); // (column, table, data_type, doc)
                        for table in &unique_tables {
                            let columns = schema.columns_by_table.get(table).or_else(|| {
                                let lower = table.to_lowercase();
                                schema
                                    .columns_by_table
                                    .iter()
                                    .find(|(k, _)| k.to_lowercase() == lower)
                                    .map(|(_, v)| v)
                            });
                            if let Some(cols) = columns {
                                for (column, data_type, doc) in cols {
                                    all_columns.push((
                                        column.clone(),
                                        table.clone(),
                                        data_type.clone(),
                                        doc.clone(),
                                    ));
                                }
                            }
                        }

                        // 统计每个列名出现的次数
                        let mut column_counts: std::collections::HashMap<String, usize> =
                            std::collections::HashMap::new();
                        for (column, _, _, _) in &all_columns {
                            *column_counts.entry(column.to_lowercase()).or_insert(0) += 1;
                        }

                        // 生成补全项，重复列名显示为 table.column
                        for (column, table, data_type, doc) in all_columns {
                            if matches_filter(&column) {
                                let is_duplicate =
                                    column_counts.get(&column.to_lowercase()).unwrap_or(&0) > &1;
                                let (label, new_text) = if is_duplicate {
                                    // 多表同名列：显示 table.column 格式
                                    (
                                        format!("{}.{}", table, column),
                                        format!("{}.{}", table, column),
                                    )
                                } else {
                                    // 唯一列名：只显示 column
                                    (column.clone(), column.clone())
                                };

                                // 在 detail 中显示类型信息
                                let detail = if data_type.is_empty() {
                                    format!("{}.{}", table, column)
                                } else {
                                    format!("{}: {}", column, data_type)
                                };

                                let boost = match_boost(&column);
                                let score = completion_priority::calculate_score_with_match(
                                    &context,
                                    Some(CompletionItemKind::FIELD),
                                    boost,
                                );
                                items.push(CompletionItem {
                                    label,
                                    kind: Some(CompletionItemKind::FIELD),
                                    detail: Some(detail),
                                    text_edit: Some(CompletionTextEdit::InsertAndReplace(
                                        InsertReplaceEdit {
                                            new_text,
                                            insert: replace_range,
                                            replace: replace_range,
                                        },
                                    )),
                                    filter_text: Some(matched_prefix(&column)),
                                    documentation: Some(lsp_types::Documentation::String(doc)),
                                    sort_text: Some(completion_priority::score_to_sort_text(
                                        score, &column,
                                    )),
                                    ..Default::default()
                                });
                            }
                        }
                    }
                } else {
                    // For other contexts (FunctionArgs, Start), show global columns
                    for (column, doc) in &schema.columns {
                        if matches_filter(column) {
                            let boost = match_boost(column);
                            let score = completion_priority::calculate_score_with_match(
                                &context,
                                Some(CompletionItemKind::FIELD),
                                boost,
                            );
                            items.push(CompletionItem {
                                label: column.clone(),
                                kind: Some(CompletionItemKind::FIELD),
                                detail: Some("Column".to_string()),
                                text_edit: Some(CompletionTextEdit::InsertAndReplace(
                                    InsertReplaceEdit {
                                        new_text: column.clone(),
                                        insert: replace_range,
                                        replace: replace_range,
                                    },
                                )),
                                filter_text: Some(matched_prefix(column)),
                                documentation: Some(lsp_types::Documentation::String(doc.clone())),
                                sort_text: Some(completion_priority::score_to_sort_text(
                                    score, column,
                                )),
                                ..Default::default()
                            });
                        }
                    }
                }
            }

            // Keywords - lower priority than context-specific items
            if show_keywords {
                // Standard SQL keywords
                for (keyword, doc) in SQL_KEYWORDS {
                    if matches_filter(keyword) {
                        let boost = match_boost(keyword);
                        let score = completion_priority::calculate_score_with_match(
                            &context,
                            Some(CompletionItemKind::KEYWORD),
                            boost,
                        );
                        items.push(CompletionItem {
                            label: keyword.to_string(),
                            kind: Some(CompletionItemKind::KEYWORD),
                            text_edit: Some(CompletionTextEdit::InsertAndReplace(
                                InsertReplaceEdit {
                                    new_text: keyword.to_string(),
                                    insert: replace_range,
                                    replace: replace_range,
                                },
                            )),
                            filter_text: Some(matched_prefix(keyword)),
                            documentation: Some(lsp_types::Documentation::String(doc.to_string())),
                            sort_text: Some(completion_priority::score_to_sort_text(
                                score, keyword,
                            )),
                            ..Default::default()
                        });
                    }
                }
                // Database-specific keywords
                if let Some(ref info) = db_info {
                    for (keyword, doc) in &info.keywords {
                        if matches_filter(keyword) {
                            let boost = match_boost(keyword);
                            let score = completion_priority::calculate_score_with_match(
                                &context,
                                Some(CompletionItemKind::KEYWORD),
                                boost,
                            );
                            items.push(CompletionItem {
                                label: keyword.to_string(),
                                kind: Some(CompletionItemKind::KEYWORD),
                                text_edit: Some(CompletionTextEdit::InsertAndReplace(
                                    InsertReplaceEdit {
                                        new_text: keyword.to_string(),
                                        insert: replace_range,
                                        replace: replace_range,
                                    },
                                )),
                                filter_text: Some(matched_prefix(keyword)),
                                documentation: Some(lsp_types::Documentation::String(
                                    doc.to_string(),
                                )),
                                sort_text: Some(completion_priority::score_to_sort_text(
                                    score, keyword,
                                )),
                                ..Default::default()
                            });
                        }
                    }
                    // Database-specific operators - higher priority in Condition context
                    for (op, doc) in &info.operators {
                        if matches_filter(op) {
                            let boost = match_boost(op);
                            let score = completion_priority::calculate_score_with_match(
                                &context,
                                Some(CompletionItemKind::OPERATOR),
                                boost,
                            );
                            items.push(CompletionItem {
                                label: op.to_string(),
                                kind: Some(CompletionItemKind::OPERATOR),
                                text_edit: Some(CompletionTextEdit::InsertAndReplace(
                                    InsertReplaceEdit {
                                        new_text: op.to_string(),
                                        insert: replace_range,
                                        replace: replace_range,
                                    },
                                )),
                                filter_text: Some(matched_prefix(op)),
                                documentation: Some(lsp_types::Documentation::String(
                                    doc.to_string(),
                                )),
                                sort_text: Some(completion_priority::score_to_sort_text(score, op)),
                                ..Default::default()
                            });
                        }
                    }
                }
            }

            // Functions - priority based on context (Requirement 5.3)
            if show_functions {
                for (func, doc) in &schema.functions {
                    let func_name = func.split('(').next().unwrap_or("");
                    if matches_filter(func_name) {
                        let boost = match_boost(func_name);
                        let score = completion_priority::calculate_score_with_match(
                            &context,
                            Some(CompletionItemKind::FUNCTION),
                            boost,
                        );
                        items.push(CompletionItem {
                            label: func.to_string(),
                            kind: Some(CompletionItemKind::FUNCTION),
                            text_edit: Some(CompletionTextEdit::InsertAndReplace(
                                InsertReplaceEdit {
                                    new_text: func.to_string(),
                                    insert: replace_range,
                                    replace: replace_range,
                                },
                            )),
                            filter_text: Some(matched_prefix(func_name)),
                            documentation: Some(lsp_types::Documentation::String(doc.to_string())),
                            sort_text: Some(completion_priority::score_to_sort_text(score, func)),
                            ..Default::default()
                        });
                    }
                }

                // Standard SQL functions
                for (func, doc) in SQL_FUNCTIONS {
                    let func_name = func.split('(').next().unwrap_or("");
                    if matches_filter(func_name) {
                        let boost = match_boost(func_name);
                        let score = completion_priority::calculate_score_with_match(
                            &context,
                            Some(CompletionItemKind::FUNCTION),
                            boost,
                        );
                        items.push(CompletionItem {
                            label: func.to_string(),
                            kind: Some(CompletionItemKind::FUNCTION),
                            text_edit: Some(CompletionTextEdit::InsertAndReplace(
                                InsertReplaceEdit {
                                    new_text: func.to_string(),
                                    insert: replace_range,
                                    replace: replace_range,
                                },
                            )),
                            filter_text: Some(matched_prefix(func_name)),
                            documentation: Some(lsp_types::Documentation::String(doc.to_string())),
                            sort_text: Some(completion_priority::score_to_sort_text(score, func)),
                            ..Default::default()
                        });
                    }
                }
                // Database-specific functions
                if let Some(ref info) = db_info {
                    for (func, doc) in &info.functions {
                        let func_name = func.split('(').next().unwrap_or("");
                        if matches_filter(func_name) {
                            let boost = match_boost(func_name);
                            let score = completion_priority::calculate_score_with_match(
                                &context,
                                Some(CompletionItemKind::FUNCTION),
                                boost,
                            );
                            items.push(CompletionItem {
                                label: func.to_string(),
                                kind: Some(CompletionItemKind::FUNCTION),
                                text_edit: Some(CompletionTextEdit::InsertAndReplace(
                                    InsertReplaceEdit {
                                        new_text: func.to_string(),
                                        insert: replace_range,
                                        replace: replace_range,
                                    },
                                )),
                                filter_text: Some(matched_prefix(func_name)),
                                documentation: Some(lsp_types::Documentation::String(
                                    doc.to_string(),
                                )),
                                sort_text: Some(completion_priority::score_to_sort_text(
                                    score, func,
                                )),
                                ..Default::default()
                            });
                        }
                    }
                }
            }

            // Data types - priority based on context
            // 在 CreateTable 上下文中，数据类型有最高优先级
            if show_types {
                // 收集数据库特定的数据类型名（大写），用于去重
                let mut seen_types = std::collections::HashSet::new();

                // 先添加数据库特定的数据类型（优先级更高，因为更精确）
                if let Some(ref info) = db_info {
                    for (dtype, doc) in &info.data_types {
                        seen_types.insert(dtype.to_uppercase());
                        if matches_filter(dtype) {
                            let boost = match_boost(dtype);
                            let score = completion_priority::calculate_score_with_match(
                                &context,
                                Some(CompletionItemKind::TYPE_PARAMETER),
                                boost,
                            );
                            items.push(CompletionItem {
                                label: dtype.to_string(),
                                kind: Some(CompletionItemKind::TYPE_PARAMETER),
                                text_edit: Some(CompletionTextEdit::InsertAndReplace(
                                    InsertReplaceEdit {
                                        new_text: dtype.to_string(),
                                        insert: replace_range,
                                        replace: replace_range,
                                    },
                                )),
                                filter_text: Some(matched_prefix(dtype)),
                                documentation: Some(lsp_types::Documentation::String(
                                    doc.to_string(),
                                )),
                                sort_text: Some(completion_priority::score_to_sort_text(
                                    score, dtype,
                                )),
                                ..Default::default()
                            });
                        }
                    }
                }

                // 再添加内置标准 SQL 数据类型（去除与数据库特定类型重复的）
                for (dtype, doc) in SQL_DATA_TYPES {
                    if seen_types.contains(&dtype.to_uppercase()) {
                        continue;
                    }
                    if matches_filter(dtype) {
                        let boost = match_boost(dtype);
                        let score = completion_priority::calculate_score_with_match(
                            &context,
                            Some(CompletionItemKind::TYPE_PARAMETER),
                            boost,
                        );
                        items.push(CompletionItem {
                            label: dtype.to_string(),
                            kind: Some(CompletionItemKind::TYPE_PARAMETER),
                            text_edit: Some(CompletionTextEdit::InsertAndReplace(
                                InsertReplaceEdit {
                                    new_text: dtype.to_string(),
                                    insert: replace_range,
                                    replace: replace_range,
                                },
                            )),
                            filter_text: Some(matched_prefix(dtype)),
                            documentation: Some(lsp_types::Documentation::String(doc.to_string())),
                            sort_text: Some(completion_priority::score_to_sort_text(score, dtype)),
                            ..Default::default()
                        });
                    }
                }
            }

            // Snippets - lowest priority (only at start)
            if matches!(context, SqlContext::Start) {
                // Default snippets
                let default_snippets: &[(&str, &str, &str)] = &[
                    ("sel*", "SELECT * FROM $1 WHERE $2", "Select all columns"),
                    ("selc", "SELECT COUNT(*) FROM $1 WHERE $2", "Count rows"),
                    ("ins", "INSERT INTO $1 ($2) VALUES ($3)", "Insert row"),
                    ("upd", "UPDATE $1 SET $2 WHERE $3", "Update rows"),
                    ("del", "DELETE FROM $1 WHERE $2", "Delete rows"),
                ];
                for (label, insert_text, doc) in default_snippets {
                    if matches_filter(label) {
                        let boost = match_boost(label);
                        let score = completion_priority::calculate_score_with_match(
                            &context,
                            Some(CompletionItemKind::SNIPPET),
                            boost,
                        );
                        items.push(CompletionItem {
                            label: label.to_string(),
                            kind: Some(CompletionItemKind::SNIPPET),
                            text_edit: Some(CompletionTextEdit::InsertAndReplace(
                                InsertReplaceEdit {
                                    new_text: insert_text.to_string(),
                                    insert: replace_range,
                                    replace: replace_range,
                                },
                            )),
                            insert_text_format: Some(InsertTextFormat::SNIPPET),
                            filter_text: Some(matched_prefix(label)),
                            documentation: Some(lsp_types::Documentation::String(doc.to_string())),
                            sort_text: Some(completion_priority::score_to_sort_text(score, label)),
                            ..Default::default()
                        });
                    }
                }
                // Database-specific snippets
                if let Some(ref info) = db_info {
                    for (label, insert_text, doc) in &info.snippets {
                        if matches_filter(label) {
                            let boost = match_boost(label);
                            let score = completion_priority::calculate_score_with_match(
                                &context,
                                Some(CompletionItemKind::SNIPPET),
                                boost,
                            );
                            items.push(CompletionItem {
                                label: label.to_string(),
                                kind: Some(CompletionItemKind::SNIPPET),
                                text_edit: Some(CompletionTextEdit::InsertAndReplace(
                                    InsertReplaceEdit {
                                        new_text: insert_text.to_string(),
                                        insert: replace_range,
                                        replace: replace_range,
                                    },
                                )),
                                insert_text_format: Some(InsertTextFormat::SNIPPET),
                                filter_text: Some(matched_prefix(label)),
                                documentation: Some(lsp_types::Documentation::String(
                                    doc.to_string(),
                                )),
                                sort_text: Some(completion_priority::score_to_sort_text(
                                    score, label,
                                )),
                                ..Default::default()
                            });
                        }
                    }
                }
            }

            items.sort_by(|a, b| {
                a.sort_text
                    .as_ref()
                    .unwrap_or(&a.label)
                    .cmp(b.sort_text.as_ref().unwrap_or(&b.label))
            });
            items.truncate(50);
            Ok(CompletionResponse::Array(items))
        })
    }

    fn inline_completion(
        &self,
        rope: &Rope,
        offset: usize,
        _trigger: InlineCompletionContext,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<InlineCompletionResponse>> {
        let rope = rope.clone();
        let SqlCompletionSources {
            schema,
            db_completion_info: db_info,
        } = self.sources();

        cx.background_spawn(async move {
            let text = rope.to_string();
            let completer =
                crate::sql_inline_completion::SqlInlineCompleter::new(&schema, db_info.as_ref());

            match completer.suggest(&text, offset) {
                Some(insert_text) => Ok(InlineCompletionResponse::Array(vec![
                    InlineCompletionItem {
                        insert_text,
                        filter_text: None,
                        range: None,
                        command: None,
                        insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                    },
                ])),
                None => Ok(InlineCompletionResponse::Array(vec![])),
            }
        })
    }

    fn is_completion_trigger(&self, _offset: usize, new_text: &str, _cx: &mut App) -> bool {
        self.is_completion_trigger_check(new_text)
    }
}

fn find_schema_columns<'a>(
    schema: &'a SqlSchema,
    table: &str,
) -> Option<&'a Vec<(String, String, String)>> {
    schema.columns_by_table.get(table).or_else(|| {
        schema
            .columns_by_table
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(table))
            .map(|(_, columns)| columns)
    })
}

impl DefaultSqlCompletionProvider {
    /// 检查给定文本是否应该触发自动完成。
    /// 此方法可在测试中直接调用，无需 GPUI Context。
    ///
    /// 设计原则：只有 ASCII 字符触发补全，中文字符和中文标点不触发。
    /// SQL 语法使用 ASCII 字符，中文主要出现在注释或字符串值中。
    pub fn is_completion_trigger_check(&self, new_text: &str) -> bool {
        // 获取最后一个字符
        let last_char = match new_text.chars().last() {
            Some(c) => c,
            None => return false,
        };

        // 非 ASCII 字符不触发（中文字符、中文标点等）
        if !last_char.is_ascii() {
            return false;
        }

        // 换行符/制表符后不触发（用户正在格式化代码）
        if last_char == '\n' || last_char == '\r' || last_char == '\t' {
            return false;
        }

        true
    }
}

#[derive(Clone)]
pub struct TableMentionCompletionProvider {
    schema: SqlSchema,
}

impl TableMentionCompletionProvider {
    pub fn new(schema: SqlSchema) -> Self {
        Self { schema }
    }

    pub(crate) fn format_table_mention(table: &str) -> String {
        if Self::is_simple_mention_name(table) {
            return format!("@{} ", table);
        }
        if !table.contains('`') {
            return format!("@`{}` ", table);
        }
        format!("@\"{}\" ", table)
    }

    pub(crate) fn extract_mention_query(text: &str, offset: usize) -> Option<(usize, String)> {
        let mut offset = offset.min(text.len());
        while offset > 0 && !text.is_char_boundary(offset) {
            offset = offset.saturating_sub(1);
        }
        let before_cursor = &text[..offset];
        let at_index = before_cursor.rfind('@')?;
        if at_index > 0 {
            let before_at = before_cursor[..at_index].chars().last();
            if before_at.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                return None;
            }
        }
        let after_at = &before_cursor[at_index + 1..];
        if after_at.is_empty() {
            return Some((at_index, String::new()));
        }
        let first = after_at.chars().next()?;
        if first == '`' || first == '"' {
            let rest = &after_at[first.len_utf8()..];
            if rest.contains(first) {
                return None;
            }
            return Some((at_index, rest.to_string()));
        }
        if !after_at.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return None;
        }
        Some((at_index, after_at.to_string()))
    }

    fn is_simple_mention_name(name: &str) -> bool {
        let mut chars = name.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        if !(first.is_alphabetic() || first == '_') {
            return false;
        }
        chars.all(|c| c.is_alphanumeric() || c == '_')
    }
}

impl CompletionProvider for TableMentionCompletionProvider {
    fn completions(
        &self,
        rope: &Rope,
        offset: usize,
        _trigger: CompletionContext,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<CompletionResponse>> {
        let rope = rope.clone();
        let schema = self.schema.clone();

        cx.background_spawn(async move {
            let offset = rope.clip_offset(offset, Bias::Left);
            let text = rope.to_string();
            let Some((start_offset, prefix)) =
                TableMentionCompletionProvider::extract_mention_query(&text, offset)
            else {
                return Ok(CompletionResponse::Array(vec![]));
            };

            let start_pos = rope.offset_to_position(start_offset);
            let end_pos = rope.offset_to_position(offset);
            let replace_range = LspRange::new(start_pos, end_pos);

            let mut items = Vec::new();
            for (table, doc) in &schema.tables {
                let Some(rank) = identifier_match_rank(table, &prefix.to_uppercase()) else {
                    continue;
                };
                let table_lower = table.to_lowercase();
                let mention_text = TableMentionCompletionProvider::format_table_mention(table);
                let documentation = if doc.is_empty() {
                    None
                } else {
                    Some(lsp_types::Documentation::String(doc.clone()))
                };
                items.push(CompletionItem {
                    label: mention_text.clone(),
                    kind: Some(CompletionItemKind::STRUCT),
                    detail: Some(t!("SqlEditor.table_detail").to_string()),
                    documentation,
                    text_edit: Some(CompletionTextEdit::InsertAndReplace(InsertReplaceEdit {
                        new_text: mention_text,
                        insert: replace_range,
                        replace: replace_range,
                    })),
                    filter_text: if prefix.is_empty() {
                        None
                    } else {
                        Some(prefix.clone())
                    },
                    sort_text: Some(format!("{}_{}", rank, table_lower)),
                    ..Default::default()
                });
            }

            items.sort_by(|a, b| {
                a.sort_text
                    .as_ref()
                    .unwrap_or(&a.label)
                    .cmp(b.sort_text.as_ref().unwrap_or(&b.label))
            });
            items.truncate(50);
            Ok(CompletionResponse::Array(items))
        })
    }

    fn is_completion_trigger(&self, _offset: usize, new_text: &str, _cx: &mut App) -> bool {
        let Some(last_char) = new_text.chars().last() else {
            return false;
        };
        !last_char.is_whitespace()
    }
}

/// Result of one full-document diagnostics analysis.
pub struct SqlDiagnosticSnapshot {
    /// Document revision the analysis ran against. Consumers drop the result
    /// when the current revision has moved on (spec §12.6 stale guard).
    pub document_revision: u64,
    /// Input-layer diagnostics ready for the squiggle renderer.
    pub diagnostics: Vec<Diagnostic>,
}

/// Convert a `SqlSchema` snapshot into the metadata view used by the
/// conservative semantic checker. Identifiers are normalized to uppercase by
/// the builder; an empty `SqlSchema::default()` yields `has_metadata() ==
/// false` so the checker stays silent until real metadata is loaded.
fn schema_to_metadata_view(schema: &SqlSchema) -> SqlMetadataView {
    let mut view = SqlMetadataView::default();
    let tables: Vec<String> = schema.tables.iter().map(|(name, _)| name.clone()).collect();
    view = view.with_tables(tables);
    for (table, columns) in &schema.columns_by_table {
        let names: Vec<String> = columns.iter().map(|(name, _, _)| name.clone()).collect();
        view = view.with_columns(table, names);
    }
    if let Some(schema_name) = &schema.current_schema {
        view = view.with_current_schema(schema_name);
    }
    if let Some(database) = &schema.current_database {
        view = view.with_current_database(database);
    }
    view
}

/// Convert a `SqlDiagnostic` (UTF-8 byte range) into the input-layer
/// `Diagnostic` (line/character positions) used by the squiggle renderer.
fn sql_diagnostic_to_input(diag: &SqlDiagnostic, rope: &Rope) -> Diagnostic {
    let start = rope.offset_to_position(diag.range.start_byte);
    let end = rope.offset_to_position(diag.range.end_byte);
    Diagnostic {
        range: start..end,
        severity: match diag.severity {
            SqlDiagnosticSeverity::Error => DiagnosticSeverity::Error,
            SqlDiagnosticSeverity::Warning => DiagnosticSeverity::Warning,
            SqlDiagnosticSeverity::Information => DiagnosticSeverity::Info,
            SqlDiagnosticSeverity::Hint => DiagnosticSeverity::Hint,
        },
        code: diag.code.as_deref().map(gpui::SharedString::from),
        code_description: None,
        source: Some("sql".into()),
        message: diag.message.to_string().into(),
        related_information: None,
        tags: None,
        data: None,
    }
}

/// Pure full-document diagnostics analysis. The schema is an `Arc` snapshot,
/// so callers capture the document text (one String copy) and a cheap shared
/// schema handle on the UI thread, then hand all three to a background worker.
fn analyze_diagnostics_pure(
    text: String,
    dialect: SqlDialect,
    schema: Arc<SqlSchema>,
    document_revision: u64,
) -> SqlDiagnosticSnapshot {
    let mut next_id = 0u64;
    let snapshot = SqlStatementSnapshot::new(text, dialect);
    let text = snapshot.text();
    let mut sql_diags = analyze_parser_diagnostics(text, dialect, document_revision, &mut next_id);
    let metadata = schema_to_metadata_view(&schema);
    sql_diags.extend(analyze_semantic_diagnostics(
        text,
        &snapshot,
        &metadata,
        document_revision,
        &mut next_id,
    ));
    let rope = Rope::from_str(text);
    let diagnostics = sql_diags
        .iter()
        .map(|diag| sql_diagnostic_to_input(diag, &rope))
        .collect();
    SqlDiagnosticSnapshot {
        document_revision,
        diagnostics,
    }
}

/// A reusable SQL editor component built on top of `Input`.
pub struct SqlEditor {
    editor: Entity<EditorState>,
    extended_editor: Entity<ExtendedEditorState>,
    gutter_lane: GutterLane,
    default_completion_provider: Rc<DefaultSqlCompletionProvider>,
    default_hover_provider: Option<Rc<DefaultSqlHoverProvider>>,
    default_signature_help_provider: Option<Rc<DefaultSqlSignatureHelpProvider>>,
    font_cache: Option<SqlEditorFontCache>,
    /// 宿主页签容器：对象详情页签要开在编辑器所在的那个容器里（数据库页签内部的
    /// 容器），而不是主窗口的页签栏。由 [`SqlEditorTab`] 在构造时注入；共用一个
    /// 句柄给 Cmd/Ctrl+点击的处理器。
    tab_container: Rc<RefCell<Option<Entity<TabContainer>>>>,
}

struct SqlEditorFontCache {
    requested_family: String,
    font: Font,
}

impl SqlEditor {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let default_completion_provider =
            Rc::new(DefaultSqlCompletionProvider::new(SqlSchema::default()));
        let default_provider_trait: Rc<dyn CompletionProvider> =
            default_completion_provider.clone();
        let default_hover_provider = Rc::new(DefaultSqlHoverProvider::new(SqlSchema::default()));
        // Cmd/Ctrl+click reuses the same schema snapshot as the hover
        // provider; gpui-kit underlines the identifier on Cmd/Ctrl+hover and
        // hands the click over to `show_document` (see the handler below).
        let default_definition_provider = Rc::new(DefaultSqlDefinitionProvider::new(
            default_hover_provider.sources_handle(),
        ));
        let default_definition_provider_trait: Rc<dyn DefinitionProvider> =
            default_definition_provider.clone();
        let tab_container: Rc<RefCell<Option<Entity<TabContainer>>>> = Rc::new(RefCell::new(None));
        let show_object_details =
            sql_object_details_handler(default_definition_provider.clone(), tab_container.clone());
        let default_signature_help_provider =
            Rc::new(DefaultSqlSignatureHelpProvider::new(SqlSchema::default()));
        let default_signature_help_provider_trait: Rc<dyn SignatureHelpProvider> =
            default_signature_help_provider.clone();
        let editor = cx.new(|cx| {
            let mut editor = EditorState::new(window, cx)
                .language("sql")
                .line_number(true)
                .searchable(true)
                .indent_guides(true)
                .tab_size(TabSize {
                    tab_size: 2,
                    hard_tabs: false,
                })
                .soft_wrap(false)
                .placeholder(t!("Query.editor_placeholder").to_string());

            editor.lsp_mut().completion_provider = Some(default_provider_trait);
            editor.lsp_mut().definition_provider = Some(default_definition_provider_trait);
            editor.lsp_mut().show_document = Some(show_object_details);
            // The default hover provider is intentionally NOT installed as a
            // mouse-hover trigger: hover details are shown on demand from the
            // context menu (ShowHoverDetails) instead, so pointer movement
            // over identifiers never pops a popover.

            editor
        });
        let gutter_lane = editor.update(cx, |state, cx| {
            state.create_gutter_lane(
                Vec::new(),
                GutterLaneOptions::new(render_sql_gutter_marker),
                cx,
            )
        });
        let extended_editor = cx.new(|cx| {
            let mut state = ExtendedEditorState::new(editor.clone(), window, cx);
            state.set_signature_help_provider(Some(default_signature_help_provider_trait), cx);
            state
        });
        Self {
            editor,
            extended_editor,
            gutter_lane,
            default_completion_provider,
            default_hover_provider: Some(default_hover_provider),
            default_signature_help_provider: Some(default_signature_help_provider),
            font_cache: None,
            tab_container,
        }
    }

    /// 注入宿主页签容器（编辑器所在的那个容器）。
    ///
    /// 由 [`SqlEditorTab`] 在把自己的编辑器挂进数据库页签容器后调用，这样对象
    /// 详情页签会开在同一个容器里，而不是主窗口的页签栏。
    pub fn set_tab_container(&mut self, container: Entity<TabContainer>) {
        *self.tab_container.borrow_mut() = Some(container);
    }

    /// 注入建表 DDL 的来源（连接 + 方言）。
    ///
    /// 由 [`SqlEditorView`] 在发布元数据时调用；嵌在单元格里的编辑器没有连接
    /// 上下文，那种场景下详情页签不带 DDL 区段。
    pub fn set_table_ddl_sources(&mut self, sources: Option<TableDdlSources>) {
        if let Some(provider) = &self.default_hover_provider {
            provider.set_table_ddl(sources);
        }
    }

    /// 当前建表 DDL 来源。
    fn table_ddl_sources(&self) -> Option<TableDdlSources> {
        self.default_hover_provider
            .as_ref()
            .and_then(|provider| provider.table_ddl())
    }

    /// 当前宿主页签容器。
    fn host_tab_container(&self) -> Option<Entity<TabContainer>> {
        self.tab_container.borrow().clone()
    }

    fn editor_font(&mut self, cx: &mut Context<Self>) -> Font {
        let font_family = AppSettings::global(cx).sql_editor_font_family.clone();
        if let Some(cache) = &self.font_cache
            && cache.requested_family == font_family
        {
            return cache.font.clone();
        }

        let installed_font_names = cx.text_system().all_font_names();
        let font = installed_grid_monospace_font(&font_family, &installed_font_names);
        self.font_cache = Some(SqlEditorFontCache {
            requested_family: font_family,
            font: font.clone(),
        });
        font
    }

    /// Set database-specific completion information from plugin
    pub fn set_db_completion_info(
        &mut self,
        info: SqlCompletionInfo,
        schema: SqlSchema,
        cx: &mut Context<Self>,
    ) {
        self.update_default_completion_sources(schema, info, cx);
    }

    /// Access underlying editor state.
    pub fn input(&self) -> Entity<EditorState> {
        self.editor.clone()
    }

    /// The gutter lane owned by this editor.
    pub fn gutter_lane(&self) -> &GutterLane {
        &self.gutter_lane
    }

    /// Invalidate active completion popup and inline completion requests.
    pub fn invalidate_completions(&self, cx: &mut Context<Self>) {
        self.editor.update(cx, |state, cx| {
            state.dismiss_completion_overlay(cx);
        });
    }

    /// Invalidate metadata-dependent completion and hover state.
    pub fn invalidate_metadata_context(&self, cx: &mut Context<Self>) {
        self.editor.update(cx, |state, cx| {
            state.dismiss_lsp_overlays(cx);
            state.clear_hover_state(cx);
        });
        self.extended_editor
            .update(cx, |state, cx| state.close_signature_help(cx));
    }

    /// Replace default completion provider.
    pub fn set_completion_provider(
        &mut self,
        provider: Rc<dyn CompletionProvider>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editor.update(cx, |state, cx| {
            state.dismiss_completion_overlay(cx);
            state.lsp_mut().completion_provider = Some(provider);
        });
    }

    /// Set schema for default completion provider.
    pub fn set_schema(&mut self, schema: SqlSchema, _window: &mut Window, cx: &mut Context<Self>) {
        self.update_default_completion_sources(schema, SqlCompletionInfo::default(), cx);
    }

    /// Update the default provider's metadata without replacing its trait object.
    ///
    /// A custom provider remains untouched because metadata refreshes must not
    /// override an explicitly installed provider.
    fn update_default_completion_sources(
        &self,
        schema: SqlSchema,
        info: SqlCompletionInfo,
        cx: &mut Context<Self>,
    ) {
        // Provider objects are intentionally kept alive across metadata
        // refreshes, so explicitly invalidate requests that captured their old
        // source snapshot before replacing it.
        self.invalidate_metadata_context(cx);
        self.update_default_hover_sources(schema.clone(), cx);
        self.update_default_signature_sources(schema.clone(), cx);
        let default_provider = self.default_completion_provider.clone();
        let default_provider_trait: Rc<dyn CompletionProvider> = default_provider.clone();
        self.editor.update(cx, |state, _| {
            let is_default_provider_installed = state
                .lsp()
                .completion_provider
                .as_ref()
                .is_some_and(|provider| Rc::ptr_eq(provider, &default_provider_trait));
            if is_default_provider_installed {
                default_provider.set_sources(schema, info);
            }
        });
    }

    /// Update the default hover provider's metadata without replacing its trait
    /// object. A custom provider remains untouched (spec §25.1).
    fn update_default_hover_sources(&self, schema: SqlSchema, cx: &mut Context<Self>) {
        let Some(default_hover_provider) = self.default_hover_provider.clone() else {
            return;
        };
        self.editor.update(cx, |_, _| {
            // Always refresh the default provider's snapshot: it backs the
            // context-menu hover even though it is not installed as a
            // mouse-hover trigger. A custom provider installed via
            // `set_hover_provider` keeps its own sources and stays untouched.
            default_hover_provider.set_schema(schema);
        });
    }

    fn update_default_signature_sources(&self, schema: SqlSchema, cx: &mut Context<Self>) {
        let Some(default_provider) = self.default_signature_help_provider.clone() else {
            return;
        };
        default_provider.set_schema(schema);
        self.extended_editor
            .update(cx, |state, cx| state.close_signature_help(cx));
    }

    /// Replace hover provider.
    pub fn set_hover_provider(
        &mut self,
        provider: Rc<dyn HoverProvider>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editor.update(cx, |state, cx| {
            state.clear_hover_state(cx);
            state.lsp_mut().hover_provider = Some(provider);
        });
    }

    pub fn set_signature_help_provider(
        &mut self,
        provider: Rc<dyn SignatureHelpProvider>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.default_signature_help_provider = None;
        self.extended_editor.update(cx, |state, cx| {
            state.set_signature_help_provider(Some(provider), cx)
        });
    }

    /// Add a custom code action provider.
    pub fn add_code_action_provider(
        &mut self,
        provider: Rc<dyn CodeActionProvider>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editor.update(cx, |state, _| {
            state.lsp_mut().code_action_providers.push(provider)
        });
    }

    /// Convenient toggles for consumers
    pub fn set_line_number(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.editor
            .update(cx, |s, cx| s.set_line_number(on, window, cx));
    }
    pub fn set_soft_wrap(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.editor
            .update(cx, |s, cx| s.set_soft_wrap(on, window, cx));
    }
    pub fn set_indent_guides(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.editor
            .update(cx, |s, cx| s.set_indent_guides(on, window, cx));
    }
    pub fn set_placeholder(
        &mut self,
        placeholder: impl Into<gpui::SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let placeholder = placeholder.into();
        self.editor
            .update(cx, |s, cx| s.set_placeholder(placeholder, window, cx));
    }
    pub fn set_value(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.editor
            .update(cx, |s, cx| s.set_value(text, window, cx));
        self.extended_editor
            .update(cx, |state, cx| state.refresh_signature_help(window, cx));
    }

    pub fn replace_range_and_select(
        &mut self,
        range: Range<usize>,
        replacement: String,
        selection: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editor.update(cx, |state, cx| {
            state.set_selected_range(range, cx);
            state.replace(replacement, window, cx);
            state.set_selected_range(selection, cx);
        });
    }

    /// Get the current text content of the editor.
    /// This is a convenience method that accesses the underlying EditorState.
    pub fn get_text(&self, cx: &App) -> String {
        self.editor.read(cx).text().to_string()
    }

    /// Get the current schema metadata snapshot as a cheap shared `Arc`.
    ///
    /// This is the same source consumed by the default completion/hover
    /// providers, so diagnostics, completion and hover always agree on scope.
    /// Callers that only need to read schema metadata (e.g. to hand a snapshot
    /// to a background task) never pay for a deep copy here.
    pub fn current_schema(&self) -> Arc<SqlSchema> {
        self.default_completion_provider.sources().schema
    }

    /// Run the full parser + semantic diagnostics analysis for the current
    /// editor content against the current schema snapshot.
    ///
    /// The returned snapshot carries the document revision it was computed
    /// against; the caller drops it if the document has moved on
    /// (spec §12.6). Viewport-incremental analysis is a future optimization —
    /// the squiggle layer already clips rendering to the visible range.
    pub fn analyze_diagnostics(&self, cx: &App, dialect: SqlDialect) -> SqlDiagnosticSnapshot {
        let input = self.editor.read(cx);
        let text = input.text().to_string();
        let document_revision = input.document_revision();
        let schema = self.current_schema();
        analyze_diagnostics_pure(text, dialect, schema, document_revision)
    }

    /// Run the full parser + semantic diagnostics analysis without blocking the
    /// UI thread.
    ///
    /// The document text, revision and schema snapshot are captured on the UI
    /// thread, then the tokenizer/semantic passes run on a background worker.
    /// Like the synchronous variant, the caller drops the returned snapshot if
    /// the document revision has moved on before it lands (spec §12.6).
    pub fn analyze_diagnostics_async(
        &self,
        cx: &App,
        dialect: SqlDialect,
    ) -> Task<SqlDiagnosticSnapshot> {
        let input = self.editor.read(cx);
        let text = input.text().to_string();
        let document_revision = input.document_revision();
        let schema = self.current_schema();
        cx.background_spawn(async move {
            analyze_diagnostics_pure(text, dialect, schema, document_revision)
        })
    }

    /// Get the currently selected text.
    /// Returns an empty string if no text is selected.
    pub fn get_selected_text(&self, cx: &App) -> String {
        self.editor.read(cx).selected_text().to_string()
    }

    /// Get the current cursor byte offset.
    pub fn cursor_offset(&self, cx: &App) -> usize {
        self.editor.read(cx).cursor()
    }

    /// Resolve the identifier under the cursor (or in the selection) against
    /// the current schema snapshot and open its details tab.
    ///
    /// Triggered from the editor context menu and by Cmd/Ctrl+click. With a
    /// selection the selection body is probed first, so right-clicking a
    /// selected table name works even when the cursor sits at a selection edge.
    pub fn show_hover_details(&self, window: &mut Window, cx: &mut Context<Self>) {
        let (text, cursor, selection) = self.editor_snapshot(cx);
        let Some(details) = crate::sql_editor_hover::resolve_object_details_for_selection(
            &text,
            selection,
            cursor,
            &self.current_schema(),
        ) else {
            window.push_notification(t!("Query.no_hover_details_at_cursor").to_string(), cx);
            return;
        };
        crate::sql_object_details_tab::open_object_details_tab(
            self.object_details_request(details),
            window,
            cx,
        );
    }

    /// Copy the DDL for the table under the cursor (or in the selection) to the
    /// clipboard. No-op notification when nothing resolves.
    ///
    /// 与[表设计器] (@see SqlEditorView) 同一条驱动链路，所以是异步的：先取
    /// 元数据再让驱动生成方言正确的建表 DDL。
    pub fn copy_hover_ddl(&self, window: &mut Window, cx: &mut Context<Self>) {
        let (text, cursor, selection) = self.editor_snapshot(cx);
        let details = crate::sql_editor_hover::resolve_object_details_for_selection(
            &text,
            selection,
            cursor,
            &self.current_schema(),
        );
        let table = details.and_then(|details| details.table);
        let Some(table) = table else {
            window.push_notification(t!("Query.no_hover_details_at_cursor").to_string(), cx);
            return;
        };
        let sources = self.table_ddl_sources();
        let window_handle = window.window_handle();
        cx.spawn(async move |_this, cx: &mut AsyncApp| {
            let section =
                crate::table_ddl::load_ddl_section(Some(&table), sources.as_ref(), cx).await;
            let _ = cx.update_window(window_handle, |_, window, cx| {
                let message = copy_ddl_message(section, cx);
                window.push_notification(message, cx);
            });
        })
        .detach();
    }

    /// 编辑器当前的文本、光标与选区。
    fn editor_snapshot(&self, cx: &mut App) -> (String, usize, Option<Range<usize>>) {
        self.editor.update(cx, |state, _| {
            (
                state.text().to_string(),
                state.cursor(),
                Some(state.selected_range()),
            )
        })
    }

    /// 打开对象详情页签要的请求：对象身份 + DDL 来源 + 宿主编容器。
    fn object_details_request(
        &self,
        details: crate::sql_editor_hover::SqlObjectDetails,
    ) -> crate::sql_object_details_tab::ObjectDetailsRequest {
        crate::sql_object_details_tab::ObjectDetailsRequest {
            details,
            ddl: self.table_ddl_sources(),
            host: self.host_tab_container(),
        }
    }

    /// Get the current selection as UTF-8 byte offsets.
    pub fn selected_range(&self, cx: &App) -> Range<usize> {
        self.editor.read(cx).selected_range()
    }

    fn handle_show_hover_details_action(
        &mut self,
        _: &ShowHoverDetails,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_hover_details(window, cx);
    }

    fn handle_copy_hover_ddl_action(
        &mut self,
        _: &CopyHoverDdl,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_hover_ddl(window, cx);
    }

    pub fn document_revision(&self, cx: &App) -> u64 {
        self.editor.read(cx).document_revision()
    }
}

/// 拿到 DDL 后写进剪贴板，并给出要提示用户的文案。
fn copy_ddl_message(section: Option<TableDdlSection>, cx: &mut App) -> String {
    match section {
        Some(TableDdlSection::Ready(ddl)) => {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(ddl));
            t!("Query.ddl_copied").to_string()
        }
        Some(TableDdlSection::Failed(error)) => {
            t!("Query.object_details_ddl_failed", error = error.as_str()).to_string()
        }
        // 没有连接上下文的内嵌编辑器，或还没生成完：这里没有 DDL 可复制。
        _ => t!("Query.no_hover_details_at_cursor").to_string(),
    }
}

impl Render for SqlEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let font = self.editor_font(cx);
        let font_size = AppSettings::global(cx).sql_editor_font_size as f32;
        // The right-click menu must be attached through the editor element
        // builder: gpui-component reinstalls its own context menu on the
        // EditorState on every frame, so a handler set once at construction
        // is silently replaced and never shown. The builder is invoked while
        // the editor entity is still being updated, so reading it back from
        // inside the callback panics — snapshot the capabilities here, where
        // the editor is quiescent, and reuse the copy at click time.
        let capabilities = self.editor.read(cx).context_menu_capabilities();
        // Mouse hover is opt-in (settings): install/uninstall the default
        // hover provider on render so flipping the setting takes effect
        // without recreating the editor. The provider itself debounces, so
        // the pointer must dwell on an identifier before anything pops.
        let hover_enabled = AppSettings::global(cx).sql_editor_hover_enabled;
        if let Some(default_provider) = self.default_hover_provider.clone() {
            let default_trait: Rc<dyn HoverProvider> = default_provider.clone();
            self.editor.update(cx, |state, cx| {
                let default_installed = state
                    .lsp()
                    .hover_provider
                    .as_ref()
                    .is_some_and(|p| Rc::ptr_eq(p, &default_trait));
                // Never touch a custom provider installed via set_hover_provider.
                if hover_enabled != default_installed {
                    state.clear_hover_state(cx);
                    state.lsp_mut().hover_provider = hover_enabled.then(|| default_trait);
                }
            });
        }
        div()
            .size_full()
            .on_action(cx.listener(Self::handle_show_hover_details_action))
            .on_action(cx.listener(Self::handle_copy_hover_ddl_action))
            .child(
                ExtendedEditor::new(&self.extended_editor)
                    .context_menu(move |_, _, cx| {
                        sql_editor_native_menu(capabilities, cx.read_from_clipboard().is_some())
                    })
                    .font(font)
                    .text_size(gpui::px(font_size))
                    .line_height(gpui::px(font_size * 1.5))
                    .size_full(),
            )
    }
}

/// `show_document` callback backing Cmd/Ctrl+click on an identifier.
///
/// gpui-kit offers the go-to-definition target here before falling back to its
/// own handling. We claim only our synthetic URI (see
/// [`crate::sql_editor_definition`]) and open the details tab that the context
/// menu also uses; anything else returns `false` so normal go-to-definition
/// behaviour is untouched.
fn sql_object_details_handler(
    provider: Rc<DefaultSqlDefinitionProvider>,
    tab_container: Rc<RefCell<Option<Entity<TabContainer>>>>,
) -> ShowDocumentHandler {
    Rc::new(move |params, window, cx| {
        if !crate::sql_editor_definition::is_object_details_uri(&params.uri) {
            return false;
        }
        let Some(details) = provider.take_pending_details() else {
            return false;
        };
        let request = crate::sql_object_details_tab::ObjectDetailsRequest {
            details,
            ddl: provider.table_ddl(),
            host: tab_container.borrow().clone(),
        };
        crate::sql_object_details_tab::open_object_details_tab(request, window, cx);
        true
    })
}

fn sql_editor_native_menu(
    capabilities: gpui_base::input::InputContextMenuCapabilities,
    clipboard_available: bool,
) -> PlatformNativeMenu {
    let model = sql_editor_context_menu(capabilities, clipboard_available);
    model
        .items
        .into_iter()
        .fold(PlatformNativeMenu::new(), |menu, item| match item {
            gpui_base::input::NativeMenuItem::Separator => menu.separator(),
            gpui_base::input::NativeMenuItem::Action {
                label,
                disabled,
                action,
            } => menu.menu_with_disabled(label, disabled, action),
        })
}

fn sql_editor_context_menu(
    capabilities: gpui_base::input::InputContextMenuCapabilities,
    clipboard_available: bool,
) -> gpui_base::input::NativeMenu {
    let editable = capabilities.is_editable();
    let copyable = capabilities.is_copyable();
    gpui_base::input::NativeMenu::new()
        .menu_with_disabled(
            t!("Query.run_selected").to_string(),
            !capabilities.has_selection(),
            Box::new(RunSelectedSql),
        )
        .menu(
            t!("Query.run_cursor_statement").to_string(),
            Box::new(RunCursorStatementSql),
        )
        .menu(
            t!("Query.show_hover_details").to_string(),
            Box::new(ShowHoverDetails),
        )
        .menu(
            t!("Query.copy_hover_ddl").to_string(),
            Box::new(CopyHoverDdl),
        )
        .separator()
        .menu_with_disabled(t!("Input.Cut"), !(editable && copyable), Box::new(Cut))
        .menu_with_disabled(t!("Input.Copy"), !copyable, Box::new(Copy))
        .menu_with_disabled(
            t!("Input.Paste"),
            !(editable && clipboard_available),
            Box::new(Paste),
        )
        .separator()
        .menu(t!("Input.Select All"), Box::new(SelectAll))
}

fn render_sql_gutter_marker(marker: &GutterMarker) -> gpui::AnyElement {
    let animation_id = gpui::SharedString::from(format!("sql-gutter-{}", marker.row()));
    let icon = match marker.icon().as_ref() {
        SQL_GUTTER_RUNNING => Spinner::new()
            .with_size(Size::Small)
            .color(gpui::hsla(0.603, 0.91, 0.6, 1.0))
            .animation_id(animation_id.clone())
            .into_any_element(),
        SQL_GUTTER_SUCCEEDED => Icon::new(IconName::CircleCheck)
            .text_color(gpui::hsla(0.394, 0.71, 0.45, 1.0))
            .into_any_element(),
        SQL_GUTTER_FAILED => Icon::new(IconName::CircleX)
            .text_color(gpui::hsla(0.0, 0.84, 0.6, 1.0))
            .into_any_element(),
        SQL_GUTTER_CANCELLED => Icon::new(IconName::Minus)
            .text_color(gpui::hsla(0.0, 0.0, 0.45, 1.0))
            .into_any_element(),
        _ => Icon::new(IconName::Play)
            .text_color(gpui::hsla(0.306, 0.53, 0.4, 1.0))
            .into_any_element(),
    };
    let tooltip = marker.tooltip().cloned();
    div()
        .id(animation_id)
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .when(marker.is_enabled(), |this| this.cursor_pointer())
        .when(!marker.is_enabled(), |this| this.opacity(0.5))
        .child(icon)
        .when_some(tooltip, |this, tooltip| {
            this.tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{
        RunCursorStatementSql, RunSelectedSql, SQL_GUTTER_IDLE, ShowHoverDetails, SqlContext,
        SqlEditor, SqlSchema, analyze_diagnostics_pure, completion_priority, copy_ddl_message,
        identifier_match_rank, schema_to_metadata_view, sql_diagnostic_to_input,
        sql_editor_context_menu,
    };
    use crate::sql_editor_hover::SqlObjectDetailsKind;
    use db::sql_editor::diagnostics::{
        SqlDiagnosticSeverity, analyze_parser_diagnostics, analyze_semantic_diagnostics,
    };
    use db::sql_editor::statement_ranges::{SqlDialect, SqlStatementSnapshot};
    use gpui::{
        App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
        Modifiers, MouseButton, ParentElement as _, Render, Styled as _, Subscription,
        VisualTestContext, Window, div,
    };
    use gpui_component::highlighter::DiagnosticSeverity;
    use gpui_component::input::{
        Copy, Cut, GutterMarker, InlineWidget, InputEvent, Paste, RangeDecoration, SelectAll,
    };
    use gpui_component::{Rope, RopeExt};
    use lsp_types::{CompletionItemKind, ShowDocumentParams};
    use one_core::settings::AppSettings;
    use one_core::tab_container::{
        GlobalTabContainer, TabContainer, TabContent, TabContentEvent, TabItem,
    };
    use rust_i18n::t;
    use std::collections::HashMap;
    use std::{cell::RefCell, rc::Rc, sync::Arc};

    struct SqlEditorHarness {
        editor: Entity<SqlEditor>,
        _subscription: Option<Subscription>,
    }

    impl Render for SqlEditorHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(self.editor.clone())
        }
    }

    #[gpui::test]
    fn sql_editor_installs_and_renders_host_hooks(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            cx.set_global(AppSettings::default());
        });
        let mut sql_editor = None;
        let clicked_marker = Rc::new(RefCell::new(None));
        let (_, visual) = cx.add_window_view(|window, cx| {
            let editor = cx.new(|cx| SqlEditor::new(window, cx));
            let input = editor.read(cx).input();
            let clicked_marker_for_event = clicked_marker.clone();
            let subscription = cx.subscribe(&input, move |_, _, event: &InputEvent, _| {
                if let InputEvent::GutterMarkerMouseDown {
                    index, logical_row, ..
                } = event
                {
                    *clicked_marker_for_event.borrow_mut() = Some((*index, *logical_row));
                }
            });
            sql_editor = Some(editor.clone());
            SqlEditorHarness {
                editor,
                _subscription: Some(subscription),
            }
        });
        let sql_editor = sql_editor.expect("SQL editor should be created");
        let input = visual.read(|cx| sql_editor.read(cx).input());

        VisualTestContext::update(visual, |window, cx| {
            sql_editor.update(cx, |editor, cx| {
                editor.set_value("select 1;".to_string(), window, cx)
            });
            let revision = input.read(cx).document_revision();
            let lane = sql_editor.read(cx).gutter_lane().clone();
            lane.set(vec![GutterMarker::new(0, SQL_GUTTER_IDLE)], cx);
            input.update(cx, |state, cx| {
                let _ =
                    state.create_range_decorations_collection(vec![RangeDecoration::new(0..8)], cx);
                let _ =
                    state.create_inline_widgets_collection(vec![InlineWidget::new(7, "value")], cx);
                assert_eq!(revision, state.document_revision());
            });
            window.draw(cx).clear(cx);
        });
        VisualTestContext::update(visual, |window, cx| window.draw(cx).clear(cx));

        let marker_bounds = visual.read(|cx| {
            sql_editor
                .read(cx)
                .gutter_lane()
                .marker_bounds(0, cx)
                .expect("custom gutter marker should render")
        });
        visual.simulate_mouse_down(
            marker_bounds.center(),
            MouseButton::Left,
            Modifiers::default(),
        );
        assert_eq!(Some((0, 0)), *clicked_marker.borrow());
        visual.read(|cx| {
            assert_eq!(1, sql_editor.read(cx).gutter_lane().get_markers(cx).len());
        });
    }

    #[test]
    fn sql_context_menu_offers_show_hover_details() {
        let capabilities = gpui_base::input::InputContextMenuCapabilities::new()
            .code_editor(true)
            .selection(true);
        let menu = sql_editor_context_menu(capabilities, true);
        let has_hover_item = menu
            .items
            .iter()
            .any(|item| matches!(item, gpui_base::input::NativeMenuItem::Action { action, .. } if action.partial_eq(&ShowHoverDetails)));
        assert!(has_hover_item);
    }

    #[gpui::test]
    fn sql_editor_installs_mouse_hover_provider_only_when_enabled(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            cx.set_global(AppSettings::default());
        });
        let mut sql_editor = None;
        let (_, visual) = cx.add_window_view(|window, cx| {
            let editor = cx.new(|cx| SqlEditor::new(window, cx));
            sql_editor = Some(editor.clone());
            SqlEditorHarness {
                editor,
                _subscription: None,
            }
        });
        let sql_editor = sql_editor.expect("SQL editor should be created");

        // Default setting: hover off -> no provider after a render pass.
        visual.run_until_parked();
        visual.read(|cx| {
            let input = sql_editor.read(cx).input();
            assert!(
                input.read(cx).lsp().hover_provider.is_none(),
                "hover must be opt-in: default settings install no provider"
            );
        });

        // Flip the setting inside the window context and redraw; the editor
        // reconciles its hover provider on render.
        visual.update(|_, cx| {
            AppSettings::update_and_save(cx, |settings| {
                settings.sql_editor_hover_enabled = true;
            });
            cx.refresh_windows();
        });
        visual.run_until_parked();
        visual.read(|cx| {
            let input = sql_editor.read(cx).input();
            assert!(
                input.read(cx).lsp().hover_provider.is_some(),
                "enabling the setting installs the default hover provider"
            );
        });
    }

    /// Metadata for the SQL snippets used by the Cmd/Ctrl+click tests.
    fn users_schema() -> SqlSchema {
        SqlSchema::default()
            .with_scope(Some("app".into()), Some("public".into()))
            .with_tables(vec![("users".to_string(), "doc".to_string())])
            .with_table_detail(
                "users",
                crate::sql_editor::SqlTableDetail {
                    object_type: crate::sql_editor::SqlObjectType::Table,
                    schema: Some("public".into()),
                    comment: None,
                    engine: None,
                    columns: vec![crate::sql_editor::SqlColumnDetail {
                        name: "id".into(),
                        data_type: "INT".into(),
                        is_nullable: false,
                        is_primary_key: true,
                        default_value: None,
                        comment: None,
                    }],
                },
            )
    }

    /// Window hosting a SQL editor showing `select * from users` against
    /// [`users_schema`], plus both tab containers the app has: the database
    /// tab's inner container (where the editor lives) and the window-level one.
    struct SqlEditorFixture {
        editor: Entity<SqlEditor>,
        /// 数据库页签内部的容器：详情页签应该开在这里。
        tab_container: Entity<TabContainer>,
        /// 主窗口（外层）页签栏：详情页签不应该开到这里。
        window_tab_container: Entity<TabContainer>,
        table_range: std::ops::Range<usize>,
    }

    fn editor_over_users_table(
        cx: &mut gpui::TestAppContext,
    ) -> (SqlEditorFixture, &mut VisualTestContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            cx.set_global(AppSettings::default());
        });
        let text = "select * from users";
        let table_range = text.find("users").expect("table name")..text.len();
        let mut sql_editor = None;
        let mut tab_container = None;
        let mut window_tab_container = None;
        let (_, visual) = cx.add_window_view(|window, cx| {
            let editor = cx.new(|cx| SqlEditor::new(window, cx));
            // Same wiring as the app: the editor sits in the database tab's
            // inner container, while `GlobalTabContainer` holds the window-level
            // one that must *not* collect the details tabs.
            let container = cx.new(|cx| TabContainer::new(window, cx));
            let window_container = cx.new(|cx| TabContainer::new(window, cx));
            cx.set_global(GlobalTabContainer {
                tab_container: window_container.clone(),
            });
            editor.update(cx, |editor, _| {
                editor.set_tab_container(container.clone());
            });
            sql_editor = Some(editor.clone());
            tab_container = Some(container);
            window_tab_container = Some(window_container);
            SqlEditorHarness {
                editor,
                _subscription: None,
            }
        });
        let fixture = SqlEditorFixture {
            editor: sql_editor.expect("SQL editor should be created"),
            tab_container: tab_container.expect("tab container should be created"),
            window_tab_container: window_tab_container.expect("window tab container"),
            table_range,
        };
        VisualTestContext::update(visual, |window, cx| {
            fixture.editor.update(cx, |editor, cx| {
                editor.set_schema(users_schema(), window, cx);
                editor.set_value(text.to_string(), window, cx);
            });
            // Layout the editor so identifier bounds are known to the mouse
            // simulation below.
            window.draw(cx).clear(cx);
        });
        (fixture, visual)
    }

    /// Ids and titles of the tabs currently open in `fixture`'s inner
    /// (database tab) container.
    fn details_tabs(fixture: &SqlEditorFixture, cx: &App) -> Vec<(String, String)> {
        tab_ids(&fixture.tab_container, cx)
    }

    /// Ids and titles of the tabs currently open in the window-level container.
    fn window_tabs(fixture: &SqlEditorFixture, cx: &App) -> Vec<(String, String)> {
        tab_ids(&fixture.window_tab_container, cx)
    }

    fn tab_ids(container: &Entity<TabContainer>, cx: &App) -> Vec<(String, String)> {
        container
            .read(cx)
            .tabs()
            .iter()
            .map(|tab| (tab.id().to_string(), tab.title(cx).to_string()))
            .collect()
    }

    /// Window-space center of `range` in the editor's text.
    fn text_range_center(
        visual: &mut VisualTestContext,
        sql_editor: &Entity<SqlEditor>,
        range: &std::ops::Range<usize>,
    ) -> gpui::Point<gpui::Pixels> {
        visual
            .read(|cx| {
                sql_editor
                    .read(cx)
                    .input()
                    .read(cx)
                    .range_to_bounds(range)
                    .map(|bounds| bounds.center())
            })
            .expect("the table name should be laid out")
    }

    /// Pumps the dispatcher (foreground tasks included): the definition
    /// lookup completes in a spawned task, so `run_until_parked` alone is not
    /// enough before clicking.
    fn settle(visual: &mut VisualTestContext) {
        visual.cx.run_until_parked();
    }

    #[gpui::test]
    fn sql_editor_installs_object_details_definition_provider(cx: &mut gpui::TestAppContext) {
        let (fixture, visual) = editor_over_users_table(cx);

        // Cmd/Ctrl+click is the primary entry point for object details, so the
        // provider and the host handler are installed unconditionally - unlike
        // the mouse-hover popover, which stays opt-in via settings.
        visual.read(|cx| {
            let input = fixture.editor.read(cx).input();
            let lsp = input.read(cx).lsp();
            assert!(
                lsp.definition_provider.is_some(),
                "Cmd/Ctrl+click needs a definition provider"
            );
            assert!(
                lsp.show_document.is_some(),
                "the host must intercept the synthetic details URI"
            );
        });
    }

    #[gpui::test]
    fn sql_editor_secondary_click_opens_object_details_tab(cx: &mut gpui::TestAppContext) {
        let (fixture, visual) = editor_over_users_table(cx);
        let point = text_range_center(visual, &fixture.editor, &fixture.table_range);

        // Cmd/Ctrl+hover first: gpui-kit only offers an identifier to the
        // definition provider while the secondary modifier is held.
        visual.simulate_mouse_move(
            point,
            Option::<MouseButton>::None,
            Modifiers::secondary_key(),
        );
        settle(visual);

        visual.simulate_mouse_down(point, MouseButton::Left, Modifiers::secondary_key());
        settle(visual);
        let tabs = visual.read(|cx| details_tabs(&fixture, cx));
        assert_eq!(1, tabs.len(), "Cmd/Ctrl+click opens the details tab");
        assert_eq!("sql-object-details:app.public:table:users", tabs[0].0);
        assert!(
            tabs[0].1.contains("users"),
            "tab title should name the object, got {:?}",
            tabs[0].1
        );
        // 详情页签是 SQL 编辑器页签的兄弟：必须开在数据库页签内部的容器里，
        // 不能跑到主窗口的页签栏上。
        visual.read(|cx| {
            assert!(
                window_tabs(&fixture, cx).is_empty(),
                "details must not open in the window-level tab bar"
            );
        });

        // Re-resolving the same object must reuse its tab, not stack a copy.
        visual.simulate_mouse_down(point, MouseButton::Left, Modifiers::secondary_key());
        settle(visual);
        visual.read(|cx| {
            assert_eq!(1, details_tabs(&fixture, cx).len(), "details tabs reuse");
            assert!(window_tabs(&fixture, cx).is_empty());
        });
    }

    #[gpui::test]
    fn sql_editor_plain_click_on_table_keeps_normal_editing(cx: &mut gpui::TestAppContext) {
        let (fixture, visual) = editor_over_users_table(cx);
        let point = text_range_center(visual, &fixture.editor, &fixture.table_range);

        // Cmd/Ctrl+hover arms the link, but a plain click must only move the
        // caret - opening details on every click would be unusable.
        visual.simulate_mouse_move(
            point,
            Option::<MouseButton>::None,
            Modifiers::secondary_key(),
        );
        settle(visual);

        visual.simulate_click(point, Modifiers::default());
        settle(visual);
        visual.read(|cx| {
            assert!(
                details_tabs(&fixture, cx).is_empty(),
                "a plain click must not open the details tab"
            );
            assert!(window_tabs(&fixture, cx).is_empty());
            assert_eq!(
                fixture.table_range.start + fixture.table_range.len() / 2,
                fixture.editor.read(cx).input().read(cx).cursor(),
                "a plain click must still place the caret"
            );
        });
    }

    #[gpui::test]
    fn sql_object_details_handler_keeps_foreign_documents_to_the_kit(
        cx: &mut gpui::TestAppContext,
    ) {
        let (fixture, visual) = editor_over_users_table(cx);
        let handler = visual
            .read(|cx| {
                fixture
                    .editor
                    .read(cx)
                    .input()
                    .read(cx)
                    .lsp()
                    .show_document
                    .clone()
            })
            .expect("host handler should be installed");

        // A regular go-to-definition target (a real file) must fall through to
        // gpui-kit's own handling instead of opening a details tab.
        let handled = visual.update(|window, cx| {
            let params = ShowDocumentParams {
                uri: "file:///tmp/query.sql".parse().expect("valid uri"),
                external: Some(false),
                take_focus: Some(true),
                selection: None,
            };
            handler(&params, window, cx)
        });

        assert!(!handled, "only the synthetic details URI belongs to us");
        visual.read(|cx| {
            assert!(details_tabs(&fixture, cx).is_empty());
            assert!(window_tabs(&fixture, cx).is_empty());
        });
    }

    #[test]
    fn object_details_are_identified_per_object_and_kind() {
        let schema = users_schema();
        let details = |sql: &str| {
            crate::sql_editor_hover::resolve_object_details(
                sql,
                sql.find("users").expect("identifier") + 1,
                &schema,
            )
            .expect("details should resolve")
        };

        let table = details("select * from users");
        assert_eq!("app.public:table:users", table.id);
        assert_eq!("users", table.label);
        assert_eq!(SqlObjectDetailsKind::Table, table.kind);

        let column = details("select users.id from users");
        assert_eq!("app.public:column:users.id", column.id);
        assert_eq!("users.id", column.label);
        assert_eq!(SqlObjectDetailsKind::Column, column.kind);
    }

    #[gpui::test]
    fn sql_editor_show_hover_details_resolves_selection(cx: &mut gpui::TestAppContext) {
        let (fixture, visual) = editor_over_users_table(cx);
        // "users" is selected; the cursor sits at the selection start, which on
        // its own anchors "from" (a keyword) and fails to resolve.
        VisualTestContext::update(visual, |window, cx| {
            fixture.editor.update(cx, |editor, cx| {
                let input = editor.input();
                input.update(cx, |state, cx| {
                    let start = "select * from ".len();
                    state.set_selected_range(start..start + "users".len(), cx);
                });
            });
            window.draw(cx).clear(cx);
        });

        visual.update(|window, cx| {
            fixture.editor.update(cx, |editor, cx| {
                editor.show_hover_details(window, cx);
            });
        });
        let tabs = visual.read(|cx| details_tabs(&fixture, cx));
        assert_eq!(1, tabs.len(), "the menu entry opens the details tab");
        assert_eq!("sql-object-details:app.public:table:users", tabs[0].0);
        assert!(tabs[0].1.contains("users"));
        visual.read(|cx| {
            assert!(
                window_tabs(&fixture, cx).is_empty(),
                "the menu entry must open into the database tab as well"
            );
        });
    }

    #[gpui::test]
    fn object_details_fall_back_to_the_window_container_without_a_host(
        cx: &mut gpui::TestAppContext,
    ) {
        // 内嵌编辑器（表格单元格里的 SQL 编辑器）没有自己的页签容器，
        // 只能退回主窗口容器——不能变成一个点了没反应的入口。
        cx.update(|cx| {
            gpui_component::init(cx);
            cx.set_global(AppSettings::default());
        });
        let mut tab_container = None;
        let (_, visual) = cx.add_window_view(|window, cx| {
            let container = cx.new(|cx| TabContainer::new(window, cx));
            cx.set_global(GlobalTabContainer {
                tab_container: container.clone(),
            });
            tab_container = Some(container);
            SqlEditorHarness {
                editor: cx.new(|cx| SqlEditor::new(window, cx)),
                _subscription: None,
            }
        });
        let tab_container = tab_container.expect("tab container should be created");
        let schema = users_schema();
        let details = crate::sql_editor_hover::resolve_object_details(
            "select * from users",
            "select * from ".len() + 1,
            &schema,
        )
        .expect("details should resolve");

        let opened = visual.update(|window, cx| {
            crate::sql_object_details_tab::open_object_details_tab(
                crate::sql_object_details_tab::ObjectDetailsRequest {
                    details,
                    ddl: None,
                    host: None,
                },
                window,
                cx,
            )
        });

        assert!(
            opened,
            "a container-less editor still opens the details tab"
        );
        visual.read(|cx| {
            let tabs = tab_ids(&tab_container, cx);
            assert_eq!(1, tabs.len());
            assert_eq!("sql-object-details:app.public:table:users", tabs[0].0);
        });
    }

    /// 桩页签：被取消激活时回头去改编辑器，复刻 `SqlEditorTab::on_deactivate`
    /// 的行为（先改 `SqlEditor` 视图，再由它改到编辑器输入状态）。
    struct DeactivatingEditorStub {
        view: Entity<SqlEditor>,
        focus_handle: FocusHandle,
    }

    impl EventEmitter<TabContentEvent> for DeactivatingEditorStub {}

    impl Focusable for DeactivatingEditorStub {
        fn focus_handle(&self, _: &App) -> FocusHandle {
            self.focus_handle.clone()
        }
    }

    impl gpui::Render for DeactivatingEditorStub {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            gpui::div()
        }
    }

    impl TabContent for DeactivatingEditorStub {
        fn content_key(&self) -> &'static str {
            "DeactivatingEditorStub"
        }

        fn title(&self, _cx: &App) -> gpui::SharedString {
            "stub".into()
        }

        fn on_deactivate(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
            self.view.update(cx, |editor, cx| {
                editor.invalidate_metadata_context(cx);
                editor.invalidate_completions(cx);
            });
        }
    }

    /// 让宿主编容器里站着一个「被取消激活就去改编辑器」的页签。
    ///
    /// 真实场景就是 SQL 编辑器页签：Cmd/Ctrl+点击时它就是宿主编容器里
    /// 当前的页签，开详情页签会先把它取消激活。
    fn host_with_deactivating_tab(fixture: &SqlEditorFixture, visual: &mut VisualTestContext) {
        let view = fixture.editor.clone();
        let host = fixture.tab_container.clone();
        visual.update(|_window, cx| {
            let stub = cx.new(|cx| DeactivatingEditorStub {
                view,
                focus_handle: cx.focus_handle(),
            });
            host.update(cx, |container, cx| {
                container.add_and_activate_tab(TabItem::new("stub", "stub", stub), cx);
            });
        });
    }

    /// `select * from users` 里 `users` 的对象详情。
    fn users_table_details() -> crate::sql_editor_hover::SqlObjectDetails {
        crate::sql_editor_hover::resolve_object_details(
            "select * from users",
            "select * from ".len() + 1,
            &users_schema(),
        )
        .expect("details should resolve")
    }

    #[gpui::test]
    fn object_details_open_leaves_the_requesting_input_update_first(cx: &mut gpui::TestAppContext) {
        // Cmd/Ctrl+点击这条路径：`go_to_definition` 是在编辑器输入状态自己的
        // update 里回调进 `show_document` 的，同步开页签必然 double-lease。
        let (fixture, visual) = editor_over_users_table(cx);
        host_with_deactivating_tab(&fixture, visual);
        let input = visual.read(|cx| fixture.editor.read(cx).input());
        let details = users_table_details();
        let host = fixture.tab_container.clone();

        let opened = visual.update(|window, cx| {
            input.update(cx, |_state, cx| {
                crate::sql_object_details_tab::open_object_details_tab(
                    crate::sql_object_details_tab::ObjectDetailsRequest {
                        details,
                        ddl: None,
                        host: Some(host.clone()),
                    },
                    window,
                    cx,
                )
            })
        });
        assert!(opened, "有宿主容器时应当接住详情页签");

        visual.read(|cx| {
            let tabs = details_tabs(&fixture, cx);
            assert_eq!(2, tabs.len(), "详情页签要和宿主的当前页签并排");
            assert_eq!("sql-object-details:app.public:table:users", tabs[1].0);
        });
    }

    #[gpui::test]
    fn object_details_open_leaves_the_requesting_editor_update_first(
        cx: &mut gpui::TestAppContext,
    ) {
        // 右键菜单这条路径：动作由 `cx.listener` 派发，也就是在 `SqlEditor`
        // 自己的 update 里；取消激活时被改的正是它。
        let (fixture, visual) = editor_over_users_table(cx);
        host_with_deactivating_tab(&fixture, visual);
        let details = users_table_details();
        let host = fixture.tab_container.clone();

        let opened = visual.update(|window, cx| {
            fixture.editor.update(cx, |_editor, cx| {
                crate::sql_object_details_tab::open_object_details_tab(
                    crate::sql_object_details_tab::ObjectDetailsRequest {
                        details,
                        ddl: None,
                        host: Some(host.clone()),
                    },
                    window,
                    cx,
                )
            })
        });
        assert!(opened, "有宿主容器时应当接住详情页签");

        visual.read(|cx| {
            let tabs = details_tabs(&fixture, cx);
            assert_eq!(2, tabs.len(), "详情页签要和宿主的当前页签并排");
            assert_eq!("sql-object-details:app.public:table:users", tabs[1].0);
        });
    }

    /// 「复制 DDL」写进剪贴板的是驱动生成的 DDL（与表设计器一致），不再从
    /// hover markdown 里抠代码块。
    #[gpui::test]
    fn copy_ddl_writes_the_driver_ddl_to_the_clipboard(cx: &mut gpui::TestAppContext) {
        const DDL: &str =
            "CREATE TABLE `users` (\n  `id` INT NOT NULL,\n  PRIMARY KEY (`id`)\n) ENGINE=InnoDB";
        let message = cx.update(|cx| {
            copy_ddl_message(
                Some(crate::table_ddl::TableDdlSection::Ready(DDL.into())),
                cx,
            )
        });
        let copied = cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text()));

        assert_eq!(Some(DDL.to_string()), copied);
        assert_eq!(t!("Query.ddl_copied").to_string(), message);
    }

    /// 生成失败或光标不在表上时，不写剪贴板。
    #[gpui::test]
    fn copy_ddl_reports_failures_without_touching_the_clipboard(cx: &mut gpui::TestAppContext) {
        let failed = cx.update(|cx| {
            copy_ddl_message(
                Some(crate::table_ddl::TableDdlSection::Failed(
                    "connection closed".into(),
                )),
                cx,
            )
        });
        let missing = cx.update(|cx| copy_ddl_message(None, cx));
        let copied = cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text()));

        assert_eq!(
            t!(
                "Query.object_details_ddl_failed",
                error = "connection closed"
            )
            .to_string(),
            failed
        );
        assert_eq!(t!("Query.no_hover_details_at_cursor").to_string(), missing);
        assert_eq!(None, copied, "没有 DDL 时不该动剪贴板");
    }

    #[gpui::test]
    fn extended_editor_renders_sql_signature_help(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            cx.set_global(AppSettings::default());
        });
        let mut sql_editor = None;
        let (_, visual) = cx.add_window_view(|window, cx| {
            let editor = cx.new(|cx| SqlEditor::new(window, cx));
            sql_editor = Some(editor.clone());
            SqlEditorHarness {
                editor,
                _subscription: None,
            }
        });
        let sql_editor = sql_editor.expect("SQL editor should be created");

        VisualTestContext::update(visual, |window, cx| {
            sql_editor.update(cx, |editor, cx| {
                editor.set_schema(
                    SqlSchema::default().with_functions([("coalesce(a, b)", "first value")]),
                    window,
                    cx,
                );
                editor.set_value("SELECT coalesce(".to_string(), window, cx);
                let input = editor.input();
                input.update(cx, |state, cx| {
                    let cursor = "SELECT coalesce(".len();
                    state.set_selected_range(cursor..cursor, cx);
                });
                editor
                    .extended_editor
                    .update(cx, |state, cx| state.refresh_signature_help(window, cx));
            });
        });
        visual.run_until_parked();

        visual.read(|cx| {
            let extended = sql_editor.read(cx).extended_editor.read(cx);
            let help = extended.help().expect("signature help should be visible");
            assert_eq!("coalesce(a, b)", help.signatures[0].label);
            assert_eq!(Some(0), help.active_parameter);
        });
    }

    #[test]
    fn sql_context_menu_keeps_run_and_default_editing_actions() {
        let capabilities = gpui_base::input::InputContextMenuCapabilities::new()
            .code_editor(true)
            .selection(true);
        let menu = sql_editor_context_menu(capabilities, true);
        let actions = menu
            .items
            .iter()
            .filter_map(|item| match item {
                gpui_base::input::NativeMenuItem::Action { action, .. } => Some(action.as_ref()),
                gpui_base::input::NativeMenuItem::Separator => None,
            })
            .collect::<Vec<_>>();

        assert!(
            actions
                .iter()
                .any(|action| action.partial_eq(&RunSelectedSql))
        );
        assert!(
            actions
                .iter()
                .any(|action| action.partial_eq(&RunCursorStatementSql))
        );
        assert!(actions.iter().any(|action| action.partial_eq(&Cut)));
        assert!(actions.iter().any(|action| action.partial_eq(&Copy)));
        assert!(actions.iter().any(|action| action.partial_eq(&Paste)));
        assert!(actions.iter().any(|action| action.partial_eq(&SelectAll)));
    }

    #[test]
    fn identifier_match_rank_prefers_prefix_then_boundary_then_substring() {
        // Prefix match
        assert_eq!(Some(0), identifier_match_rank("users", "USER"));
        assert_eq!(Some(0), identifier_match_rank("Users", "USER"));
        // Word-boundary match (right after `_`)
        assert_eq!(Some(1), identifier_match_rank("admin_users", "USER"));
        // Plain substring match
        assert_eq!(Some(2), identifier_match_rank("users", "SER"));
        assert_eq!(Some(2), identifier_match_rank("busers", "USER"));
        // No match
        assert_eq!(None, identifier_match_rank("orders", "USER"));
        // Empty word matches everything as prefix
        assert_eq!(Some(0), identifier_match_rank("anything", ""));
    }

    #[test]
    fn infix_matches_still_outrank_irrelevant_context_items() {
        // A substring-matched table in TableName context should score better
        // than a prefix-matched keyword.
        let table_score = completion_priority::calculate_score_with_match(
            &SqlContext::TableName,
            Some(CompletionItemKind::STRUCT),
            0,
        );
        let keyword_score = completion_priority::calculate_score_with_match(
            &SqlContext::TableName,
            Some(CompletionItemKind::KEYWORD),
            completion_priority::PREFIX_MATCH_BOOST,
        );
        assert!(table_score < keyword_score);
    }

    #[test]
    fn match_boost_orders_prefix_before_boundary_before_substring() {
        let prefix = completion_priority::calculate_score_with_match(
            &SqlContext::TableName,
            Some(CompletionItemKind::STRUCT),
            completion_priority::PREFIX_MATCH_BOOST,
        );
        let boundary = completion_priority::calculate_score_with_match(
            &SqlContext::TableName,
            Some(CompletionItemKind::STRUCT),
            completion_priority::BOUNDARY_MATCH_BOOST,
        );
        let substring = completion_priority::calculate_score_with_match(
            &SqlContext::TableName,
            Some(CompletionItemKind::STRUCT),
            0,
        );
        assert!(prefix < boundary);
        assert!(boundary < substring);
    }

    #[test]
    fn sql_editor_render_uses_cached_font() {
        let source = include_str!("sql_editor.rs");
        let editor_font = source
            .split("fn editor_font(")
            .nth(1)
            .expect("editor_font helper exists")
            .split("/// Set database-specific completion information")
            .next()
            .expect("editor_font helper has an end marker");
        let render = source
            .split("impl Render for SqlEditor")
            .nth(1)
            .expect("SqlEditor render impl exists")
            .split("#[cfg(test)]")
            .next()
            .expect("SqlEditor render impl has an end marker");

        assert!(editor_font.contains("cache.requested_family == font_family"));
        assert!(editor_font.contains("cx.text_system().all_font_names()"));
        assert!(editor_font.contains("installed_grid_monospace_font("));
        assert!(!editor_font.contains("installed_business_grid_monospace_font("));
        assert!(!editor_font.contains("cache.installed_font_names == installed_font_names"));
        assert!(!editor_font.contains("installed_font_names,"));
        assert!(!render.contains("cx.text_system().all_font_names()"));
        assert!(render.contains("AppSettings::global(cx).sql_editor_font_size"));
        assert!(render.contains(".text_size(gpui::px(font_size))"));
    }

    #[test]
    fn sql_editor_installs_the_sql_gutter_renderer_on_its_lane() {
        let source = include_str!("sql_editor.rs");

        assert!(source.contains("GutterLaneOptions::new(render_sql_gutter_marker)"));
    }

    #[test]
    fn sql_editor_wires_context_menu_through_the_editor_element_builder() {
        let source = include_str!("sql_editor.rs");

        // gpui-component reinstalls the editor context menu on every frame, so
        // the SQL run items must be attached via the element builder in render
        // instead of a one-time state-level handler. The builder runs while the
        // editor is being updated, so capabilities are snapshotted in render
        // and must not be re-read from the entity at click time.
        let render = source
            .split("impl Render for SqlEditor")
            .nth(1)
            .expect("SqlEditor render impl exists");
        assert!(render.contains(".context_menu("));
        assert!(render.contains("context_menu_capabilities()"));
        assert!(!render.contains("input.read(cx)"));
    }

    #[test]
    fn empty_schema_has_no_metadata() {
        let view = schema_to_metadata_view(&SqlSchema::default());
        assert!(!view.has_metadata());
        assert!(view.tables.is_empty());
    }

    #[test]
    fn schema_to_metadata_view_maps_tables_columns_and_scope() {
        let mut columns = HashMap::new();
        columns.insert(
            "Users".to_string(),
            vec![("id".to_string(), "int".to_string(), String::new())],
        );
        let schema = SqlSchema {
            tables: vec![("Users".to_string(), String::new())],
            columns_by_table: columns,
            current_database: Some("app".to_string()),
            current_schema: Some("public".to_string()),
            ..Default::default()
        };
        let view = schema_to_metadata_view(&schema);
        assert!(view.has_metadata());
        assert!(view.table_exists("users"));
        assert!(view.table_exists("USERS"));
        assert_eq!(view.column_status("users", "id"), Some(true));
        assert_eq!(view.column_status("users", "missing"), Some(false));
        assert!(view.is_schema_or_db("public"));
        assert!(view.is_schema_or_db("app"));
    }

    #[test]
    fn parser_diagnostic_converts_to_input_severity_and_source() {
        let text = "SELECT 'oops FROM users";
        let mut next_id = 0;
        let diags = analyze_parser_diagnostics(text, SqlDialect::Standard, 7, &mut next_id);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, SqlDiagnosticSeverity::Error);

        let rope = Rope::from_str(text);
        let input_diag = sql_diagnostic_to_input(&diags[0], &rope);
        assert_eq!(input_diag.severity, DiagnosticSeverity::Error);
        assert_eq!(input_diag.source.as_deref(), Some("sql"));
        assert_eq!(input_diag.code.as_deref(), Some("parser.unclosed_string"));
        assert!(input_diag.range.start <= input_diag.range.end);
        // The range covers the unclosed literal token.
        assert!(input_diag.range.start.character >= 7);
    }

    #[test]
    fn diagnostic_byte_range_converts_for_multibyte_text() {
        let text = "-- 中文注释\nSELECT * FROM missing;";
        let mut next_id = 0;
        let diags = analyze_semantic_diagnostics(
            text,
            &SqlStatementSnapshot::new(text.to_string(), SqlDialect::Standard),
            &schema_to_metadata_view(&SqlSchema::default()),
            1,
            &mut next_id,
        );
        // No metadata -> semantic checker stays silent.
        assert!(diags.is_empty());

        let parser_diag_text = "SELECT '未闭合";
        let mut parser_next_id = 0;
        let parser_diags = analyze_parser_diagnostics(
            parser_diag_text,
            SqlDialect::Standard,
            1,
            &mut parser_next_id,
        );
        assert_eq!(parser_diags.len(), 1);
        let rope = Rope::from_str(parser_diag_text);
        let input_diag = sql_diagnostic_to_input(&parser_diags[0], &rope);
        assert_eq!(
            input_diag.range,
            rope.offset_to_position(parser_diags[0].range.start_byte)
                ..rope.offset_to_position(parser_diags[0].range.end_byte)
        );
    }

    #[test]
    fn diagnostics_pipeline_flags_unknown_table_with_real_schema() {
        let text = "SELECT * FROM missing_table;\nSELECT COUNT(*) FROM users;";
        let dialect = SqlDialect::Standard;
        let snapshot = SqlStatementSnapshot::new(text.to_string(), dialect);

        let mut columns = HashMap::new();
        columns.insert(
            "Users".to_string(),
            vec![("id".to_string(), "int".to_string(), String::new())],
        );
        let schema = SqlSchema {
            tables: vec![("Users".to_string(), String::new())],
            columns_by_table: columns,
            ..Default::default()
        };
        let metadata = schema_to_metadata_view(&schema);

        let mut next_id = 0;
        let mut diags = analyze_parser_diagnostics(text, dialect, 0, &mut next_id);
        diags.extend(analyze_semantic_diagnostics(
            text,
            &snapshot,
            &metadata,
            0,
            &mut next_id,
        ));
        let rope = Rope::from_str(text);
        let input_diags: Vec<_> = diags
            .iter()
            .map(|d| sql_diagnostic_to_input(d, &rope))
            .collect();

        assert_eq!(input_diags.len(), 1);
        let start_byte = text.find("missing_table").expect("token present");
        let rope = Rope::from_str(text);
        assert_eq!(
            input_diags[0].range.start,
            rope.offset_to_position(start_byte)
        );
    }

    #[test]
    fn analyze_diagnostics_pure_runs_parser_and_semantic_pass_off_thread() {
        // `analyze_diagnostics_pure` is the computation the background worker
        // runs; the cached `Arc` schema and the document revision must flow
        // through unchanged.
        let text = "SELECT * FROM missing_table;\nSELECT 'oops";
        let dialect = SqlDialect::Standard;
        let mut columns = HashMap::new();
        columns.insert(
            "Users".to_string(),
            vec![("id".to_string(), "int".to_string(), String::new())],
        );
        let schema = Arc::new(SqlSchema {
            tables: vec![("Users".to_string(), String::new())],
            columns_by_table: columns,
            ..Default::default()
        });

        let snapshot = analyze_diagnostics_pure(text.to_string(), dialect, schema, 42);
        assert_eq!(snapshot.document_revision, 42);

        let codes: Vec<&str> = snapshot
            .diagnostics
            .iter()
            .map(|diag| diag.code.as_deref().unwrap_or(""))
            .collect();
        // Unclosed string literal raises a parser diagnostic, and the dangling
        // `missing_table` reference raises a semantic one against the schema.
        assert!(
            codes.iter().any(|code| *code == "parser.unclosed_string"),
            "missing parser diagnostic: {codes:?}"
        );
        assert!(
            codes.iter().any(|code| code == &"semantic.unknown_table"),
            "missing semantic diagnostic: {codes:?}"
        );

        let clean = analyze_diagnostics_pure(
            "SELECT 1;".to_string(),
            dialect,
            Arc::new(SqlSchema::default()),
            7,
        );
        assert_eq!(clean.document_revision, 7);
        assert!(clean.diagnostics.is_empty());
    }
}
