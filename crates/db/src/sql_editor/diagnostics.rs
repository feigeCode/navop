//! SQL editor diagnostics.
//!
//! Phase 7 of the SQL editor implementation (spec §12). Provides parser and
//! semantic diagnostics (unknown table / column / alias) on a shared
//! `SqlStatementSnapshot` and `SqlMetadataView`. Execution error mapping is a
//! later phase (it lives in the result panel, not the squiggle layer).
//!
//! Conservative-by-design: when metadata is incomplete (a table has no loaded
//! column list, or no metadata at all) the checker stays silent instead of
//! emitting false positives (spec §12.3).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::sql_editor::sql_symbol_table::SymbolTable;
use crate::sql_editor::sql_tokenizer::{SqlKeyword, SqlToken, SqlTokenKind, SqlTokenizer};
use crate::sql_editor::statement_ranges::{
    SqlDialect, SqlStatementKind, SqlStatementRange, SqlStatementSnapshot, SqlTextRange,
};

/// Severity of a SQL diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlDiagnosticSeverity {
    Error,
    Warning,
    Information,
    Hint,
}

/// Which layer produced the diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlDiagnosticSource {
    Parser,
    Semantic,
    Metadata,
    Execution,
}

/// A related range/message pair attached to a diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlDiagnosticRelatedInfo {
    pub range: SqlTextRange,
    pub message: Arc<str>,
}

/// One diagnostic in document coordinates (UTF-8 byte offsets).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlDiagnostic {
    pub id: u64,
    /// Document revision the diagnostic was computed against; the consumer
    /// drops diagnostics whose revision does not match the current document.
    pub document_revision: u64,
    pub range: SqlTextRange,
    pub severity: SqlDiagnosticSeverity,
    pub code: Option<Arc<str>>,
    pub message: Arc<str>,
    pub source: SqlDiagnosticSource,
    pub related: Arc<[SqlDiagnosticRelatedInfo]>,
}

impl SqlDiagnostic {
    fn build(
        id: u64,
        document_revision: u64,
        range: SqlTextRange,
        severity: SqlDiagnosticSeverity,
        code: Option<&str>,
        message: impl Into<Arc<str>>,
        source: SqlDiagnosticSource,
        related: Vec<SqlDiagnosticRelatedInfo>,
    ) -> Self {
        Self {
            id,
            document_revision,
            range,
            severity,
            code: code.map(Into::into),
            message: message.into(),
            source,
            related: related.into(),
        }
    }
}

/// A read-only view over the schema metadata used by the semantic checker.
///
/// Identifiers are normalized to uppercase for case-insensitive matching.
/// A table with an *empty* column list means its columns have not been loaded
/// yet: the checker must not emit unknown-column errors for it.
#[derive(Debug, Clone, Default)]
pub struct SqlMetadataView {
    pub tables: HashSet<String>,
    pub columns_by_table: HashMap<String, Vec<String>>,
    pub schemas: HashSet<String>,
    pub current_schema: Option<String>,
    pub current_database: Option<String>,
}

impl SqlMetadataView {
    pub fn has_metadata(&self) -> bool {
        !self.tables.is_empty()
            || !self.schemas.is_empty()
            || self.current_schema.is_some()
            || self.current_database.is_some()
    }

    /// Case-insensitive existence check for a bare unquoted table reference.
    pub fn table_exists(&self, name: &str) -> bool {
        self.tables
            .iter()
            .any(|table| table.eq_ignore_ascii_case(name))
    }

    fn table_exists_with_quote(&self, name: &str, quoted: bool) -> bool {
        if quoted {
            self.tables.contains(name)
        } else {
            self.table_exists(name)
        }
    }

    /// True when `name` is the current database/schema or a known schema.
    pub fn is_schema_or_db(&self, name: &str) -> bool {
        let name = normalize_ident(name);
        self.schemas.contains(&name)
            || self
                .current_schema
                .as_deref()
                .map(|s| normalize_ident(s) == name)
                .unwrap_or(false)
            || self
                .current_database
                .as_deref()
                .map(|s| normalize_ident(s) == name)
                .unwrap_or(false)
    }

    /// `Some(true)` if the column is known; `Some(false)` if the table's
    /// columns are loaded and the column is missing; `None` when the table's
    /// columns have not been loaded (caller must skip the check).
    pub fn column_status(&self, table: &str, column: &str) -> Option<bool> {
        self.column_status_with_quote(table, column, false)
    }

    fn column_status_with_quote(&self, table: &str, column: &str, quoted: bool) -> Option<bool> {
        let columns = self
            .columns_by_table
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(table))?
            .1;
        if columns.is_empty() {
            return None;
        }
        Some(if quoted {
            columns.iter().any(|candidate| candidate == column)
        } else {
            columns
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(column))
        })
    }

    pub fn with_tables(mut self, tables: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.tables = tables.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_columns(
        mut self,
        table: impl Into<String>,
        columns: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.columns_by_table
            .insert(table.into(), columns.into_iter().map(Into::into).collect());
        self
    }

    pub fn with_current_schema(mut self, schema: impl Into<String>) -> Self {
        self.current_schema = Some(schema.into());
        self
    }

    pub fn with_current_database(mut self, database: impl Into<String>) -> Self {
        self.current_database = Some(database.into());
        self
    }

    pub fn with_schemas(mut self, schemas: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.schemas = schemas
            .into_iter()
            .map(|s| normalize_ident(&s.into()))
            .collect();
        self
    }
}

/// Parser diagnostics over the whole document.
///
/// Currently flags unclosed string literals, unclosed quoted identifiers and
/// unclosed block comments. `dialect` is reserved for future rules.
pub fn analyze_parser_diagnostics(
    text: &str,
    _dialect: SqlDialect,
    document_revision: u64,
    next_id: &mut u64,
) -> Vec<SqlDiagnostic> {
    let tokens = SqlTokenizer::new(text).tokenize();
    let text_len = text.len();
    let mut out = Vec::new();

    for token in tokens.iter() {
        let (closed, code, message) = match token.kind {
            SqlTokenKind::String => (
                count_char(&token.text, '\'') % 2 == 0,
                "parser.unclosed_string",
                "Unclosed string literal",
            ),
            SqlTokenKind::QuotedIdent => (
                count_char(&token.text, '"') % 2 == 0,
                "parser.unclosed_quoted_ident",
                "Unclosed quoted identifier",
            ),
            SqlTokenKind::BlockComment => (
                token.text.ends_with("*/"),
                "parser.unclosed_block_comment",
                "Unclosed block comment",
            ),
            _ => continue,
        };
        if closed {
            continue;
        }

        *next_id += 1;
        out.push(SqlDiagnostic::build(
            *next_id,
            document_revision,
            SqlTextRange {
                start_byte: token.start.min(text_len),
                end_byte: token.end.min(text_len),
            },
            SqlDiagnosticSeverity::Error,
            Some(code),
            message,
            SqlDiagnosticSource::Parser,
            Vec::new(),
        ));
    }

    out
}

/// Semantic diagnostics (unknown table / column / alias) per executable
/// statement, using the shared statement snapshot so the same splitting
/// algorithm is used everywhere.
pub fn analyze_semantic_diagnostics(
    text: &str,
    snapshot: &SqlStatementSnapshot,
    metadata: &SqlMetadataView,
    document_revision: u64,
    next_id: &mut u64,
) -> Vec<SqlDiagnostic> {
    if !metadata.has_metadata() {
        return Vec::new();
    }

    let tokens = SqlTokenizer::new(text).tokenize();
    let mut out = Vec::new();

    for statement in snapshot.statement_ranges() {
        if statement.kind != SqlStatementKind::Sql {
            // Oracle PL/SQL, procedure/function/trigger batches: semantic
            // rules are unreliable there (spec §12.4.7).
            continue;
        }
        let stmt_tokens: Vec<SqlToken> = tokens
            .iter()
            .filter(|t| {
                !matches!(t.kind, SqlTokenKind::Eof)
                    && t.start >= statement.sql_range.start_byte
                    && t.end <= statement.sql_range.end_byte
            })
            .cloned()
            .collect();
        analyze_statement_semantic(
            &stmt_tokens,
            statement,
            metadata,
            document_revision,
            next_id,
            &mut out,
        );
    }

    out
}

/// One table reference found in a statement. `token_index` points into the
/// meaningful token slice. `check_existence` is false for schema-qualified
/// references (registered for column checks but never existence-checked).
struct TableRef {
    display: String,
    token_index: usize,
    check_existence: bool,
    quoted: bool,
}

#[allow(clippy::too_many_arguments)]
fn analyze_statement_semantic(
    stmt_tokens: &[SqlToken],
    statement: &SqlStatementRange,
    metadata: &SqlMetadataView,
    document_revision: u64,
    next_id: &mut u64,
    out: &mut Vec<SqlDiagnostic>,
) {
    let _ = statement;
    let meaningful: Vec<&SqlToken> = stmt_tokens
        .iter()
        .filter(|t| {
            !matches!(
                t.kind,
                SqlTokenKind::Whitespace | SqlTokenKind::LineComment | SqlTokenKind::BlockComment
            )
        })
        .collect();
    if meaningful.is_empty() {
        return;
    }

    // Skip DDL statements: the target object may legitimately not exist yet.
    if let SqlTokenKind::Keyword(kw) = &meaningful[0].kind {
        if matches!(
            kw,
            SqlKeyword::Create | SqlKeyword::Alter | SqlKeyword::Drop | SqlKeyword::Truncate
        ) {
            return;
        }
    }

    let symbols = SymbolTable::build_from_tokens(stmt_tokens);

    // ---- Pass 1: collect table references (FROM/JOIN/UPDATE/INTO) ----
    let mut ref_tables: Vec<TableRef> = Vec::new();
    let mut table_indices: HashSet<usize> = HashSet::new();

    let mut i = 0;
    while i < meaningful.len() {
        let token = meaningful[i];
        let is_trigger = matches!(
            token.kind,
            SqlTokenKind::Keyword(SqlKeyword::From)
                | SqlTokenKind::Keyword(SqlKeyword::Join)
                | SqlTokenKind::Keyword(SqlKeyword::Update)
                | SqlTokenKind::Keyword(SqlKeyword::Into)
        );
        if !is_trigger {
            i += 1;
            continue;
        }

        let mut j = i + 1;
        // Parse a comma-separated table list following the trigger.
        loop {
            if j >= meaningful.len() {
                break;
            }
            // Subquery — the symbol table resolves its alias; skip the body.
            if matches!(meaningful[j].kind, SqlTokenKind::LParen) {
                let mut depth = 1usize;
                j += 1;
                while j < meaningful.len() && depth > 0 {
                    match meaningful[j].kind {
                        SqlTokenKind::LParen => depth += 1,
                        SqlTokenKind::RParen => depth -= 1,
                        _ => {}
                    }
                    j += 1;
                }
                break;
            }
            if !matches!(
                meaningful[j].kind,
                SqlTokenKind::Ident | SqlTokenKind::QuotedIdent
            ) {
                break;
            }

            let table_name = ident_text(meaningful[j]);
            table_indices.insert(j);
            let mut k = j + 1;

            // schema.table → the last part is the table; schema-qualified
            // references are registered but never existence-checked (the
            // table may live in a schema we did not load).
            if k < meaningful.len() && matches!(meaningful[k].kind, SqlTokenKind::Dot) {
                k += 1;
                if k < meaningful.len()
                    && matches!(
                        meaningful[k].kind,
                        SqlTokenKind::Ident | SqlTokenKind::QuotedIdent
                    )
                {
                    let real_table = ident_text(meaningful[k]);
                    table_indices.insert(k);
                    ref_tables.push(TableRef {
                        display: real_table.clone(),
                        token_index: k,
                        check_existence: false,
                        quoted: meaningful[k].kind == SqlTokenKind::QuotedIdent,
                    });
                    j = k + 1;
                } else {
                    // Malformed `schema.` — ignore.
                    j = k + 1;
                }
            } else {
                ref_tables.push(TableRef {
                    display: table_name.clone(),
                    token_index: j,
                    check_existence: true,
                    quoted: meaningful[j].kind == SqlTokenKind::QuotedIdent,
                });
                j += 1;
            }

            // Optional alias is part of the table reference, not a column.
            if j < meaningful.len() && meaningful[j].is_keyword_of(SqlKeyword::As) {
                j += 1;
            }
            if j < meaningful.len() && matches!(meaningful[j].kind, SqlTokenKind::Ident) {
                table_indices.insert(j);
                j += 1;
            }

            if j < meaningful.len() && matches!(meaningful[j].kind, SqlTokenKind::Comma) {
                j += 1;
                continue;
            }
            break;
        }
        i = j;
    }

    // Unknown-table reporting.
    for tr in ref_tables.iter().filter(|tr| tr.check_existence) {
        if let Some(resolved) = symbols.resolve(&tr.display) {
            if resolved == "#cte" || resolved == "#subquery" {
                continue;
            }
        }
        if !metadata.table_exists_with_quote(&tr.display, tr.quoted) {
            *next_id += 1;
            out.push(SqlDiagnostic::build(
                *next_id,
                document_revision,
                token_range(meaningful[tr.token_index]),
                SqlDiagnosticSeverity::Error,
                Some("semantic.unknown_table"),
                format!("Unknown table `{}`", tr.display),
                SqlDiagnosticSource::Semantic,
                Vec::new(),
            ));
        }
    }

    // Real (non-CTE / non-subquery) tables referenced by this statement.
    let known_tables: Vec<&TableRef> = ref_tables
        .iter()
        .filter(|tr| {
            !matches!(
                symbols.resolve(&tr.display),
                Some("#cte") | Some("#subquery")
            )
        })
        .collect();
    let single_table = known_tables.len() == 1;

    // ---- Pass 2: column references ----
    let mut i = 0;
    while i < meaningful.len() {
        let token = meaningful[i];
        if table_indices.contains(&i)
            || !matches!(token.kind, SqlTokenKind::Ident | SqlTokenKind::QuotedIdent)
        {
            i += 1;
            continue;
        }

        // Function name: `foo(`.
        if i + 1 < meaningful.len() && matches!(meaningful[i + 1].kind, SqlTokenKind::LParen) {
            i += 1;
            continue;
        }

        // Qualifier token (`x` in `x.y`) — validated when the column after the
        // dot is processed; do not treat it as a bare column here.
        if i + 1 < meaningful.len() && matches!(meaningful[i + 1].kind, SqlTokenKind::Dot) {
            i += 1;
            continue;
        }

        let name = ident_text(token);

        // Qualified reference: the previous token is a dot.
        if i > 0 && matches!(meaningful[i - 1].kind, SqlTokenKind::Dot) {
            // Three-part `schema.table.col` — not checkable here.
            if i >= 3 && matches!(meaningful[i - 3].kind, SqlTokenKind::Dot) {
                i += 1;
                continue;
            }
            let qualifier = meaningful[i - 2];
            if !matches!(
                qualifier.kind,
                SqlTokenKind::Ident | SqlTokenKind::QuotedIdent
            ) {
                i += 1;
                continue;
            }
            let qual_name = ident_text(qualifier);
            let table = match symbols.resolve(&qual_name) {
                Some(t) if t != "#cte" && t != "#subquery" => Some(t.to_string()),
                Some(_) => None, // CTE / subquery alias → skip the column check.
                None => {
                    if ref_tables
                        .iter()
                        .any(|tr| tr.display.eq_ignore_ascii_case(&qual_name))
                    {
                        // Bare table name used as qualifier (e.g. `users.id`).
                        Some(qual_name.clone())
                    } else if metadata.is_schema_or_db(&qual_name) {
                        None // Schema-qualified column reference → skip.
                    } else {
                        // Unknown alias.
                        *next_id += 1;
                        out.push(SqlDiagnostic::build(
                            *next_id,
                            document_revision,
                            token_range(qualifier),
                            SqlDiagnosticSeverity::Error,
                            Some("semantic.unknown_alias"),
                            format!("Unknown alias `{qual_name}`"),
                            SqlDiagnosticSource::Semantic,
                            Vec::new(),
                        ));
                        i += 1;
                        continue;
                    }
                }
            };
            if let Some(table) = table {
                if let Some(known) = metadata.column_status_with_quote(
                    &table,
                    &name,
                    token.kind == SqlTokenKind::QuotedIdent,
                ) {
                    if !known {
                        *next_id += 1;
                        out.push(SqlDiagnostic::build(
                            *next_id,
                            document_revision,
                            token_range(token),
                            SqlDiagnosticSeverity::Error,
                            Some("semantic.unknown_column"),
                            format!("Unknown column `{name}` in table `{table}`"),
                            SqlDiagnosticSource::Semantic,
                            Vec::new(),
                        ));
                    }
                }
            }
            i += 1;
            continue;
        }

        // Bare column reference — only when the statement references exactly
        // one known real table and that table's columns are loaded.
        if !single_table {
            i += 1;
            continue;
        }
        // Aliases / CTE names / subquery aliases are not columns.
        if symbols.is_alias(&name) {
            i += 1;
            continue;
        }
        let display = &known_tables[0].display;
        if let Some(known) = metadata.column_status_with_quote(
            display,
            &name,
            token.kind == SqlTokenKind::QuotedIdent,
        ) {
            if !known {
                *next_id += 1;
                out.push(SqlDiagnostic::build(
                    *next_id,
                    document_revision,
                    token_range(token),
                    SqlDiagnosticSeverity::Error,
                    Some("semantic.unknown_column"),
                    format!("Unknown column `{name}` in table `{display}`"),
                    SqlDiagnosticSource::Semantic,
                    Vec::new(),
                ));
            }
        }
        i += 1;
    }
}

fn normalize_ident(s: &str) -> String {
    s.to_uppercase()
}

fn count_char(s: &str, needle: char) -> usize {
    s.chars().filter(|c| *c == needle).count()
}

fn ident_text(token: &SqlToken) -> String {
    match token.kind {
        SqlTokenKind::QuotedIdent => {
            let s = &token.text;
            if let Some(body) = s.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                body.replace("\"\"", "\"")
            } else if let Some(body) = s.strip_prefix('`').and_then(|s| s.strip_suffix('`')) {
                body.replace("``", "`")
            } else if let Some(body) = s.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                body.replace("]]", "]")
            } else {
                s.clone()
            }
        }
        _ => token.text.clone(),
    }
}

fn token_range(token: &SqlToken) -> SqlTextRange {
    SqlTextRange {
        start_byte: token.start,
        end_byte: token.end,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn analyze(text: &str, metadata: SqlMetadataView) -> Vec<SqlDiagnostic> {
        let mut next_id = 0u64;
        let mut diagnostics =
            analyze_parser_diagnostics(text, SqlDialect::Standard, 1, &mut next_id);
        let snapshot = SqlStatementSnapshot::new(text, SqlDialect::Standard);
        diagnostics.extend(analyze_semantic_diagnostics(
            text,
            &snapshot,
            &metadata,
            1,
            &mut next_id,
        ));
        diagnostics
    }

    fn parser_codes(diagnostics: &[SqlDiagnostic]) -> Vec<&str> {
        diagnostics
            .iter()
            .filter(|d| d.source == SqlDiagnosticSource::Parser)
            .filter_map(|d| d.code.as_deref())
            .collect()
    }

    fn semantic_codes(diagnostics: &[SqlDiagnostic]) -> Vec<&str> {
        diagnostics
            .iter()
            .filter(|d| d.source == SqlDiagnosticSource::Semantic)
            .filter_map(|d| d.code.as_deref())
            .collect()
    }

    #[test]
    fn parser_unclosed_string() {
        let diagnostics = analyze("SELECT 'oops FROM t;", SqlMetadataView::default());
        let codes = parser_codes(&diagnostics);
        assert!(codes.contains(&"parser.unclosed_string"));
        assert!(!codes.contains(&"parser.unclosed_quoted_ident"));
    }

    #[test]
    fn parser_unclosed_quoted_ident() {
        let diagnostics = analyze("SELECT \"col FROM t;", SqlMetadataView::default());
        assert!(parser_codes(&diagnostics).contains(&"parser.unclosed_quoted_ident"));
    }

    #[test]
    fn parser_unclosed_block_comment() {
        let diagnostics = analyze("SELECT 1 /* comment", SqlMetadataView::default());
        assert!(parser_codes(&diagnostics).contains(&"parser.unclosed_block_comment"));
    }

    #[test]
    fn parser_escaped_quote_is_closed() {
        let diagnostics = analyze("SELECT 'it''s' AS label;", SqlMetadataView::default());
        assert!(parser_codes(&diagnostics).is_empty());
    }

    #[test]
    fn known_table_not_reported() {
        let metadata = SqlMetadataView::default()
            .with_tables(["users"])
            .with_columns("users", ["id", "name"]);
        let diagnostics = analyze("SELECT id, name FROM users WHERE id = 1;", metadata);
        assert!(semantic_codes(&diagnostics).is_empty());
    }

    #[test]
    fn empty_metadata_no_semantic_diagnostics() {
        let diagnostics = analyze(
            "SELECT missing_col FROM missing_table;",
            SqlMetadataView::default(),
        );
        // Spec §12.3: no metadata ⇒ avoid false positives.
        assert!(semantic_codes(&diagnostics).is_empty());
    }

    #[test]
    fn columns_not_loaded_no_unknown_column() {
        let metadata = SqlMetadataView::default().with_tables(["users"]);
        let diagnostics = analyze("SELECT id FROM users;", metadata);
        assert!(semantic_codes(&diagnostics).is_empty());
    }

    #[test]
    fn unknown_table_reported() {
        let metadata = SqlMetadataView::default().with_tables(["users"]);
        let diagnostics = analyze("SELECT * FROM orders;", metadata);
        let diag = diagnostics
            .iter()
            .find(|d| d.code.as_deref() == Some("semantic.unknown_table"))
            .expect("unknown table diagnostic");
        assert!(diag.message.contains("orders"));
        assert_eq!(diag.range.start_byte, "SELECT * FROM ".len());
        assert_eq!(diag.range.end_byte, "SELECT * FROM orders".len());
    }

    #[test]
    fn unknown_column_reported_case_insensitive() {
        let metadata = SqlMetadataView::default()
            .with_tables(["USERS"])
            .with_columns("USERS", ["ID", "NAME"]);
        let diagnostics = analyze("SELECT id, name, phone FROM users;", metadata);
        let codes = semantic_codes(&diagnostics);
        assert!(codes.contains(&"semantic.unknown_column"));
        // `id`/`name` match case-insensitively and must not be reported.
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.message.as_ref().contains("`id`"))
        );
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.message.as_ref().contains("`name`"))
        );
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.as_ref().contains("`phone`"))
        );
    }

    #[test]
    fn quoted_identifiers_use_case_sensitive_metadata_matching() {
        let metadata = SqlMetadataView::default()
            .with_tables(["Users"])
            .with_columns("Users", ["Id"]);

        let exact = analyze("SELECT \"Id\" FROM \"Users\";", metadata.clone());
        assert!(semantic_codes(&exact).is_empty());

        let wrong_case = analyze("SELECT \"id\" FROM \"Users\";", metadata);
        assert!(semantic_codes(&wrong_case).contains(&"semantic.unknown_column"));
    }

    #[test]
    fn unknown_alias_reported() {
        let metadata = SqlMetadataView::default()
            .with_tables(["users"])
            .with_columns("users", ["id"]);
        let diagnostics = analyze("SELECT o.id FROM users u;", metadata);
        let diag = diagnostics
            .iter()
            .find(|d| d.code.as_deref() == Some("semantic.unknown_alias"))
            .expect("unknown alias diagnostic");
        assert!(diag.message.contains("o"));
    }

    #[test]
    fn schema_qualified_table_not_reported() {
        let metadata = SqlMetadataView::default()
            .with_current_schema("public")
            .with_schemas(["public", "other"]);
        let diagnostics = analyze("SELECT * FROM public.users;", metadata);
        assert!(semantic_codes(&diagnostics).is_empty());
    }

    #[test]
    fn cte_not_reported_as_unknown_table() {
        let metadata = SqlMetadataView::default()
            .with_tables(["orders"])
            .with_columns("orders", ["id"]);
        let diagnostics = analyze(
            "WITH recent AS (SELECT * FROM orders) SELECT * FROM recent;",
            metadata,
        );
        assert!(!semantic_codes(&diagnostics).contains(&"semantic.unknown_table"));
    }

    #[test]
    fn multi_table_ambiguous_column_not_reported() {
        let metadata = SqlMetadataView::default()
            .with_tables(["users", "orders"])
            .with_columns("users", ["id", "name"])
            .with_columns("orders", ["id"]);
        let diagnostics = analyze("SELECT id FROM users, orders;", metadata);
        assert!(semantic_codes(&diagnostics).is_empty());
    }

    #[test]
    fn subquery_alias_not_reported_as_column() {
        let metadata = SqlMetadataView::default()
            .with_tables(["users"])
            .with_columns("users", ["id"]);
        let diagnostics = analyze("SELECT id FROM (SELECT id FROM users) sub;", metadata);
        assert!(semantic_codes(&diagnostics).is_empty());
    }

    #[test]
    fn byte_range_with_multibyte_chars() {
        let text = "SELECT 名称 FROM users;";
        let metadata = SqlMetadataView::default()
            .with_tables(["users"])
            .with_columns("users", ["id"]);
        let diagnostics = analyze(text, metadata);
        let diag = diagnostics
            .iter()
            .find(|d| d.code.as_deref() == Some("semantic.unknown_column"))
            .expect("unknown column diagnostic");
        let expected_start = "SELECT ".len();
        let expected_end = expected_start + "名称".len();
        assert_eq!(diag.range.start_byte, expected_start);
        assert_eq!(diag.range.end_byte, expected_end);
    }

    #[test]
    fn update_unknown_table_reported() {
        let metadata = SqlMetadataView::default().with_tables(["users"]);
        let diagnostics = analyze("UPDATE missing SET name = 'x';", metadata);
        assert!(semantic_codes(&diagnostics).contains(&"semantic.unknown_table"));
    }

    #[test]
    fn update_target_alias_is_not_reported_as_unknown_column() {
        let metadata = SqlMetadataView::default()
            .with_tables(["ai_tool_info"])
            .with_columns("ai_tool_info", ["status", "call_status"]);

        for sql in [
            "UPDATE ai_tool_info ati SET status = 1 WHERE ati.status = 0;",
            "UPDATE ai_tool_info AS ati SET status = 1 WHERE ati.status = 0;",
        ] {
            let diagnostics = analyze(sql, metadata.clone());
            assert!(
                semantic_codes(&diagnostics).is_empty(),
                "unexpected diagnostics for {sql}: {diagnostics:?}"
            );
        }
    }

    #[test]
    fn ddl_not_reported() {
        let metadata = SqlMetadataView::default()
            .with_tables(["users"])
            .with_columns("users", ["id"]);
        let diagnostics = analyze("CREATE TABLE brand_new (id INT, name TEXT);", metadata);
        assert!(semantic_codes(&diagnostics).is_empty());
    }
}
