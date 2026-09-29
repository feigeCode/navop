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
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::Result;
use db::sql_editor::sql_tokenizer::{SqlTokenKind, SqlTokenizer};
use gpui::{App, AsyncApp, Task, Window};
use gpui_component::Rope;
use gpui_component::input::HoverProvider;
use lsp_types::{
    Hover as LspHover, HoverContents, MarkupContent, MarkupKind, Position as LspPosition,
    Range as LspRange,
};
use rust_i18n::t;

use crate::sql_editor::{
    ForeignSchema, SqlColumnDetail, SqlObjectType, SqlSchema, SqlTableDetail, find_foreign_schema,
};
use crate::table_ddl::{SqlTableRef, TableDdlSources, load_ddl_section, with_ddl_section};

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
        /// 表的完整坐标（当前 scope 或限定名指向的外库/外 schema）。
        table: SqlTableRef,
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
    /// Latest hover request offset, used to drop stale debounced requests.
    /// `Arc<AtomicUsize>` so the debounce future stays `Send` for
    /// `background_spawn`.
    latest_offset: Arc<AtomicUsize>,
}

#[derive(Clone)]
pub(crate) struct SqlHoverSources {
    pub(crate) schema: Arc<SqlSchema>,
    /// 建表 DDL 的来源（连接上下文 + 加载器）。编辑器注入；嵌在单元格里的
    /// 编辑器没有，那种场景下 hover/详情不展示 DDL 区段。
    pub(crate) table_ddl: Option<TableDdlSources>,
}

impl Default for SqlHoverSources {
    fn default() -> Self {
        Self {
            schema: Arc::new(SqlSchema::default()),
            table_ddl: None,
        }
    }
}

impl DefaultSqlHoverProvider {
    pub fn new(schema: SqlSchema) -> Self {
        Self {
            sources: Rc::new(RefCell::new(SqlHoverSources {
                schema: Arc::new(schema),
                table_ddl: None,
            })),
            latest_offset: Arc::new(AtomicUsize::new(usize::MAX)),
        }
    }

    /// Atomically replace the schema snapshot while keeping the provider alive.
    pub fn set_schema(&self, schema: SqlSchema) {
        self.sources.borrow_mut().schema = Arc::new(schema);
    }

    /// 注入（或清空）建表 DDL 的来源。
    pub fn set_table_ddl(&self, table_ddl: Option<TableDdlSources>) {
        self.sources.borrow_mut().table_ddl = table_ddl;
    }

    /// 当前建表 DDL 来源。
    pub fn table_ddl(&self) -> Option<TableDdlSources> {
        self.sources.borrow().table_ddl.clone()
    }

    pub(crate) fn snapshot(&self) -> SqlHoverSources {
        self.sources.borrow().clone()
    }

    /// Shared schema snapshot, so other providers (Cmd/Ctrl+click definition)
    /// see exactly the same metadata generation.
    pub(crate) fn sources_handle(&self) -> Rc<RefCell<SqlHoverSources>> {
        self.sources.clone()
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
        let sources = self.snapshot();
        let latest_offset = self.latest_offset.clone();
        latest_offset.store(offset, Ordering::SeqCst);
        // Dwell debounce: the caller (gpui-kit) already waits ~150ms before
        // the first popover, but that still pops while the pointer sweeps
        // across identifiers. Holding here means the pointer must rest on the
        // same offset for the full window before a hover resolves; requests
        // superseded by a newer offset resolve to None and the popover never
        // appears for the stale one.
        const HOVER_DWELL_MS: u64 = 600;
        // 前景（非 Send）任务：DDL 要经 [`TableDdlSources`] 的加载器取，它持有
        // `Rc` 句柄。
        cx.spawn(async move |cx: &mut AsyncApp| {
            smol::Timer::after(std::time::Duration::from_millis(HOVER_DWELL_MS)).await;
            if latest_offset.load(Ordering::SeqCst) != offset {
                return Ok(None);
            }
            build_lsp_hover_async(&text, offset, &sources, cx).await
        })
    }
}

/// The semantic kind of a resolved object, used to pick the details tab icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SqlObjectDetailsKind {
    Table,
    Column,
    Function,
}

/// A resolved object: its details markdown plus the identity of the source
/// identifier it was resolved from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlObjectDetails {
    /// Markdown body（不含 DDL 区段）shared with the hover popover and the
    /// details tab. DDL 要按连接上下文异步取，见 [`SqlObjectDetails::table`]。
    pub markdown: String,
    /// Byte range of the whole (possibly qualified) identifier.
    pub range: Range<usize>,
    /// Stable slug for the object inside its database/schema scope, so the same
    /// object always maps to the same details tab.
    pub id: String,
    /// Object label shown on the details tab (`users`, `users.id`, ...).
    pub label: String,
    /// Kind of the resolved object.
    pub kind: SqlObjectDetailsKind,
    /// 解析到的表坐标：只有表对象（不含视图、列、函数）有值。调用方拿到连接
    /// 上下文后用它取驱动生成的建表 DDL。
    pub table: Option<SqlTableRef>,
}

/// Full hover pipeline: locate identifier -> resolve -> render markdown.
///
/// 主体 markdown（不含 DDL 区段）：DDL 需要异步取驱动结果，见
/// [`build_lsp_hover_async`]。这里只服务单测（生产 hover 一律走异步版）。
#[cfg(test)]
pub fn build_lsp_hover(text: &str, offset: usize, schema: &SqlSchema) -> Option<LspHover> {
    let details = resolve_object_details(text, offset, schema)?;
    Some(lsp_hover_for(text, &details, details.markdown.clone()))
}

/// 带建表 DDL 的 hover：表对象等驱动生成 DDL 后一并渲染。
pub async fn build_lsp_hover_async(
    text: &str,
    offset: usize,
    sources: &SqlHoverSources,
    cx: &mut AsyncApp,
) -> Result<Option<LspHover>> {
    let Some(details) = resolve_object_details(text, offset, &sources.schema) else {
        return Ok(None);
    };
    let section = load_ddl_section(details.table.as_ref(), sources.table_ddl.as_ref(), cx).await;
    let markdown = with_ddl_section(&details.markdown, section.as_ref());
    Ok(Some(lsp_hover_for(text, &details, markdown)))
}

fn lsp_hover_for(text: &str, details: &SqlObjectDetails, markdown: String) -> LspHover {
    let start = offset_to_lsp_position(text, details.range.start);
    let end = offset_to_lsp_position(text, details.range.end);
    LspHover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: markdown,
        }),
        range: Some(LspRange::new(start, end)),
    }
}

/// Render the object under `offset` (or nothing when it is not a known
/// table/column/function). Shared by the hover popover and the
/// Cmd/Ctrl+click definition provider, so both must agree on what counts as
/// a hit.
pub fn resolve_object_details(
    text: &str,
    offset: usize,
    schema: &SqlSchema,
) -> Option<SqlObjectDetails> {
    let ident = locate_identifier(text, offset)?;
    let object = resolve_hover(schema, &ident)?;
    Some(SqlObjectDetails {
        markdown: build_hover(&object),
        range: ident.range,
        id: object_details_id(schema, &object),
        label: object_label(&object),
        kind: object_kind(&object),
        table: ddl_table_of(&object),
    })
}

/// 能生成建表 DDL 的对象坐标：只有表（视图没有建表 DDL，列/函数不是表）。
fn ddl_table_of(object: &SqlHoverObject) -> Option<SqlTableRef> {
    match object {
        SqlHoverObject::Table { table, detail } if detail.object_type == SqlObjectType::Table => {
            Some(table.clone())
        }
        _ => None,
    }
}

/// Stable slug for `object` inside its database/schema scope.
fn object_details_id(schema: &SqlSchema, object: &SqlHoverObject) -> String {
    let scope = schema
        .current_database
        .iter()
        .chain(schema.current_schema.iter())
        .cloned()
        .collect::<Vec<_>>()
        .join(".");
    let (kind, name) = match object {
        SqlHoverObject::Table { table, .. } => ("table", table.name.clone()),
        SqlHoverObject::Column { table, column } => ("column", format!("{table}.{}", column.name)),
        SqlHoverObject::Function { signature, .. } => ("function", signature.clone()),
    };
    if scope.is_empty() {
        format!("{kind}:{name}")
    } else {
        format!("{scope}:{kind}:{name}")
    }
}

/// Label shown on the details tab: the object itself, not its scope.
fn object_label(object: &SqlHoverObject) -> String {
    match object {
        SqlHoverObject::Table { table, .. } => table.name.clone(),
        SqlHoverObject::Column { table, column } => format!("{table}.{}", column.name),
        SqlHoverObject::Function { signature, .. } => signature.clone(),
    }
}

fn object_kind(object: &SqlHoverObject) -> SqlObjectDetailsKind {
    match object {
        SqlHoverObject::Table { .. } => SqlObjectDetailsKind::Table,
        SqlHoverObject::Column { .. } => SqlObjectDetailsKind::Column,
        SqlHoverObject::Function { .. } => SqlObjectDetailsKind::Function,
    }
}

/// Offsets to probe for a selection-aware resolution: the selection body first
/// (start, mid, end), then the bare cursor.
fn selection_probe_offsets(selection: Option<Range<usize>>, cursor: usize) -> Vec<usize> {
    match selection {
        Some(sel) if !sel.is_empty() => {
            let mid = sel.start + (sel.end - sel.start) / 2;
            vec![sel.start, mid, sel.end - 1, sel.end, cursor]
        }
        _ => vec![cursor],
    }
}

/// Selection-aware variant of [`resolve_object_details`], used by the context
/// menu.
pub fn resolve_object_details_for_selection(
    text: &str,
    selection: Option<Range<usize>>,
    cursor: usize,
    schema: &SqlSchema,
) -> Option<SqlObjectDetails> {
    selection_probe_offsets(selection, cursor)
        .into_iter()
        .find_map(|offset| resolve_object_details(text, offset, schema))
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
    if let Some(body) = trimmed.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        body.replace("\"\"", "\"")
    } else if let Some(body) = trimmed.strip_prefix('`').and_then(|s| s.strip_suffix('`')) {
        body.replace("``", "`")
    } else if let Some(body) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        body.replace("]]", "]")
    } else {
        trimmed.to_string()
    }
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

/// 当前 scope 里的表坐标：database/schema 取自快照的当前 scope。
fn current_table_ref(schema: &SqlSchema, name: String) -> SqlTableRef {
    SqlTableRef {
        name,
        database: schema.current_database.clone().unwrap_or_default(),
        schema: schema.current_schema.clone(),
    }
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
                return Some(SqlHoverObject::Table {
                    table: current_table_ref(schema, name),
                    detail,
                });
            }
            if let Some((signature, doc)) = find_function(schema, name) {
                return Some(SqlHoverObject::Function { signature, doc });
            }
            None
        }
        [a, b] => {
            // schema.table / database.table (scope-validated)
            if looks_like_current_schema(schema, a)
                && let Some((name, detail)) = find_table_detail(schema, b)
            {
                return Some(SqlHoverObject::Table {
                    table: current_table_ref(schema, name),
                    detail,
                });
            }
            // 其他 database/schema 的表：qualifier.table
            if let Some((table, detail)) = find_foreign_table_detail(schema, a, b) {
                return Some(SqlHoverObject::Table { table, detail });
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
            if looks_like_current_database(schema, a)
                && looks_like_current_schema(schema, b)
                && let Some((name, detail)) = find_table_detail(schema, c)
            {
                return Some(SqlHoverObject::Table {
                    table: current_table_ref(schema, name),
                    detail,
                });
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
            // 其他 database/schema 的列：qualifier.table.column
            if let Some((SqlTableRef { name: table, .. }, detail)) =
                find_foreign_table_detail(schema, a, b)
                && let Some(column) = find_column(&detail, c)
            {
                return Some(SqlHoverObject::Column { table, column });
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

/// 在外部 qualifier（其他 database/schema）缓存中查表坐标与详情。
fn find_foreign_table_detail(
    schema: &SqlSchema,
    qualifier: &str,
    name: &str,
) -> Option<(SqlTableRef, SqlTableDetail)> {
    let foreign: &ForeignSchema = find_foreign_schema(schema, qualifier)?;
    foreign
        .table_details
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(key, detail)| {
            (
                SqlTableRef {
                    name: key.clone(),
                    database: foreign.scope.database.clone(),
                    schema: foreign.scope.schema.clone(),
                },
                detail.clone(),
            )
        })
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

/// Render the body markdown for a resolved object.
///
/// 建表 DDL 不在里面：它由驱动按方言生成，需要连接上下文与异步查询，
/// 由 [`crate::table_ddl`] 单独拼成区段。
pub fn build_hover(object: &SqlHoverObject) -> String {
    match object {
        SqlHoverObject::Table { table, detail } => build_table_hover(&table.name, detail),
        SqlHoverObject::Column { table, column } => build_column_hover(table, column),
        SqlHoverObject::Function { signature, doc } => build_function_hover(signature, doc),
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
    md.push_str("\n");
    // 视图没有建表 DDL，而列信息仍来自元数据，这里明说一下免得用户等一个不会来的区段。
    if detail.object_type == SqlObjectType::View {
        md.push_str(t!("Query.object_details_view_ddl_unavailable").as_ref());
        md.push('\n');
    }
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

fn escape_md(value: &str) -> String {
    value.replace('|', "\\|")
}

/// Convert a byte offset to an LSP position (line, character-in-chars).
pub(crate) fn offset_to_lsp_position(text: &str, offset: usize) -> LspPosition {
    let offset = clip_utf8_offset_left(text, offset);
    let before = &text[..offset];
    let line = before.matches('\n').count();
    let line_start = before.rfind('\n').map(|p| p + 1).unwrap_or(0);
    let character = before[line_start..].encode_utf16().count();
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
    use crate::sql_editor::{
        ForeignSchemaScope, SqlColumnDetail, SqlObjectType, SqlSchema, SqlTableDetail,
    };

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

    /// 建表 DDL 不在 hover markdown 里（它由驱动异步生成），但表对象要带着
    /// 驱动查询需要的坐标。
    #[test]
    fn table_details_carry_the_ddl_coordinates() {
        let schema = sample_schema();
        let details = resolve_object_details("select * from users", 17, &schema).unwrap();

        assert_eq!(
            Some(SqlTableRef {
                name: "users".into(),
                database: "app".into(),
                schema: Some("public".into()),
            }),
            details.table
        );
        assert!(!details.markdown.contains("CREATE TABLE"));
        assert!(!details.markdown.contains("DDL"));
    }

    #[test]
    fn only_tables_carry_ddl_coordinates() {
        let schema = sample_schema();
        // 列对象
        let column = resolve_object_details("select users.id", 13, &schema).unwrap();
        assert_eq!(SqlObjectDetailsKind::Column, column.kind);
        assert!(column.table.is_none());
        // 函数对象
        let function = resolve_object_details("select count_orders('a')", 8, &schema).unwrap();
        assert_eq!(SqlObjectDetailsKind::Function, function.kind);
        assert!(function.table.is_none());
    }

    #[test]
    fn view_details_have_no_ddl_coordinates_and_say_so() {
        let schema = SqlSchema::default()
            .with_scope(Some("app".into()), Some("public".into()))
            .with_table_detail(
                "active_users",
                SqlTableDetail {
                    object_type: SqlObjectType::View,
                    schema: Some("public".into()),
                    comment: None,
                    engine: None,
                    columns: vec![SqlColumnDetail {
                        name: "id".into(),
                        data_type: "INT".into(),
                        is_nullable: true,
                        is_primary_key: false,
                        default_value: None,
                        comment: None,
                    }],
                },
            );
        let details = resolve_object_details("select * from active_users", 17, &schema).unwrap();

        assert_eq!(SqlObjectDetailsKind::Table, details.kind);
        assert!(details.table.is_none());
        assert!(
            !details.markdown.contains("```sql"),
            "视图不该摆建表 DDL 代码块"
        );
        assert!(
            details
                .markdown
                .contains(&t!("Query.object_details_view_ddl_unavailable").to_string())
        );
    }

    /// 跨库限定名：DDL 坐标取自拉取该 qualifier 时记录的 scope，而不是当前 scope。
    #[test]
    fn foreign_table_details_use_the_foreign_scope() {
        let schema = sample_schema().with_foreign_schema(ForeignSchema {
            name: "shop".into(),
            scope: ForeignSchemaScope {
                database: "shop".into(),
                schema: None,
            },
            tables: vec![("orders".into(), String::new())],
            columns_by_table: std::collections::HashMap::new(),
            table_details: std::collections::HashMap::from([(
                "orders".to_string(),
                SqlTableDetail {
                    object_type: SqlObjectType::Table,
                    schema: None,
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
                },
            )]),
        });

        let details = resolve_object_details("select * from shop.orders", 21, &schema).unwrap();

        assert_eq!(
            Some(SqlTableRef {
                name: "orders".into(),
                database: "shop".into(),
                schema: None,
            }),
            details.table
        );
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
    fn lsp_range_uses_utf16_columns_after_emoji() {
        let schema = sample_schema();
        let text = "select '🙂', users";
        let hover = hover(text, text.len(), &schema).expect("users hover");
        let range = hover.range.expect("hover range");

        assert_eq!(13, range.start.character);
        assert_eq!(18, range.end.character);
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

    #[test]
    fn object_details_id_is_scoped_and_kind_qualified() {
        let schema = sample_schema();
        let details = |sql: &str, needle: &str| {
            let offset = sql.find(needle).expect("identifier") + 1;
            resolve_object_details(sql, offset, &schema).expect("details")
        };

        let table = details("select * from users", "users");
        assert_eq!("app.public:table:users", table.id);
        assert_eq!("users", table.label);
        assert_eq!(SqlObjectDetailsKind::Table, table.kind);

        let column = details("select * from users where users.id = 1", "id");
        assert_eq!("app.public:column:users.id", column.id);
        assert_eq!("users.id", column.label);
        assert_eq!(SqlObjectDetailsKind::Column, column.kind);

        // Same table name in another scope must not share a tab.
        let other_scope = sample_schema().with_scope(Some("other".into()), Some("public".into()));
        let elsewhere = resolve_object_details(
            "select * from users",
            "select * from users".find("users").expect("users") + 1,
            &other_scope,
        )
        .expect("details");
        assert_eq!("other.public:table:users", elsewhere.id);
    }

    #[test]
    fn selection_details_resolve_when_the_cursor_sits_at_an_edge() {
        let schema = sample_schema();
        let text = "select * from users";
        let start = text.find("users").expect("users");
        // Right-clicking a selection leaves the cursor at its start, which
        // anchors the keyword `from`; the selection body must win.
        let details = resolve_object_details_for_selection(
            text,
            Some(start..start + "users".len()),
            start,
            &schema,
        )
        .expect("the selection body resolves");

        assert_eq!("app.public:table:users", details.id);

        // Without a selection only the cursor counts.
        let cursor_only = resolve_object_details_for_selection(text, None, start, &schema)
            .expect("the bare cursor still resolves");
        assert_eq!(details.id, cursor_only.id);
    }
}
