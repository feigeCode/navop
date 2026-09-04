use crate::sidebar::execution_history_panel::ExecutionHistoryPanel;
use crate::sql_editor::{
    ForeignSchema, RunCursorStatementSql, RunSelectedSql, SQL_GUTTER_CANCELLED, SQL_GUTTER_FAILED,
    SQL_GUTTER_IDLE, SQL_GUTTER_RUNNING, SQL_GUTTER_SUCCEEDED, SqlColumnDetail, SqlEditor,
    SqlObjectType, SqlSchema, SqlTableDetail, pending_foreign_qualifiers,
};
use crate::sql_result_tab::{
    ExecutionState, SessionSchemaInvalidation, SessionSqlRun, SqlResultTabContainer,
    emit_schema_changed_events,
};
use db::cache_manager::{GlobalNodeCache, SchemaInvalidationPlan};
use db::plugin::SqlCompletionInfo;
use db::sql_editor::execution::{
    SqlDocumentSnapshot, SqlExecutionRequest, SqlExecutionResultSource, SqlExecutionTarget,
    SqlMetadataScope, SqlTransactionMode as SqlExecutionTransactionMode,
};
use db::sql_editor::insert_hints::insert_value_hints;
use db::sql_editor::sql_tokenizer::{SqlKeyword, SqlTokenKind, SqlTokenizer};
use db::sql_editor::statement_ranges::{
    SqlDialect, SqlStatementRange, SqlStatementSnapshot, SqlTextRange, StatementIndex,
    WindowedStatementScan, line_scans_neutral,
};
use db::types::TableObjectType;
use db::{DbManager, GlobalDbState, SqlFormatOptions, format_sql_with_options};
use futures::channel::oneshot;
use futures::stream::{self, StreamExt};
use gpui::prelude::*;
use gpui::{
    AnyWindowHandle, App, AppContext, AsyncApp, Axis, Bounds, ClickEvent, ColorExt, Context,
    Element, Entity, EventEmitter, FocusHandle, Focusable, Hsla, IntoElement, KeyBinding,
    MouseMoveEvent, MouseUpEvent, NoAction, ParentElement, Pixels, Point, Render, SharedString,
    Styled, Subscription, Task, WeakEntity, Window, div, px,
};
use gpui_component::button::{Button, ButtonCustomVariant, ButtonVariants};
use gpui_component::dialog::DialogFooter;
use gpui_component::input::{
    GutterMarker, Input, InputEvent, InputState, RangeDecoration, RangeDecorationStyle,
};
use gpui_component::notification::Notification;
use gpui_component::select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, IndexPath, Sizable, Size, WindowExt, h_flex, v_flex,
};
use one_core::connection_notifier::{ConnectionDataEvent, GlobalConnectionNotifier};
use one_core::gpui_tokio::Tokio;
use one_core::keybindings::{action_id, rebind_keybindings, shortcuts_for};
use one_core::settings::AppSettings;
use one_core::storage::{DatabaseType, QueryDirectoryScope, default_query_directory};
use one_core::tab_container::{TabContainer, TabContent, TabContentEvent};
use one_core::utils::auto_save_config::AutoSaveConfig;
use one_ui::resize_handle::{ResizePanel, resize_handle};
use parking_lot::{Mutex, RwLock};
use ropey::{LineType, Rope};
use rust_i18n::t;
use smol::Timer;
use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::future::Future;
use std::io;
use std::ops::{Deref, Range};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tracing::log::error;

const PANEL_MIN_SIZE: Pixels = px(100.0);
const RESULT_PANEL_DEFAULT_SIZE: Pixels = px(400.0);
const SQL_EDITOR_CONTEXT: &str = "SqlEditor";
const SQL_EDITOR_INPUT_CONTEXT: &str = "SqlEditor > Input";
const RUN_CURRENT_QUERY_KEY_BINDINGS: [&str; 2] = ["cmd-enter", "ctrl-enter"];
const RUN_ALL_QUERY_KEY_BINDINGS: [&str; 2] = ["cmd-shift-enter", "ctrl-shift-enter"];
const TOGGLE_LINE_COMMENT_KEY_BINDINGS: [&str; 2] = ["cmd-/", "ctrl-/"];
/// Maximum number of concurrent `list_columns` requests while refreshing the
/// SQL editor schema for a database. Keeps large-schema loads from serializing
/// a full per-table catalog scan or saturating the backend with an unbounded
/// burst of queries.
const SCHEMA_COLUMN_FETCH_CONCURRENCY: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SqlGutterMarkerState {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl SqlGutterMarkerState {
    fn icon_token(self) -> &'static str {
        match self {
            Self::Running => SQL_GUTTER_RUNNING,
            Self::Succeeded => SQL_GUTTER_SUCCEEDED,
            Self::Failed => SQL_GUTTER_FAILED,
            Self::Cancelled => SQL_GUTTER_CANCELLED,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum QueryFileNameError {
    Empty,
    Invalid,
    AlreadyExists,
    ReadDirectory(String),
}

fn query_file_path_for_name(directory: &Path, name: &str) -> Result<PathBuf, QueryFileNameError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(QueryFileNameError::Empty);
    }
    if is_invalid_query_file_name(name) {
        return Err(QueryFileNameError::Invalid);
    }

    let file_name = if Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("sql"))
    {
        name.to_owned()
    } else {
        format!("{name}.sql")
    };
    if file_name.eq_ignore_ascii_case(".sql") {
        return Err(QueryFileNameError::Invalid);
    }

    match std::fs::read_dir(directory) {
        Ok(entries) => {
            if entries.flatten().any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&file_name)
            }) {
                return Err(QueryFileNameError::AlreadyExists);
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(QueryFileNameError::ReadDirectory(error.to_string())),
    }

    Ok(directory.join(file_name))
}

fn is_invalid_query_file_name(name: &str) -> bool {
    const INVALID_CHARACTERS: [char; 9] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
    const RESERVED_NAMES: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];

    if matches!(name, "." | "..")
        || name.chars().any(char::is_control)
        || name
            .chars()
            .any(|character| INVALID_CHARACTERS.contains(&character))
    {
        return true;
    }
    let base_name = name.split('.').next().unwrap_or(name);
    RESERVED_NAMES
        .iter()
        .any(|reserved| base_name.eq_ignore_ascii_case(reserved))
}

fn write_sql_file(file_path: &Path, sql: &str) -> io::Result<()> {
    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(file_path, sql)
}

fn write_new_sql_file(file_path: &Path, sql: &str) -> io::Result<()> {
    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(file_path)
        .and_then(|mut file| std::io::Write::write_all(&mut file, sql.as_bytes()))
}

gpui::actions!(
    sql_editor_view,
    [RunCurrentQuery, RunAllQuery, ToggleLineComment]
);

pub fn init(cx: &mut App) {
    cx.bind_keys(init_keybindings(cx));
}

pub fn refresh_keybindings(cx: &mut App) {
    cx.bind_keys(refreshable_keybindings(cx));
}

fn init_keybindings(cx: &App) -> Vec<KeyBinding> {
    let current_shortcuts = shortcuts_for(
        cx,
        action_id::SQL_RUN_QUERY,
        &RUN_CURRENT_QUERY_KEY_BINDINGS,
    );
    let mut keybindings = current_shortcuts
        .iter()
        .map(|key| KeyBinding::new(key, RunCurrentQuery, Some(SQL_EDITOR_CONTEXT)))
        .collect::<Vec<_>>();
    keybindings.push(secondary_enter_binding(&current_shortcuts));
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::SQL_RUN_ALL_QUERY,
            &RUN_ALL_QUERY_KEY_BINDINGS,
        )
        .into_iter()
        .map(|key| KeyBinding::new(&key, RunAllQuery, Some(SQL_EDITOR_CONTEXT))),
    );
    keybindings.extend(
        TOGGLE_LINE_COMMENT_KEY_BINDINGS
            .into_iter()
            .map(|key| KeyBinding::new(key, ToggleLineComment, Some(SQL_EDITOR_INPUT_CONTEXT))),
    );
    keybindings
}

fn refreshable_keybindings(cx: &App) -> Vec<KeyBinding> {
    let current_shortcuts = shortcuts_for(
        cx,
        action_id::SQL_RUN_QUERY,
        &RUN_CURRENT_QUERY_KEY_BINDINGS,
    );
    let mut keybindings = rebind_keybindings(
        cx,
        action_id::SQL_RUN_QUERY,
        &RUN_CURRENT_QUERY_KEY_BINDINGS,
        Some(SQL_EDITOR_CONTEXT),
        RunCurrentQuery,
    );
    keybindings.push(secondary_enter_binding(&current_shortcuts));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::SQL_RUN_ALL_QUERY,
        &RUN_ALL_QUERY_KEY_BINDINGS,
        Some(SQL_EDITOR_CONTEXT),
        RunAllQuery,
    ));
    keybindings.extend(
        TOGGLE_LINE_COMMENT_KEY_BINDINGS
            .into_iter()
            .map(|key| KeyBinding::new(key, ToggleLineComment, Some(SQL_EDITOR_INPUT_CONTEXT))),
    );
    keybindings
}

fn secondary_enter_binding(current_shortcuts: &[String]) -> KeyBinding {
    if should_bind_secondary_enter(current_shortcuts) {
        KeyBinding::new(
            "secondary-enter",
            RunCurrentQuery,
            Some(SQL_EDITOR_INPUT_CONTEXT),
        )
    } else {
        KeyBinding::new("secondary-enter", NoAction, Some(SQL_EDITOR_INPUT_CONTEXT))
    }
}

fn should_bind_secondary_enter(shortcuts: &[String]) -> bool {
    shortcuts
        .iter()
        .any(|shortcut| matches!(shortcut.as_str(), "cmd-enter" | "ctrl-enter"))
}

fn sql_text_for_toolbar_run(editor_text: &str, selected_text: &str) -> String {
    if selected_text.trim().is_empty() {
        editor_text.to_string()
    } else {
        selected_text.to_string()
    }
}

fn sql_text_for_run_all(editor_text: &str, _selected_text: &str) -> String {
    editor_text.to_string()
}

fn statement_marker_id(revision: u64, statement: &SqlStatementRange) -> String {
    format!(
        "sql-statement:{revision}:{}:{}",
        statement.sql_range.start_byte, statement.sql_range.end_byte
    )
}

fn statement_for_gutter_marker<'a>(
    statements: &'a [SqlStatementRange],
    revision: u64,
    marker_id: &str,
    logical_row: usize,
) -> Option<&'a SqlStatementRange> {
    statements.iter().find(|statement| {
        statement.start_line == logical_row && statement_marker_id(revision, statement) == marker_id
    })
}

/// Compute the current-statement frame decorations for one snapshot/cursor.
///
/// The frame covers the executable statement the cursor sits in, extended
/// through its trailing delimiter so it visually matches the gutter range.
/// A non-empty selection suppresses the frame (run-selection mode), and an
/// empty statement produces no decoration. When a values-region highlight is
/// available (INSERT value hints), it is appended as a Fill decoration.
fn current_statement_frame_decorations(
    index: &dyn StatementIndex,
    revision: u64,
    cursor: usize,
    selection: &Range<usize>,
    doc_len: usize,
    values_highlight: Option<Range<usize>>,
) -> Vec<RangeDecoration> {
    let mut decorations = Vec::new();
    if let Some(highlight) = values_highlight {
        let start = highlight.start.min(doc_len);
        let end = highlight.end.min(doc_len).max(start);
        if start < end {
            decorations.push(
                RangeDecoration::new(
                    format!("insert-values:{revision}:{start}:{end}"),
                    start..end,
                )
                .with_style(RangeDecorationStyle::Fill),
            );
        }
    }
    if !selection.is_empty() {
        return decorations;
    }
    let Some(statement) = index.statement_at_cursor(cursor, doc_len) else {
        return decorations;
    };
    let len = doc_len;
    let start = statement.sql_range.start_byte.min(len);
    let mut end = statement.sql_range.end_byte.min(len);
    if let Some(delimiter) = &statement.delimiter_range {
        end = end.max(delimiter.end_byte.min(len));
    }
    if start >= end {
        return decorations;
    }
    decorations.push(RangeDecoration::new(
        format!("sql-frame:{revision}:{start}:{end}"),
        start..end,
    ));
    decorations
}

/// Extract the target table name of an INSERT statement (`INSERT [INTO] t`).
///
/// Returns `None` when the statement has no INSERT keyword or no identifier
/// follows it. Quoted identifiers keep their inner value after unquoting.
fn insert_target_table(statement: &str) -> Option<String> {
    let tokens = SqlTokenizer::new(statement).tokenize();
    let insert = tokens
        .iter()
        .position(|token| matches!(token.kind, SqlTokenKind::Keyword(SqlKeyword::Insert)))?;
    let mut name: Option<String> = None;
    for token in &tokens[insert + 1..] {
        match token.kind {
            SqlTokenKind::Whitespace | SqlTokenKind::Keyword(SqlKeyword::Into) => {}
            SqlTokenKind::Ident | SqlTokenKind::QuotedIdent => {
                name = Some(token.text.trim().to_string());
                break;
            }
            _ => break,
        }
    }
    name.map(|name| unquote_sql_identifier(&name))
}

fn insert_values_range(statement: &str) -> Option<Range<usize>> {
    let tokens = SqlTokenizer::new(statement).tokenize();
    let values = tokens
        .iter()
        .position(|token| matches!(token.kind, SqlTokenKind::Keyword(SqlKeyword::Values)))?;
    let mut start = None;
    let mut end = None;
    let mut depth = 0usize;
    for token in &tokens[values + 1..] {
        if depth == 0 {
            match token.kind {
                SqlTokenKind::LParen => {
                    start.get_or_insert(token.start);
                    depth = 1;
                }
                SqlTokenKind::Whitespace | SqlTokenKind::Comma => {}
                _ if start.is_some() => break,
                _ => {}
            }
            continue;
        }
        match token.kind {
            SqlTokenKind::LParen => depth += 1,
            SqlTokenKind::RParen => {
                depth -= 1;
                if depth == 0 {
                    end = Some(token.end);
                }
            }
            _ => {}
        }
    }
    start.zip(end).map(|(start, end)| start..end)
}

/// Look up a table's columns (case-insensitively) for INSERT ordinal hints.
fn lookup_table_columns(schema: &SqlSchema, table: &str) -> Option<Vec<String>> {
    if let Some(columns) = schema.columns_by_table.get(table) {
        return Some(columns.iter().map(|(name, _, _)| name.clone()).collect());
    }
    schema.columns_by_table.iter().find_map(|(key, columns)| {
        key.eq_ignore_ascii_case(table)
            .then(|| columns.iter().map(|(name, _, _)| name.clone()).collect())
    })
}

/// Strip quoting from a SQL identifier (`"name"`, `` `name` ``, `[name]`).
fn unquote_sql_identifier(text: &str) -> String {
    let trimmed = text.trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"')
            || (first == b'`' && last == b'`')
            || (first == b'[' && last == b']')
        {
            let inner = &trimmed[1..trimmed.len() - 1];
            return inner.replace("\"\"", "\"").replace("``", "`");
        }
    }
    trimmed.to_string()
}

/// Resolve the gutter marker id for a statement whose text exactly matches the
/// SQL being executed, at the given cursor. Returns `None` when the cursor is
/// not inside a statement or the executed SQL is not exactly that statement
/// (e.g. multi-statement or selection runs).
fn match_sql_to_statement_marker(
    snapshot: &SqlStatementSnapshot,
    revision: u64,
    cursor: usize,
    sql: &str,
) -> Option<String> {
    let trimmed = sql.trim();
    snapshot.statement_at_cursor(cursor).and_then(|statement| {
        let statement_text = snapshot.statement_text(statement).trim();
        (!statement_text.is_empty() && statement_text == trimmed)
            .then(|| statement_marker_id(revision, statement))
    })
}

fn clamp_to_char_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SqlDiagnosticIdentity {
    run_id: u64,
    document_revision: u64,
    context_generation: u64,
}

fn is_current_diagnostic_identity(
    expected: SqlDiagnosticIdentity,
    current: SqlDiagnosticIdentity,
) -> bool {
    expected == current
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ForeignQualifierKind {
    Databases,
    Schemas,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ForeignQualifierScope {
    kind: ForeignQualifierKind,
    database: String,
    current_name: Option<String>,
}

fn foreign_qualifier_scope(
    scope: &SqlMetadataScope,
    uses_schema_as_database: bool,
    supports_schema: bool,
) -> ForeignQualifierScope {
    if uses_schema_as_database {
        ForeignQualifierScope {
            kind: ForeignQualifierKind::Schemas,
            database: String::new(),
            current_name: scope.schema.clone(),
        }
    } else if supports_schema {
        ForeignQualifierScope {
            kind: ForeignQualifierKind::Schemas,
            database: scope.database.clone().unwrap_or_default(),
            current_name: scope.schema.clone(),
        }
    } else {
        ForeignQualifierScope {
            kind: ForeignQualifierKind::Databases,
            database: String::new(),
            current_name: scope.database.clone(),
        }
    }
}

fn foreign_qualifier_fetch_scope(
    scope: &SqlMetadataScope,
    qualifier: &str,
    uses_schema_as_database: bool,
    supports_schema: bool,
) -> Option<(String, Option<String>)> {
    let qualifier_scope = foreign_qualifier_scope(scope, uses_schema_as_database, supports_schema);
    match qualifier_scope.kind {
        ForeignQualifierKind::Databases => Some((qualifier.to_string(), None)),
        ForeignQualifierKind::Schemas if uses_schema_as_database => {
            Some((String::new(), Some(qualifier.to_string())))
        }
        ForeignQualifierKind::Schemas if qualifier_scope.database.is_empty() => None,
        ForeignQualifierKind::Schemas => {
            Some((qualifier_scope.database, Some(qualifier.to_string())))
        }
    }
}

fn foreign_prefetch_key(scope: &SqlMetadataScope, qualifier: &str) -> (SqlMetadataScope, String) {
    (scope.clone(), qualifier.to_lowercase())
}

/// 加载其他 database/schema（qualifier）名称列表，用于跨库限定名补全。
///
/// 数据库型 qualifier（MySQL/ClickHouse）取其他数据库；schema 型 qualifier
/// （PG/MSSQL/Oracle）取其他 schema。排除当前 qualifier，只取名字，
/// 完整元数据按需懒加载（见 `schedule_foreign_schema_prefetch`）。
async fn load_foreign_qualifier_names(
    global_state: &GlobalDbState,
    cx: &mut AsyncApp,
    connection_id: String,
    scope: &SqlMetadataScope,
    uses_schema_as_database: bool,
    supports_schema: bool,
) -> anyhow::Result<Vec<(String, String)>> {
    let qualifier_scope = foreign_qualifier_scope(scope, uses_schema_as_database, supports_schema);
    let kind_label = match qualifier_scope.kind {
        ForeignQualifierKind::Databases => t!("SqlEditor.database_object").to_string(),
        ForeignQualifierKind::Schemas => t!("SqlEditor.schema_object").to_string(),
    };
    let names = match qualifier_scope.kind {
        ForeignQualifierKind::Databases => global_state.list_databases(cx, connection_id).await?,
        ForeignQualifierKind::Schemas => {
            global_state
                .list_schemas(cx, connection_id, qualifier_scope.database.clone())
                .await?
        }
    };
    let excluded = |name: &str| {
        qualifier_scope
            .current_name
            .as_deref()
            .is_some_and(|current| current.eq_ignore_ascii_case(name))
    };
    let mut seen = HashSet::new();
    Ok(names
        .into_iter()
        .filter(|name| !name.is_empty() && !excluded(name) && seen.insert(name.to_lowercase()))
        .map(|name| (name, kind_label.clone()))
        .collect())
}

/// 在 Tokio runtime 内拉取一个外部 qualifier 的表/列元数据（懒加载）。
/// 必须通过 `Tokio::spawn_result` 调用，不能在 GPUI executor 上直接轮询。
async fn fetch_foreign_schema_metadata(
    global_state: &GlobalDbState,
    connection_id: &str,
    database: &str,
    schema: Option<String>,
    qualifier: &str,
) -> anyhow::Result<ForeignSchema> {
    let tables = global_state
        .list_tables_direct(connection_id, database, schema.clone())
        .await?;
    // 先拷贝出表名，避免借用闭包在 Tokio 'static future 中的高阶生命周期推断问题
    let mut table_names: Vec<(usize, String)> = Vec::with_capacity(tables.len());
    for (index, table) in tables.iter().enumerate() {
        table_names.push((index, table.name.clone()));
    }
    let column_results = collect_bounded(
        table_names,
        SCHEMA_COLUMN_FETCH_CONCURRENCY,
        |(index, table_name)| {
            let global_state = global_state.clone();
            let connection_id = connection_id.to_string();
            let database = database.to_string();
            let schema = schema.clone();
            async move {
                let columns = global_state
                    .list_columns_direct(&connection_id, &database, schema, &table_name)
                    .await;
                (index, columns)
            }
        },
    )
    .await;

    let mut foreign = ForeignSchema {
        name: qualifier.to_string(),
        tables: Vec::with_capacity(tables.len()),
        columns_by_table: HashMap::new(),
        table_details: HashMap::new(),
    };
    for (table_index, columns) in column_results {
        let Some(table) = tables.get(table_index) else {
            continue;
        };
        let description = match &table.comment {
            Some(comment) => format!("Table: {} - {}", table.name, comment),
            None => format!("Table: {}", table.name),
        };
        foreign.tables.push((table.name.clone(), description));
        if let Ok(columns) = columns {
            foreign.columns_by_table.insert(
                table.name.clone(),
                columns
                    .iter()
                    .map(|c| {
                        (
                            c.name.clone(),
                            c.data_type.clone(),
                            c.comment.clone().unwrap_or_default(),
                        )
                    })
                    .collect(),
            );
            let detail = SqlTableDetail {
                object_type: match table.object_type {
                    TableObjectType::Table => SqlObjectType::Table,
                    TableObjectType::View => SqlObjectType::View,
                },
                schema: table.schema.clone(),
                comment: table.comment.clone(),
                engine: table.engine.clone(),
                columns: columns
                    .iter()
                    .map(|c| SqlColumnDetail {
                        name: c.name.clone(),
                        data_type: c.data_type.clone(),
                        is_nullable: c.is_nullable,
                        is_primary_key: c.is_primary_key,
                        default_value: c.default_value.clone(),
                        comment: c.comment.clone(),
                    })
                    .collect(),
            };
            foreign.table_details.insert(table.name.clone(), detail);
        }
    }
    Ok(foreign)
}

/// Poll a stream of futures with a fixed upper bound on how many are in flight
/// at once, collecting every output in arbitrary completion order.
///
/// This is the workhorse for bounded-concurrency metadata loads: a large schema
/// must not serialize a full per-table catalog scan, but it also must not burst
/// the backend with one query per table at once.
///
/// Contributions passed to `poll` must be owned clones: concurrent futures need
/// exclusive handles (e.g. a cloned `AsyncApp`), and the `&mut AsyncApp` of the
/// caller cannot be shared across `buffer_unordered` futures.
async fn collect_bounded<T, Fut>(
    items: impl IntoIterator<Item = T>,
    concurrency: usize,
    poll: impl Fn(T) -> Fut,
) -> Vec<Fut::Output>
where
    Fut: Future,
{
    stream::iter(items.into_iter().map(poll))
        .buffer_unordered(concurrency)
        .collect()
        .await
}

#[derive(Debug, PartialEq, Eq)]
struct LineCommentResult {
    range: Range<usize>,
    replacement: String,
    selection: Range<usize>,
}

impl LineCommentResult {
    #[cfg(test)]
    fn apply_to(&self, text: &str) -> String {
        let mut text = text.to_owned();
        text.replace_range(self.range.clone(), &self.replacement);
        text
    }
}

#[derive(Debug)]
struct OffsetEdit {
    range: Range<usize>,
    replacement_len: usize,
}

fn toggle_sql_line_comments(text: &str, selection: Range<usize>) -> LineCommentResult {
    let selection_start = clamp_to_char_boundary(text, selection.start.min(text.len()));
    let selection_end =
        clamp_to_char_boundary(text, selection.end.min(text.len()).max(selection_start));
    let line_start = text[..selection_start]
        .rfind('\n')
        .map_or(0, |newline| newline + 1);
    let effective_end = if selection_end > selection_start
        && text.as_bytes().get(selection_end - 1) == Some(&b'\n')
    {
        selection_end - 1
    } else {
        selection_end
    };
    let line_end = text[effective_end..]
        .find('\n')
        .map_or(text.len(), |newline| effective_end + newline);
    let target = &text[line_start..line_end];
    let lines = target.split('\n').collect::<Vec<_>>();
    let uncomment = lines
        .iter()
        .filter(|line| !line.trim_matches([' ', '\t', '\r']).is_empty())
        .all(|line| line.trim_start_matches([' ', '\t']).starts_with("--"))
        && lines
            .iter()
            .any(|line| !line.trim_matches([' ', '\t', '\r']).is_empty());

    let mut edits = Vec::new();
    let mut relative_line_start = 0;
    for line in lines {
        let content = line.strip_suffix('\r').unwrap_or(line);
        if content.trim_matches([' ', '\t']).is_empty() {
            relative_line_start += line.len() + 1;
            continue;
        }
        let indentation_len = content.len() - content.trim_start_matches([' ', '\t']).len();
        let edit_start = line_start + relative_line_start + indentation_len;

        if uncomment {
            let comment = &content[indentation_len..];
            let removed_len = if comment.starts_with("-- ") { 3 } else { 2 };
            edits.push(OffsetEdit {
                range: edit_start..edit_start + removed_len,
                replacement_len: 0,
            });
        } else {
            edits.push(OffsetEdit {
                range: edit_start..edit_start,
                replacement_len: 3,
            });
        }

        relative_line_start += line.len() + 1;
    }

    let mut replacement = target.to_owned();
    for edit in edits.iter().rev() {
        let edit_range = edit.range.start - line_start..edit.range.end.saturating_sub(line_start);
        let inserted_text = if edit.replacement_len == 0 { "" } else { "-- " };
        replacement.replace_range(edit_range, inserted_text);
    }

    let mapped_selection = if selection_start == selection_end {
        let cursor = map_offset_after_edits(selection_start, &edits, true);
        cursor..cursor
    } else {
        map_offset_after_edits(selection_start, &edits, false)
            ..map_offset_after_edits(selection_end, &edits, true)
    };

    LineCommentResult {
        range: line_start..line_end,
        replacement,
        selection: mapped_selection,
    }
}

fn map_offset_after_edits(offset: usize, edits: &[OffsetEdit], bias_after_insert: bool) -> usize {
    let mut delta = 0_isize;

    for edit in edits {
        if offset < edit.range.start {
            break;
        }

        let removed_len = edit.range.len();
        if removed_len == 0 {
            if offset > edit.range.start || (offset == edit.range.start && bias_after_insert) {
                delta += edit.replacement_len as isize;
            }
        } else if offset >= edit.range.end {
            delta += edit.replacement_len as isize - removed_len as isize;
        } else {
            return (edit.range.start as isize + delta + edit.replacement_len as isize).max(0)
                as usize;
        }
    }

    (offset as isize + delta).max(0) as usize
}

fn should_render_schema_select(supports_schema: bool, uses_schema_as_database: bool) -> bool {
    supports_schema || uses_schema_as_database
}

fn non_empty_initial_value(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}

fn initial_database_select_value(
    initial_database: Option<String>,
    initial_schema: Option<String>,
    uses_schema_as_database: bool,
) -> Option<String> {
    if uses_schema_as_database {
        non_empty_initial_value(initial_schema)
            .or_else(|| non_empty_initial_value(initial_database))
    } else {
        non_empty_initial_value(initial_database)
    }
}

/// 返回连接登录配置中的默认数据库，但仅当它仍在可选数据库列表中时才生效，
/// 避免默认选中一个当前账号不可见或不可执行的库。
fn preferred_default_database(
    login_database: Option<String>,
    available_databases: &[String],
) -> Option<String> {
    let database = login_database.map(|database| database.trim().to_string());
    non_empty_initial_value(database)
        .filter(|database| available_databases.iter().any(|item| item == database))
}

fn set_select_items_with_initial_value(
    state: &mut SelectState<SearchableVec<String>>,
    values: Vec<String>,
    selected_name: Option<&str>,
    empty_label: String,
    window: &mut Window,
    cx: &mut Context<SelectState<SearchableVec<String>>>,
) {
    if values.is_empty() {
        let items = SearchableVec::new(vec![
            t!("Common.no_available", item = empty_label).to_string(),
        ]);
        state.set_items(items, window, cx);
        state.set_selected_index(None, window, cx);
        return;
    }

    let selected_index = selected_name
        .and_then(|name| values.iter().position(|value| value == name))
        .unwrap_or(0);
    state.set_items(SearchableVec::new(values), window, cx);
    state.set_selected_index(Some(IndexPath::new(selected_index)), window, cx);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManualTransactionAction {
    Begin,
    Commit,
    Rollback,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SqlTransactionMode {
    Auto,
    Manual,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TransactionModeOption {
    mode: SqlTransactionMode,
}

impl TransactionModeOption {
    fn new(mode: SqlTransactionMode) -> Self {
        Self { mode }
    }
}

impl gpui_component::select::SelectItem for TransactionModeOption {
    type Value = SqlTransactionMode;

    fn title(&self) -> SharedString {
        match self.mode {
            SqlTransactionMode::Auto => t!("Query.transaction_auto").into(),
            SqlTransactionMode::Manual => t!("Query.transaction_manual").into(),
        }
    }

    fn value(&self) -> &Self::Value {
        &self.mode
    }
}

fn transaction_mode_options() -> SearchableVec<TransactionModeOption> {
    SearchableVec::new(vec![
        TransactionModeOption::new(SqlTransactionMode::Auto),
        TransactionModeOption::new(SqlTransactionMode::Manual),
    ])
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SqlExecutionScope {
    database: Option<String>,
    schema: Option<String>,
}

impl SqlExecutionScope {
    fn new(database: Option<String>, schema: Option<String>) -> Self {
        Self { database, schema }
    }
}

#[derive(Clone, Debug)]
struct ManualTransactionSession {
    session_id: String,
    database: Option<String>,
    schema: Option<String>,
    pending_invalidation: Arc<Mutex<SchemaInvalidationPlan>>,
}

struct ManualTransactionPrepare<'a> {
    database_type: &'a DatabaseType,
    scope: &'a SqlExecutionScope,
    session_id: &'a str,
}

impl ManualTransactionSession {
    fn new(session_id: String, database: Option<String>, schema: Option<String>) -> Self {
        Self {
            session_id,
            database,
            schema,
            pending_invalidation: Arc::new(Mutex::new(SchemaInvalidationPlan::default())),
        }
    }

    fn session_id(&self) -> &str {
        &self.session_id
    }

    fn matches_scope(&self, database: Option<&str>, schema: Option<&str>) -> bool {
        self.database.as_deref() == database && self.schema.as_deref() == schema
    }

    fn matches_execution_scope(&self, scope: &SqlExecutionScope) -> bool {
        self.matches_scope(scope.database.as_deref(), scope.schema.as_deref())
    }

    fn pending_invalidation(&self) -> Arc<Mutex<SchemaInvalidationPlan>> {
        self.pending_invalidation.clone()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManualTransactionInvalidationMode {
    Immediate,
    Deferred,
}

fn manual_transaction_invalidation_mode(
    database_type: &DatabaseType,
) -> ManualTransactionInvalidationMode {
    match database_type {
        DatabaseType::PostgreSQL
        | DatabaseType::SQLite
        | DatabaseType::DuckDB
        | DatabaseType::MSSQL => ManualTransactionInvalidationMode::Deferred,
        DatabaseType::MySQL
        | DatabaseType::Oracle
        | DatabaseType::ClickHouse
        // TDengine 无事务,与 ClickHouse 同臂立即失效缓存。
        | DatabaseType::TDengine
        | DatabaseType::External { .. } => ManualTransactionInvalidationMode::Immediate,
    }
}

fn supports_manual_transactions(database_type: &DatabaseType) -> bool {
    matches!(
        database_type,
        DatabaseType::MySQL
            | DatabaseType::PostgreSQL
            | DatabaseType::SQLite
            | DatabaseType::DuckDB
            | DatabaseType::MSSQL
            | DatabaseType::Oracle
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManualSqlExecutionAction {
    Unsupported,
    ScopeMismatch,
    Busy,
    RunInstalledSession,
    StartSession,
}

impl ManualSqlExecutionAction {
    #[cfg(test)]
    fn binds_execution_marker(self) -> bool {
        matches!(self, Self::RunInstalledSession | Self::StartSession)
    }
}

fn manual_sql_execution_action(
    database_type: &DatabaseType,
    installed_session_matches_scope: Option<bool>,
    lifecycle_busy: bool,
) -> ManualSqlExecutionAction {
    if !supports_manual_transactions(database_type) {
        ManualSqlExecutionAction::Unsupported
    } else if installed_session_matches_scope == Some(false) {
        ManualSqlExecutionAction::ScopeMismatch
    } else if lifecycle_busy {
        ManualSqlExecutionAction::Busy
    } else if installed_session_matches_scope == Some(true) {
        ManualSqlExecutionAction::RunInstalledSession
    } else {
        ManualSqlExecutionAction::StartSession
    }
}

fn manual_transaction_control_sql(
    database_type: &DatabaseType,
    action: ManualTransactionAction,
) -> Option<&'static str> {
    match action {
        ManualTransactionAction::Begin => match database_type {
            DatabaseType::MSSQL => Some("BEGIN TRANSACTION"),
            DatabaseType::Oracle => None,
            _ => Some("BEGIN"),
        },
        ManualTransactionAction::Commit => Some("COMMIT"),
        ManualTransactionAction::Rollback => Some("ROLLBACK"),
    }
}

fn manual_transaction_control_options() -> db::ExecOptions {
    db::ExecOptions {
        stop_on_error: true,
        max_rows: None,
        ..Default::default()
    }
}

fn transaction_control_failed(result: &anyhow::Result<Vec<db::SqlResult>>) -> bool {
    match result {
        Ok(results) => results.iter().any(db::SqlResult::is_error),
        Err(_) => true,
    }
}

fn query_connection_context_label(connection_name: &str, server_info: &str) -> String {
    let connection_name = connection_name.trim();
    let server_info = server_info.trim();

    match (connection_name.is_empty(), server_info.is_empty()) {
        (false, false) => format!("{connection_name} · {server_info}"),
        (false, true) => connection_name.to_string(),
        (true, false) => server_info.to_string(),
        (true, true) => String::new(),
    }
}

fn query_connection_ids(available_connection_ids: &[String], connection_id: &str) -> Vec<String> {
    let mut connection_ids = Vec::new();
    for available_connection_id in available_connection_ids {
        let available_connection_id = available_connection_id.trim();
        if !available_connection_id.is_empty()
            && !connection_ids
                .iter()
                .any(|connection_id| connection_id == available_connection_id)
        {
            connection_ids.push(available_connection_id.to_string());
        }
    }

    let connection_id = connection_id.trim();
    if !connection_id.is_empty()
        && !connection_ids
            .iter()
            .any(|available_connection_id| available_connection_id == connection_id)
    {
        connection_ids.push(connection_id.to_string());
    }
    connection_ids
}

fn can_switch_query_connection(is_executing: bool, has_manual_transaction: bool) -> bool {
    !is_executing && !has_manual_transaction
}

fn can_start_query_execution(is_executing: bool) -> bool {
    !is_executing
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManualTransactionStopAction {
    None,
    CancelStart,
    CloseInstalledSession,
}

fn manual_transaction_stop_action(
    cancelled_execution: bool,
    transaction_starting: bool,
    has_installed_session: bool,
) -> ManualTransactionStopAction {
    if cancelled_execution && has_installed_session {
        ManualTransactionStopAction::CloseInstalledSession
    } else if transaction_starting {
        ManualTransactionStopAction::CancelStart
    } else {
        ManualTransactionStopAction::None
    }
}

fn is_current_manual_transaction_owner(
    expected_generation: u64,
    expected_session_id: &str,
    current_generation: u64,
    current_session_id: Option<&str>,
) -> bool {
    expected_generation == current_generation && current_session_id == Some(expected_session_id)
}

fn is_current_manual_transaction_start(
    expected_generation: u64,
    current_generation: u64,
    is_starting: bool,
) -> bool {
    expected_generation == current_generation && is_starting
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QueryToolbarAction {
    Run,
    RunSelected,
    Stop,
}

/// 查询工具栏高度，参考 dbx EditorToolbar 的单行紧凑布局。
const QUERY_TOOLBAR_HEIGHT: Pixels = px(36.0);

/// 工具栏下拉框与图标按钮的统一控件高度（Button Small），保证单行对齐。
const QUERY_TOOLBAR_CONTROL_HEIGHT: Pixels = px(28.0);

/// 查询工具栏图标按钮的描述。
struct QueryToolbarButtonSpec {
    id: &'static str,
    icon: IconName,
    color: Hsla,
    tooltip: SharedString,
    disabled: bool,
}

/// 工具栏按钮组之间的竖向分隔线。
fn query_toolbar_divider(cx: &App) -> impl IntoElement {
    div()
        .h_4()
        .w(px(1.0))
        .mx_0p5()
        .flex_shrink_0()
        .bg(cx.theme().border)
}

fn query_toolbar_action(is_executing: bool, has_selection: bool) -> QueryToolbarAction {
    if is_executing {
        QueryToolbarAction::Stop
    } else if has_selection {
        QueryToolbarAction::RunSelected
    } else {
        QueryToolbarAction::Run
    }
}

fn is_current_query_context_generation(expected: u64, current: u64) -> bool {
    expected == current
}

#[derive(Clone, Debug)]
struct QueryConnectionOption {
    id: String,
    label: SharedString,
}

impl SelectItem for QueryConnectionOption {
    type Value = String;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }
}

fn query_connection_options(
    available_connection_ids: &[String],
    connection_id: &str,
    global_state: &GlobalDbState,
) -> Vec<QueryConnectionOption> {
    query_connection_ids(available_connection_ids, connection_id)
        .into_iter()
        .map(|connection_id| {
            let label = global_state
                .get_config(&connection_id)
                .map(|connection| {
                    query_connection_context_label(&connection.name, &connection.server_info())
                })
                .filter(|label| !label.is_empty())
                .unwrap_or_else(|| connection_id.clone())
                .into();
            QueryConnectionOption {
                id: connection_id,
                label,
            }
        })
        .collect()
}

// Events emitted by SqlEditorTabContent
#[derive(Debug, Clone)]
pub enum SqlEditorEvent {
    /// Query was saved successfully
    QuerySaved {
        connection_id: String,
        database: Option<String>,
    },
}

pub struct SqlEditorTabConfig {
    pub title: SharedString,
    pub connection_id: String,
    pub available_connection_ids: Vec<String>,
    pub database_type: DatabaseType,
    pub file_path: Option<PathBuf>,
    pub new_file_directory: Option<PathBuf>,
    pub initial_database: Option<String>,
    pub initial_schema: Option<String>,
    pub execution_history: Entity<ExecutionHistoryPanel>,
}

/// A windowed statement scan plus the buffer rows it covers.
///
/// `analyzed_rows` is the buffer-row range the scan was computed for
/// (viewport plus margins, after sync-point alignment); scrolling outside it
/// schedules a re-scan.
#[derive(Clone)]
struct ViewportStatements {
    /// Document revision the scan was computed against.
    revision: u64,
    /// Buffer-row range covered by the scan.
    analyzed_rows: Range<usize>,
    scan: WindowedStatementScan,
}

enum StatementScanInput {
    Full {
        text: String,
    },
    Window {
        text: String,
        base_byte: usize,
        base_line: usize,
        analyzed_rows: Range<usize>,
    },
}

enum StatementScanResult {
    Full(SqlStatementSnapshot),
    Window {
        scan: WindowedStatementScan,
        analyzed_rows: Range<usize>,
    },
}

const FULL_STATEMENT_SCAN_LINE_THRESHOLD: usize = 2_000;
const VIEWPORT_SCAN_MARGIN_LINES: usize = 120;
const VIEWPORT_SCAN_SYNC_LIMIT_LINES: usize = 800;

fn statement_scan_sync_line(rope: &Rope, row: usize) -> bool {
    let line = rope.line(row, LineType::LF).to_string();
    let trimmed = line.trim_end();
    !trimmed.starts_with('*') && trimmed.ends_with(';') && line_scans_neutral(trimmed)
}

fn viewport_statement_scan_input(
    text: &Rope,
    visible_rows: Option<Range<usize>>,
) -> StatementScanInput {
    let total_lines = text.len_lines(LineType::LF);
    let Some(visible_rows) = visible_rows else {
        return StatementScanInput::Full {
            text: text.to_string(),
        };
    };
    if total_lines < FULL_STATEMENT_SCAN_LINE_THRESHOLD {
        return StatementScanInput::Full {
            text: text.to_string(),
        };
    }

    let margin = visible_rows.len().max(VIEWPORT_SCAN_MARGIN_LINES);
    let target_start = visible_rows.start.saturating_sub(margin);
    let target_end = visible_rows.end.saturating_add(margin).min(total_lines);
    let base_search_start = target_start.saturating_sub(VIEWPORT_SCAN_SYNC_LIMIT_LINES);
    let base_line = (base_search_start..target_start)
        .rev()
        .find(|row| statement_scan_sync_line(text, *row))
        .map_or(target_start, |row| row + 1);
    let end_search_end = target_end
        .saturating_add(VIEWPORT_SCAN_SYNC_LIMIT_LINES)
        .min(total_lines);
    let end_line = (target_end..end_search_end)
        .find(|row| statement_scan_sync_line(text, *row))
        .map_or(target_end, |row| row + 1);

    let base_byte = text.line_to_byte_idx(base_line, LineType::LF);
    let end_byte = if end_line >= total_lines {
        text.len()
    } else {
        text.line_to_byte_idx(end_line, LineType::LF)
    };
    StatementScanInput::Window {
        text: text.slice(base_byte..end_byte).to_string(),
        base_byte,
        base_line,
        analyzed_rows: base_line..end_line,
    }
}

pub struct SqlEditorTab {
    title: SharedString,
    editor: Entity<SqlEditor>,
    connection_id: String,
    database_type: DatabaseType,
    sql_result_tab_container: Entity<SqlResultTabContainer>,
    connection_select: Entity<SelectState<SearchableVec<QueryConnectionOption>>>,
    database_select: Entity<SelectState<SearchableVec<String>>>,
    schema_select: Entity<SelectState<SearchableVec<String>>>,
    transaction_mode_select: Entity<SelectState<SearchableVec<TransactionModeOption>>>,
    supports_schema: bool,
    uses_schema_as_database: bool,
    focus_handle: FocusHandle,
    file_path: Arc<RwLock<PathBuf>>,
    requires_name: Arc<AtomicBool>,
    _save_task: Option<Task<()>>,
    result_panel_size: Pixels,
    resizing: bool,
    bounds: Bounds<Pixels>,
    transaction_mode: SqlTransactionMode,
    manual_transaction: Option<ManualTransactionSession>,
    /// Manual transaction lifecycle generation. Starting, finishing, cancelling,
    /// and context invalidation all advance it so late async completions cannot
    /// install or clear a session owned by a newer operation.
    manual_transaction_generation: Arc<AtomicU64>,
    manual_transaction_starting: bool,
    manual_transaction_finishing: bool,
    /// 自动保存序列号，用于防抖
    auto_save_seq: Arc<AtomicU64>,
    /// 是否有未保存的修改
    is_dirty: Arc<AtomicBool>,
    /// 查询上下文代次，用于丢弃连接、数据库或 Schema 切换前发起的异步回写。
    context_generation: Arc<AtomicU64>,
    /// Monotonic identity for SQL execution requests from this editor.
    execution_request_id: Arc<AtomicU64>,
    _connection_subscription: Option<Subscription>,
    statement_snapshot: SqlStatementSnapshot,
    /// Windowed statement scan covering the viewport (plus margins), used for
    /// display-only consumers (gutter markers, current-statement frame) on
    /// large documents. `None` whenever the full `statement_snapshot` is
    /// authoritative (after execution-path refreshes or before first layout).
    viewport_statements: Option<ViewportStatements>,
    /// Execution state per gutter marker, keyed by `statement_marker_id`.
    /// Editing invalidates states because the revision is part of the id.
    statement_marker_states: HashMap<String, SqlGutterMarkerState>,
    /// Marker id currently bound to the in-flight execution, if any.
    active_statement_marker: Option<String>,
    /// Last editor state used to drive the current-statement decorations.
    last_frame_key: Option<(u64, usize, Range<usize>, Option<Range<usize>>)>,
    _execution_state_subscription: Option<Subscription>,
    _editor_input_subscription: Option<Subscription>,
    /// 诊断分析运行序号，用于防抖并丢弃过期任务。
    diagnostic_run_id: Arc<AtomicU64>,
    /// 在途的 SQL 诊断分析任务。
    _diagnostic_task: Option<Task<()>>,
    /// 语句快照分析运行序号，用于防抖并丢弃过期的后台 tokenize。
    statement_run_id: Arc<AtomicU64>,
    /// 在途的语句快照后台分析任务（防抖的 onChange 生效路径）。
    _statement_task: Option<Task<()>>,
    /// 最新元数据快照，供 INSERT 值区域等本地消费（与 completion source 同步更新）。
    schema_snapshot: Arc<RwLock<SqlSchema>>,
    /// 最近一次加载的数据库特定补全信息（合并外部 qualifier 元数据时复用）。
    db_completion_info: Arc<RwLock<Option<SqlCompletionInfo>>>,
    /// 在途的外部 qualifier 元数据懒加载任务（按 metadata scope + qualifier 隔离）。
    foreign_prefetch_inflight: Arc<Mutex<HashSet<(SqlMetadataScope, String)>>>,
    /// INSERT values 区域在文档中的字节范围，用于 Fill 装饰。
    insert_values_highlight: Option<Range<usize>>,
    /// 最近一次计算 INSERT values 区域的 (语句起点, 版本)。
    last_insert_hints_key: Option<(usize, u64)>,
}

struct SqlSchemaUpdateRequest {
    database: String,
    generation: u64,
    window_handle: AnyWindowHandle,
    entity: WeakEntity<SqlEditorTab>,
}

impl SqlEditorTab {
    pub fn new_with_config(
        config: SqlEditorTabConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editor = cx.new(|cx| SqlEditor::new(window, cx));
        let focus_handle = cx.focus_handle();
        let global_state = cx.global::<GlobalDbState>().clone();
        let execution_history = config.execution_history.clone();
        let connection_id = config.connection_id;
        let connection_options = query_connection_options(
            &config.available_connection_ids,
            &connection_id,
            &global_state,
        );
        let selected_connection_index = connection_options
            .iter()
            .position(|option| option.id == connection_id)
            .map(IndexPath::new);
        let connection_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(connection_options),
                selected_connection_index,
                window,
                cx,
            )
            .searchable(true)
        });
        let database_select = cx.new(|cx| {
            SelectState::new(SearchableVec::new(vec![]), None, window, cx).searchable(true)
        });
        let schema_select = cx.new(|cx| {
            SelectState::new(SearchableVec::new(vec![]), None, window, cx).searchable(true)
        });
        let transaction_mode_select = cx.new(|cx| {
            SelectState::new(
                transaction_mode_options(),
                Some(IndexPath::new(0)),
                window,
                cx,
            )
        });

        let capabilities = global_state.capabilities(&config.database_type);
        let supports_schema = capabilities.supports_schema;
        let uses_schema_as_database = capabilities.uses_schema_as_database;
        let initial_database = config.initial_database;
        let initial_schema = config.initial_schema;
        let initial_select_value = initial_database_select_value(
            initial_database.clone(),
            initial_schema.clone(),
            uses_schema_as_database,
        );

        let should_load_file = config.file_path.is_some();
        let requires_name = Arc::new(AtomicBool::new(!should_load_file));
        let resolved_file_path = match config.file_path {
            Some(path) => path,
            None => match config.new_file_directory {
                Some(directory) => Self::generate_new_file_path_in_directory(&directory),
                None => Self::generate_new_file_path(
                    &config.database_type,
                    &connection_id,
                    initial_select_value.as_deref().unwrap_or("default"),
                ),
            },
        };

        let auto_save_seq = Arc::new(AtomicU64::new(0));
        let is_dirty = Arc::new(AtomicBool::new(false));
        let context_generation = Arc::new(AtomicU64::new(0));
        let execution_request_id = Arc::new(AtomicU64::new(0));
        let diagnostic_run_id = Arc::new(AtomicU64::new(0));
        let statement_run_id = Arc::new(AtomicU64::new(0));
        let manual_transaction_generation = Arc::new(AtomicU64::new(0));

        let initial_dialect = SqlDialect::from(&config.database_type);
        let mut instance = Self {
            title: config.title,
            editor: editor.clone(),
            connection_id,
            database_type: config.database_type,
            sql_result_tab_container: cx
                .new(|cx| SqlResultTabContainer::new(execution_history.clone(), window, cx)),
            connection_select: connection_select.clone(),
            database_select: database_select.clone(),
            schema_select: schema_select.clone(),
            transaction_mode_select: transaction_mode_select.clone(),
            supports_schema,
            uses_schema_as_database,
            focus_handle,
            file_path: Arc::new(RwLock::new(resolved_file_path.clone())),
            requires_name: requires_name.clone(),
            _save_task: None,
            result_panel_size: RESULT_PANEL_DEFAULT_SIZE,
            resizing: false,
            bounds: Bounds::default(),
            transaction_mode: SqlTransactionMode::Auto,
            manual_transaction: None,
            manual_transaction_generation,
            manual_transaction_starting: false,
            manual_transaction_finishing: false,
            auto_save_seq: auto_save_seq.clone(),
            is_dirty: is_dirty.clone(),
            context_generation,
            execution_request_id,
            _connection_subscription: None,
            statement_snapshot: SqlStatementSnapshot::new(String::new(), initial_dialect),
            viewport_statements: None,
            statement_marker_states: HashMap::new(),
            active_statement_marker: None,
            last_frame_key: None,
            _execution_state_subscription: None,
            _editor_input_subscription: None,
            diagnostic_run_id: diagnostic_run_id.clone(),
            _diagnostic_task: None,
            statement_run_id: statement_run_id.clone(),
            _statement_task: None,
            schema_snapshot: Arc::new(RwLock::new(SqlSchema::default())),
            db_completion_info: Arc::new(RwLock::new(None)),
            foreign_prefetch_inflight: Arc::new(Mutex::new(HashSet::new())),
            insert_values_highlight: None,
            last_insert_hints_key: None,
        };

        instance.bind_gutter_marker_event(window, cx);
        instance.bind_select_event(window, cx);
        instance.bind_transaction_mode_select_event(window, cx);
        instance.bind_auto_save(auto_save_seq, is_dirty, requires_name, window, cx);
        instance.bind_execution_marker_event(cx);
        instance.bind_editor_input_observe(window, cx);
        instance.bind_connection_data_event(window, cx);
        instance.refresh_statement_snapshot(cx);
        instance.run_diagnostics(cx);
        instance.load_databases_async(
            initial_select_value,
            initial_schema,
            resolved_file_path,
            should_load_file,
            0,
            cx,
            window,
        );

        instance
    }

    fn generate_new_file_path(
        database_type: &DatabaseType,
        connection_id: &str,
        database: &str,
    ) -> PathBuf {
        let scope = QueryDirectoryScope::new(database_type.path_key(), connection_id, database);
        let dir_path = default_query_directory(&scope).unwrap_or_else(|_| PathBuf::from("."));
        Self::generate_new_file_path_in_directory(&dir_path)
    }

    fn generate_new_file_path_in_directory(dir_path: &Path) -> PathBuf {
        let mut next_number = 1;
        if let Ok(entries) = std::fs::read_dir(&dir_path) {
            for entry in entries.flatten() {
                let file_name = entry.file_name();
                let name = file_name.to_string_lossy();
                let prefix = t!("Query.query_editor_prefix");
                if name.starts_with(&*prefix) && name.ends_with(".sql") {
                    if let Some(num_str) = name
                        .strip_prefix(&*prefix)
                        .and_then(|s| s.strip_suffix(".sql"))
                    {
                        if let Ok(num) = num_str.parse::<u32>() {
                            next_number = next_number.max(num + 1);
                        }
                    }
                }
            }
        }

        let file_name = format!("{} {}.sql", t!("Query.query_editor_prefix"), next_number);
        dir_path.join(file_name)
    }

    pub fn get_file_path(&self) -> PathBuf {
        self.file_path.read().clone()
    }

    fn bind_select_event(&self, window: &mut Window, cx: &mut Context<Self>) {
        cx.subscribe_in(
            &self.connection_select,
            window,
            |this,
             _select,
             event: &SelectEvent<SearchableVec<QueryConnectionOption>>,
             window,
             cx| {
                if let SelectEvent::Confirm(Some(connection_id)) = event {
                    this.switch_connection(connection_id, window, cx);
                }
            },
        )
        .detach();

        cx.subscribe_in(
            &self.database_select,
            window,
            |this, _select, event: &SelectEvent<SearchableVec<String>>, window, cx| {
                let global_state = cx.global::<GlobalDbState>().clone();
                if let SelectEvent::Confirm(Some(db_name)) = event {
                    let generation = this.next_context_generation(cx);
                    let window_handle = window.window_handle();
                    if this.supports_schema && !this.uses_schema_as_database {
                        Self::clear_string_select(&this.schema_select, window, cx);
                    }
                    let db = db_name.clone();
                    let instance = this.clone();
                    cx.spawn(async move |handle, cx| {
                        if instance.supports_schema && !instance.uses_schema_as_database {
                            instance
                                .load_schemas_for_db(
                                    global_state.clone(),
                                    &db,
                                    None,
                                    generation,
                                    window_handle,
                                    cx,
                                )
                                .await;
                        }
                        instance
                            .update_schema_for_db(
                                global_state,
                                SqlSchemaUpdateRequest {
                                    database: db,
                                    generation,
                                    window_handle,
                                    entity: handle,
                                },
                                cx,
                            )
                            .await;
                    })
                    .detach();
                }
            },
        )
        .detach();

        cx.subscribe_in(
            &self.schema_select,
            window,
            |this, _select, event: &SelectEvent<SearchableVec<String>>, window, cx| {
                let global_state = cx.global::<GlobalDbState>().clone();
                if let SelectEvent::Confirm(Some(schema_name)) = event {
                    let generation = this.next_context_generation(cx);
                    let window_handle = window.window_handle();
                    let database_or_schema = if this.uses_schema_as_database {
                        Some(schema_name.clone())
                    } else {
                        this.database_select.read(cx).selected_value().cloned()
                    };
                    if let Some(db) = database_or_schema {
                        let instance = this.clone();
                        cx.spawn(async move |handle, cx| {
                            instance
                                .update_schema_for_db(
                                    global_state,
                                    SqlSchemaUpdateRequest {
                                        database: db,
                                        generation,
                                        window_handle,
                                        entity: handle,
                                    },
                                    cx,
                                )
                                .await;
                        })
                        .detach();
                    }
                }
            },
        )
        .detach();
    }

    fn clear_string_select(
        select: &Entity<SelectState<SearchableVec<String>>>,
        window: &mut Window,
        cx: &mut App,
    ) {
        select.update(cx, |state, cx| {
            state.set_items(SearchableVec::new(Vec::new()), window, cx);
            state.set_selected_index(None, window, cx);
        });
    }

    fn next_context_generation(&self, cx: &mut Context<Self>) -> u64 {
        let generation = self.context_generation.fetch_add(1, Ordering::SeqCst) + 1;
        *self.schema_snapshot.write() = SqlSchema::default();
        self.foreign_prefetch_inflight.lock().clear();
        self.editor.update(cx, |editor, cx| {
            editor.invalidate_metadata_context(cx);
        });
        generation
    }

    fn has_manual_transaction_lifecycle(&self) -> bool {
        self.manual_transaction.is_some()
            || self.manual_transaction_starting
            || self.manual_transaction_finishing
    }

    fn refresh_statement_snapshot(&mut self, cx: &mut Context<Self>) {
        let snapshot =
            SqlStatementSnapshot::new(self.get_sql_text(cx), SqlDialect::from(&self.database_type));
        let revision = self.editor.read(cx).document_revision(cx);

        // The full snapshot is authoritative again: drop any windowed scan so
        // display consumers fall back to it until the debounced window
        // refresh rebuilds one.
        self.viewport_statements = None;
        self.retain_current_revision_markers(revision);
        self.statement_snapshot = snapshot;
        self.set_statement_gutter_markers(cx);
        self.refresh_current_statement_frame(cx);
        self.refresh_insert_value_hints(cx);
    }

    fn retain_current_revision_markers(&mut self, revision: u64) {
        let prefix = format!("sql-statement:{revision}:");
        self.statement_marker_states
            .retain(|id, _| id.starts_with(&prefix));
        if self
            .active_statement_marker
            .as_ref()
            .is_some_and(|id| !id.starts_with(&prefix))
        {
            self.active_statement_marker = None;
        }
    }

    fn set_statement_gutter_markers(&self, cx: &mut Context<Self>) {
        let revision = self.editor.read(cx).document_revision(cx);
        let ranges: &[SqlStatementRange] = match &self.viewport_statements {
            Some(viewport) => viewport.scan.statement_ranges(),
            None => self.statement_snapshot.statement_ranges(),
        };
        let markers = ranges
            .iter()
            .map(|statement| {
                let id = statement_marker_id(revision, statement);
                let icon = self
                    .statement_marker_states
                    .get(&id)
                    .copied()
                    .map(SqlGutterMarkerState::icon_token)
                    .unwrap_or(SQL_GUTTER_IDLE);
                GutterMarker::new(id, statement.start_line, icon)
                    .with_tooltip(t!("Query.run_cursor_statement").to_string())
            })
            .collect();
        self.editor
            .read(cx)
            .input()
            .update(cx, |state, cx| state.set_gutter_markers(markers, cx));
    }

    fn statement_index_for_document(
        &self,
        revision: u64,
        document: &str,
    ) -> Option<&dyn StatementIndex> {
        match &self.viewport_statements {
            Some(viewport) if viewport.revision == revision => Some(&viewport.scan),
            Some(_) => None,
            None if self.statement_snapshot.text() == document => Some(&self.statement_snapshot),
            None => None,
        }
    }

    /// Keep the current-statement frame decoration in sync with the cursor.
    ///
    /// Called from the snapshot refresh (document edits) and from an observer
    /// on the editor input (cursor/selection movement). Caches the last
    /// cursor/selection so notifying observers never loop.
    fn refresh_current_statement_frame(&mut self, cx: &mut Context<Self>) {
        let sql_editor = self.editor.read(cx);
        let revision = sql_editor.document_revision(cx);
        let cursor = sql_editor.cursor_offset(cx);
        let selection = sql_editor.selected_range(cx);
        let frame_key = (
            revision,
            cursor,
            selection.clone(),
            self.insert_values_highlight.clone(),
        );
        if self.last_frame_key.as_ref() == Some(&frame_key) {
            return;
        }

        let document = self.get_sql_text(cx);
        let Some(index) = self.statement_index_for_document(revision, &document) else {
            self.editor
                .read(cx)
                .input()
                .update(cx, |state, cx| state.clear_range_decorations(cx));
            self.last_frame_key = None;
            return;
        };
        let decorations = current_statement_frame_decorations(
            index,
            revision,
            cursor,
            &selection,
            document.len(),
            self.insert_values_highlight.clone(),
        )
        .into_iter()
        .map(|decoration| match decoration.style() {
            RangeDecorationStyle::Fill => decoration.with_color(cx.theme().primary.opacity(0.07)),
            RangeDecorationStyle::Frame => decoration.with_color(cx.theme().primary),
        })
        .collect();
        self.editor
            .read(cx)
            .input()
            .update(cx, |state, cx| state.set_range_decorations(decorations, cx));
        self.last_frame_key = Some(frame_key);
    }

    /// Compute INSERT value slots for the statement under the cursor and
    /// highlight the values region (spec §14).
    ///
    /// Only the cursor's current statement is analyzed, so large documents stay
    /// cheap. Cached by (statement start, revision): moving the cursor within
    /// the same statement is a no-op, while editing recomputes the offsets.
    fn refresh_insert_value_hints(&mut self, cx: &mut Context<Self>) {
        let (cursor, revision) = {
            let sql_editor = self.editor.read(cx);
            (
                sql_editor.cursor_offset(cx),
                sql_editor.document_revision(cx),
            )
        };

        let doc = self.get_sql_text(cx);
        let Some((statement_start, statement_text)) = self
            .statement_index_for_document(revision, &doc)
            .map(|index| {
                index
                    .statement_at_cursor(cursor, doc.len())
                    .map(|statement| {
                        let start = statement.sql_range.start_byte.min(doc.len());
                        let end = statement.sql_range.end_byte.min(doc.len()).max(start);
                        (start, doc[start..end].to_string())
                    })
                    .unwrap_or((0, String::new()))
            })
        else {
            self.insert_values_highlight = None;
            self.last_insert_hints_key = None;
            self.editor
                .read(cx)
                .input()
                .update(cx, |state, cx| state.clear_inline_widgets(cx));
            return;
        };
        if self.last_insert_hints_key == Some((statement_start, revision)) {
            return;
        }
        self.last_insert_hints_key = Some((statement_start, revision));

        let schema = self.schema_snapshot.read().clone();
        let ordinal_columns = insert_target_table(&statement_text)
            .and_then(|table| lookup_table_columns(&schema, &table))
            .unwrap_or_default();

        let has_value_slots = !insert_value_hints(&statement_text, &ordinal_columns).is_empty();
        let values_highlight = has_value_slots
            .then(|| insert_values_range(&statement_text))
            .flatten()
            .map(|range| statement_start + range.start..statement_start + range.end);

        self.insert_values_highlight = values_highlight;

        self.editor
            .read(cx)
            .input()
            .update(cx, |state, cx| state.clear_inline_widgets(cx));
        self.refresh_current_statement_frame(cx);
    }

    /// Schedule a statement-snapshot refresh after the document settles.
    ///
    /// Each document change bumps a run id; the debounced task coalesces rapid
    /// typing and only proceeds if its run id is still current. The heavy
    /// tokenize/statement-range pass runs on a background worker, then the
    /// result is applied back on the UI thread guarded by both the run id and
    /// the document revision (spec §12.6). Execution paths (gutter click, run
    /// current/cursor statement, connection switch) intentionally keep using
    /// the synchronous `refresh_statement_snapshot` so they always act on the
    /// freshest snapshot.
    fn schedule_statement_snapshot_refresh(&mut self, cx: &mut Context<Self>) {
        const STATEMENT_DEBOUNCE_MS: u64 = 24;

        let run_id = self.statement_run_id.fetch_add(1, Ordering::SeqCst) + 1;
        let run_id_clone = self.statement_run_id.clone();
        let context_generation = self.context_generation.load(Ordering::SeqCst);
        let context_generation_clone = self.context_generation.clone();
        let task = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            Timer::after(Duration::from_millis(STATEMENT_DEBOUNCE_MS)).await;
            if run_id_clone.load(Ordering::SeqCst) != run_id
                || context_generation_clone.load(Ordering::SeqCst) != context_generation
            {
                return;
            }
            // Capture either the whole document (small/unlaid-out editors) or
            // a viewport-centered Rope slice. Only the selected slice is
            // converted to String on the UI thread for large documents.
            let Some((scan_input, revision, dialect)) = this
                .read_with(cx, |this, cx| {
                    let input_entity = this.editor.read(cx).input();
                    let input = input_entity.read(cx);
                    let revision = this.editor.read(cx).document_revision(cx);
                    let dialect = SqlDialect::from(&this.database_type);
                    let scan_input =
                        viewport_statement_scan_input(input.text(), input.visible_row_range());
                    (scan_input, revision, dialect)
                })
                .ok()
            else {
                return;
            };

            let heavy = cx.background_spawn(async move {
                match scan_input {
                    StatementScanInput::Full { text } => {
                        StatementScanResult::Full(SqlStatementSnapshot::new(text, dialect))
                    }
                    StatementScanInput::Window {
                        text,
                        base_byte,
                        base_line,
                        analyzed_rows,
                    } => StatementScanResult::Window {
                        scan: WindowedStatementScan::scan(text, dialect, base_byte, base_line),
                        analyzed_rows,
                    },
                }
            });
            let result = heavy.await;
            if run_id_clone.load(Ordering::SeqCst) != run_id
                || context_generation_clone.load(Ordering::SeqCst) != context_generation
            {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                let current_revision = this.editor.read(cx).document_revision(cx);
                if current_revision != revision
                    || !this.is_context_generation_current(context_generation)
                {
                    return;
                }
                this.retain_current_revision_markers(revision);
                match result {
                    StatementScanResult::Full(snapshot) => {
                        this.statement_snapshot = snapshot;
                        this.viewport_statements = None;
                    }
                    StatementScanResult::Window {
                        scan,
                        analyzed_rows,
                    } => {
                        this.viewport_statements = Some(ViewportStatements {
                            revision,
                            analyzed_rows,
                            scan,
                        });
                    }
                }
                this.set_statement_gutter_markers(cx);
                this.refresh_current_statement_frame(cx);
                this.refresh_insert_value_hints(cx);
            });
        });
        self._statement_task = Some(task);
    }

    /// Observe editor input notification so cursor/selection movement refreshes
    /// the current-statement frame without requiring a document change.
    ///
    /// Also refreshes INSERT value hints and signature help, which depend on
    /// the cursor's current statement/call.
    fn bind_editor_input_observe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editor_input = self.editor.read(cx).input();
        self._editor_input_subscription =
            Some(
                cx.observe_in(&editor_input, window, |this, _editor_input, _window, cx| {
                    if this.viewport_statement_scan_is_stale(cx) {
                        this.schedule_statement_snapshot_refresh(cx);
                    }
                    this.refresh_current_statement_frame(cx);
                    this.refresh_insert_value_hints(cx);
                }),
            );
    }

    fn viewport_statement_scan_is_stale(&self, cx: &App) -> bool {
        let Some(viewport) = &self.viewport_statements else {
            return false;
        };
        let input_entity = self.editor.read(cx).input();
        let input = input_entity.read(cx);
        if viewport.revision != self.editor.read(cx).document_revision(cx) {
            return true;
        }
        input.visible_row_range().is_some_and(|visible| {
            visible.start < viewport.analyzed_rows.start || visible.end > viewport.analyzed_rows.end
        })
    }

    /// Observe the container execution state so gutter markers can reflect
    /// running/success/error/cancel for the bound statement.
    fn bind_execution_marker_event(&mut self, cx: &mut Context<Self>) {
        let execution_state = self
            .sql_result_tab_container
            .read(cx)
            .execution_state
            .clone();
        self._execution_state_subscription =
            Some(cx.observe(&execution_state, |this, _execution_state, cx| {
                this.handle_execution_state_changed(cx);
            }));
    }

    fn handle_execution_state_changed(&mut self, cx: &mut Context<Self>) {
        let state = self
            .sql_result_tab_container
            .read(cx)
            .execution_state
            .read(cx)
            .clone();
        match state {
            ExecutionState::Executing { .. } => {
                // Re-assert running for run paths that launched without binding
                // (e.g. toolbar run); harmless when already Running.
                if let Some(id) = self.active_statement_marker.clone() {
                    self.statement_marker_states
                        .insert(id, SqlGutterMarkerState::Running);
                    self.set_statement_gutter_markers(cx);
                }
            }
            ExecutionState::Completed => {
                let failed = self
                    .sql_result_tab_container
                    .read(cx)
                    .all_results
                    .read(cx)
                    .iter()
                    .any(|result| result.is_error());
                let state = if failed {
                    SqlGutterMarkerState::Failed
                } else {
                    SqlGutterMarkerState::Succeeded
                };
                self.finalize_execution_marker(state, cx);
            }
            ExecutionState::Cancelled => {
                self.finalize_execution_marker(SqlGutterMarkerState::Cancelled, cx);
            }
            ExecutionState::Idle => {
                // Transport-layer failures reset the state to Idle without
                // producing a result; an active marker means the run failed.
                self.finalize_execution_marker(SqlGutterMarkerState::Failed, cx);
            }
        }
    }

    fn finalize_execution_marker(&mut self, state: SqlGutterMarkerState, cx: &mut Context<Self>) {
        if let Some(id) = self.active_statement_marker.take() {
            self.statement_marker_states.insert(id, state);
            self.set_statement_gutter_markers(cx);
        }
    }

    /// Bind the exact statement under the cursor to the in-flight execution.
    ///
    /// Single-statement run paths (gutter click, run current, run cursor
    /// statement) execute exactly `statement_text`, so the marker id is bound
    /// to Running and finalized by the execution-state observer. Multi-statement
    /// or selection runs conservatively leave markers untouched.
    fn bind_execution_marker_for_sql(&mut self, sql: &str, cx: &mut Context<Self>) {
        let cursor = self.editor.read(cx).cursor_offset(cx);
        let marker_id = match_sql_to_statement_marker(
            &self.statement_snapshot,
            self.editor.read(cx).document_revision(cx),
            cursor,
            sql,
        );

        match marker_id {
            Some(id) => {
                self.statement_marker_states
                    .insert(id.clone(), SqlGutterMarkerState::Running);
                self.active_statement_marker = Some(id);
                self.set_statement_gutter_markers(cx);
            }
            None => self.active_statement_marker = None,
        }
    }

    fn is_context_generation_current(&self, generation: u64) -> bool {
        is_current_query_context_generation(
            generation,
            self.context_generation.load(Ordering::SeqCst),
        )
    }

    fn is_metadata_scope_current(&self, scope: &SqlMetadataScope, cx: &AsyncApp) -> bool {
        self.is_context_generation_current(scope.generation)
            && self
                .current_metadata_scope(scope.generation, scope.database.as_deref(), cx)
                .as_ref()
                == Some(scope)
    }

    /// Resolve the current completion metadata scope.
    ///
    /// For databases that use schema as database (notably Oracle), the schema
    /// value is the effective database/catalog.
    fn current_metadata_scope(
        &self,
        generation: u64,
        database: Option<&str>,
        cx: &impl AppContext,
    ) -> Option<SqlMetadataScope> {
        let selected_database = if self.uses_schema_as_database || database.is_some() {
            None
        } else {
            self.database_select
                .read_with(cx, |state, _| state.selected_value().cloned())
        };
        let selected_schema = if self.uses_schema_as_database || self.supports_schema {
            self.schema_select
                .read_with(cx, |state, _| state.selected_value().cloned())
        } else {
            None
        };
        let (database, schema) = metadata_scope_selection(
            database,
            selected_database,
            selected_schema,
            self.supports_schema,
            self.uses_schema_as_database,
        );

        Some(
            SqlMetadataScope::new(
                self.connection_id.clone(),
                self.database_type.clone(),
                generation,
            )
            .with_database(database)
            .with_schema(schema),
        )
    }

    fn restore_connection_selection(&self, window: &mut Window, cx: &mut App) {
        self.connection_select.update(cx, |state, cx| {
            state.set_selected_value(&self.connection_id, window, cx);
        });
    }

    fn switch_connection(
        &mut self,
        connection_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if connection_id == self.connection_id {
            return;
        }

        let is_executing = self.sql_result_tab_container.read(cx).is_executing(cx);
        let has_manual_transaction = self.has_manual_transaction_lifecycle();
        if !can_switch_query_connection(is_executing, has_manual_transaction) {
            self.restore_connection_selection(window, cx);
            let message = if has_manual_transaction {
                t!("Query.transaction_finish_before_switch_connection").to_string()
            } else {
                t!("Query.connection_switch_during_execution").to_string()
            };
            window.push_notification(message, cx);
            return;
        }

        let global_state = cx.global::<GlobalDbState>().clone();
        let Some(connection) = global_state.get_config(connection_id) else {
            self.restore_connection_selection(window, cx);
            window.push_notification(t!("Query.connection_unavailable").to_string(), cx);
            return;
        };

        let previous_connection_id = self.connection_id.clone();
        let generation = self.next_context_generation(cx);
        let capabilities = global_state.capabilities(&connection.database_type);
        self.connection_id = connection_id.to_string();
        self.database_type = connection.database_type.clone();
        self.supports_schema = capabilities.supports_schema;
        self.uses_schema_as_database = capabilities.uses_schema_as_database;
        self.statement_run_id.fetch_add(1, Ordering::SeqCst);
        self.refresh_statement_snapshot(cx);
        cx.emit(TabContentEvent::SourceChanged {
            from: previous_connection_id.into(),
        });

        Self::clear_string_select(&self.database_select, window, cx);
        Self::clear_string_select(&self.schema_select, window, cx);

        if !supports_manual_transactions(&self.database_type) {
            self.transaction_mode = SqlTransactionMode::Auto;
            self.transaction_mode_select.update(cx, |state, cx| {
                state.set_selected_value(&SqlTransactionMode::Auto, window, cx);
            });
        }

        let completion_info = DbManager::default()
            .get_plugin(&self.database_type)
            .map(|plugin| plugin.get_completion_info())
            .unwrap_or_default();
        self.editor.update(cx, |editor, cx| {
            editor.set_db_completion_info(completion_info, SqlSchema::default(), cx);
        });
        self.sql_result_tab_container
            .update(cx, |container, cx| container.hide(cx));

        self.load_databases_async(
            None,
            None,
            self.get_file_path(),
            false,
            generation,
            cx,
            window,
        );
        cx.notify();
    }

    fn bind_transaction_mode_select_event(&self, window: &mut Window, cx: &mut Context<Self>) {
        cx.subscribe_in(
            &self.transaction_mode_select,
            window,
            |this,
             _select,
             event: &SelectEvent<SearchableVec<TransactionModeOption>>,
             window,
             cx| {
                if let SelectEvent::Confirm(Some(mode)) = event {
                    if this.has_manual_transaction_lifecycle() && *mode != this.transaction_mode {
                        window.push_notification(
                            t!("Query.transaction_finish_before_switch").to_string(),
                            cx,
                        );
                        return;
                    }
                    this.transaction_mode = *mode;
                    cx.notify();
                }
            },
        )
        .detach();
    }

    /// 绑定自动保存功能
    /// 监听编辑器内容变化，当内容变化时启动防抖计时器进行自动保存
    fn bind_auto_save(
        &self,
        auto_save_seq: Arc<AtomicU64>,
        is_dirty: Arc<AtomicBool>,
        requires_name: Arc<AtomicBool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor_input = self.editor.read(cx).input();
        let file_path = self.file_path.clone();
        let editor_entity = self.editor.clone();
        let window_handle = window.window_handle();

        cx.subscribe_in(
            &editor_input,
            window,
            move |this, _input, event: &InputEvent, _window, cx| {
                if let InputEvent::Change = event {
                    this.schedule_statement_snapshot_refresh(cx);
                    this.schedule_diagnostics(cx);
                    // 跨库/跨 schema 限定名引用的元数据懒加载
                    this.schedule_foreign_schema_prefetch(window_handle, cx);
                    // 标记为已修改
                    is_dirty.store(true, Ordering::Relaxed);

                    // 检查自动保存是否启用
                    let auto_save_config = cx.try_global::<AutoSaveConfig>();
                    let (enabled, interval_ms) = match auto_save_config {
                        Some(config) => (config.is_enabled(), config.interval_ms()),
                        None => (true, 5000), // 默认值：启用，5秒间隔
                    };

                    if !enabled {
                        return;
                    }

                    // 增加序列号以取消之前的保存任务
                    let my_seq = auto_save_seq.fetch_add(1, Ordering::SeqCst) + 1;
                    let seq_clone = auto_save_seq.clone();
                    let dirty_clone = is_dirty.clone();
                    let file_path_clone = file_path.clone();
                    let requires_name_clone = requires_name.clone();
                    let editor_clone = editor_entity.clone();

                    // 启动防抖定时保存
                    cx.spawn(async move |_handle, cx| {
                        // 等待指定间隔
                        Timer::after(Duration::from_millis(interval_ms)).await;

                        // 检查是否被更新的请求取代
                        if seq_clone.load(Ordering::SeqCst) != my_seq {
                            return;
                        }

                        // 检查是否有未保存的修改
                        if !dirty_clone.load(Ordering::Relaxed) {
                            return;
                        }

                        if requires_name_clone.load(Ordering::Relaxed) {
                            return;
                        }

                        // 执行保存
                        let _ = cx.update(|cx| {
                            let sql = editor_clone.read(cx).get_text(cx);
                            if sql.trim().is_empty() {
                                return;
                            }

                            let file_path = file_path_clone.read().clone();

                            // 写入文件
                            if let Err(e) = write_sql_file(&file_path, &sql) {
                                error!(
                                    "{}",
                                    t!(
                                        "SqlEditorView.auto_save_failed",
                                        path = format!("{:?}", file_path),
                                        error = e
                                    )
                                );
                            } else {
                                // 保存成功，清除脏标记
                                dirty_clone.store(false, Ordering::Relaxed);
                            }
                        });
                    })
                    .detach();
                }
            },
        )
        .detach();
    }

    /// Schedule a SQL diagnostics analysis after the document settles.
    ///
    /// Each document change bumps a run id; the debounced task only proceeds
    /// if its run id is still current, so stale analyses from earlier edits
    /// (or a deactivated tab) are discarded (spec §12.6).
    fn schedule_diagnostics(&mut self, cx: &mut Context<Self>) {
        const DIAGNOSTIC_DEBOUNCE_MS: u64 = 500;

        let run_id = self.diagnostic_run_id.fetch_add(1, Ordering::SeqCst) + 1;
        let run_id_clone = self.diagnostic_run_id.clone();
        let instance = self.clone();
        let task = cx.spawn(async move |_handle, cx| {
            Timer::after(Duration::from_millis(DIAGNOSTIC_DEBOUNCE_MS)).await;
            if run_id_clone.load(Ordering::SeqCst) != run_id {
                return;
            }
            cx.update(|cx| instance.run_diagnostics_with_id(run_id, cx));
        });
        self._diagnostic_task = Some(task);
    }

    /// Analyze the current document and publish diagnostics into the editor
    /// squiggle layer. The heavy tokenizer/semantic passes run on a background
    /// worker; only the result publication happens back on the UI thread.
    /// Stale results (document moved on since the analysis started) are dropped
    /// (spec §12.6 stale guard).
    fn run_diagnostics(&self, cx: &App) {
        let run_id = self.diagnostic_run_id.fetch_add(1, Ordering::SeqCst) + 1;
        self.run_diagnostics_with_id(run_id, cx);
    }

    fn run_diagnostics_with_id(&self, run_id: u64, cx: &App) {
        let dialect = SqlDialect::from(&self.database_type);
        let document_revision = self.editor.read(cx).document_revision(cx);
        let identity = SqlDiagnosticIdentity {
            run_id,
            document_revision,
            context_generation: self.context_generation.load(Ordering::SeqCst),
        };
        Self::refresh_diagnostics_async(
            &self.editor,
            dialect,
            identity,
            self.diagnostic_run_id.clone(),
            self.context_generation.clone(),
            cx,
        );
    }

    /// Shared diagnostics publication: capture `editor`'s current content,
    /// revision and schema snapshot, analyze them on a background worker, then
    /// write the result into the squiggle layer back on the UI thread. Called
    /// after the debounce (document edits) and immediately after a
    /// schema/database refresh.
    fn refresh_diagnostics_async(
        editor: &Entity<SqlEditor>,
        dialect: SqlDialect,
        identity: SqlDiagnosticIdentity,
        diagnostic_run_id: Arc<AtomicU64>,
        context_generation: Arc<AtomicU64>,
        cx: &App,
    ) {
        let weak = editor.downgrade();
        let heavy = editor.read(cx).analyze_diagnostics_async(cx, dialect);
        cx.spawn(async move |cx: &mut AsyncApp| {
            let snapshot = heavy.await;
            let _ = weak.update(cx, |e, cx| {
                let input = e.input();
                let current_revision = e.document_revision(cx);
                let current_identity = SqlDiagnosticIdentity {
                    run_id: diagnostic_run_id.load(Ordering::SeqCst),
                    document_revision: current_revision,
                    context_generation: context_generation.load(Ordering::SeqCst),
                };
                if snapshot.document_revision != identity.document_revision
                    || !is_current_diagnostic_identity(identity, current_identity)
                {
                    return;
                }
                input.update(cx, |state, _| {
                    let text = state.text().clone();
                    if let Some(diags) = state.diagnostics_mut() {
                        diags.reset(&text);
                        diags.extend(snapshot.diagnostics);
                    }
                });
            });
        })
        .detach();
    }

    fn bind_gutter_marker_event(&self, window: &mut Window, cx: &mut Context<Self>) {
        let editor_input = self.editor.read(cx).input();
        cx.subscribe_in(
            &editor_input,
            window,
            |this, _, event: &InputEvent, window, cx| {
                let InputEvent::GutterMarkerMouseDown {
                    marker_id,
                    logical_row,
                } = event
                else {
                    return;
                };
                let revision = this.editor.read(cx).document_revision(cx);
                let Some(statement) = statement_for_gutter_marker(
                    this.statement_snapshot.statement_ranges(),
                    revision,
                    marker_id,
                    *logical_row,
                ) else {
                    return;
                };
                let sql = this
                    .statement_snapshot
                    .statement_text(statement)
                    .to_string();
                this.execute_sql_text(sql, window, cx);
            },
        )
        .detach();
    }

    fn bind_connection_data_event(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(notifier) = cx.try_global::<GlobalConnectionNotifier>().cloned() else {
            self._connection_subscription = None;
            return;
        };

        self._connection_subscription = Some(cx.subscribe_in(
            &notifier.0,
            window,
            |this, _notifier, event: &ConnectionDataEvent, window, cx| {
                this.handle_connection_data_event(event, window, cx);
            },
        ));
    }

    fn handle_connection_data_event(
        &mut self,
        event: &ConnectionDataEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ConnectionDataEvent::SchemaChanged {
            connection_id,
            database,
            schema,
        } = event
        else {
            return;
        };

        let current_generation = self.context_generation.load(Ordering::SeqCst);
        let Some(current_scope) = self.current_metadata_scope(current_generation, None, cx) else {
            return;
        };
        if !schema_changed_event_matches_scope(
            connection_id,
            database,
            schema.as_deref(),
            &current_scope,
            self.supports_schema,
            self.uses_schema_as_database,
        ) {
            return;
        }

        let target_database = if self.uses_schema_as_database {
            current_scope.schema.clone()
        } else {
            current_scope.database.clone()
        };
        let Some(target_database) = target_database else {
            return;
        };

        let global_state = cx.global::<GlobalDbState>().clone();
        let generation = self.next_context_generation(cx);
        let window_handle = window.window_handle();
        let instance = self.clone();
        cx.spawn(async move |handle, cx| {
            instance
                .update_schema_for_db(
                    global_state,
                    SqlSchemaUpdateRequest {
                        database: target_database,
                        generation,
                        window_handle,
                        entity: handle,
                    },
                    cx,
                )
                .await;
        })
        .detach();
    }

    /// Load schemas for a database
    async fn load_schemas_for_db(
        &self,
        global_state: GlobalDbState,
        database: &str,
        initial_schema: Option<String>,
        generation: u64,
        window_handle: AnyWindowHandle,
        cx: &mut AsyncApp,
    ) {
        let Some(scope) = self.current_metadata_scope(generation, Some(database), cx) else {
            return;
        };

        let connection_id = self.connection_id.clone();
        let schema_select = self.schema_select.clone();
        let context_generation = self.context_generation.clone();
        let db = database.to_string();

        let schemas = match global_state
            .list_schemas(cx, connection_id.clone(), db.clone())
            .await
        {
            Ok(result) => result,
            Err(e) => {
                error!("Failed to load schemas for {}: {}", db, e);
                return;
            }
        };
        if !self.is_metadata_scope_current(&scope, cx) {
            return;
        }

        let this = self.clone();
        let _ = cx.update_window(window_handle, move |_entity, window, cx| {
            if !is_current_query_context_generation(
                generation,
                context_generation.load(Ordering::SeqCst),
            ) {
                return;
            }
            if this
                .current_metadata_scope(generation, Some(&db), cx)
                .as_ref()
                != Some(&scope)
            {
                return;
            }
            schema_select.update(cx, |state, cx| {
                if schemas.is_empty() {
                    let items = SearchableVec::new(vec![
                        t!("Common.no_available", item = &t!("Schema.schema")).to_string(),
                    ]);
                    state.set_items(items, window, cx);
                    state.set_selected_index(None, window, cx);
                } else {
                    let items = SearchableVec::new(schemas.clone());
                    state.set_items(items, window, cx);

                    if let Some(schema_name) = initial_schema.as_ref() {
                        if let Some(index) = schemas.iter().position(|s| s == schema_name) {
                            state.set_selected_index(Some(IndexPath::new(index)), window, cx);
                        } else {
                            state.set_selected_index(Some(IndexPath::new(0)), window, cx);
                        }
                    } else {
                        state.set_selected_index(Some(IndexPath::new(0)), window, cx);
                    }
                }
            });
        });
    }

    pub fn set_sql(&self, sql: String, window: &mut Window, cx: &mut App) {
        self.editor.update(cx, |e, cx| e.set_value(sql, window, cx));
    }

    /// Load databases into the select dropdown
    fn load_databases_async(
        &self,
        init_db: Option<String>,
        init_schema: Option<String>,
        file_path: PathBuf,
        should_load_file: bool,
        generation: u64,
        cx: &mut Context<Self>,
        window: &mut Window,
    ) {
        let window_handle = window.window_handle();
        let global_state = cx.global::<GlobalDbState>().clone();
        let connection_id = self.connection_id.clone();
        let database_select = self.database_select.clone();
        let schema_select = self.schema_select.clone();
        let editor = self.editor.clone();
        let initial_database = init_db.clone();
        let instance = self.clone();
        let context_generation = self.context_generation.clone();
        let uses_schema_as_database = self.uses_schema_as_database;

        cx.spawn(async move |handle, cx: &mut AsyncApp| {
            if !instance.is_context_generation_current(generation) {
                return;
            }

            let select_items = if uses_schema_as_database {
                match global_state
                    .list_schemas(cx, connection_id.clone(), String::new())
                    .await
                {
                    Ok(result) => result,
                    Err(e) => {
                        error!("Failed to load schemas for {}: {}", connection_id, e);
                        if instance.is_context_generation_current(generation) {
                            Self::notify_async(cx, format!("Failed to load schemas: {}", e));
                        }
                        return;
                    }
                }
            } else {
                match global_state.list_databases(cx, connection_id.clone()).await {
                    Ok(result) => result,
                    Err(e) => {
                        error!("Failed to load databases for {}: {}", connection_id, e);
                        if instance.is_context_generation_current(generation) {
                            Self::notify_async(cx, format!("Failed to load databases: {}", e));
                        }
                        return;
                    }
                }
            };
            if !instance.is_context_generation_current(generation) {
                return;
            }

            let sql_content = if should_load_file && file_path.exists() {
                match std::fs::read_to_string(&file_path) {
                    Ok(content) => Some(content),
                    Err(e) => {
                        error!("Failed to read SQL file {:?}: {}", file_path, e);
                        None
                    }
                }
            } else {
                None
            };
            if !instance.is_context_generation_current(generation) {
                return;
            }

            // issue #125：未显式指定数据库时，默认选中与连接登录配置一致的数据库，
            // 而不是任意取列表第一项；登录数据库不在可选列表中时才回退到第一项。
            let login_database = if uses_schema_as_database {
                None
            } else {
                global_state
                    .get_config(&connection_id)
                    .and_then(|config| config.database)
            };
            let selected_name = initial_database
                .clone()
                .or_else(|| preferred_default_database(login_database, &select_items))
                .or_else(|| select_items.first().cloned());
            let resolved_database = selected_name.clone();

            let _ = cx.update_window(window_handle, |_entity, window, cx| {
                if !is_current_query_context_generation(
                    generation,
                    context_generation.load(Ordering::SeqCst),
                ) {
                    return;
                }
                let target_select = if uses_schema_as_database {
                    schema_select.clone()
                } else {
                    database_select.clone()
                };
                let empty_label = if uses_schema_as_database {
                    t!("Schema.schema").to_string()
                } else {
                    t!("Database.database").to_string()
                };
                target_select.update(cx, |state, cx| {
                    set_select_items_with_initial_value(
                        state,
                        select_items.clone(),
                        selected_name.as_deref(),
                        empty_label,
                        window,
                        cx,
                    );
                });
                if let Some(sql) = sql_content {
                    editor.update(cx, |e, cx| {
                        e.set_value(sql.clone(), window, cx);
                    });
                }
            });

            if !instance.is_context_generation_current(generation) {
                return;
            }
            if let Some(ref db) = resolved_database {
                if instance.supports_schema && !instance.uses_schema_as_database {
                    instance
                        .load_schemas_for_db(
                            global_state.clone(),
                            db,
                            init_schema,
                            generation,
                            window_handle,
                            cx,
                        )
                        .await;
                }
                if instance.is_context_generation_current(generation) {
                    instance
                        .update_schema_for_db(
                            global_state,
                            SqlSchemaUpdateRequest {
                                database: db.clone(),
                                generation,
                                window_handle,
                                entity: handle,
                            },
                            cx,
                        )
                        .await;
                }
            }
        })
        .detach();
    }

    /// Update SQL editor schema with tables and columns from current database
    async fn update_schema_for_db(
        &self,
        global_state: GlobalDbState,
        request: SqlSchemaUpdateRequest,
        cx: &mut AsyncApp,
    ) {
        let SqlSchemaUpdateRequest {
            database,
            generation,
            window_handle,
            entity,
        } = request;
        let connection_id = self.connection_id.clone();
        let Some(scope) = self.current_metadata_scope(generation, Some(&database), cx) else {
            return;
        };
        let (db, selected_schema) = (
            scope.database.clone().unwrap_or_default(),
            scope.schema.clone(),
        );

        let tables = match global_state
            .list_tables(
                cx,
                connection_id.clone(),
                db.clone(),
                selected_schema.clone(),
            )
            .await
        {
            Ok(result) => result,
            Err(e) => {
                eprintln!("Failed to get tables: {}", e);
                return;
            }
        };
        if !self.is_metadata_scope_current(&scope, cx) {
            return;
        }

        // Get database-specific completion info
        let db_completion_info = match global_state.get_completion_info(cx, connection_id.clone()) {
            Ok(info) => info,
            Err(e) => {
                eprintln!("Failed to get completion info: {}", e);
                return;
            }
        };
        if !self.is_metadata_scope_current(&scope, cx) {
            return;
        }

        let mut schema = SqlSchema::default();
        schema = schema.with_scope(scope.database.clone(), scope.schema.clone());

        // Add tables to schema
        let table_items: Vec<(String, String)> = tables
            .iter()
            .map(|t| {
                let description = if let Some(comment) = &t.comment {
                    format!("Table: {} - {}", t.name, comment)
                } else {
                    format!("Table: {}", t.name)
                };
                (t.name.clone(), description)
            })
            .collect();
        schema = schema.with_tables(table_items);

        // 表名先发布：大 schema 的逐表列扫描耗时较长，先让表名补全可用，
        // 列/函数/qualifier 等完整元数据由随后的终态发布补齐。
        let early_scope = scope.clone();
        let early_schema = schema.clone();
        let early_info = db_completion_info.clone();
        let early_db = db.clone();
        let early_entity = entity.clone();
        let _ = cx.update_window(window_handle, move |_view, _window, cx| {
            let _ = early_entity.update(cx, |this, cx| {
                if this
                    .current_metadata_scope(generation, Some(&early_db), cx)
                    .as_ref()
                    != Some(&early_scope)
                {
                    return;
                }
                this.editor.update(cx, |editor, cx| {
                    editor.set_db_completion_info(early_info, early_schema, cx);
                });
            });
        });
        if !self.is_metadata_scope_current(&scope, cx) {
            return;
        }

        // Load columns for each table with bounded concurrency instead of a
        // serial full-schema catalog scan. Results arrive in completion order,
        // while the original table index keeps duplicate names correctly
        // associated with their metadata record.
        let column_results = collect_bounded(
            tables
                .iter()
                .enumerate()
                .map(|(table_index, table)| (table_index, table.name.clone())),
            SCHEMA_COLUMN_FETCH_CONCURRENCY,
            |(table_index, table_name)| {
                let mut cx = cx.clone();
                let global_state = global_state.clone();
                let connection_id = connection_id.clone();
                let db = db.clone();
                let selected_schema = selected_schema.clone();
                async move {
                    let columns = global_state
                        .list_columns(
                            &mut cx,
                            connection_id,
                            db,
                            selected_schema,
                            table_name.clone(),
                        )
                        .await;
                    (table_index, columns)
                }
            },
        )
        .await;
        if !self.is_metadata_scope_current(&scope, cx) {
            return;
        }
        for (table_index, columns) in column_results {
            let Some(table) = tables.get(table_index) else {
                continue;
            };
            if let Ok(columns) = columns {
                let column_items: Vec<(String, String, String)> = columns
                    .iter()
                    .map(|c| {
                        (
                            c.name.clone(),
                            c.data_type.clone(),
                            c.comment.as_ref().unwrap_or(&String::new()).clone(),
                        )
                    })
                    .collect();
                schema = schema.with_table_columns_typed(&table.name, column_items);
                let detail_columns: Vec<SqlColumnDetail> = columns
                    .iter()
                    .map(|c| SqlColumnDetail {
                        name: c.name.clone(),
                        data_type: c.data_type.clone(),
                        is_nullable: c.is_nullable,
                        is_primary_key: c.is_primary_key,
                        default_value: c.default_value.clone(),
                        comment: c.comment.clone(),
                    })
                    .collect();
                let detail = SqlTableDetail {
                    object_type: match table.object_type {
                        TableObjectType::Table => SqlObjectType::Table,
                        TableObjectType::View => SqlObjectType::View,
                    },
                    schema: table.schema.clone(),
                    comment: table.comment.clone(),
                    engine: table.engine.clone(),
                    columns: detail_columns,
                };
                schema = schema.with_table_detail(&table.name, detail);
            }
        }

        let functions = global_state
            .list_functions(cx, connection_id.clone(), db.clone())
            .await;
        if !self.is_metadata_scope_current(&scope, cx) {
            return;
        }
        if let Ok(functions) = functions {
            let function_items = functions.into_iter().map(|function| {
                let signature = if function.parameters.is_empty() {
                    format!("{}()", function.name)
                } else {
                    format!("{}({})", function.name, function.parameters.join(", "))
                };
                let description = function
                    .comment
                    .or(function.definition)
                    .unwrap_or_else(|| "Function".to_string());
                (signature, description)
            });
            schema = schema.with_functions(function_items);
        }

        // Load other database/schema qualifier names (cross-qualified completion)
        // Names only — full metadata is lazily fetched on demand (see prefetch).
        let qualifier_items = match load_foreign_qualifier_names(
            &global_state,
            cx,
            connection_id.clone(),
            &scope,
            self.uses_schema_as_database,
            self.supports_schema,
        )
        .await
        {
            Ok(items) => items,
            Err(error) => {
                error!("Failed to load SQL completion qualifiers: {error}");
                Vec::new()
            }
        };
        if !self.is_metadata_scope_current(&scope, cx) {
            return;
        }
        schema = schema.with_qualifiers(qualifier_items);

        // Publish metadata and refresh the popup atomically against the current
        // tab scope so a late database/schema load cannot overwrite a newer one.
        let _ = cx.update_window(window_handle, move |_view, window, cx| {
            let handle = window.window_handle();
            let _ = entity.update(cx, |this, cx| {
                if this
                    .current_metadata_scope(generation, Some(&db), cx)
                    .as_ref()
                    != Some(&scope)
                {
                    return;
                }
                *this.schema_snapshot.write() = schema.clone();
                *this.db_completion_info.write() = Some(db_completion_info.clone());
                this.editor.update(cx, |editor, cx| {
                    editor.set_db_completion_info(db_completion_info, schema, cx);
                    editor
                        .input()
                        .update(cx, |state, cx| state.refresh_completion_popup(window, cx));
                });
                this.schedule_foreign_schema_prefetch(handle, cx);
                this.run_diagnostics(cx);
            });
        });
    }

    /// 文本变化时检测外部 qualifier 引用（`q.` 模式）并触发元数据懒加载。
    fn schedule_foreign_schema_prefetch(
        &mut self,
        window_handle: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        let text = self.get_sql_text(cx);
        if text.is_empty() {
            return;
        }
        let schema = self.schema_snapshot.read().clone();
        for name in pending_foreign_qualifiers(&text, &schema) {
            self.spawn_foreign_schema_fetch(name, window_handle, cx);
        }
    }

    /// 懒加载一个外部 qualifier 的表/列元数据，完成后合并进补全快照。
    fn spawn_foreign_schema_fetch(
        &mut self,
        qualifier: String,
        window_handle: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        let generation = self.context_generation.load(Ordering::SeqCst);
        let Some(scope) = self.current_metadata_scope(generation, None, cx) else {
            return;
        };
        let key = foreign_prefetch_key(&scope, &qualifier);
        if !self.foreign_prefetch_inflight.lock().insert(key.clone()) {
            return;
        }
        let Some((database, schema_name)) = foreign_qualifier_fetch_scope(
            &scope,
            &qualifier,
            self.uses_schema_as_database,
            self.supports_schema,
        ) else {
            self.foreign_prefetch_inflight.lock().remove(&key);
            return;
        };

        let global_state = cx.global::<GlobalDbState>().clone();
        let connection_id = self.connection_id.clone();
        let editor = self.editor.clone();
        let task = Tokio::spawn_result(cx, {
            let global_state = global_state.clone();
            let connection_id = connection_id.clone();
            let qualifier = qualifier.clone();
            async move {
                fetch_foreign_schema_metadata(
                    &global_state,
                    &connection_id,
                    &database,
                    schema_name,
                    &qualifier,
                )
                .await
            }
        });
        let inflight = self.foreign_prefetch_inflight.clone();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            inflight.lock().remove(&key);
            match result {
                Ok(foreign) => {
                    let merged = this
                        .update(cx, |this, cx| {
                            if this
                                .current_metadata_scope(scope.generation, None, cx)
                                .as_ref()
                                != Some(&scope)
                            {
                                return false;
                            }
                            this.merge_foreign_schema(foreign, cx);
                            true
                        })
                        .unwrap_or(false);
                    if !merged {
                        return;
                    }
                    let _ = cx.update_window(window_handle, move |_view, window, cx| {
                        editor.update(cx, |editor, cx| {
                            let input = editor.input();
                            input
                                .update(cx, |state, cx| state.refresh_completion_popup(window, cx));
                        });
                    });
                }
                Err(e) => {
                    error!("Failed to lazy-load schema '{qualifier}': {e}");
                }
            }
        })
        .detach();
    }

    /// 将懒加载完成的外部 qualifier 元数据合并进快照并刷新补全。
    fn merge_foreign_schema(&mut self, foreign: ForeignSchema, cx: &mut Context<Self>) {
        let mut schema = self.schema_snapshot.read().clone();
        schema
            .foreign_schemas
            .insert(foreign.name.to_lowercase(), foreign);
        *self.schema_snapshot.write() = schema.clone();
        if let Some(info) = self.db_completion_info.read().clone() {
            self.editor.update(cx, |editor, cx| {
                editor.set_db_completion_info(info, schema, cx)
            });
        }
    }

    fn get_sql_text(&self, cx: &App) -> String {
        self.editor.read(cx).get_text(cx)
    }

    fn current_execution_scope(&self, cx: &App) -> Result<SqlExecutionScope, String> {
        let selected_value = self.database_select.read(cx).selected_value().cloned();
        if !self.uses_schema_as_database && selected_value.is_none() {
            return Err(t!("Query.please_select_database").to_string());
        }

        let scope = if self.uses_schema_as_database {
            (None, self.schema_select.read(cx).selected_value().cloned())
        } else {
            let schema = if self.supports_schema {
                self.schema_select.read(cx).selected_value().cloned()
            } else {
                None
            };
            (selected_value, schema)
        };
        Ok(SqlExecutionScope::new(scope.0, scope.1))
    }

    fn execute_sql_text(&mut self, sql: String, window: &mut Window, cx: &mut Context<Self>) {
        let is_executing = self.sql_result_tab_container.read(cx).is_executing(cx);
        if !can_start_query_execution(is_executing) {
            window.push_notification(t!("Query.running").to_string(), cx);
            return;
        }

        let scope = match self.current_execution_scope(cx) {
            Ok(scope) => scope,
            Err(message) => {
                window.push_notification(message, cx);
                return;
            }
        };

        if sql.trim().is_empty() {
            window.push_notification(t!("Query.please_enter_query").to_string(), cx);
            return;
        }
        let source = self
            .execution_request_for_sql(&sql, &scope, cx)
            .result_source();

        if self.transaction_mode == SqlTransactionMode::Manual {
            self.execute_manual_sql_text(sql, source, scope, window, cx);
            return;
        }

        // 把光标处精确语句绑定为执行 marker，跟踪 running/success/error/cancel。
        self.bind_execution_marker_for_sql(&sql, cx);
        self.run_auto_sql_text(sql, source, scope, window, cx);
    }

    fn execution_request_for_sql(
        &self,
        sql: &str,
        scope: &SqlExecutionScope,
        cx: &App,
    ) -> SqlExecutionRequest {
        let revision = self.editor.read(cx).document_revision(cx);
        let selection = self.editor.read(cx).selected_range(cx);
        let selected_text = self.editor.read(cx).get_selected_text(cx);
        let (target, statement_index) =
            if !selection.is_empty() && selected_text.trim() == sql.trim() {
                (
                    SqlExecutionTarget::Selection(SqlTextRange {
                        start_byte: selection.start,
                        end_byte: selection.end,
                    }),
                    None,
                )
            } else if let Some((index, statement)) = self
                .statement_snapshot
                .statement_ranges()
                .iter()
                .enumerate()
                .find(|(_, statement)| {
                    self.statement_snapshot.statement_text(statement).trim() == sql.trim()
                })
            {
                (
                    SqlExecutionTarget::ExactRange(statement.sql_range),
                    Some(index),
                )
            } else {
                (SqlExecutionTarget::AllStatements, None)
            };
        let metadata_scope = SqlMetadataScope::new(
            self.connection_id.clone(),
            self.database_type.clone(),
            self.context_generation.load(Ordering::SeqCst),
        )
        .with_database(scope.database.clone())
        .with_schema(scope.schema.clone());
        let document = SqlDocumentSnapshot::new(
            revision,
            Arc::<str>::from(self.get_sql_text(cx)),
            SqlDialect::from(&self.database_type),
            metadata_scope,
        );
        SqlExecutionRequest::new(
            self.execution_request_id.fetch_add(1, Ordering::SeqCst) + 1,
            document,
            target,
            Arc::<str>::from(sql.to_string()),
            statement_index,
            match self.transaction_mode {
                SqlTransactionMode::Auto => SqlExecutionTransactionMode::Auto,
                SqlTransactionMode::Manual => SqlExecutionTransactionMode::Manual,
            },
        )
    }

    fn run_auto_sql_text(
        &self,
        sql: String,
        source: SqlExecutionResultSource,
        scope: SqlExecutionScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let connection_id = self.connection_id.clone();
        let sql_result_tab_container = self.sql_result_tab_container.clone();
        sql_result_tab_container.update(cx, |container, cx| {
            container.handle_run_query(
                sql,
                source,
                connection_id,
                scope.database,
                scope.schema,
                window,
                cx,
            );
        })
    }

    fn execute_manual_sql_text(
        &mut self,
        sql: String,
        source: SqlExecutionResultSource,
        scope: SqlExecutionScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let installed_session_matches_scope = self
            .manual_transaction
            .as_ref()
            .map(|session| session.matches_execution_scope(&scope));
        let action = manual_sql_execution_action(
            &self.database_type,
            installed_session_matches_scope,
            self.manual_transaction_starting || self.manual_transaction_finishing,
        );

        match action {
            ManualSqlExecutionAction::Unsupported => {
                window.push_notification(t!("Query.transaction_not_supported").to_string(), cx);
            }
            ManualSqlExecutionAction::ScopeMismatch => {
                window.push_notification(t!("Query.transaction_scope_changed").to_string(), cx);
            }
            ManualSqlExecutionAction::Busy => {
                window.push_notification(t!("Query.running").to_string(), cx);
            }
            ManualSqlExecutionAction::RunInstalledSession => {
                let session = self
                    .manual_transaction
                    .as_ref()
                    .expect("validated installed manual transaction");
                let session_id = session.session_id().to_string();
                let pending_invalidation = session.pending_invalidation();
                self.bind_execution_marker_for_sql(&sql, cx);
                let schema_invalidation = self.session_schema_invalidation(pending_invalidation);
                self.run_manual_sql_on_session(
                    sql,
                    source,
                    session_id,
                    scope,
                    schema_invalidation,
                    cx,
                );
            }
            ManualSqlExecutionAction::StartSession => {
                self.bind_execution_marker_for_sql(&sql, cx);
                self.start_manual_transaction_and_run(sql, source, scope, cx);
            }
        }
    }

    fn run_manual_sql_on_session(
        &self,
        sql: String,
        source: SqlExecutionResultSource,
        session_id: String,
        scope: SqlExecutionScope,
        schema_invalidation: SessionSchemaInvalidation,
        cx: &mut App,
    ) {
        let request = SessionSqlRun {
            sql,
            session_id,
            connection_id: self.connection_id.clone(),
            database: scope.database,
            schema: scope.schema,
            database_type: self.database_type.clone(),
            schema_invalidation,
            source,
        };
        self.sql_result_tab_container.update(cx, |container, cx| {
            container.handle_run_query_with_session(request, cx);
        });
    }

    fn session_schema_invalidation(
        &self,
        pending: Arc<Mutex<SchemaInvalidationPlan>>,
    ) -> SessionSchemaInvalidation {
        match manual_transaction_invalidation_mode(&self.database_type) {
            ManualTransactionInvalidationMode::Immediate => SessionSchemaInvalidation::Immediate,
            ManualTransactionInvalidationMode::Deferred => {
                SessionSchemaInvalidation::Deferred(pending)
            }
        }
    }

    fn start_manual_transaction_and_run(
        &mut self,
        sql: String,
        source: SqlExecutionResultSource,
        scope: SqlExecutionScope,
        cx: &mut Context<Self>,
    ) {
        let generation = self
            .manual_transaction_generation
            .fetch_add(1, Ordering::SeqCst)
            + 1;
        let transaction_generation = self.manual_transaction_generation.clone();
        let context_generation = self.context_generation.load(Ordering::SeqCst);
        let context_generation_guard = self.context_generation.clone();
        self.manual_transaction_starting = true;
        self.manual_transaction_finishing = false;
        cx.notify();

        let global_state = cx.global::<GlobalDbState>().clone();
        let connection_id = self.connection_id.clone();
        let database_type = self.database_type.clone();

        cx.spawn(async move |entity: WeakEntity<Self>, cx: &mut AsyncApp| {
            let session_id = match global_state
                .create_session(cx, connection_id.clone(), scope.database.clone())
                .await
            {
                Ok(session_id) => session_id,
                Err(error) => {
                    let is_current = entity
                        .update(cx, |this, cx| {
                            if transaction_generation.load(Ordering::SeqCst) != generation {
                                return false;
                            }
                            this.manual_transaction_starting = false;
                            this.finalize_execution_marker(SqlGutterMarkerState::Failed, cx);
                            cx.notify();
                            true
                        })
                        .unwrap_or(false);
                    if is_current {
                        Self::notify_async(
                            cx,
                            t!("Query.transaction_start_failed", error = error.to_string())
                                .to_string(),
                        );
                    }
                    return;
                }
            };

            if transaction_generation.load(Ordering::SeqCst) != generation
                || context_generation_guard.load(Ordering::SeqCst) != context_generation
            {
                let _ = entity.update(cx, |this, cx| {
                    if is_current_manual_transaction_start(
                        generation,
                        transaction_generation.load(Ordering::SeqCst),
                        this.manual_transaction_starting,
                    ) {
                        this.manual_transaction_starting = false;
                        this.finalize_execution_marker(SqlGutterMarkerState::Cancelled, cx);
                        cx.notify();
                    }
                });
                if let Err(error) = global_state.close_session(cx, session_id).await {
                    error!(
                        "Failed to close stale manual transaction session: {:?}",
                        error
                    );
                }
                return;
            }

            let prepare = ManualTransactionPrepare {
                database_type: &database_type,
                scope: &scope,
                session_id: &session_id,
            };
            if let Err(error) =
                Self::prepare_manual_transaction_session(&global_state, prepare, cx).await
            {
                if let Err(close_error) = global_state.close_session(cx, session_id).await {
                    error!(
                        "Failed to close unprepared manual transaction session: {:?}",
                        close_error
                    );
                }
                let is_current = entity
                    .update(cx, |this, cx| {
                        if !is_current_manual_transaction_start(
                            generation,
                            transaction_generation.load(Ordering::SeqCst),
                            this.manual_transaction_starting,
                        ) {
                            return false;
                        }
                        this.manual_transaction_starting = false;
                        this.finalize_execution_marker(SqlGutterMarkerState::Failed, cx);
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if is_current {
                    Self::notify_async(
                        cx,
                        t!("Query.transaction_start_failed", error = error.to_string()).to_string(),
                    );
                }
                return;
            }

            let installed = entity
                .update(cx, |this, cx| {
                    if transaction_generation.load(Ordering::SeqCst) != generation
                        || context_generation_guard.load(Ordering::SeqCst) != context_generation
                        || !this.manual_transaction_starting
                        || this.manual_transaction.is_some()
                    {
                        return false;
                    }
                    let session = ManualTransactionSession::new(
                        session_id.clone(),
                        scope.database.clone(),
                        scope.schema.clone(),
                    );
                    let schema_invalidation =
                        this.session_schema_invalidation(session.pending_invalidation());
                    this.manual_transaction = Some(session);
                    this.manual_transaction_starting = false;
                    this.run_manual_sql_on_session(
                        sql.clone(),
                        source,
                        session_id.clone(),
                        scope.clone(),
                        schema_invalidation,
                        cx,
                    );
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if installed {
                Self::notify_async(cx, t!("Query.transaction_started").to_string());
            } else {
                let _ = entity.update(cx, |this, cx| {
                    if is_current_manual_transaction_start(
                        generation,
                        transaction_generation.load(Ordering::SeqCst),
                        this.manual_transaction_starting,
                    ) {
                        this.manual_transaction_starting = false;
                        this.finalize_execution_marker(SqlGutterMarkerState::Cancelled, cx);
                        cx.notify();
                    }
                });
                if let Err(error) = global_state.close_session(cx, session_id).await {
                    error!(
                        "Failed to close uninstalled manual transaction session: {:?}",
                        error
                    );
                }
            }
        })
        .detach();
    }

    async fn prepare_manual_transaction_session(
        global_state: &GlobalDbState,
        prepare: ManualTransactionPrepare<'_>,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<()> {
        let global_state = global_state.clone();
        let schema = prepare.scope.schema.clone();
        let session_id = prepare.session_id.to_string();
        let begin_sql =
            manual_transaction_control_sql(prepare.database_type, ManualTransactionAction::Begin)
                .map(str::to_string);
        Tokio::spawn_result(cx, async move {
            if let Some(schema) = schema {
                global_state
                    .switch_session_schema(session_id.clone(), schema)
                    .await?;
            }
            if let Some(begin_sql) = begin_sql {
                let result = global_state
                    .execute_session(
                        session_id,
                        begin_sql,
                        Some(manual_transaction_control_options()),
                    )
                    .await;
                if transaction_control_failed(&result) {
                    return Err(anyhow::anyhow!("BEGIN failed"));
                }
            }
            Ok(())
        })
        .await
    }

    fn handle_commit_transaction(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.finish_manual_transaction(ManualTransactionAction::Commit, window, cx);
    }

    fn handle_rollback_transaction(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.finish_manual_transaction(ManualTransactionAction::Rollback, window, cx);
    }

    fn finish_manual_transaction(
        &mut self,
        action: ManualTransactionAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.manual_transaction.clone() else {
            window.push_notification(t!("Query.transaction_not_started").to_string(), cx);
            return;
        };
        let Some(sql) = manual_transaction_control_sql(&self.database_type, action) else {
            window.push_notification(t!("Query.transaction_control_unavailable").to_string(), cx);
            return;
        };
        if self.manual_transaction_finishing {
            window.push_notification(t!("Query.running").to_string(), cx);
            return;
        }

        let generation = self
            .manual_transaction_generation
            .fetch_add(1, Ordering::SeqCst)
            + 1;
        let transaction_generation = self.manual_transaction_generation.clone();
        let session_id = session.session_id().to_string();
        self.manual_transaction_finishing = true;
        cx.notify();

        let global_state = cx.global::<GlobalDbState>().clone();
        let cache = cx.try_global::<GlobalNodeCache>().cloned();
        let notifier = cx.try_global::<GlobalConnectionNotifier>().cloned();
        let connection_id = self.connection_id.clone();
        cx.spawn(async move |entity: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = global_state
                .execute_session_on_runtime(
                    cx,
                    session_id.clone(),
                    sql.to_string(),
                    Some(manual_transaction_control_options()),
                )
                .await;
            if transaction_control_failed(&result) {
                let is_current = entity
                    .update(cx, |this, cx| {
                        let current_session_id = this
                            .manual_transaction
                            .as_ref()
                            .map(ManualTransactionSession::session_id);
                        if !is_current_manual_transaction_owner(
                            generation,
                            &session_id,
                            transaction_generation.load(Ordering::SeqCst),
                            current_session_id,
                        ) {
                            return false;
                        }
                        this.manual_transaction_finishing = false;
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if is_current {
                    Self::notify_async(cx, t!("Query.transaction_control_failed").to_string());
                }
                return;
            }

            if action == ManualTransactionAction::Commit {
                let pending = std::mem::take(&mut *session.pending_invalidation.lock());
                if let Some(cache) = cache {
                    let scopes = global_state
                        .apply_sql_cache_invalidation_plan(&cache, &connection_id, &pending)
                        .await;
                    if let Some(notifier) = notifier {
                        for (connection_id, database, schema) in scopes {
                            let _ = cx.update(|cx| {
                                notifier.0.update(cx, |_, cx| {
                                    cx.emit(ConnectionDataEvent::SchemaChanged {
                                        connection_id,
                                        database,
                                        schema,
                                    });
                                });
                            });
                        }
                    }
                }
            }

            if let Err(error) = global_state.close_session(cx, session_id.clone()).await {
                error!(
                    "Failed to close finished manual transaction session {}: {:?}",
                    session_id, error
                );
            }
            let cleared = entity
                .update(cx, |this, cx| {
                    let current_session_id = this
                        .manual_transaction
                        .as_ref()
                        .map(ManualTransactionSession::session_id);
                    if !is_current_manual_transaction_owner(
                        generation,
                        &session_id,
                        transaction_generation.load(Ordering::SeqCst),
                        current_session_id,
                    ) {
                        return false;
                    }
                    this.manual_transaction = None;
                    this.manual_transaction_finishing = false;
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            let message = match action {
                ManualTransactionAction::Commit => t!("Query.transaction_committed").to_string(),
                ManualTransactionAction::Rollback => {
                    t!("Query.transaction_rolled_back").to_string()
                }
                ManualTransactionAction::Begin => t!("Query.transaction_started").to_string(),
            };
            if cleared {
                Self::notify_async(cx, message);
            }
        })
        .detach();
    }

    fn notify_async(cx: &mut AsyncApp, message: String) {
        let _ = cx.update(|cx| {
            if let Some(window_id) = cx.active_window() {
                let notification = message.clone();
                cx.update_window(window_id, move |_entity, window, cx| {
                    window.push_notification(notification.clone(), cx);
                })
            } else {
                Err(anyhow::anyhow!("No active window"))
            }
        });
    }

    fn handle_run_query(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let selected_text = self.editor.read(cx).get_selected_text(cx);
        let sql = sql_text_for_toolbar_run(&self.get_sql_text(cx), &selected_text);
        self.execute_sql_text(sql, window, cx);
    }

    fn handle_stop_query(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let cancelled = self
            .sql_result_tab_container
            .update(cx, |container, cx| container.cancel_execution(cx));
        match manual_transaction_stop_action(
            cancelled,
            self.manual_transaction_starting,
            self.manual_transaction.is_some(),
        ) {
            ManualTransactionStopAction::None => {}
            ManualTransactionStopAction::CancelStart => {
                self.manual_transaction_generation
                    .fetch_add(1, Ordering::SeqCst);
                self.manual_transaction_starting = false;
                self.manual_transaction_finishing = false;
                self.finalize_execution_marker(SqlGutterMarkerState::Cancelled, cx);
                cx.notify();
            }
            ManualTransactionStopAction::CloseInstalledSession => {
                self.manual_transaction_generation
                    .fetch_add(1, Ordering::SeqCst);
                self.manual_transaction_starting = false;
                self.manual_transaction_finishing = false;
                let Some(session) = self.manual_transaction.take() else {
                    return;
                };
                let session_id = session.session_id().to_string();
                let database = session.database.clone();
                let schema = session.schema.clone();
                let global_state = cx.global::<GlobalDbState>().clone();
                let cache = cx.try_global::<GlobalNodeCache>().cloned();
                let notifier = cx.try_global::<GlobalConnectionNotifier>().cloned();
                let connection_id = self.connection_id.clone();
                cx.spawn(async move |_entity: WeakEntity<Self>, cx: &mut AsyncApp| {
                    if let Err(error) = global_state.close_session(cx, session_id.clone()).await {
                        error!(
                            "Failed to close cancelled manual transaction session {}: {:?}",
                            session_id, error
                        );
                    }
                    if let Some(cache) = cache {
                        let plan = global_state.conservative_sql_cache_invalidation_plan(
                            &connection_id,
                            database.as_deref(),
                            schema.as_deref(),
                        );
                        let scopes = global_state
                            .apply_sql_cache_invalidation_plan(&cache, &connection_id, &plan)
                            .await;
                        emit_schema_changed_events(cx, notifier.as_ref(), scopes);
                    }
                })
                .detach();
                cx.notify();
            }
        }
    }

    fn handle_run_current_query_action(
        &mut self,
        _: &RunCurrentQuery,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected_text = self.editor.read(cx).get_selected_text(cx);
        let sql = if selected_text.trim().is_empty() {
            let cursor_offset = self.editor.read(cx).cursor_offset(cx);
            self.refresh_statement_snapshot(cx);
            self.statement_snapshot
                .statement_at_cursor(cursor_offset)
                .map(|statement| {
                    self.statement_snapshot
                        .statement_text(statement)
                        .to_string()
                })
                .unwrap_or_default()
        } else {
            selected_text
        };
        self.execute_sql_text(sql, window, cx);
    }

    fn handle_run_all_query_action(
        &mut self,
        _: &RunAllQuery,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected_text = self.editor.read(cx).get_selected_text(cx);
        let sql = sql_text_for_run_all(&self.get_sql_text(cx), &selected_text);
        self.execute_sql_text(sql, window, cx);
    }

    fn handle_toggle_line_comment_action(
        &mut self,
        _: &ToggleLineComment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = self.get_sql_text(cx);
        let selection = self.editor.read(cx).selected_range(cx);
        let result = toggle_sql_line_comments(&text, selection);
        if text[result.range.clone()] == result.replacement {
            return;
        }
        self.editor.update(cx, |editor, cx| {
            editor.replace_range_and_select(
                result.range,
                result.replacement,
                result.selection,
                window,
                cx,
            );
        });
    }

    fn handle_run_selected_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected_text = self.editor.read(cx).get_selected_text(cx);
        if selected_text.trim().is_empty() {
            window.push_notification(t!("Query.please_select_sql_to_run").to_string(), cx);
            return;
        }
        self.execute_sql_text(selected_text, window, cx);
    }

    fn handle_run_selected_sql_action(
        &mut self,
        _: &RunSelectedSql,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_run_selected_query(window, cx);
    }

    fn handle_run_cursor_statement_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cursor_offset = self.editor.read(cx).cursor_offset(cx);
        self.refresh_statement_snapshot(cx);
        let sql = self
            .statement_snapshot
            .statement_at_cursor(cursor_offset)
            .map(|statement| {
                self.statement_snapshot
                    .statement_text(statement)
                    .to_string()
            })
            .unwrap_or_default();
        if sql.trim().is_empty() {
            window.push_notification(t!("Query.query_content_empty").to_string(), cx);
            return;
        }
        self.execute_sql_text(sql, window, cx);
    }

    fn handle_run_cursor_statement_sql_action(
        &mut self,
        _: &RunCursorStatementSql,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_run_cursor_statement_query(window, cx);
    }

    fn handle_format_query(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.get_sql_text(cx);
        if text.trim().is_empty() {
            window.push_notification(t!("Query.no_sql_to_format").to_string(), cx);
            return;
        }
        let format_revision = self.editor.read(cx).document_revision(cx);
        let format_context_generation = self.context_generation.load(Ordering::SeqCst);
        let window_option = cx.active_window();
        let format_options = SqlFormatOptions::from_settings(&AppSettings::global(cx).sql_format);
        // 格式化在后台线程执行，避免大文本卡住 UI 线程；完成后回到主线程写回编辑器。
        let heavy =
            cx.background_spawn(async move { format_sql_with_options(&text, format_options) });
        cx.spawn(async move |entity: WeakEntity<Self>, cx: &mut AsyncApp| {
            let formatted = heavy.await;
            entity
                .update(cx, |this, cx| {
                    let current_revision = this.editor.read(cx).document_revision(cx);
                    if current_revision != format_revision
                        || !this.is_context_generation_current(format_context_generation)
                    {
                        return;
                    }
                    if let Some(window_id) = window_option {
                        cx.update_window(window_id, move |_entity, window, cx| {
                            this.editor
                                .update(cx, |s, cx| s.set_value(formatted, window, cx));
                        })
                        .ok();
                    }
                })
                .ok()
        })
        .detach();
    }

    pub fn save_query(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let sql = self.get_sql_text(cx);
        if self.requires_name.load(Ordering::Relaxed) {
            if sql.trim().is_empty() {
                return true;
            }
            self.show_save_name_dialog(window, cx);
            return false;
        }
        match self.save_to_file(cx) {
            Ok(()) => true,
            Err(error) => {
                self.notify_save_failed(error, window, cx);
                false
            }
        }
    }

    fn save_to_file(&self, cx: &App) -> io::Result<()> {
        let sql = self.get_sql_text(cx);
        let file_path = self.file_path.read().clone();
        write_sql_file(&file_path, &sql)?;
        self.is_dirty.store(false, Ordering::Relaxed);
        Ok(())
    }

    fn show_save_name_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("Query.enter_query_name").to_string())
        });
        let input_for_focus = input.clone();
        let view = cx.entity();

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let input_for_ok = input.clone();
            let view_for_ok = view.clone();
            dialog
                .title(t!("Query.save_query_title").to_string())
                .w(px(380.0))
                .confirm()
                .on_ok(move |_, window, cx| {
                    let name = input_for_ok.read(cx).value().trim().to_owned();
                    view_for_ok.update(cx, |view, cx| view.save_named_query(&name, window, cx))
                })
                .child(
                    v_flex()
                        .gap_3()
                        .child(h_flex().child(t!("Query.enter_query_name").to_string()))
                        .child(Input::new(&input).w_full()),
                )
        });
        window.defer(cx, move |window, cx| {
            input_for_focus.update(cx, |input, cx| input.focus(window, cx));
        });
    }

    fn show_close_save_name_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("Query.enter_query_name").to_string())
        });
        let input_for_focus = input.clone();
        let view = cx.entity();
        let (tx, rx) = oneshot::channel::<bool>();
        let tx = Arc::new(Mutex::new(Some(tx)));

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let input_for_save = input.clone();
            let view_for_save = view.clone();
            let tx_cancel = tx.clone();
            let tx_discard = tx.clone();
            let tx_save = tx.clone();

            let footer = DialogFooter::new().children(vec![
                Button::new("cancel-close-query")
                    .label(t!("Common.cancel").to_string())
                    .on_click(move |_, window: &mut Window, cx| {
                        window.close_dialog(cx);
                        if let Some(tx) = tx_cancel.lock().take() {
                            let _ = tx.send(false);
                        }
                    })
                    .into_any_element(),
                Button::new("discard-close-query")
                    .label(t!("Query.dont_save").to_string())
                    .on_click(move |_, window: &mut Window, cx| {
                        window.close_dialog(cx);
                        if let Some(tx) = tx_discard.lock().take() {
                            let _ = tx.send(true);
                        }
                    })
                    .into_any_element(),
                Button::new("save-close-query")
                    .label(t!("Common.save").to_string())
                    .primary()
                    .on_click(move |_, window: &mut Window, cx| {
                        let name = input_for_save.read(cx).value().trim().to_owned();
                        let saved = view_for_save
                            .update(cx, |view, cx| view.save_named_query(&name, window, cx));
                        if saved {
                            window.close_dialog(cx);
                            if let Some(tx) = tx_save.lock().take() {
                                let _ = tx.send(true);
                            }
                        }
                    })
                    .into_any_element(),
            ]);

            dialog
                .title(t!("Query.save_query_title").to_string())
                .w(px(380.0))
                .overlay_closable(false)
                .close_button(false)
                .footer(footer)
                .child(
                    v_flex()
                        .gap_3()
                        .child(h_flex().child(t!("Query.enter_query_name").to_string()))
                        .child(Input::new(&input).w_full()),
                )
        });
        window.defer(cx, move |window, cx| {
            input_for_focus.update(cx, |input, cx| input.focus(window, cx));
        });

        cx.spawn(async move |_handle, _cx| rx.await.unwrap_or(false))
    }

    fn save_named_query(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let directory = self
            .file_path
            .read()
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let file_path = match query_file_path_for_name(&directory, name) {
            Ok(file_path) => file_path,
            Err(error) => {
                self.notify_query_name_error(error, window, cx);
                return false;
            }
        };
        let sql = self.get_sql_text(cx);
        if sql.trim().is_empty() {
            window.push_notification(t!("Query.query_content_empty").to_string(), cx);
            return false;
        }
        if let Err(error) = write_new_sql_file(&file_path, &sql) {
            if error.kind() == io::ErrorKind::AlreadyExists {
                self.notify_query_name_error(QueryFileNameError::AlreadyExists, window, cx);
            } else {
                self.notify_save_failed(error, window, cx);
            }
            return false;
        }

        *self.file_path.write() = file_path;
        self.requires_name.store(false, Ordering::Relaxed);
        self.finish_successful_save(window, cx);
        true
    }

    fn notify_query_name_error(
        &self,
        error: QueryFileNameError,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let message = match error {
            QueryFileNameError::Empty => t!("Query.query_name_empty").to_string(),
            QueryFileNameError::Invalid => t!("Query.query_name_invalid").to_string(),
            QueryFileNameError::AlreadyExists => t!("Query.query_name_exists").to_string(),
            QueryFileNameError::ReadDirectory(error) => {
                t!("Query.query_save_failed", error = error).to_string()
            }
        };
        window.push_notification(Notification::error(message).autohide(true), cx);
    }

    fn notify_save_failed(&self, error: io::Error, window: &mut Window, cx: &mut Context<Self>) {
        let message = t!("Query.query_save_failed", error = error).to_string();
        window.push_notification(Notification::error(message).autohide(true), cx);
    }

    fn finish_successful_save(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.is_dirty.store(false, Ordering::Relaxed);
        window.push_notification(t!("Query.query_saved").to_string(), cx);
        cx.emit(SqlEditorEvent::QuerySaved {
            connection_id: self.connection_id.clone(),
            database: self.database_select.read(cx).selected_value().cloned(),
        });
    }

    pub fn save_and_close(
        &mut self,
        tab_container: Entity<TabContainer>,
        tab_id: String,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.save_query(_window, cx) {
            return;
        }
        tab_container.update(cx, |container, cx| {
            container.force_close_tab_by_id(&tab_id, _window, cx);
        });
        cx.emit(SqlEditorEvent::QuerySaved {
            connection_id: self.connection_id.clone(),
            database: self.database_select.read(cx).selected_value().cloned(),
        });
    }

    fn handle_save_query(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let sql = self.get_sql_text(cx);
        if sql.trim().is_empty() {
            window.push_notification(t!("Query.query_content_empty").to_string(), cx);
            return;
        }

        if self.requires_name.load(Ordering::Relaxed) {
            self.show_save_name_dialog(window, cx);
            return;
        }
        match self.save_to_file(cx) {
            Ok(()) => self.finish_successful_save(window, cx),
            Err(error) => self.notify_save_failed(error, window, cx),
        }
    }

    fn handle_show_results(
        &mut self,
        _: &ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sql_result_tab_container.update(cx, |container, cx| {
            container.show(cx);
        });
    }

    fn render_resize_handle(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity().clone();

        resize_handle::<ResizePanel, ResizePanel>("result-resize-handle", Axis::Vertical).on_drag(
            ResizePanel,
            move |info, _, _, cx| {
                cx.stop_propagation();
                view.update(cx, |view, cx| {
                    view.resizing = true;
                    cx.notify();
                });
                cx.new(|_| info.deref().clone())
            },
        )
    }

    fn resize(
        &mut self,
        mouse_position: Point<Pixels>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.resizing {
            return;
        }

        let available_height = self.bounds.size.height;
        let new_size = self.bounds.bottom() - mouse_position.y;
        let max_size = (available_height - PANEL_MIN_SIZE).max(PANEL_MIN_SIZE);
        self.result_panel_size = new_size.clamp(PANEL_MIN_SIZE, max_size);

        cx.notify();
    }

    fn done_resizing(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.resizing = false;
        cx.notify();
    }

    fn render_has_results(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let result_panel_size = self.result_panel_size;
        let border_color = cx.theme().border;

        v_flex()
            .size_full()
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_sql_editor(cx)),
            )
            .child(
                div()
                    .relative()
                    .h(result_panel_size)
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(border_color)
                    .child(self.sql_result_tab_container.clone())
                    .child(self.render_resize_handle(window, cx)),
            )
    }

    fn handle_explain_sql(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let selected_text = self.editor.read(cx).get_selected_text(cx);
        let sql = if selected_text.trim().is_empty() {
            self.get_sql_text(cx)
        } else {
            selected_text
        };

        if sql.trim().is_empty() {
            window.push_notification(t!("Query.please_enter_query").to_string(), cx);
            return;
        }

        let selected_value = self.database_select.read(cx).selected_value().cloned();

        // For non-Oracle databases, database selection is required
        if !self.uses_schema_as_database && selected_value.is_none() {
            window.push_notification(t!("Query.please_select_database").to_string(), cx);
            return;
        }

        // For Oracle (uses_schema_as_database), schema_select contains schema values.
        let (current_database_value, current_schema_value) = if self.uses_schema_as_database {
            (None, self.schema_select.read(cx).selected_value().cloned())
        } else {
            let schema = if self.supports_schema {
                self.schema_select.read(cx).selected_value().cloned()
            } else {
                None
            };
            (selected_value, schema)
        };

        let Ok(plugin) = DbManager::default().get_plugin(&self.database_type) else {
            window.push_notification(t!("Query.plugin_not_found").to_string(), cx);
            return;
        };

        let Some(explain_sql) = plugin.build_explain_sql(&sql) else {
            window.push_notification(t!("Query.explain_query_only").to_string(), cx);
            return;
        };
        let scope =
            SqlExecutionScope::new(current_database_value.clone(), current_schema_value.clone());
        let source = self
            .execution_request_for_sql(&sql, &scope, cx)
            .result_source();

        let connection_id = self.connection_id.clone();
        let sql_result_tab_container = self.sql_result_tab_container.clone();

        sql_result_tab_container.update(cx, |container, cx| {
            container.handle_run_query(
                explain_sql,
                source,
                connection_id,
                current_database_value,
                current_schema_value,
                window,
                cx,
            );
        })
    }

    fn render_sql_editor(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let editor = self.editor.clone();

        // Check if there are any results and if the panel is visible
        let has_results = self.sql_result_tab_container.read(cx).has_results(cx);
        let results_visible = self.sql_result_tab_container.read(cx).is_visible(cx);

        v_flex()
            .size_full()
            .gap_2()
            .child(self.render_query_toolbar(cx))
            .child(
                // Editor
                v_flex()
                    .p_1()
                    .flex_1()
                    .child(
                        div()
                            .size_full()
                            .key_context(SQL_EDITOR_CONTEXT)
                            .child(editor.clone()),
                    )
                    .when(has_results && !results_visible, |this| {
                        this.child(
                            h_flex().w_full().justify_end().child(
                                Button::new("show-results")
                                    .with_size(Size::Small)
                                    .ghost()
                                    .tooltip(t!("Query.show_results"))
                                    .icon(IconName::ArrowUp)
                                    .on_click(cx.listener(Self::handle_show_results)),
                            ),
                        )
                    }),
            )
    }

    /// 参考 dbx EditorToolbar 的紧凑工具栏：左侧彩色图标按钮，右侧连接/数据库选择。
    /// 手动事务组（模式切换 + 提交/回滚）保留在左侧动作区之后。
    fn render_query_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let connection_select = self.connection_select.clone();
        let database_select = self.database_select.clone();
        let schema_select = self.schema_select.clone();
        let transaction_mode_select = self.transaction_mode_select.clone();
        let supports_schema = self.supports_schema;
        let uses_schema_as_database = self.uses_schema_as_database;
        let supports_transactions = supports_manual_transactions(&self.database_type);
        let is_manual_mode = self.transaction_mode == SqlTransactionMode::Manual;
        let has_manual_transaction = self.manual_transaction.is_some();
        let is_manual_transaction_starting = self.manual_transaction_starting;
        let is_manual_transaction_finishing = self.manual_transaction_finishing;
        let has_manual_transaction_lifecycle = self.has_manual_transaction_lifecycle();

        let is_query_executing = self.sql_result_tab_container.read(cx).is_executing(cx);
        let has_selection = !self.editor.read(cx).get_selected_text(cx).trim().is_empty();
        let toolbar_action = if is_manual_transaction_starting {
            QueryToolbarAction::Stop
        } else {
            query_toolbar_action(is_query_executing, has_selection)
        };
        let transaction_finishing =
            is_manual_transaction_starting || is_manual_transaction_finishing;
        let transaction_unavailable =
            is_query_executing || transaction_finishing || !has_manual_transaction;

        h_flex()
            .w_full()
            .h(QUERY_TOOLBAR_HEIGHT)
            .px_2()
            .gap_1()
            .items_center()
            .bg(cx.theme().background)
            .border_b_1()
            .border_color(cx.theme().border)
            .child(match toolbar_action {
                QueryToolbarAction::Stop => Self::query_toolbar_button(
                    QueryToolbarButtonSpec {
                        id: "stop-query",
                        icon: IconName::CircleX,
                        color: cx.theme().danger,
                        tooltip: t!("Query.stop").into(),
                        disabled: false,
                    },
                    cx.listener(Self::handle_stop_query),
                    cx,
                ),
                QueryToolbarAction::RunSelected => Self::query_toolbar_button(
                    QueryToolbarButtonSpec {
                        id: "run-query",
                        icon: IconName::Play,
                        color: cx.theme().success,
                        tooltip: t!("Query.run_selected").into(),
                        disabled: transaction_finishing,
                    },
                    cx.listener(Self::handle_run_query),
                    cx,
                ),
                QueryToolbarAction::Run => Self::query_toolbar_button(
                    QueryToolbarButtonSpec {
                        id: "run-query",
                        icon: IconName::Play,
                        color: cx.theme().success,
                        tooltip: t!("Query.run").into(),
                        disabled: transaction_finishing,
                    },
                    cx.listener(Self::handle_run_query),
                    cx,
                ),
            })
            .child(Self::query_toolbar_button(
                QueryToolbarButtonSpec {
                    id: "explain-sql",
                    icon: IconName::GitBranch,
                    color: cx.theme().info,
                    tooltip: t!("Query.explain").into(),
                    disabled: is_query_executing || transaction_finishing,
                },
                cx.listener(Self::handle_explain_sql),
                cx,
            ))
            .child(Self::query_toolbar_button(
                QueryToolbarButtonSpec {
                    id: "format-query",
                    icon: IconName::AlignLeft,
                    color: cx.theme().warning,
                    tooltip: t!("Query.format").into(),
                    disabled: false,
                },
                cx.listener(Self::handle_format_query),
                cx,
            ))
            .child(Self::query_toolbar_button(
                QueryToolbarButtonSpec {
                    id: "save-query",
                    icon: IconName::Save,
                    color: cx.theme().primary,
                    tooltip: t!("Query.save").into(),
                    disabled: false,
                },
                cx.listener(Self::handle_save_query),
                cx,
            ))
            .when(supports_transactions, |toolbar| {
                toolbar
                    .child(query_toolbar_divider(cx))
                    .child(
                        h_flex().h(QUERY_TOOLBAR_CONTROL_HEIGHT).child(
                            Select::new(&transaction_mode_select)
                                .with_size(Size::Small)
                                .title_prefix(t!("Query.transaction_mode_prefix"))
                                .disabled(is_query_executing || has_manual_transaction_lifecycle)
                                .h(QUERY_TOOLBAR_CONTROL_HEIGHT)
                                .w(px(128.)),
                        ),
                    )
                    .when(is_manual_mode, |group| {
                        group
                            .child(Self::query_toolbar_button(
                                QueryToolbarButtonSpec {
                                    id: "transaction-commit",
                                    icon: IconName::Check,
                                    color: cx.theme().success,
                                    tooltip: t!("Query.transaction_commit").into(),
                                    disabled: transaction_unavailable,
                                },
                                cx.listener(Self::handle_commit_transaction),
                                cx,
                            ))
                            .child(Self::query_toolbar_button(
                                QueryToolbarButtonSpec {
                                    id: "transaction-rollback",
                                    icon: IconName::Undo,
                                    color: cx.theme().danger,
                                    tooltip: t!("Query.transaction_rollback").into(),
                                    disabled: transaction_unavailable,
                                },
                                cx.listener(Self::handle_rollback_transaction),
                                cx,
                            ))
                    })
            })
            .child(div().flex_1())
            .child(
                h_flex().h(QUERY_TOOLBAR_CONTROL_HEIGHT).child(
                    Select::new(&connection_select)
                        .with_size(Size::Small)
                        .placeholder(t!("Query.select_connection"))
                        .search_placeholder(t!("Query.search_connection"))
                        .disabled(is_query_executing || has_manual_transaction_lifecycle)
                        .h(QUERY_TOOLBAR_CONTROL_HEIGHT)
                        .w(px(220.)),
                ),
            )
            .when(!uses_schema_as_database, |toolbar| {
                toolbar.child(
                    // Database selector (for non-Oracle databases)
                    h_flex().h(QUERY_TOOLBAR_CONTROL_HEIGHT).child(
                        Select::new(&database_select)
                            .with_size(Size::Small)
                            .placeholder(t!("Query.select_database"))
                            .search_placeholder(t!("Query.search_database"))
                            .disabled(has_manual_transaction_lifecycle)
                            .h(QUERY_TOOLBAR_CONTROL_HEIGHT)
                            .w(px(200.)),
                    ),
                )
            })
            .when(
                should_render_schema_select(supports_schema, uses_schema_as_database),
                |toolbar| {
                    toolbar.child(
                        // Schema selector for PostgreSQL
                        h_flex().h(QUERY_TOOLBAR_CONTROL_HEIGHT).child(
                            Select::new(&schema_select)
                                .with_size(Size::Small)
                                .placeholder(t!("Query.select_schema"))
                                .search_placeholder(t!("Query.search_schema"))
                                .disabled(has_manual_transaction_lifecycle)
                                .h(QUERY_TOOLBAR_CONTROL_HEIGHT)
                                .w(if uses_schema_as_database {
                                    px(200.)
                                } else {
                                    px(150.)
                                }),
                        ),
                    )
                },
            )
    }

    /// 无文字、仅带主题色图标与 tooltip 的工具栏按钮。
    fn query_toolbar_button(
        spec: QueryToolbarButtonSpec,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
        cx: &App,
    ) -> Button {
        let color = spec.color;
        Button::new(spec.id)
            .with_size(Size::Small)
            .h(QUERY_TOOLBAR_CONTROL_HEIGHT)
            .w(QUERY_TOOLBAR_CONTROL_HEIGHT)
            .custom(
                ButtonCustomVariant::new(cx)
                    .foreground(color)
                    .hover(color.opacity(0.1))
                    .active(color.opacity(0.16)),
            )
            .disabled(spec.disabled)
            .tooltip(spec.tooltip)
            .icon(spec.icon)
            .on_click(on_click)
    }
}

fn metadata_scope_selection(
    database: Option<&str>,
    selected_database: Option<String>,
    selected_schema: Option<String>,
    supports_schema: bool,
    uses_schema_as_database: bool,
) -> (Option<String>, Option<String>) {
    // 空串 override（update_schema_for_db 在 scope.database 为 None 时传 db=""）
    // 必须忽略，否则会覆盖下拉当前选择，导致发布守卫永远不匹配。
    let database = database.filter(|database| !database.trim().is_empty());
    if uses_schema_as_database {
        (None, database.map(str::to_string).or(selected_schema))
    } else {
        (
            database.map(str::to_string).or(selected_database),
            supports_schema.then_some(selected_schema).flatten(),
        )
    }
}

fn schema_changed_event_matches_scope(
    event_connection_id: &str,
    event_database: &str,
    event_schema: Option<&str>,
    current: &SqlMetadataScope,
    supports_schema: bool,
    uses_schema_as_database: bool,
) -> bool {
    if event_connection_id != current.connection_id {
        return false;
    }

    if uses_schema_as_database {
        event_schema.is_some_and(|schema| current.schema.as_deref() == Some(schema))
            || event_schema.is_none()
                && current
                    .schema
                    .as_deref()
                    .is_some_and(|schema| schema == event_database)
    } else {
        current.database.as_deref() == Some(event_database)
            && event_schema
                .map(|schema| {
                    !supports_schema
                        || current.schema.is_none()
                        || current.schema.as_deref() == Some(schema)
                })
                .unwrap_or(true)
    }
}

impl Render for SqlEditorTab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let has_results = self.sql_result_tab_container.read(cx).has_results(cx);
        let results_visible = self.sql_result_tab_container.read(cx).is_visible(cx);
        let view = cx.entity().clone();

        let mut div = v_flex()
            .size_full()
            .on_action(cx.listener(Self::handle_run_current_query_action))
            .on_action(cx.listener(Self::handle_run_all_query_action))
            .on_action(cx.listener(Self::handle_run_selected_sql_action))
            .on_action(cx.listener(Self::handle_run_cursor_statement_sql_action))
            .on_action(cx.listener(Self::handle_toggle_line_comment_action));
        if has_results && results_visible {
            div = div
                .child(self.render_has_results(window, cx))
                .child(ResizeEventHandler { view });
        } else {
            div = div.child(self.render_sql_editor(cx));
        }
        div
    }
}

// Make it Clone so we can use it in closures
impl Clone for SqlEditorTab {
    fn clone(&self) -> Self {
        Self {
            title: self.title.clone(),
            editor: self.editor.clone(),
            connection_id: self.connection_id.clone(),
            database_type: self.database_type.clone(),
            sql_result_tab_container: self.sql_result_tab_container.clone(),
            connection_select: self.connection_select.clone(),
            database_select: self.database_select.clone(),
            schema_select: self.schema_select.clone(),
            transaction_mode_select: self.transaction_mode_select.clone(),
            supports_schema: self.supports_schema,
            uses_schema_as_database: self.uses_schema_as_database,
            focus_handle: self.focus_handle.clone(),
            file_path: self.file_path.clone(),
            requires_name: self.requires_name.clone(),
            _save_task: None,
            result_panel_size: self.result_panel_size,
            resizing: false,
            bounds: self.bounds,
            transaction_mode: self.transaction_mode,
            manual_transaction: self.manual_transaction.clone(),
            manual_transaction_generation: self.manual_transaction_generation.clone(),
            manual_transaction_starting: self.manual_transaction_starting,
            manual_transaction_finishing: self.manual_transaction_finishing,
            auto_save_seq: self.auto_save_seq.clone(),
            is_dirty: self.is_dirty.clone(),
            context_generation: self.context_generation.clone(),
            execution_request_id: self.execution_request_id.clone(),
            _connection_subscription: None,
            statement_snapshot: self.statement_snapshot.clone(),
            viewport_statements: self.viewport_statements.clone(),
            statement_marker_states: self.statement_marker_states.clone(),
            active_statement_marker: self.active_statement_marker.clone(),
            last_frame_key: self.last_frame_key.clone(),
            _execution_state_subscription: None,
            _editor_input_subscription: None,
            diagnostic_run_id: self.diagnostic_run_id.clone(),
            _diagnostic_task: None,
            statement_run_id: self.statement_run_id.clone(),
            _statement_task: None,
            schema_snapshot: self.schema_snapshot.clone(),
            db_completion_info: self.db_completion_info.clone(),
            foreign_prefetch_inflight: self.foreign_prefetch_inflight.clone(),
            insert_values_highlight: self.insert_values_highlight.clone(),
            last_insert_hints_key: self.last_insert_hints_key,
        }
    }
}

impl Focusable for SqlEditorTab {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<SqlEditorEvent> for SqlEditorTab {}

impl EventEmitter<TabContentEvent> for SqlEditorTab {}

impl TabContent for SqlEditorTab {
    fn content_key(&self) -> &'static str {
        "SqlEditor"
    }

    fn title(&self, _cx: &App) -> SharedString {
        self.title.clone()
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        Some(IconName::Query.color())
    }

    fn closeable(&self, _cx: &App) -> bool {
        true
    }

    /// Deactivate the tab: drop in-flight async state without destroying the
    /// editor entity or its undo history (spec §19.4).
    ///
    /// Completion/hover/signature requests that captured the deactivated editor
    /// are invalidated, debounced tasks are cancelled, and transient popovers
    /// are cleared so stale results never repaint a hidden tab.
    fn on_deactivate(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, cx| {
            editor.invalidate_metadata_context(cx);
            editor.invalidate_completions(cx);
        });
        self.diagnostic_run_id.fetch_add(1, Ordering::SeqCst);
        self._diagnostic_task.take();
        self._statement_task.take();
    }

    /// Activate the tab: re-sync metadata-dependent decorations and schedule
    /// visible diagnostics for the restored viewport (spec §19.4).
    fn on_activate(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_statement_snapshot(cx);
        self.refresh_insert_value_hints(cx);
        self.run_diagnostics(cx);
    }

    fn try_close(
        &mut self,
        _tab_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        if self.has_manual_transaction_lifecycle() {
            window.push_notification(t!("Query.transaction_finish_before_close").to_string(), cx);
            return Task::ready(false);
        }
        if self.requires_name.load(Ordering::Relaxed) && !self.get_sql_text(cx).trim().is_empty() {
            return self.show_close_save_name_dialog(window, cx);
        }
        Task::ready(self.save_query(window, cx))
    }
}

struct ResizeEventHandler {
    view: Entity<SqlEditorTab>,
}

impl IntoElement for ResizeEventHandler {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for ResizeEventHandler {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        (window.request_layout(gpui::Style::default(), None, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let bounds = window.bounds();
        self.view.update(cx, |view, _| {
            view.bounds = Bounds {
                origin: Point::default(),
                size: bounds.size,
            };
        });
    }

    fn paint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.on_mouse_event({
            let view = self.view.clone();
            let resizing = view.read(cx).resizing;
            move |e: &MouseMoveEvent, phase, window, cx| {
                if !resizing {
                    return;
                }
                if !phase.bubble() {
                    return;
                }
                view.update(cx, |view, cx| view.resize(e.position, window, cx));
            }
        });

        window.on_mouse_event({
            let view = self.view.clone();
            move |_: &MouseUpEvent, phase, window, cx| {
                if phase.bubble() {
                    view.update(cx, |view, cx| view.done_resizing(window, cx));
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ForeignQualifierKind, ManualSqlExecutionAction, ManualTransactionAction,
        ManualTransactionInvalidationMode, ManualTransactionSession, ManualTransactionStopAction,
        QueryFileNameError, QueryToolbarAction, RUN_ALL_QUERY_KEY_BINDINGS,
        RUN_CURRENT_QUERY_KEY_BINDINGS, RunCurrentQuery, SCHEMA_COLUMN_FETCH_CONCURRENCY,
        SQL_EDITOR_CONTEXT, SQL_EDITOR_INPUT_CONTEXT, SqlDiagnosticIdentity, SqlMetadataScope,
        StatementScanInput, ToggleLineComment, can_start_query_execution,
        can_switch_query_connection, collect_bounded, current_statement_frame_decorations,
        foreign_prefetch_key, foreign_qualifier_fetch_scope, foreign_qualifier_scope,
        initial_database_select_value, insert_target_table, insert_values_range,
        is_current_diagnostic_identity, is_current_manual_transaction_owner,
        is_current_manual_transaction_start, is_current_query_context_generation,
        lookup_table_columns, manual_sql_execution_action, manual_transaction_control_sql,
        manual_transaction_invalidation_mode, manual_transaction_stop_action,
        match_sql_to_statement_marker, metadata_scope_selection, preferred_default_database,
        query_connection_context_label, query_connection_ids, query_file_path_for_name,
        query_toolbar_action, schema_changed_event_matches_scope, should_render_schema_select,
        sql_text_for_run_all, sql_text_for_toolbar_run, statement_for_gutter_marker,
        statement_marker_id, supports_manual_transactions, toggle_sql_line_comments,
        unquote_sql_identifier, viewport_statement_scan_input, write_new_sql_file, write_sql_file,
    };
    use db::DbManager;
    use db::sql_editor::statement_ranges::{SqlDialect, SqlStatementSnapshot};
    use gpui::{KeyBinding, KeyContext, Keymap, Keystroke};
    use gpui_component::input;
    use gpui_component::input::RangeDecorationStyle;
    use one_core::storage::DatabaseType;
    use ropey::Rope;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    const WIRE_PREFIX: &str = "/*onetcli-ipc-wire*/ ";

    #[test]
    fn insert_value_hints_do_not_paint_over_sql_text() {
        let source = include_str!("sql_editor_view.rs");
        let refresh = source
            .split("fn refresh_insert_value_hints")
            .nth(1)
            .unwrap()
            .split("fn statement_index_for_document")
            .next()
            .unwrap();

        assert!(!refresh.contains("InlineWidget"));
        assert!(!refresh.contains("set_inline_widgets"));
        assert!(refresh.contains("state.clear_inline_widgets(cx)"));
    }

    #[test]
    fn gutter_run_does_not_rescan_the_full_document_before_execution() {
        let source = include_str!("sql_editor_view.rs");
        let gutter_handler = source
            .split("fn bind_gutter_marker_event")
            .nth(1)
            .unwrap()
            .split("fn bind_connection_data_event")
            .next()
            .unwrap();
        let marker_binding = source
            .split("fn bind_execution_marker_for_sql")
            .nth(1)
            .unwrap()
            .split("fn is_context_generation_current")
            .next()
            .unwrap();

        assert!(!gutter_handler.contains("refresh_statement_snapshot(cx)"));
        assert!(!marker_binding.contains("refresh_statement_snapshot(cx)"));
    }

    #[test]
    fn foreign_prefetch_keys_are_isolated_by_metadata_scope() {
        let first = SqlMetadataScope::new("connection", DatabaseType::MySQL, 1)
            .with_database(Some("app".into()));
        let next_generation = SqlMetadataScope::new("connection", DatabaseType::MySQL, 2)
            .with_database(Some("app".into()));
        let other_database = SqlMetadataScope::new("connection", DatabaseType::MySQL, 1)
            .with_database(Some("other".into()));

        let first_key = foreign_prefetch_key(&first, "Analytics");
        assert_eq!(first_key.1, "analytics");
        assert_ne!(
            first_key,
            foreign_prefetch_key(&next_generation, "analytics")
        );
        assert_ne!(
            first_key,
            foreign_prefetch_key(&other_database, "analytics")
        );
    }

    #[test]
    fn foreign_qualifier_scopes_match_database_capabilities() {
        let pg_scope = SqlMetadataScope::new("connection", DatabaseType::PostgreSQL, 1)
            .with_database(Some("app".into()))
            .with_schema(Some("public".into()));
        let pg = foreign_qualifier_scope(&pg_scope, false, true);
        assert_eq!(pg.kind, ForeignQualifierKind::Schemas);
        assert_eq!(pg.current_name.as_deref(), Some("public"));
        assert_eq!(
            foreign_qualifier_fetch_scope(&pg_scope, "analytics", false, true),
            Some(("app".into(), Some("analytics".into())))
        );

        let mysql = foreign_qualifier_scope(&pg_scope, false, false);
        assert_eq!(mysql.kind, ForeignQualifierKind::Databases);
        assert_eq!(mysql.current_name.as_deref(), Some("app"));
        assert_eq!(
            foreign_qualifier_fetch_scope(&pg_scope, "other_db", false, false),
            Some(("other_db".into(), None))
        );

        let oracle_scope = SqlMetadataScope::new("connection", DatabaseType::Oracle, 1)
            .with_schema(Some("APP".into()));
        let oracle = foreign_qualifier_scope(&oracle_scope, true, false);
        assert_eq!(oracle.kind, ForeignQualifierKind::Schemas);
        assert_eq!(
            foreign_qualifier_fetch_scope(&oracle_scope, "OTHER", true, false),
            Some((String::new(), Some("OTHER".into())))
        );
    }

    #[test]
    fn large_documents_capture_only_a_viewport_statement_window() {
        let text = (0..3_000)
            .map(|row| format!("select {row};\n"))
            .collect::<String>();
        let rope = Rope::from_str(&text);

        let input = viewport_statement_scan_input(&rope, Some(1_500..1_520));

        let StatementScanInput::Window {
            text: window_text,
            base_line,
            analyzed_rows,
            ..
        } = input
        else {
            panic!("large laid-out document should use a viewport window");
        };
        assert!(base_line <= 1_500);
        assert!(analyzed_rows.start <= 1_500);
        assert!(analyzed_rows.end >= 1_520);
        assert!(window_text.len() < text.len() / 4);
    }

    #[test]
    fn small_or_unlaid_out_documents_keep_full_statement_scans() {
        let small = Rope::from_str("select 1;\nselect 2;");
        assert!(matches!(
            viewport_statement_scan_input(&small, Some(0..2)),
            StatementScanInput::Full { .. }
        ));

        let large = Rope::from_str(&"select 1;\n".repeat(2_100));
        assert!(matches!(
            viewport_statement_scan_input(&large, None),
            StatementScanInput::Full { .. }
        ));
    }

    fn metadata_scope_for_schema_changed_tests(
        database: Option<&str>,
        schema: Option<&str>,
    ) -> SqlMetadataScope {
        SqlMetadataScope {
            connection_id: "connection-1".to_string(),
            catalog: None,
            database: database.map(str::to_string),
            schema: schema.map(str::to_string),
            database_type: DatabaseType::PostgreSQL,
            generation: 7,
        }
    }

    #[test]
    fn schema_changed_matches_same_database_and_schema_scope() {
        let scope = metadata_scope_for_schema_changed_tests(Some("app"), Some("public"));

        assert!(schema_changed_event_matches_scope(
            "connection-1",
            "app",
            Some("public"),
            &scope,
            true,
            false
        ));
    }

    #[test]
    fn schema_changed_rejects_other_connection_database_or_schema() {
        let scope = metadata_scope_for_schema_changed_tests(Some("app"), Some("public"));

        assert!(!schema_changed_event_matches_scope(
            "connection-2",
            "app",
            Some("public"),
            &scope,
            true,
            false
        ));
        assert!(!schema_changed_event_matches_scope(
            "connection-1",
            "other",
            Some("public"),
            &scope,
            true,
            false
        ));
        assert!(!schema_changed_event_matches_scope(
            "connection-1",
            "app",
            Some("private"),
            &scope,
            true,
            false
        ));
    }

    #[test]
    fn database_wide_schema_changed_matches_current_schema() {
        let scope = metadata_scope_for_schema_changed_tests(Some("app"), Some("public"));

        assert!(schema_changed_event_matches_scope(
            "connection-1",
            "app",
            None,
            &scope,
            true,
            false
        ));
    }

    #[test]
    fn schema_as_database_schema_changed_uses_event_schema() {
        let mut scope = metadata_scope_for_schema_changed_tests(None, Some("APP"));
        scope.database_type = DatabaseType::Oracle;

        assert!(schema_changed_event_matches_scope(
            "connection-1",
            "ignored-database",
            Some("APP"),
            &scope,
            false,
            true
        ));
        assert!(!schema_changed_event_matches_scope(
            "connection-1",
            "ignored-database",
            Some("OTHER"),
            &scope,
            false,
            true
        ));
    }

    #[test]
    fn schema_as_database_schema_changed_accepts_encoded_database_fallback() {
        let mut scope = metadata_scope_for_schema_changed_tests(None, Some("APP"));
        scope.database_type = DatabaseType::Oracle;

        assert!(schema_changed_event_matches_scope(
            "connection-1",
            "APP",
            None,
            &scope,
            false,
            true
        ));
    }

    fn build_explain_sql(database_type: DatabaseType, sql: &str) -> Option<String> {
        let plugin = DbManager::default()
            .get_plugin(&database_type)
            .expect("plugin should exist");
        normalize_explain_sql(plugin.build_explain_sql(sql))
    }

    fn normalize_explain_sql(sql: Option<String>) -> Option<String> {
        let sql = sql?;
        let Some(request) = sql.strip_prefix(WIRE_PREFIX) else {
            return Some(sql);
        };
        serde_json::from_str::<serde_json::Value>(request)
            .ok()
            .and_then(|value| {
                value
                    .get("params")
                    .and_then(|params| params.get("fallback_sql"))
                    .and_then(|fallback| fallback.as_str())
                    .map(str::to_string)
            })
            .or(Some(sql))
    }

    fn temp_query_dir(test_name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "navop-sql-editor-{test_name}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).expect("temporary query directory should be created");
        path
    }

    fn sql_text_for_run_current(
        snapshot: &SqlStatementSnapshot,
        selected_text: &str,
        cursor_offset: usize,
    ) -> String {
        if selected_text.trim().is_empty() {
            snapshot
                .statement_at_cursor(cursor_offset)
                .map(|statement| snapshot.statement_text(statement).to_string())
                .unwrap_or_default()
        } else {
            selected_text.to_string()
        }
    }

    fn sql_text_for_run_cursor_statement(
        snapshot: &SqlStatementSnapshot,
        cursor_offset: usize,
    ) -> String {
        snapshot
            .statement_at_cursor(cursor_offset)
            .map(|statement| snapshot.statement_text(statement).to_string())
            .unwrap_or_default()
    }

    #[test]
    fn query_connection_context_distinguishes_same_database_across_connections() {
        let production = query_connection_context_label("生产环境", "prod.example.com:5432");
        let staging = query_connection_context_label("测试环境", "staging.example.com:5432");

        assert_eq!("生产环境 · prod.example.com:5432", production);
        assert_eq!("测试环境 · staging.example.com:5432", staging);
        assert_ne!(production, staging);
    }

    #[test]
    fn query_connection_context_uses_available_connection_details() {
        assert_eq!(
            "本地数据库",
            query_connection_context_label("本地数据库", "")
        );
        assert_eq!(
            "/tmp/app.db",
            query_connection_context_label("", "/tmp/app.db")
        );
    }

    #[test]
    fn query_connection_ids_keep_workspace_order_and_current_connection() {
        let available = vec![
            "connection-2".to_string(),
            "connection-1".to_string(),
            "connection-2".to_string(),
        ];

        assert_eq!(
            vec!["connection-2", "connection-1"],
            query_connection_ids(&available, "connection-1")
        );
        assert_eq!(
            vec!["connection-2", "connection-1", "connection-3"],
            query_connection_ids(&available, "connection-3")
        );
    }

    #[test]
    fn query_connection_switch_is_blocked_while_query_or_transaction_is_active() {
        assert!(can_switch_query_connection(false, false));
        assert!(!can_switch_query_connection(true, false));
        assert!(!can_switch_query_connection(false, true));
        assert!(!can_switch_query_connection(true, true));
    }

    #[test]
    fn a_running_query_cannot_be_superseded_by_another_editor_action() {
        assert!(can_start_query_execution(false));
        assert!(!can_start_query_execution(true));
    }

    #[test]
    fn manual_transaction_stop_assigns_exactly_one_session_close_owner() {
        assert_eq!(
            ManualTransactionStopAction::CancelStart,
            manual_transaction_stop_action(false, true, false)
        );
        assert_eq!(
            ManualTransactionStopAction::CloseInstalledSession,
            manual_transaction_stop_action(true, true, true)
        );
        assert_eq!(
            ManualTransactionStopAction::CloseInstalledSession,
            manual_transaction_stop_action(true, false, true)
        );
        assert_eq!(
            ManualTransactionStopAction::None,
            manual_transaction_stop_action(false, false, true)
        );
        assert_eq!(
            ManualTransactionStopAction::None,
            manual_transaction_stop_action(true, false, false)
        );
    }

    #[test]
    fn manual_transaction_sql_binds_marker_only_after_all_synchronous_validation_passes() {
        let unsupported = manual_sql_execution_action(&DatabaseType::ClickHouse, None, false);
        let starting = manual_sql_execution_action(&DatabaseType::PostgreSQL, None, true);
        let finishing = manual_sql_execution_action(&DatabaseType::PostgreSQL, Some(true), true);
        let scope_mismatch =
            manual_sql_execution_action(&DatabaseType::PostgreSQL, Some(false), false);
        let installed = manual_sql_execution_action(&DatabaseType::PostgreSQL, Some(true), false);
        let start = manual_sql_execution_action(&DatabaseType::PostgreSQL, None, false);

        for rejected in [unsupported, starting, finishing, scope_mismatch] {
            assert!(!rejected.binds_execution_marker());
        }
        assert_eq!(ManualSqlExecutionAction::RunInstalledSession, installed);
        assert_eq!(ManualSqlExecutionAction::StartSession, start);
        assert!(installed.binds_execution_marker());
        assert!(start.binds_execution_marker());
    }

    #[test]
    fn stale_query_context_generation_is_rejected() {
        assert!(is_current_query_context_generation(3, 3));
        assert!(!is_current_query_context_generation(2, 3));
    }

    #[test]
    fn diagnostic_identity_rejects_stale_run_revision_or_context() {
        let expected = SqlDiagnosticIdentity {
            run_id: 8,
            document_revision: 13,
            context_generation: 5,
        };

        assert!(is_current_diagnostic_identity(expected, expected));
        assert!(!is_current_diagnostic_identity(
            expected,
            SqlDiagnosticIdentity {
                run_id: 9,
                ..expected
            }
        ));
        assert!(!is_current_diagnostic_identity(
            expected,
            SqlDiagnosticIdentity {
                document_revision: 14,
                ..expected
            }
        ));
        assert!(!is_current_diagnostic_identity(
            expected,
            SqlDiagnosticIdentity {
                context_generation: 6,
                ..expected
            }
        ));
    }

    #[test]
    fn manual_transaction_owner_requires_generation_and_session_id() {
        assert!(is_current_manual_transaction_owner(
            3,
            "session-1",
            3,
            Some("session-1")
        ));
        assert!(!is_current_manual_transaction_owner(
            3,
            "session-1",
            4,
            Some("session-1")
        ));
        assert!(!is_current_manual_transaction_owner(
            3,
            "session-1",
            3,
            Some("session-2")
        ));
        assert!(!is_current_manual_transaction_owner(
            3,
            "session-1",
            3,
            None
        ));
    }

    #[test]
    fn manual_transaction_start_cleanup_requires_current_start_generation() {
        assert!(is_current_manual_transaction_start(3, 3, true));
        assert!(!is_current_manual_transaction_start(3, 4, true));
        assert!(!is_current_manual_transaction_start(3, 3, false));
    }

    #[test]
    fn stale_metadata_scope_identity_is_rejected() {
        let make_scope = |connection_id: &str,
                          database: Option<&str>,
                          schema: Option<&str>,
                          generation: u64| SqlMetadataScope {
            connection_id: connection_id.to_string(),
            catalog: None,
            database: database.map(str::to_string),
            schema: schema.map(str::to_string),
            database_type: DatabaseType::MySQL,
            generation,
        };
        let scope = make_scope("conn-1", Some("sales"), Some("public"), 4);

        assert_eq!(Some(scope.clone()), Some(scope.clone()));
        assert_ne!(
            Some(scope.clone()),
            Some(make_scope("conn-2", Some("sales"), Some("public"), 4))
        );
        assert_ne!(
            Some(scope.clone()),
            Some(make_scope("conn-1", Some("hr"), Some("public"), 4))
        );
        assert_ne!(
            Some(scope.clone()),
            Some(make_scope("conn-1", Some("sales"), Some("private"), 4))
        );
        assert_ne!(
            Some(scope),
            Some(make_scope("conn-1", Some("sales"), Some("public"), 5))
        );
    }

    #[test]
    fn metadata_scope_selection_respects_database_semantics() {
        let selected_database = Some("selected-db".to_string());
        let selected_schema = Some("selected-schema".to_string());
        assert_eq!(
            metadata_scope_selection(
                Some("sales"),
                selected_database.clone(),
                selected_schema.clone(),
                true,
                false
            ),
            (
                Some("sales".to_string()),
                Some("selected-schema".to_string())
            )
        );
        assert_eq!(
            metadata_scope_selection(
                None,
                selected_database.clone(),
                selected_schema.clone(),
                true,
                false
            ),
            (
                Some("selected-db".to_string()),
                Some("selected-schema".to_string())
            )
        );
        assert_eq!(
            metadata_scope_selection(
                Some("hr"),
                selected_database.clone(),
                selected_schema.clone(),
                true,
                true
            ),
            (None, Some("hr".to_string()))
        );
        assert_eq!(
            metadata_scope_selection(None, selected_database, selected_schema, false, false),
            (Some("selected-db".to_string()), None)
        );
    }

    #[test]
    fn metadata_scope_selection_ignores_empty_override_for_schema_as_database() {
        // Oracle（uses_schema_as_database）：update_schema_for_db 用 db=""（scope.database
        // 为 None 时的 unwrap_or_default）回调 current_metadata_scope，空串必须被忽略，
        // 否则会覆盖 schema 下拉的当前选择，导致元数据发布守卫永远不匹配。
        assert_eq!(
            metadata_scope_selection(
                Some(""),
                Some("ignored-db".to_string()),
                Some("APP".to_string()),
                false,
                true
            ),
            (None, Some("APP".to_string()))
        );
    }

    #[test]
    fn metadata_scope_selection_ignores_empty_override_for_regular_databases() {
        assert_eq!(
            metadata_scope_selection(
                Some(""),
                Some("selected-db".to_string()),
                None,
                false,
                false
            ),
            (Some("selected-db".to_string()), None)
        );
    }

    #[test]
    fn query_file_path_requires_non_empty_name() {
        let directory = temp_query_dir("empty-name");

        assert_eq!(
            Err(QueryFileNameError::Empty),
            query_file_path_for_name(&directory, "")
        );
        assert_eq!(
            Err(QueryFileNameError::Empty),
            query_file_path_for_name(&directory, "   ")
        );

        std::fs::remove_dir_all(directory).expect("temporary query directory should be removed");
    }

    #[test]
    fn query_file_path_rejects_path_components() {
        let directory = temp_query_dir("path-components");

        assert_eq!(
            Err(QueryFileNameError::Invalid),
            query_file_path_for_name(&directory, "../report")
        );
        assert_eq!(
            Err(QueryFileNameError::Invalid),
            query_file_path_for_name(&directory, "nested/report")
        );

        std::fs::remove_dir_all(directory).expect("temporary query directory should be removed");
    }

    #[test]
    fn query_file_path_rejects_cross_platform_invalid_names() {
        let directory = temp_query_dir("invalid-names");

        assert_eq!(
            Err(QueryFileNameError::Invalid),
            query_file_path_for_name(&directory, "report:daily")
        );
        assert_eq!(
            Err(QueryFileNameError::Invalid),
            query_file_path_for_name(&directory, "CON")
        );
        assert_eq!(
            Err(QueryFileNameError::Invalid),
            query_file_path_for_name(&directory, "nul.sql")
        );

        std::fs::remove_dir_all(directory).expect("temporary query directory should be removed");
    }

    #[test]
    fn query_file_path_appends_sql_extension_once() {
        let directory = temp_query_dir("extension");

        assert_eq!(
            Ok(directory.join("report.sql")),
            query_file_path_for_name(&directory, "report")
        );
        assert_eq!(
            Ok(directory.join("report.sql")),
            query_file_path_for_name(&directory, "report.sql")
        );

        std::fs::remove_dir_all(directory).expect("temporary query directory should be removed");
    }

    #[test]
    fn query_file_path_rejects_duplicate_name_case_insensitively() {
        let directory = temp_query_dir("duplicate");
        let existing_path = directory.join("Report.sql");
        std::fs::write(&existing_path, "select 1;").expect("fixture query should be written");

        assert_eq!(
            Err(QueryFileNameError::AlreadyExists),
            query_file_path_for_name(&directory, "report")
        );
        assert_eq!(
            "select 1;",
            std::fs::read_to_string(existing_path).expect("fixture query should remain readable")
        );

        std::fs::remove_dir_all(directory).expect("temporary query directory should be removed");
    }

    #[test]
    fn write_sql_file_overwrites_current_named_query() {
        let directory = temp_query_dir("overwrite");
        let file_path = directory.join("report.sql");
        std::fs::write(&file_path, "select 1;").expect("fixture query should be written");

        write_sql_file(&file_path, "select 2;").expect("named query should be overwritten");

        assert_eq!(
            "select 2;",
            std::fs::read_to_string(file_path).expect("saved query should be readable")
        );
        std::fs::remove_dir_all(directory).expect("temporary query directory should be removed");
    }

    #[test]
    fn write_new_sql_file_does_not_overwrite_existing_query() {
        let directory = temp_query_dir("create-new");
        let file_path = directory.join("report.sql");
        std::fs::write(&file_path, "select 1;").expect("fixture query should be written");

        let error = write_new_sql_file(&file_path, "select 2;")
            .expect_err("new query save should reject an existing file");

        assert_eq!(std::io::ErrorKind::AlreadyExists, error.kind());
        assert_eq!(
            "select 1;",
            std::fs::read_to_string(file_path).expect("existing query should remain unchanged")
        );
        std::fs::remove_dir_all(directory).expect("temporary query directory should be removed");
    }

    #[test]
    fn run_query_text_prefers_selected_sql_when_present() {
        let snapshot = SqlStatementSnapshot::new(
            "select * from users;",
            SqlDialect::from(&DatabaseType::MySQL),
        );
        let actual = sql_text_for_run_current(&snapshot, "select id from users;", 0);

        assert_eq!("select id from users;", actual);
    }

    #[test]
    fn toolbar_run_text_prefers_selection_when_present() {
        let actual = sql_text_for_toolbar_run(
            "select * from users;\nselect * from orders;",
            "select * from users;",
        );

        assert_eq!("select * from users;", actual);
    }

    #[test]
    fn toolbar_run_text_uses_full_editor_sql_without_selection() {
        let sql = "select * from users;\nselect * from orders;";
        let actual = sql_text_for_toolbar_run(sql, "   ");

        assert_eq!(sql, actual);
    }

    #[test]
    fn run_query_text_uses_current_statement_when_selection_is_blank() {
        let sql = "select * from users;\nselect * from orders;\nselect * from products;";
        let cursor_offset = sql.find("orders").expect("statement exists") + "orders".len();
        let snapshot = SqlStatementSnapshot::new(sql, SqlDialect::from(&DatabaseType::MySQL));
        let actual = sql_text_for_run_current(&snapshot, "   ", cursor_offset);

        assert_eq!("select * from orders", actual);
    }

    #[test]
    fn run_query_text_uses_full_multiline_statement_when_cursor_is_inside() {
        let sql = "select * from users;\nselect id,\n       name\nfrom orders\nwhere active = 1;\nselect * from products;";
        let cursor_offset = sql.find("name").expect("statement exists") + "na".len();
        let snapshot = SqlStatementSnapshot::new(sql, SqlDialect::from(&DatabaseType::MySQL));
        let actual = sql_text_for_run_current(&snapshot, "", cursor_offset);

        assert_eq!(
            "select id,\n       name\nfrom orders\nwhere active = 1",
            actual
        );
    }

    #[test]
    fn run_query_text_ignores_semicolon_inside_string() {
        let sql = "select 1;\nselect ';not delimiter' as value;\nselect 3;";
        let cursor_offset = sql.find("value").expect("statement exists") + "value".len();
        let snapshot = SqlStatementSnapshot::new(sql, SqlDialect::from(&DatabaseType::MySQL));
        let actual = sql_text_for_run_current(&snapshot, "", cursor_offset);

        assert_eq!("select ';not delimiter' as value", actual);
    }

    #[test]
    fn run_all_query_text_uses_editor_sql_even_with_selection() {
        let sql = "select * from users;\nselect * from orders;";
        let actual = sql_text_for_run_all(sql, "select * from users;");

        assert_eq!(sql, actual);
    }

    #[test]
    fn statement_marker_id_binds_revision_and_exact_range() {
        let snapshot = SqlStatementSnapshot::new(
            "select 1;\nselect 2;",
            SqlDialect::from(&DatabaseType::MySQL),
        );
        let statement = &snapshot.statement_ranges()[1];

        assert_eq!(statement_marker_id(7, statement), "sql-statement:7:10:18");
    }

    #[test]
    fn gutter_marker_resolves_the_exact_same_line_statement() {
        let sql = "select 1; select 2;";
        let snapshot = SqlStatementSnapshot::new(sql, SqlDialect::from(&DatabaseType::MySQL));
        let statements = snapshot.statement_ranges();
        assert_eq!(2, statements.len());
        assert_eq!(statements[0].start_line, statements[1].start_line);

        let second_id = statement_marker_id(7, &statements[1]);
        let statement =
            statement_for_gutter_marker(statements, 7, &second_id, statements[1].start_line)
                .expect("second marker should resolve independently on the same line");

        assert_eq!("select 2", snapshot.statement_text(statement));
        assert!(
            statement_for_gutter_marker(statements, 8, &second_id, statements[1].start_line)
                .is_none()
        );
    }

    #[test]
    fn frame_decorations_cover_statement_through_delimiter() {
        let sql = "select 1;\n  select * from 用户表;  \nselect 3;";
        let snapshot = SqlStatementSnapshot::new(sql, SqlDialect::from(&DatabaseType::MySQL));
        // Cursor inside the second statement.
        let cursor = sql.find("用户表").unwrap() + "用户".len();
        let decorations =
            current_statement_frame_decorations(&snapshot, 5, cursor, &(0..0), sql.len(), None);

        assert_eq!(decorations.len(), 1);
        let decoration = &decorations[0];
        let statement = &snapshot.statement_ranges()[1];
        let delim_end = statement
            .delimiter_range
            .map(|delimiter| delimiter.end_byte)
            .unwrap();
        // Frame runs from the first SQL token through the trailing `;`.
        assert_eq!(
            decoration.range(),
            &(statement.sql_range.start_byte..delim_end)
        );
        assert_eq!(
            decoration.id().to_string(),
            format!(
                "sql-frame:5:{}:{}",
                decoration.range().start,
                decoration.range().end
            )
        );
        assert_eq!(&sql[decoration.range().clone()], "select * from 用户表;");
    }

    #[test]
    fn frame_decorations_suppressed_when_selection_active() {
        let sql = "select 1;\nselect 2;";
        let snapshot = SqlStatementSnapshot::new(sql, SqlDialect::from(&DatabaseType::MySQL));

        let decorations =
            current_statement_frame_decorations(&snapshot, 3, 2, &(0..9), sql.len(), None);

        assert!(decorations.is_empty());
    }

    #[test]
    fn frame_decorations_empty_outside_statement() {
        let snapshot = SqlStatementSnapshot::new("", SqlDialect::from(&DatabaseType::MySQL));
        let decorations = current_statement_frame_decorations(&snapshot, 3, 0, &(0..0), 0, None);
        assert!(decorations.is_empty());
    }

    #[test]
    fn match_sql_to_statement_marker_binds_exact_cursor_statement() {
        let sql = "select 1;\nselect 2;";
        let snapshot = SqlStatementSnapshot::new(sql, SqlDialect::from(&DatabaseType::MySQL));
        let cursor = sql.find("select 2").unwrap() + 1;

        let id = match_sql_to_statement_marker(&snapshot, 9, cursor, "select 2");

        assert_eq!(id.as_deref(), Some("sql-statement:9:10:18"));
    }

    #[test]
    fn match_sql_to_statement_marker_rejects_other_sql_or_selection() {
        let sql = "select 1;\nselect 2;";
        let snapshot = SqlStatementSnapshot::new(sql, SqlDialect::from(&DatabaseType::MySQL));
        let cursor = sql.find("select 2").unwrap() + 1;

        // Different SQL text: not the statement at the cursor.
        assert_eq!(
            match_sql_to_statement_marker(&snapshot, 9, cursor, "select 3"),
            None
        );
        // Selection-style runs with surrounding text do not bind.
        assert_eq!(
            match_sql_to_statement_marker(&snapshot, 9, cursor, "select 1;\nselect 2"),
            None
        );
        // Cursor in whitespace between statements.
        let between = sql.find("\n").unwrap();
        assert_eq!(
            match_sql_to_statement_marker(&snapshot, 9, between, "select 2"),
            None
        );
    }

    #[test]
    fn match_sql_to_statement_marker_id_changes_with_revision() {
        let sql = "select 1;\nselect 2;";
        let snapshot = SqlStatementSnapshot::new(sql, SqlDialect::from(&DatabaseType::MySQL));
        let cursor = sql.find("select 2").unwrap() + 1;

        let old = match_sql_to_statement_marker(&snapshot, 4, cursor, "select 2");
        let new = match_sql_to_statement_marker(&snapshot, 8, cursor, "select 2");

        assert_ne!(old, new);
        // 编辑使 revision 变化后，旧 id 不再能匹配当前快照。
        assert_eq!(old.as_deref(), Some("sql-statement:4:10:18"));
        assert_eq!(new.as_deref(), Some("sql-statement:8:10:18"));
    }

    #[test]
    fn run_cursor_statement_text_uses_cursor_statement() {
        let sql = "select 1;\n  select * from 用户表;  \nselect 3;";
        let cursor_offset = sql.find("用户表").expect("line exists") + "用户".len();
        let snapshot = SqlStatementSnapshot::new(sql, SqlDialect::from(&DatabaseType::MySQL));
        let actual = sql_text_for_run_cursor_statement(&snapshot, cursor_offset);

        assert_eq!("select * from 用户表", actual);
    }

    #[test]
    fn run_cursor_statement_text_uses_full_multiline_statement() {
        let sql = "select * from users;\nselect id,\n       name\nfrom orders\nwhere active = 1;\nselect * from products;";
        let cursor_offset = sql.find("name").expect("statement exists") + "na".len();
        let snapshot = SqlStatementSnapshot::new(sql, SqlDialect::from(&DatabaseType::MySQL));
        let actual = sql_text_for_run_cursor_statement(&snapshot, cursor_offset);

        assert_eq!(
            "select id,\n       name\nfrom orders\nwhere active = 1",
            actual
        );
    }

    #[test]
    fn run_cursor_statement_text_handles_last_statement() {
        let sql = "select 1;\nselect 2";
        let snapshot = SqlStatementSnapshot::new(sql, SqlDialect::from(&DatabaseType::MySQL));
        let actual = sql_text_for_run_cursor_statement(&snapshot, sql.len());

        assert_eq!("select 2", actual);
    }

    #[test]
    fn run_query_key_bindings_separate_current_and_all_modes() {
        assert!(RUN_CURRENT_QUERY_KEY_BINDINGS.contains(&"cmd-enter"));
        assert!(RUN_CURRENT_QUERY_KEY_BINDINGS.contains(&"ctrl-enter"));
        assert!(RUN_ALL_QUERY_KEY_BINDINGS.contains(&"cmd-shift-enter"));
        assert!(RUN_ALL_QUERY_KEY_BINDINGS.contains(&"ctrl-shift-enter"));
    }

    #[test]
    fn secondary_enter_binding_wins_inside_sql_input_context() {
        let keymap = Keymap::new(vec![
            KeyBinding::new(
                "secondary-enter",
                input::Enter {
                    secondary: true,
                    shift: false,
                },
                Some("Input"),
            ),
            KeyBinding::new(
                "secondary-enter",
                RunCurrentQuery,
                Some("SqlEditor > Input"),
            ),
        ]);
        let contexts = vec![
            KeyContext::parse(SQL_EDITOR_CONTEXT).expect("valid context"),
            KeyContext::parse("Input").expect("valid context"),
        ];
        let keystroke = Keystroke::parse("secondary-enter").expect("valid keystroke");
        let (bindings, _) = keymap.bindings_for_input(&[keystroke], &contexts);

        assert!(
            bindings
                .first()
                .is_some_and(|binding| binding.action().partial_eq(&RunCurrentQuery))
        );
    }

    #[test]
    fn ctrl_slash_binding_wins_inside_sql_input_context() {
        let keymap = Keymap::new(vec![
            KeyBinding::new("ctrl-/", input::SelectAll, Some("Input")),
            KeyBinding::new("ctrl-/", ToggleLineComment, Some(SQL_EDITOR_INPUT_CONTEXT)),
        ]);
        let contexts = vec![
            KeyContext::parse(SQL_EDITOR_CONTEXT).expect("valid context"),
            KeyContext::parse("Input").expect("valid context"),
        ];
        let keystroke = Keystroke::parse("ctrl-/").expect("valid keystroke");
        let (bindings, _) = keymap.bindings_for_input(&[keystroke], &contexts);

        assert!(
            bindings
                .first()
                .is_some_and(|binding| binding.action().partial_eq(&ToggleLineComment))
        );
    }

    #[test]
    fn toggle_line_comment_comments_and_uncomments_current_line() {
        let sql = "select *\n  from users";
        let cursor = sql.find("from").expect("line exists") + 2;

        let commented = toggle_sql_line_comments(sql, cursor..cursor);
        let commented_sql = commented.apply_to(sql);
        assert_eq!("select *\n  -- from users", commented_sql);
        assert_eq!(cursor + 3..cursor + 3, commented.selection);

        let uncommented = toggle_sql_line_comments(&commented_sql, commented.selection.clone());
        assert_eq!(sql, uncommented.apply_to(&commented_sql));
        assert_eq!(cursor..cursor, uncommented.selection);
    }

    #[test]
    fn toggle_line_comment_applies_one_operation_to_selected_lines() {
        let sql = "select id\n  from users\n-- where active = 1";
        let selection = 0..sql.len();

        let commented = toggle_sql_line_comments(sql, selection);

        assert_eq!(
            "-- select id\n  -- from users\n-- -- where active = 1",
            commented.apply_to(sql)
        );
    }

    #[test]
    fn toggle_line_comment_uncomments_when_all_selected_code_lines_are_commented() {
        let sql = "  -- select id\n\n\t-- from users";

        let uncommented = toggle_sql_line_comments(sql, 0..sql.len());

        assert_eq!("  select id\n\n\tfrom users", uncommented.apply_to(sql));
    }

    #[test]
    fn toggle_line_comment_does_not_include_next_line_at_selection_end() {
        let sql = "select 1\nselect 2";
        let next_line_start = sql.find("select 2").expect("line exists");

        let commented = toggle_sql_line_comments(sql, 0..next_line_start);

        assert_eq!("-- select 1\nselect 2", commented.apply_to(sql));
    }

    #[test]
    fn toggle_line_comment_preserves_utf8_and_crlf() {
        let sql = "  select * from 用户表;\r\n\twhere 名称 = '测试';";

        let commented = toggle_sql_line_comments(sql, 0..sql.len());

        assert_eq!(
            "  -- select * from 用户表;\r\n\t-- where 名称 = '测试';",
            commented.apply_to(sql)
        );
    }

    #[test]
    fn schema_select_is_visible_when_schema_is_database() {
        assert!(should_render_schema_select(true, true));
        assert!(should_render_schema_select(false, true));
        assert!(should_render_schema_select(true, false));
        assert!(!should_render_schema_select(false, false));
    }

    #[test]
    fn manual_transactions_are_only_available_for_transactional_databases() {
        assert!(supports_manual_transactions(&DatabaseType::MySQL));
        assert!(supports_manual_transactions(&DatabaseType::PostgreSQL));
        assert!(supports_manual_transactions(&DatabaseType::SQLite));
        assert!(supports_manual_transactions(&DatabaseType::DuckDB));
        assert!(supports_manual_transactions(&DatabaseType::MSSQL));
        assert!(supports_manual_transactions(&DatabaseType::Oracle));
        assert!(!supports_manual_transactions(&DatabaseType::ClickHouse));
        assert!(!supports_manual_transactions(&DatabaseType::External {
            driver_id: "demo".to_string(),
        }));
    }

    #[test]
    fn executing_query_toolbar_action_is_stop_and_remains_clickable() {
        assert_eq!(QueryToolbarAction::Stop, query_toolbar_action(true, false));
        assert_eq!(QueryToolbarAction::Stop, query_toolbar_action(true, true));
        assert_eq!(
            QueryToolbarAction::RunSelected,
            query_toolbar_action(false, true)
        );
        assert_eq!(QueryToolbarAction::Run, query_toolbar_action(false, false));
    }

    #[test]
    fn manual_transaction_control_sql_matches_database_dialect() {
        assert_eq!(
            Some("BEGIN"),
            manual_transaction_control_sql(&DatabaseType::MySQL, ManualTransactionAction::Begin)
        );
        assert_eq!(
            Some("BEGIN TRANSACTION"),
            manual_transaction_control_sql(&DatabaseType::MSSQL, ManualTransactionAction::Begin)
        );
        assert_eq!(
            None,
            manual_transaction_control_sql(&DatabaseType::Oracle, ManualTransactionAction::Begin)
        );
        assert_eq!(
            Some("COMMIT"),
            manual_transaction_control_sql(
                &DatabaseType::PostgreSQL,
                ManualTransactionAction::Commit
            )
        );
        assert_eq!(
            Some("ROLLBACK"),
            manual_transaction_control_sql(
                &DatabaseType::SQLite,
                ManualTransactionAction::Rollback
            )
        );
    }

    #[test]
    fn manual_transaction_session_scope_must_match_database_and_schema() {
        let session = ManualTransactionSession::new(
            "session-1".to_string(),
            Some("app_db".to_string()),
            Some("public".to_string()),
        );

        assert!(session.matches_scope(Some("app_db"), Some("public")));
        assert!(!session.matches_scope(Some("analytics"), Some("public")));
        assert!(!session.matches_scope(Some("app_db"), Some("private")));
        assert!(!session.matches_scope(None, Some("public")));
    }

    #[test]
    fn manual_transaction_invalidation_respects_database_ddl_semantics() {
        for database_type in [DatabaseType::MySQL, DatabaseType::Oracle] {
            assert_eq!(
                ManualTransactionInvalidationMode::Immediate,
                manual_transaction_invalidation_mode(&database_type)
            );
        }
        for database_type in [
            DatabaseType::PostgreSQL,
            DatabaseType::SQLite,
            DatabaseType::DuckDB,
            DatabaseType::MSSQL,
        ] {
            assert_eq!(
                ManualTransactionInvalidationMode::Deferred,
                manual_transaction_invalidation_mode(&database_type)
            );
        }
    }

    #[test]
    fn manual_transaction_starts_with_empty_pending_invalidation() {
        let session =
            ManualTransactionSession::new("session-1".to_string(), None, Some("public".into()));

        assert!(session.pending_invalidation().lock().is_empty());
    }

    #[test]
    fn schema_as_database_initial_selection_prefers_schema() {
        assert_eq!(
            Some("COMI_SERVER2112".to_string()),
            initial_database_select_value(
                Some(String::new()),
                Some("COMI_SERVER2112".to_string()),
                true,
            )
        );
    }

    #[test]
    fn normal_database_initial_selection_uses_database() {
        assert_eq!(
            Some("app_db".to_string()),
            initial_database_select_value(
                Some("app_db".to_string()),
                Some("public".to_string()),
                false,
            )
        );
    }

    #[test]
    fn preferred_default_database_matches_login_database_in_list() {
        let available = vec!["information_schema".to_string(), "app_db".to_string()];

        assert_eq!(
            Some("app_db".to_string()),
            preferred_default_database(Some("app_db".to_string()), &available)
        );
    }

    #[test]
    fn preferred_default_database_ignores_login_database_missing_from_list() {
        let available = vec!["information_schema".to_string()];

        assert_eq!(
            None,
            preferred_default_database(Some("app_db".to_string()), &available)
        );
    }

    #[test]
    fn preferred_default_database_ignores_empty_login_database() {
        let available = vec!["app_db".to_string()];

        assert_eq!(None, preferred_default_database(None, &available));
        assert_eq!(
            None,
            preferred_default_database(Some("   ".to_string()), &available)
        );
    }

    #[test]
    fn preferred_default_database_trims_login_database() {
        let available = vec!["app_db".to_string()];

        assert_eq!(
            Some("app_db".to_string()),
            preferred_default_database(Some("  app_db  ".to_string()), &available)
        );
    }

    #[test]
    fn test_build_explain_sql_mysql() {
        assert_eq!(
            build_explain_sql(DatabaseType::MySQL, " SELECT * FROM users "),
            Some("EXPLAIN SELECT * FROM users".to_string())
        );
    }

    #[test]
    fn test_build_explain_sql_sqlite() {
        assert_eq!(
            build_explain_sql(DatabaseType::SQLite, "select * from users"),
            Some("EXPLAIN QUERY PLAN select * from users".to_string())
        );
    }

    #[test]
    fn test_build_explain_sql_duckdb() {
        assert_eq!(
            build_explain_sql(DatabaseType::DuckDB, "select * from users"),
            Some("EXPLAIN select * from users".to_string())
        );
    }

    #[test]
    fn test_build_explain_sql_mssql() {
        assert_eq!(
            build_explain_sql(DatabaseType::MSSQL, "select * from users"),
            Some("SET SHOWPLAN_TEXT ON;\nselect * from users\nSET SHOWPLAN_TEXT OFF;".to_string())
        );
    }

    #[test]
    fn test_build_explain_sql_oracle() {
        assert_eq!(
            build_explain_sql(DatabaseType::Oracle, "select * from users"),
            Some(
                "EXPLAIN PLAN FOR select * from users;\nSELECT PLAN_TABLE_OUTPUT FROM TABLE(DBMS_XPLAN.DISPLAY())"
                    .to_string()
            )
        );
    }

    #[test]
    fn test_build_explain_sql_mysql_multiple_statements() {
        assert_eq!(
            build_explain_sql(
                DatabaseType::MySQL,
                "select * from users; select * from posts;"
            ),
            Some("EXPLAIN select * from users;\nEXPLAIN select * from posts".to_string())
        );
    }

    #[test]
    fn test_build_explain_sql_mysql_preserves_semicolon_in_string() {
        assert_eq!(
            build_explain_sql(DatabaseType::MySQL, "select ';' as semi; select 2 as id;"),
            Some("EXPLAIN select ';' as semi;\nEXPLAIN select 2 as id".to_string())
        );
    }

    #[test]
    fn test_build_explain_sql_oracle_multiple_statements() {
        assert_eq!(
            build_explain_sql(DatabaseType::Oracle, "select * from users; select * from posts;"),
            Some(
                "EXPLAIN PLAN FOR select * from users;\nSELECT PLAN_TABLE_OUTPUT FROM TABLE(DBMS_XPLAN.DISPLAY());\nEXPLAIN PLAN FOR select * from posts;\nSELECT PLAN_TABLE_OUTPUT FROM TABLE(DBMS_XPLAN.DISPLAY())"
                    .to_string()
            )
        );
    }

    #[test]
    fn test_build_explain_sql_skips_non_select_statements() {
        assert_eq!(
            build_explain_sql(
                DatabaseType::MySQL,
                "insert into users values (1); select * from users; update users set id = 2;"
            ),
            Some("EXPLAIN select * from users".to_string())
        );
    }

    #[test]
    fn test_build_explain_sql_returns_none_for_non_select_only() {
        assert_eq!(
            build_explain_sql(
                DatabaseType::MySQL,
                "insert into users values (1); update users set id = 2;"
            ),
            None
        );
    }

    #[test]
    fn test_build_explain_sql_supports_with_query_via_is_query_statement() {
        assert_eq!(
            build_explain_sql(
                DatabaseType::MySQL,
                "with active_users as (select * from users) select * from active_users"
            ),
            Some(
                "EXPLAIN with active_users as (select * from users) select * from active_users"
                    .to_string()
            )
        );
    }

    #[test]
    fn test_build_explain_sql_keeps_existing_explain_statement() {
        assert_eq!(
            build_explain_sql(DatabaseType::MySQL, "EXPLAIN select * from users"),
            Some("EXPLAIN select * from users".to_string())
        );
    }

    #[test]
    fn test_build_explain_sql_keeps_existing_explain_and_wraps_remaining_queries() {
        assert_eq!(
            build_explain_sql(
                DatabaseType::MySQL,
                "EXPLAIN select * from users; select * from posts;"
            ),
            Some("EXPLAIN select * from users;\nEXPLAIN select * from posts".to_string())
        );
    }

    #[test]
    fn test_build_explain_sql_keeps_existing_mssql_showplan_script() {
        assert_eq!(
            build_explain_sql(
                DatabaseType::MSSQL,
                "SET SHOWPLAN_TEXT ON;\nselect * from users\nSET SHOWPLAN_TEXT OFF;"
            ),
            Some("SET SHOWPLAN_TEXT ON;\nselect * from users\nSET SHOWPLAN_TEXT OFF;".to_string())
        );
    }

    #[test]
    fn collect_bounded_caps_concurrent_polls() {
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));

        let results = smol::block_on(collect_bounded(0..20usize, 5, |item| {
            let active = active.clone();
            let max_active = max_active.clone();
            async move {
                let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                max_active.fetch_max(current, Ordering::SeqCst);
                smol::Timer::after(Duration::from_millis(1)).await;
                active.fetch_sub(1, Ordering::SeqCst);
                item
            }
        }));

        assert!(max_active.load(Ordering::SeqCst) <= 5);
        assert_eq!(active.load(Ordering::SeqCst), 0);
        let mut sorted = results;
        sorted.sort_unstable();
        let expected: Vec<usize> = (0..20).collect();
        assert_eq!(sorted, expected);
    }

    #[test]
    fn collect_bounded_returns_every_item_for_large_batches() {
        let results = smol::block_on(collect_bounded(
            0..128usize,
            SCHEMA_COLUMN_FETCH_CONCURRENCY,
            |item| async move { item },
        ));

        assert_eq!(results.len(), 128);
        let mut sorted = results;
        sorted.sort_unstable();
        let expected: Vec<usize> = (0..128).collect();
        assert_eq!(sorted, expected);
    }

    #[test]
    fn insert_target_table_resolves_into_target() {
        assert_eq!(
            Some("users".to_string()),
            insert_target_table("INSERT INTO users (name) VALUES (1)")
        );
        assert_eq!(
            Some("orders".to_string()),
            insert_target_table("insert into orders\n  (id) values (1)")
        );
        assert_eq!(
            Some("My Table".to_string()),
            insert_target_table("INSERT INTO \"My Table\" (a) VALUES (1)")
        );
        assert_eq!(
            Some("t".to_string()),
            insert_target_table("INSERT t (a) VALUES (1)")
        );
        assert_eq!(None, insert_target_table("SELECT * FROM users"));
        assert_eq!(None, insert_target_table(""));
    }

    #[test]
    fn insert_values_range_covers_all_value_rows_and_nested_calls() {
        let sql = "INSERT INTO t (a, b) VALUES (1, coalesce(2, 3)), (4, 5)";
        let range = insert_values_range(sql).expect("VALUES rows should be found");

        assert_eq!("(1, coalesce(2, 3)), (4, 5)", &sql[range]);
        assert_eq!(None, insert_values_range("INSERT INTO t SELECT * FROM s"));
    }

    #[test]
    fn unquote_sql_identifier_strips_quoting() {
        assert_eq!("users", unquote_sql_identifier("users"));
        assert_eq!("My Table", unquote_sql_identifier("\"My Table\""));
        assert_eq!("a`b", unquote_sql_identifier("`a``b`"));
        assert_eq!("t", unquote_sql_identifier("[t]"));
        assert_eq!("x", unquote_sql_identifier("  x  "));
    }

    #[test]
    fn lookup_table_columns_finds_case_insensitive() {
        let schema = crate::sql_editor::SqlSchema::default()
            .with_table_columns_typed("users", [("id", "int", ""), ("name", "text", "")]);
        assert_eq!(
            Some(vec!["id".to_string(), "name".to_string()]),
            lookup_table_columns(&schema, "users")
        );
        assert_eq!(
            Some(vec!["id".to_string(), "name".to_string()]),
            lookup_table_columns(&schema, "USERS")
        );
        assert_eq!(None, lookup_table_columns(&schema, "orders"));
    }

    #[test]
    fn current_statement_frame_merges_values_highlight() {
        let text = "INSERT INTO t (a, b) VALUES (1, 2);";
        let snapshot =
            SqlStatementSnapshot::new(text.to_string(), SqlDialect::from(&DatabaseType::MySQL));
        let statement = snapshot.statement_at_cursor(5).unwrap();
        let start = statement.sql_range.start_byte;
        let end = statement
            .delimiter_range
            .map(|delimiter| delimiter.end_byte)
            .unwrap_or(statement.sql_range.end_byte);
        let values = start..end;

        let decorations = current_statement_frame_decorations(
            &snapshot,
            7,
            5,
            &(0..0),
            text.len(),
            Some(values.clone()),
        );
        assert_eq!(2, decorations.len());
        assert_eq!(RangeDecorationStyle::Fill, decorations[0].style());
        assert_eq!(&values, decorations[0].range());
        assert_eq!(RangeDecorationStyle::Frame, decorations[1].style());
        assert_eq!(&values, decorations[1].range());
    }
}
