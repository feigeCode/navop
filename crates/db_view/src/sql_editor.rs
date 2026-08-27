use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use crate::sql_editor_hover::DefaultSqlHoverProvider;
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
use gpui::{
    App, AppContext, Context, Entity, Font, IntoElement, Render, Styled as _, Subscription, Task,
    Window,
};
use gpui_component::highlighter::{Diagnostic, DiagnosticSeverity};
use gpui_component::input::{
    CodeActionProvider, CompletionProvider, HoverProvider, Input, InputContextMenuItem, InputEvent,
    InputState, TabSize,
};
use gpui_component::scroll::ScrollbarShow;
use gpui_component::{Rope, RopeExt};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit,
    InlineCompletionContext, InlineCompletionItem, InlineCompletionResponse, InsertReplaceEdit,
    InsertTextFormat, Range as LspRange,
};
use one_core::settings::{AppSettings, installed_grid_monospace_font};
use rust_i18n::t;
use sum_tree::Bias;

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
    Tables,
    Columns(String),
    None,
}

pub(crate) fn sql_dot_completion_target(
    schema: &SqlSchema,
    qualifier: &str,
) -> SqlDotCompletionTarget {
    let is_scope = schema
        .current_database
        .as_deref()
        .is_some_and(|database| database.eq_ignore_ascii_case(qualifier))
        || schema
            .current_schema
            .as_deref()
            .is_some_and(|schema| schema.eq_ignore_ascii_case(qualifier));
    if is_scope {
        return SqlDotCompletionTarget::Tables;
    }

    schema
        .columns_by_table
        .keys()
        .find(|table| table.eq_ignore_ascii_case(qualifier))
        .cloned()
        .map(SqlDotCompletionTarget::Columns)
        .unwrap_or(SqlDotCompletionTarget::None)
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
    fn parse_context_with_symbols(tokens: &[SqlToken], offset: usize) -> (SqlContext, SymbolTable) {
        let symbol_table = SymbolTable::build_from_tokens(tokens);
        let inferred = ContextInferrer::infer(tokens, offset, &symbol_table);
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

pub(crate) fn current_statement_has_from_keyword(
    text: &str,
    tokens: &[SqlToken],
    offset: usize,
) -> bool {
    let offset = clip_sql_offset(text, offset);
    let statement_start = tokens
        .iter()
        .filter(|token| token.kind == SqlTokenKind::Semicolon && token.end <= offset)
        .map(|token| token.end)
        .next_back()
        .unwrap_or(0);
    let statement_end = tokens
        .iter()
        .find(|token| token.kind == SqlTokenKind::Semicolon && token.start >= offset)
        .map(|token| token.start)
        .unwrap_or(text.len());

    tokens.iter().any(|token| {
        token.start >= statement_start
            && token.end <= statement_end
            && token.is_keyword_of(SqlKeyword::From)
    })
}

impl CompletionProvider for DefaultSqlCompletionProvider {
    fn completions(
        &self,
        rope: &Rope,
        offset: usize,
        _trigger: CompletionContext,
        _window: &mut Window,
        cx: &mut Context<InputState>,
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

            // Handle dot context (table.column) - highest priority
            // Uses SymbolTable to resolve alias to actual table name
            if let SqlContext::DotColumn(alias_or_table) = &context {
                // Resolve alias to table name using symbol table
                // If alias is found, use the resolved table name; otherwise use as-is
                let resolved_table = symbol_table
                    .resolve(alias_or_table)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| alias_or_table.clone());

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
                    items.sort_by(|a, b| {
                        a.sort_text
                            .as_ref()
                            .unwrap_or(&a.label)
                            .cmp(b.sort_text.as_ref().unwrap_or(&b.label))
                    });
                    items.truncate(50);
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
                    SqlDotCompletionTarget::Tables | SqlDotCompletionTarget::None => {
                        return Ok(CompletionResponse::Array(items));
                    }
                };
                let columns = schema.columns_by_table.get(&resolved_table).or_else(|| {
                    // Case-insensitive lookup
                    let lower = resolved_table.to_lowercase();
                    schema
                        .columns_by_table
                        .iter()
                        .find(|(k, _)| k.to_lowercase() == lower)
                        .map(|(_, v)| v)
                });

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
        cx: &mut Context<InputState>,
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

    fn is_completion_trigger(
        &self,
        _offset: usize,
        new_text: &str,
        _cx: &mut Context<InputState>,
    ) -> bool {
        self.is_completion_trigger_check(new_text)
    }
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
        cx: &mut Context<InputState>,
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

    fn is_completion_trigger(
        &self,
        _offset: usize,
        new_text: &str,
        _cx: &mut Context<InputState>,
    ) -> bool {
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
fn sql_diagnostic_to_input(diag: &SqlDiagnostic, text: &str) -> Diagnostic {
    let rope = Rope::from_str(text);
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
    let snapshot = SqlStatementSnapshot::new(text.clone(), dialect);
    let mut sql_diags = analyze_parser_diagnostics(&text, dialect, document_revision, &mut next_id);
    let metadata = schema_to_metadata_view(&schema);
    sql_diags.extend(analyze_semantic_diagnostics(
        &text,
        &snapshot,
        &metadata,
        document_revision,
        &mut next_id,
    ));
    let diagnostics = sql_diags
        .iter()
        .map(|diag| sql_diagnostic_to_input(diag, &text))
        .collect();
    SqlDiagnosticSnapshot {
        document_revision,
        diagnostics,
    }
}

/// A reusable SQL editor component built on top of `Input`.
pub struct SqlEditor {
    editor: Entity<InputState>,
    default_completion_provider: Rc<DefaultSqlCompletionProvider>,
    default_hover_provider: Option<Rc<DefaultSqlHoverProvider>>,
    _subscriptions: Vec<Subscription>,
    font_cache: Option<SqlEditorFontCache>,
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
        let default_hover_provider_trait: Rc<dyn HoverProvider> = default_hover_provider.clone();

        let editor = cx.new(|cx| {
            let mut editor = InputState::new(window, cx)
                .code_editor("sql")
                .line_number(true)
                .searchable(true)
                .indent_guides(true)
                .tab_size(TabSize {
                    tab_size: 2,
                    hard_tabs: false,
                })
                .soft_wrap(false)
                .placeholder(t!("Query.editor_placeholder").to_string());

            // Defaults: completion + hover + actions
            editor.lsp.completion_provider = Some(default_provider_trait);
            editor.lsp.hover_provider = Some(default_hover_provider_trait);

            editor
        });

        let _subscriptions =
            vec![
                cx.subscribe_in(&editor, window, move |_, _, _: &InputEvent, _window, cx| {
                    cx.notify()
                }),
            ];
        Self {
            editor,
            default_completion_provider,
            default_hover_provider: Some(default_hover_provider),
            _subscriptions,
            font_cache: None,
        }
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
    pub fn input(&self) -> Entity<InputState> {
        self.editor.clone()
    }

    /// Invalidate active completion popup and inline completion requests.
    pub fn invalidate_completions(&self, cx: &mut Context<Self>) {
        self.editor
            .update(cx, |state, cx| state.invalidate_completions(cx));
    }

    /// Invalidate metadata-dependent completion and hover state.
    pub fn invalidate_metadata_context(&self, cx: &mut Context<Self>) {
        self.editor.update(cx, |state, cx| {
            state.invalidate_completions(cx);
            state.invalidate_hover(cx);
        });
    }

    /// 设置编辑器右键菜单的扩展项（支持一级和二级菜单）。
    pub fn set_mouse_context_menu_items(
        &mut self,
        items: Vec<InputContextMenuItem>,
        cx: &mut Context<Self>,
    ) {
        self.editor.update(cx, move |state, cx| {
            state.set_mouse_context_menu_items(items, cx)
        });
    }

    /// Replace default completion provider.
    pub fn set_completion_provider(
        &mut self,
        provider: Rc<dyn CompletionProvider>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editor.update(cx, |state, cx| {
            state.invalidate_completions(cx);
            state.lsp.completion_provider = Some(provider);
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
        let default_provider = self.default_completion_provider.clone();
        let default_provider_trait: Rc<dyn CompletionProvider> = default_provider.clone();
        self.editor.update(cx, |state, _| {
            let is_default_provider_installed = state
                .lsp
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
        let default_hover_provider_trait: Rc<dyn HoverProvider> = default_hover_provider.clone();
        self.editor.update(cx, |state, _| {
            let is_default_provider_installed = state
                .lsp
                .hover_provider
                .as_ref()
                .is_some_and(|provider| Rc::ptr_eq(provider, &default_hover_provider_trait));
            if is_default_provider_installed {
                default_hover_provider.set_schema(schema);
            }
        });
    }

    /// Replace hover provider.
    pub fn set_hover_provider(
        &mut self,
        provider: Rc<dyn HoverProvider>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editor.update(cx, |state, cx| {
            state.invalidate_hover(cx);
            state.lsp.hover_provider = Some(provider);
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
            state.lsp.code_action_providers.push(provider)
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
            state.set_selected_range(range, false, window, cx);
            state.replace(replacement, window, cx);
            state.set_selected_range(selection, false, window, cx);
        });
    }

    /// Get the current text content of the editor.
    /// This is a convenience method that accesses the underlying InputState.
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
        self.editor.read(cx).selected_text_string()
    }

    /// Get the current cursor byte offset.
    pub fn cursor_offset(&self, cx: &App) -> usize {
        self.editor.read(cx).cursor()
    }

    /// Get the current selection as UTF-8 byte offsets.
    pub fn selected_range(&self, cx: &App) -> Range<usize> {
        self.editor.read(cx).selected_range()
    }
}

impl Render for SqlEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let font = self.editor_font(cx);
        let font_size = AppSettings::global(cx).sql_editor_font_size as f32;
        Input::new(&self.editor)
            .editor_scrollbar_show(ScrollbarShow::Always)
            .font(font)
            .text_size(gpui::px(font_size))
            .line_height(gpui::px(font_size * 1.5))
            .size_full()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        SqlContext, SqlSchema, analyze_diagnostics_pure, completion_priority,
        identifier_match_rank, schema_to_metadata_view, sql_diagnostic_to_input,
    };
    use db::sql_editor::diagnostics::{
        SqlDiagnosticSeverity, analyze_parser_diagnostics, analyze_semantic_diagnostics,
    };
    use db::sql_editor::statement_ranges::{SqlDialect, SqlStatementSnapshot};
    use gpui_component::highlighter::DiagnosticSeverity;
    use gpui_component::{Rope, RopeExt};
    use lsp_types::CompletionItemKind;
    use std::collections::HashMap;
    use std::sync::Arc;

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

        let input_diag = sql_diagnostic_to_input(&diags[0], text);
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
        let input_diag = sql_diagnostic_to_input(&parser_diags[0], parser_diag_text);
        let rope = Rope::from_str(parser_diag_text);
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
        let input_diags: Vec<_> = diags
            .iter()
            .map(|d| sql_diagnostic_to_input(d, text))
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
