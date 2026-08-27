//! SQL hover: locate the qualified identifier under the pointer and resolve
//! it against the metadata snapshot (tables/columns/functions).
//!
//! Resolution is fully synchronous against the local `SqlSchema` cache, so
//! tests are plain unit tests (no GPUI test context required). The provider
//! itself mirrors the long-lived default completion provider: schema refresh
//! replaces the inner source atomically without replacing the trait object.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use anyhow::Result;
use db::sql_editor::sql_tokenizer::{SqlTokenKind, SqlTokenizer};
use gpui::{App, AppContext, Task, Window};
use gpui_component::Rope;
use gpui_component::input::HoverProvider;
use lsp_types::{
    Hover as LspHover, HoverContents, MarkupContent, MarkupKind, Position as LspPosition,
    Range as LspRange,
};

use crate::sql_editor::{SqlColumnDetail, SqlObjectType, SqlSchema, SqlTableDetail};

/// One part of a qualified SQL identifier (e.g. the `users` in `db.users`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlIdentifierPart {
    /// Unquoted identifier value (`"My Table"` -> `My Table`).
    pub value: String,
    /// Whether the part was written with double quotes in the source.
    pub quoted: bool,
    /// Byte range of the part in the source text.
    pub range: Range<usize>,
}

/// A (possibly qualified) identifier located under the pointer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlQualifiedIdentifier {
    /// Parts from leftmost (catalog/database) to rightmost (object/column).
    pub parts: Vec<SqlIdentifierPart>,
    /// Byte range covering the whole dotted identifier.
    pub range: Range<usize>,
}

/// The semantic object a hover resolves to.
#[derive(Clone, Debug)]
pub enum SqlHoverObject {
    Table {
        name: String,
        detail: SqlTableDetail,
    },
    Column {
        table: String,
        column: SqlColumnDetail,
    },
    Function {
        signature: String,
        doc: String,
    },
}

/// Long-lived default hover provider. Schema refresh only swaps the inner
/// source; the provider trait object stays installed (spec §25.1).
#[derive(Clone)]
pub struct DefaultSqlHoverProvider {
    sources: Rc<RefCell<SqlHoverSources>>,
}

#[derive(Clone)]
pub(crate) struct SqlHoverSources {
    pub(crate) schema: Arc<SqlSchema>,
}

impl Default for SqlHoverSources {
    fn default() -> Self {
        Self {
            schema: Arc::new(SqlSchema::default()),
        }
    }
}

impl DefaultSqlHoverProvider {
    pub fn new(schema: SqlSchema) -> Self {
        Self {
            sources: Rc::new(RefCell::new(SqlHoverSources {
                schema: Arc::new(schema),
            })),
        }
    }

    /// Atomically replace the schema snapshot while keeping the provider alive.
    pub fn set_schema(&self, schema: SqlSchema) {
        self.sources.borrow_mut().schema = Arc::new(schema);
    }

    pub(crate) fn snapshot(&self) -> SqlHoverSources {
        self.sources.borrow().clone()
    }
}

impl HoverProvider for DefaultSqlHoverProvider {
    fn hover(
        &self,
        text: &Rope,
        offset: usize,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Option<LspHover>>> {
        let text = text.to_string();
        let schema = self.snapshot().schema;
        cx.background_spawn(async move { Ok(build_lsp_hover(&text, offset, &schema)) })
    }
}

/// Full hover pipeline: locate identifier -> resolve -> render markdown.
pub fn build_lsp_hover(text: &str, offset: usize, schema: &SqlSchema) -> Option<LspHover> {
    let ident = locate_identifier(text, offset)?;
    let object = resolve_hover(schema, &ident)?;
    let (markdown, _ddl_is_fallback) = build_hover(&object);
    let start = offset_to_lsp_position(text, ident.range.start);
    let end = offset_to_lsp_position(text, ident.range.end);
    Some(LspHover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: markdown,
        }),
        range: Some(LspRange::new(start, end)),
    })
}

/// Locate the maximal dotted identifier containing `offset`.
///
/// Rules (spec §11.1):
/// - offset may be inside a token or exactly at its end;
/// - unquoted keywords, strings, comments and whitespace never anchor an object;
/// - at most four parts (catalog.schema.table.column).
pub fn locate_identifier(text: &str, offset: usize) -> Option<SqlQualifiedIdentifier> {
    let mut tokenizer = SqlTokenizer::new(text);
    let tokens = tokenizer.tokenize();
    let offset = clip_utf8_offset_left(text, offset);

    let anchor = find_anchor(&tokens, offset)?;
    if !is_identifier_kind(&tokens[anchor].kind) {
        return None;
    }

    let mut parts = vec![part_from_token(&tokens[anchor])];

    // Extend left while the pattern is `Ident . Ident`.
    let mut left = anchor;
    loop {
        if parts.len() >= 4 {
            break;
        }
        let Some(dot) = prev_non_trivia(&tokens, left) else {
            break;
        };
        if tokens[dot].kind != SqlTokenKind::Dot {
            break;
        }
        let Some(part_idx) = prev_non_trivia(&tokens, dot) else {
            break;
        };
        if !is_identifier_kind(&tokens[part_idx].kind) {
            break;
        }
        parts.insert(0, part_from_token(&tokens[part_idx]));
        left = part_idx;
    }

    // Extend right while the pattern is `Ident . Ident`.
    let mut right = anchor;
    loop {
        if parts.len() >= 4 {
            break;
        }
        let Some(dot) = next_non_trivia(&tokens, right) else {
            break;
        };
        if tokens[dot].kind != SqlTokenKind::Dot {
            break;
        }
        let Some(part_idx) = next_non_trivia(&tokens, dot) else {
            break;
        };
        if !is_identifier_kind(&tokens[part_idx].kind) {
            break;
        }
        parts.push(part_from_token(&tokens[part_idx]));
        right = part_idx;
    }

    let range = parts.first()?.range.start..parts.last()?.range.end;
    Some(SqlQualifiedIdentifier { parts, range })
}

/// Pick the token that best covers `offset`.
///
/// Scoring rules keep a cursor right next to a boundary on the meaningful
/// token: strictly-inside or at-start of an identifier beats a `Dot` that
/// merely ends at the cursor, and a trailing identifier beats the whitespace
/// starting there. Trivia anchoring is still allowed so callers can reject
/// strings/comments/whitespace uniformly.
fn find_anchor(tokens: &[db::sql_editor::sql_tokenizer::SqlToken], offset: usize) -> Option<usize> {
    let mut best: Option<(usize, u8)> = None;
    for (i, token) in tokens.iter().enumerate() {
        if token.kind == SqlTokenKind::Eof {
            continue;
        }
        let non_trivia = !token.is_whitespace() && !token.is_comment();
        let strictly_inside = token.start < offset && offset < token.end;
        let at_start = token.start == offset;
        let at_end = token.end == offset && offset > 0;
        let score = if strictly_inside || at_start {
            if non_trivia { 4 } else { 2 }
        } else if at_end {
            if non_trivia { 3 } else { 1 }
        } else {
            continue;
        };
        if best.as_ref().is_none_or(|(_, s)| score > *s) {
            best = Some((i, score));
        }
    }
    best.map(|(i, _)| i)
}

fn is_identifier_kind(kind: &SqlTokenKind) -> bool {
    matches!(kind, SqlTokenKind::Ident | SqlTokenKind::QuotedIdent)
}

fn part_from_token(token: &db::sql_editor::sql_tokenizer::SqlToken) -> SqlIdentifierPart {
    let quoted = token.kind == SqlTokenKind::QuotedIdent;
    SqlIdentifierPart {
        value: unquote_ident(&token.text),
        quoted,
        range: token.start..token.end,
    }
}

/// Strip surrounding double quotes and unescape `""` inside a quoted identifier.
pub fn unquote_ident(raw: &str) -> String {
    let trimmed = raw.trim();
    let Some(body) = trimmed.strip_prefix('"').and_then(|s| s.strip_suffix('"')) else {
        return trimmed.to_string();
    };
    body.replace("\"\"", "\"")
}

fn prev_non_trivia(
    tokens: &[db::sql_editor::sql_tokenizer::SqlToken],
    from: usize,
) -> Option<usize> {
    let mut i = from.checked_sub(1)?;
    loop {
        if !tokens[i].is_whitespace() && !tokens[i].is_comment() {
            return Some(i);
        }
        if i == 0 {
            return None;
        }
        i -= 1;
    }
}

fn next_non_trivia(
    tokens: &[db::sql_editor::sql_tokenizer::SqlToken],
    from: usize,
) -> Option<usize> {
    let mut i = from + 1;
    while i < tokens.len() && tokens[i].kind != SqlTokenKind::Eof {
        if !tokens[i].is_whitespace() && !tokens[i].is_comment() {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Resolve a qualified identifier against the metadata snapshot.
///
/// The current database/schema scope is used to reject cross-database bare-name
/// references (spec §11.1, §23.4).
pub fn resolve_hover(schema: &SqlSchema, ident: &SqlQualifiedIdentifier) -> Option<SqlHoverObject> {
    let parts: Vec<&str> = ident.parts.iter().map(|p| p.value.as_str()).collect();
    match parts.as_slice() {
        [name] => {
            if let Some((name, detail)) = find_table_detail(schema, name) {
                return Some(SqlHoverObject::Table { name, detail });
            }
            if let Some((signature, doc)) = find_function(schema, name) {
                return Some(SqlHoverObject::Function { signature, doc });
            }
            None
        }
        [a, b] => {
            // schema.table / database.table (scope-validated)
            if looks_like_current_schema(schema, a) {
                if let Some((name, detail)) = find_table_detail(schema, b) {
                    return Some(SqlHoverObject::Table { name, detail });
                }
            }
            // table.column
            if let Some((table, detail)) = find_table_detail(schema, a)
                && let Some(column) = find_column(&detail, b)
            {
                return Some(SqlHoverObject::Column { table, column });
            }
            None
        }
        [a, b, c] => {
            // catalog.schema.table
            if looks_like_current_database(schema, a) && looks_like_current_schema(schema, b) {
                if let Some((name, detail)) = find_table_detail(schema, c) {
                    return Some(SqlHoverObject::Table { name, detail });
                }
            }
            // schema.table.column
            if looks_like_current_schema(schema, a) {
                if let Some((table, detail)) = find_table_detail(schema, b)
                    && let Some(column) = find_column(&detail, c)
                {
                    return Some(SqlHoverObject::Column { table, column });
                }
            }
            // database.table.column (covers schema-as-database dialects)
            if looks_like_current_database(schema, a) {
                if let Some((table, detail)) = find_table_detail(schema, b)
                    && let Some(column) = find_column(&detail, c)
                {
                    return Some(SqlHoverObject::Column { table, column });
                }
            }
            None
        }
        [a, b, c, d] => {
            // catalog.schema.table.column
            if looks_like_current_database(schema, a) && looks_like_current_schema(schema, b) {
                if let Some((table, detail)) = find_table_detail(schema, c)
                    && let Some(column) = find_column(&detail, d)
                {
                    return Some(SqlHoverObject::Column { table, column });
                }
            }
            None
        }
        _ => None,
    }
}

fn looks_like_current_schema(schema: &SqlSchema, name: &str) -> bool {
    match &schema.current_schema {
        Some(current) => current.eq_ignore_ascii_case(name),
        None => schema
            .current_database
            .as_deref()
            .is_some_and(|current| current.eq_ignore_ascii_case(name)),
    }
}

fn looks_like_current_database(schema: &SqlSchema, name: &str) -> bool {
    schema
        .current_database
        .as_deref()
        .is_some_and(|current| current.eq_ignore_ascii_case(name))
}

fn find_table_detail(schema: &SqlSchema, name: &str) -> Option<(String, SqlTableDetail)> {
    schema
        .table_details
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(key, detail)| (key.clone(), detail.clone()))
}

fn find_column(detail: &SqlTableDetail, name: &str) -> Option<SqlColumnDetail> {
    detail
        .columns
        .iter()
        .find(|col| col.name.eq_ignore_ascii_case(name))
        .cloned()
}

fn find_function(schema: &SqlSchema, name: &str) -> Option<(String, String)> {
    schema.functions.iter().find_map(|(signature, doc)| {
        let base = signature.split('(').next().unwrap_or(signature).trim();
        base.eq_ignore_ascii_case(name)
            .then(|| (signature.clone(), doc.clone()))
    })
}

/// Render markdown for a resolved object. Returns `(markdown, ddl_is_fallback)`.
pub fn build_hover(object: &SqlHoverObject) -> (String, bool) {
    match object {
        SqlHoverObject::Table { name, detail } => (build_table_hover(name, detail), true),
        SqlHoverObject::Column { table, column } => (build_column_hover(table, column), false),
        SqlHoverObject::Function { signature, doc } => {
            (build_function_hover(signature, doc), false)
        }
    }
}

fn build_table_hover(name: &str, detail: &SqlTableDetail) -> String {
    let mut md = String::new();
    md.push_str(&format!(
        "**{}** `{}`\n\n",
        detail.object_type.as_str(),
        name
    ));
    if let Some(schema) = &detail.schema {
        md.push_str(&format!("Schema: `{}`\n\n", schema));
    }
    if let Some(comment) = &detail.comment
        && !comment.is_empty()
    {
        md.push_str(comment.trim());
        md.push_str("\n\n");
    }
    md.push_str("| Column | Type | Nullable | Default | Key | Comment |\n");
    md.push_str("| --- | --- | --- | --- | --- | --- |\n");
    for col in &detail.columns {
        let nullable = if col.is_nullable { "NULL" } else { "NOT NULL" };
        let key = if col.is_primary_key { "PK" } else { "" };
        let default = col.default_value.as_deref().unwrap_or("");
        let comment = col.comment.as_deref().unwrap_or("");
        md.push_str(&format!(
            "| `{}` | {} | {} | {} | {} | {} |\n",
            escape_md(&col.name),
            escape_md(&col.data_type),
            nullable,
            escape_md(default),
            key,
            escape_md(comment)
        ));
    }
    md.push_str("\n---\n\n");
    md.push_str("**Generated DDL preview**（根据元数据生成的预览，不保证可执行）\n\n");
    md.push_str("```sql\n");
    md.push_str(&generate_ddl_preview(name, detail));
    md.push_str("```\n");
    md
}

fn build_column_hover(table: &str, column: &SqlColumnDetail) -> String {
    let mut md = format!("**COLUMN** `{}`.`{}`\n\n", table, column.name);
    md.push_str(&format!("Type: `{}`\n\n", column.data_type));
    md.push_str(if column.is_nullable {
        "Nullable: YES\n\n"
    } else {
        "Nullable: NO\n\n"
    });
    if column.is_primary_key {
        md.push_str("Primary Key: YES\n\n");
    }
    if let Some(default) = &column.default_value {
        md.push_str(&format!("Default: `{}`\n\n", escape_md(default)));
    }
    if let Some(comment) = &column.comment
        && !comment.is_empty()
    {
        md.push_str(comment.trim());
        md.push('\n');
    }
    md
}

fn build_function_hover(signature: &str, doc: &str) -> String {
    let mut md = format!("**FUNCTION** `{}`\n\n", signature);
    if !doc.is_empty() {
        md.push_str(doc.trim());
        md.push('\n');
    }
    md
}

/// Best-effort `CREATE TABLE` from metadata. Explicitly non-authoritative
/// (spec §11.5.5/6); callers must not route it into "copy and execute".
fn generate_ddl_preview(name: &str, detail: &SqlTableDetail) -> String {
    let mut ddl = String::new();
    let keyword = match detail.object_type {
        SqlObjectType::Table => "TABLE",
        SqlObjectType::View => "VIEW",
    };
    ddl.push_str(&format!("CREATE {} {} (\n", keyword, quote_ddl_ident(name)));
    for (i, col) in detail.columns.iter().enumerate() {
        let mut line = format!("  {} {}", quote_ddl_ident(&col.name), col.data_type);
        if !col.is_nullable {
            line.push_str(" NOT NULL");
        }
        if let Some(default) = &col.default_value {
            line.push_str(&format!(" DEFAULT {}", default));
        }
        let comma = if i + 1 < detail.columns.len() {
            ","
        } else {
            ""
        };
        ddl.push_str(&line);
        ddl.push_str(comma);
        ddl.push('\n');
    }
    ddl.push_str(");\n");
    ddl
}

fn quote_ddl_ident(name: &str) -> String {
    if name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        name.to_string()
    } else {
        format!("\"{}\"", name.replace('"', "\"\""))
    }
}

fn escape_md(value: &str) -> String {
    value.replace('|', "\\|")
}

/// Convert a byte offset to an LSP position (line, character-in-chars).
fn offset_to_lsp_position(text: &str, offset: usize) -> LspPosition {
    let offset = clip_utf8_offset_left(text, offset);
    let before = &text[..offset];
    let line = before.matches('\n').count();
    let line_start = before.rfind('\n').map(|p| p + 1).unwrap_or(0);
    let character = before[line_start..].chars().count();
    LspPosition::new(line as u32, character as u32)
}

fn clip_utf8_offset_left(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sql_editor::{SqlColumnDetail, SqlObjectType, SqlSchema, SqlTableDetail};

    fn sample_schema() -> SqlSchema {
        let columns = vec![
            SqlColumnDetail {
                name: "id".into(),
                data_type: "INT".into(),
                is_nullable: false,
                is_primary_key: true,
                default_value: None,
                comment: Some("primary key".into()),
            },
            SqlColumnDetail {
                name: "name".into(),
                data_type: "VARCHAR(255)".into(),
                is_nullable: true,
                is_primary_key: false,
                default_value: Some("'anon'".into()),
                comment: None,
            },
        ];
        let users = SqlTableDetail {
            object_type: SqlObjectType::Table,
            schema: Some("public".into()),
            comment: Some("user accounts".into()),
            engine: Some("InnoDB".into()),
            columns: columns.clone(),
        };
        let orders = SqlTableDetail {
            object_type: SqlObjectType::Table,
            schema: Some("public".into()),
            comment: None,
            engine: None,
            columns: vec![SqlColumnDetail {
                name: "total".into(),
                data_type: "DECIMAL(10,2)".into(),
                is_nullable: false,
                is_primary_key: false,
                default_value: None,
                comment: None,
            }],
        };
        SqlSchema::default()
            .with_scope(Some("app".into()), Some("public".into()))
            .with_tables(vec![("users".to_string(), "doc".to_string())])
            .with_table_detail("users", users)
            .with_table_detail("orders", orders)
            .with_functions(vec![(
                "count_orders(from_ts DATE, to_ts DATE)".to_string(),
                "counts orders".to_string(),
            )])
    }

    fn hover(text: &str, offset: usize, schema: &SqlSchema) -> Option<LspHover> {
        build_lsp_hover(text, offset, schema)
    }

    fn markup(hover: &LspHover) -> String {
        match &hover.contents {
            HoverContents::Markup(markup) => markup.value.clone(),
            other => format!("{other:?}"),
        }
    }

    #[test]
    fn locates_single_table() {
        let schema = sample_schema();
        let h = hover("select * from users", 17, &schema).unwrap();
        let contents = markup(&h);
        assert!(contents.contains("**TABLE**"));
        assert!(contents.contains("user accounts"));
    }

    #[test]
    fn locates_schema_qualified_table() {
        let schema = sample_schema();
        let h = hover("select * from public.users", 26, &schema).unwrap();
        let contents = markup(&h);
        assert!(contents.contains("**TABLE**"));
    }

    #[test]
    fn locates_catalog_schema_table() {
        let schema = sample_schema();
        let h = hover("select * from app.public.users", 30, &schema).unwrap();
        let contents = markup(&h);
        assert!(contents.contains("**TABLE**"));
    }

    #[test]
    fn locates_table_column() {
        let schema = sample_schema();
        let h = hover("select users.id", 13, &schema).unwrap();
        let contents = markup(&h);
        assert!(contents.contains("**COLUMN**"));
        assert!(contents.contains("users"));
    }

    #[test]
    fn locates_schema_table_column() {
        let schema = sample_schema();
        let h = hover("select public.users.id", 20, &schema).unwrap();
        let contents = markup(&h);
        assert!(contents.contains("**COLUMN**"));
    }

    #[test]
    fn locates_catalog_schema_table_column() {
        let schema = sample_schema();
        let h = hover("select app.public.users.id", 25, &schema).unwrap();
        let contents = markup(&h);
        assert!(contents.contains("**COLUMN**"));
    }

    #[test]
    fn rejects_cross_database_bare_name() {
        let schema = sample_schema();
        assert!(hover("select * from other_db.users", 27, &schema).is_none());
    }

    #[test]
    fn resolves_function() {
        let schema = sample_schema();
        let h = hover("select count_orders('a')", 8, &schema).unwrap();
        let contents = markup(&h);
        assert!(contents.contains("**FUNCTION**"));
        assert!(contents.contains("counts orders"));
    }

    #[test]
    fn rejects_keywords_strings_and_comments() {
        let schema = sample_schema();
        // offset on SELECT keyword
        assert!(hover("select users.id", 1, &schema).is_none());
        // offset inside string literal
        assert!(hover("select 'users'", 10, &schema).is_none());
        // offset inside line comment
        assert!(hover("select 1 -- users", 13, &schema).is_none());
    }

    #[test]
    fn quoted_identifier_with_spaces() {
        let schema = SqlSchema::default()
            .with_scope(Some("app".into()), Some("public".into()))
            .with_table_detail(
                "My Table",
                SqlTableDetail {
                    object_type: SqlObjectType::Table,
                    schema: Some("public".into()),
                    comment: Some("quoted table".into()),
                    engine: None,
                    columns: vec![SqlColumnDetail {
                        name: "weird col".into(),
                        data_type: "TEXT".into(),
                        is_nullable: true,
                        is_primary_key: false,
                        default_value: None,
                        comment: None,
                    }],
                },
            );
        let h = hover("select * from \"My Table\"", 21, &schema).unwrap();
        let contents = markup(&h);
        assert!(contents.contains("quoted table"));
        // column hover with quoted identifier part
        let h = hover("select \"My Table\".\"weird col\"", 25, &schema).unwrap();
        let contents = markup(&h);
        assert!(contents.contains("**COLUMN**"));
        assert!(contents.contains("weird col"));
    }

    #[test]
    fn case_insensitive_lookup() {
        let schema = sample_schema();
        let h = hover("select * from USERS", 18, &schema).unwrap();
        assert!(markup(&h).contains("**TABLE**"));
    }

    #[test]
    fn oracle_schema_as_database_semantics() {
        let schema = SqlSchema::default()
            .with_scope(None, Some("hr".into()))
            .with_table_detail(
                "employees",
                SqlTableDetail {
                    object_type: SqlObjectType::Table,
                    schema: Some("hr".into()),
                    comment: None,
                    engine: None,
                    columns: vec![SqlColumnDetail {
                        name: "salary".into(),
                        data_type: "NUMBER(8,2)".into(),
                        is_nullable: false,
                        is_primary_key: false,
                        default_value: None,
                        comment: None,
                    }],
                },
            );
        // schema-as-database: bare `hr.employees` resolves
        let h = hover("select * from hr.employees", 24, &schema).unwrap();
        assert!(markup(&h).contains("**TABLE**"));
        // and `hr.employees.salary`
        let h = hover("select hr.employees.salary", 26, &schema).unwrap();
        assert!(markup(&h).contains("**COLUMN**"));
    }

    #[test]
    fn ddl_preview_is_marked_as_fallback() {
        let schema = sample_schema();
        let h = hover("select * from users", 17, &schema).unwrap();
        let contents = markup(&h);
        assert!(contents.contains("Generated DDL preview"));
        assert!(contents.contains("不保证可执行"));
        assert!(contents.contains("CREATE TABLE"));
        assert!(contents.contains("INT"));
    }

    #[test]
    fn offset_boundary_after_token_resolves() {
        let schema = sample_schema();
        // offset == end of "users"
        let text = "select * from users";
        let offset = text.find("users").unwrap() + "users".len();
        assert!(hover(text, offset, &schema).is_some());
    }

    #[test]
    fn unicode_before_identifier_uses_byte_offset() {
        let schema = sample_schema();
        // 中文 + emoji before the identifier; offsets are bytes.
        let text = "SELECT * FROM 中文🎉 users";
        let byte_offset = text.find("users").unwrap() + 2; // inside "users"
        let h = hover(text, byte_offset, &schema).unwrap();
        assert!(markup(&h).contains("**TABLE**"));
    }

    #[test]
    fn lsp_range_uses_character_columns() {
        let schema = sample_schema();
        let text = "SELECT * FROM 中文 users";
        let offset = text.find("users").unwrap() + 2;
        let h = hover(text, offset, &schema).unwrap();
        let range = h.range.unwrap();
        // column is measured in characters, not bytes
        assert_eq!(range.start.character, 17);
        assert_eq!(range.end.character, 22);
    }

    #[test]
    fn clips_hover_offsets_to_utf8_boundaries() {
        let text = "中文";
        assert_eq!(offset_to_lsp_position(text, 1), LspPosition::new(0, 0));
        assert_eq!(locate_identifier(text, 1), locate_identifier(text, 0));
    }

    #[test]
    fn pointer_on_middle_part_resolves_whole_identifier() {
        let schema = sample_schema();
        // cursor on `public` in `app.public.users`
        let h = hover("select * from app.public.users", 21, &schema).unwrap();
        assert!(markup(&h).contains("**TABLE**"));
        let range = h.range.unwrap();
        assert_eq!(range.start.character, 14);
        assert_eq!(range.end.character, 30);
    }
}
