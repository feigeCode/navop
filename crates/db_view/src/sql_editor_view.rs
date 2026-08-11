use crate::sql_editor::{SqlEditor, SqlSchema};
use crate::sql_result_tab::{SessionSqlRun, SqlResultTabContainer};
use db::{DbManager, GlobalDbState, StreamingSqlParser, format_sql};
use futures::channel::oneshot;
use gpui::prelude::*;
use gpui::{
    AnyWindowHandle, App, AppContext, AsyncApp, Axis, Bounds, ClickEvent, Context, Element, Entity,
    EventEmitter, FocusHandle, Focusable, IntoElement, KeyBinding, MouseMoveEvent, MouseUpEvent,
    NoAction, ParentElement, Pixels, Point, Render, SharedString, Styled, Task, WeakEntity, Window,
    div, px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Input, InputContextMenuItem, InputEvent, InputState};
use gpui_component::notification::Notification;
use gpui_component::select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, IndexPath, Sizable, Size, WindowExt, h_flex, v_flex,
};
use one_core::keybindings::{action_id, rebind_keybindings, shortcuts_for};
use one_core::storage::{DatabaseType, QueryDirectoryScope, default_query_directory};
use one_core::tab_container::{TabContainer, TabContent, TabContentEvent};
use one_core::utils::auto_save_config::AutoSaveConfig;
use one_ui::resize_handle::{ResizePanel, resize_handle};
use parking_lot::{Mutex, RwLock};
use rust_i18n::t;
use smol::Timer;
use std::fs::OpenOptions;
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

fn sql_text_for_run_current(
    editor_text: &str,
    selected_text: &str,
    cursor_offset: usize,
    database_type: DatabaseType,
) -> String {
    if selected_text.trim().is_empty() {
        current_sql_statement(editor_text, cursor_offset, database_type).unwrap_or_default()
    } else {
        selected_text.to_string()
    }
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

fn sql_text_for_run_cursor_statement(
    editor_text: &str,
    cursor_offset: usize,
    database_type: DatabaseType,
) -> String {
    current_sql_statement(editor_text, cursor_offset, database_type).unwrap_or_default()
}

fn current_sql_statement(
    editor_text: &str,
    cursor_offset: usize,
    database_type: DatabaseType,
) -> Option<String> {
    let cursor_offset = clamp_to_char_boundary(editor_text, cursor_offset);
    let (prefix, suffix) = editor_text.split_at(cursor_offset);
    let statements = parse_sql_statements(editor_text, database_type.clone());
    if statements.is_empty() {
        return None;
    }
    if cursor_starts_next_statement(prefix, suffix) {
        if let Some(statement) = parse_sql_statements(suffix, database_type.clone())
            .into_iter()
            .next()
        {
            return Some(statement);
        }
    }
    let prefix_statement_count = parse_sql_statements(prefix, database_type).len();
    let statement_index = prefix_statement_count.saturating_sub(1);

    statements
        .get(statement_index.min(statements.len() - 1))
        .cloned()
}

fn parse_sql_statements(sql: &str, database_type: DatabaseType) -> Vec<String> {
    if sql.trim().is_empty() {
        return Vec::new();
    }
    let Ok(parser) = StreamingSqlParser::from_script(sql.to_string(), database_type) else {
        return vec![sql.trim().to_string()];
    };
    parser
        .filter_map(Result::ok)
        .map(|statement| statement.trim().to_string())
        .filter(|statement| !statement.is_empty())
        .collect()
}

fn cursor_starts_next_statement(prefix: &str, suffix: &str) -> bool {
    if !suffix.chars().next().is_some_and(|ch| !ch.is_whitespace()) {
        return false;
    }
    matches!(
        prefix.chars().rev().find(|ch| !ch.is_whitespace()),
        None | Some(';')
    )
}

fn clamp_to_char_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct ManualTransactionSession {
    session_id: String,
    database: Option<String>,
    schema: Option<String>,
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
    /// 自动保存序列号，用于防抖
    auto_save_seq: Arc<AtomicU64>,
    /// 是否有未保存的修改
    is_dirty: Arc<AtomicBool>,
    /// 查询上下文代次，用于丢弃连接、数据库或 Schema 切换前发起的异步回写。
    context_generation: Arc<AtomicU64>,
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
        let database_select =
            cx.new(|cx| SelectState::new(SearchableVec::new(vec![]), None, window, cx));
        let schema_select =
            cx.new(|cx| SelectState::new(SearchableVec::new(vec![]), None, window, cx));
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

        let instance = Self {
            title: config.title,
            editor: editor.clone(),
            connection_id,
            database_type: config.database_type,
            sql_result_tab_container: cx.new(|cx| SqlResultTabContainer::new(window, cx)),
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
            auto_save_seq: auto_save_seq.clone(),
            is_dirty: is_dirty.clone(),
            context_generation,
        };

        instance.configure_editor_context_menu(cx);
        instance.bind_select_event(window, cx);
        instance.bind_transaction_mode_select_event(window, cx);
        instance.bind_auto_save(auto_save_seq, is_dirty, requires_name, window, cx);
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

    fn configure_editor_context_menu(&self, cx: &mut Context<Self>) {
        let view = cx.entity().clone();
        self.editor.update(cx, |editor, cx| {
            editor.set_mouse_context_menu_items(
                vec![
                    InputContextMenuItem::on_click(t!("Query.run_selected").to_string(), {
                        let view = view.clone();
                        move |_, window, cx| {
                            let _ = view.update(cx, |this, cx| {
                                this.handle_run_selected_query(window, cx);
                            });
                        }
                    })
                    .icon(IconName::ArrowRight),
                    InputContextMenuItem::on_click(t!("Query.run_cursor_statement").to_string(), {
                        let view = view.clone();
                        move |_, window, cx| {
                            let _ = view.update(cx, |this, cx| {
                                this.handle_run_cursor_statement_query(window, cx);
                            });
                        }
                    })
                    .icon(IconName::ArrowRight),
                ],
                cx,
            );
        });
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
                    let generation = this.next_context_generation();
                    let window_handle = window.window_handle();
                    if this.supports_schema && !this.uses_schema_as_database {
                        Self::clear_string_select(&this.schema_select, window, cx);
                    }
                    let db = db_name.clone();
                    let instance = this.clone();
                    cx.spawn(async move |_handle, cx| {
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
                            .update_schema_for_db(global_state, &db, generation, cx)
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
            |this, _select, event: &SelectEvent<SearchableVec<String>>, _window, cx| {
                let global_state = cx.global::<GlobalDbState>().clone();
                if let SelectEvent::Confirm(Some(schema_name)) = event {
                    let generation = this.next_context_generation();
                    let database_or_schema = if this.uses_schema_as_database {
                        Some(schema_name.clone())
                    } else {
                        this.database_select.read(cx).selected_value().cloned()
                    };
                    if let Some(db) = database_or_schema {
                        let instance = this.clone();
                        cx.spawn(async move |_handle, cx| {
                            instance
                                .update_schema_for_db(global_state, &db, generation, cx)
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

    fn next_context_generation(&self) -> u64 {
        self.context_generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn is_context_generation_current(&self, generation: u64) -> bool {
        is_current_query_context_generation(
            generation,
            self.context_generation.load(Ordering::SeqCst),
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
        if !can_switch_query_connection(is_executing, self.manual_transaction.is_some()) {
            self.restore_connection_selection(window, cx);
            let message = if self.manual_transaction.is_some() {
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

        let generation = self.next_context_generation();
        let capabilities = global_state.capabilities(&connection.database_type);
        self.connection_id = connection_id.to_string();
        self.database_type = connection.database_type.clone();
        self.supports_schema = capabilities.supports_schema;
        self.uses_schema_as_database = capabilities.uses_schema_as_database;
        cx.emit(TabContentEvent::SourceChanged {
            from: self.connection_id.clone().into(),
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
                    if this.manual_transaction.is_some() && *mode != this.transaction_mode {
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

        cx.subscribe_in(
            &editor_input,
            window,
            move |_this, _input, event: &InputEvent, _window, cx| {
                if let InputEvent::Change = event {
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
        if !self.is_context_generation_current(generation) {
            return;
        }

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
        if !self.is_context_generation_current(generation) {
            return;
        }

        let _ = cx.update_window(window_handle, |_entity, window, cx| {
            if !is_current_query_context_generation(
                generation,
                context_generation.load(Ordering::SeqCst),
            ) {
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

        cx.spawn(async move |_handle, cx: &mut AsyncApp| {
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

            let selected_name = initial_database
                .clone()
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
                        .update_schema_for_db(global_state, db, generation, cx)
                        .await;
                }
            }
        })
        .detach();
    }

    /// Update SQL editor schema with tables and columns from current database
    pub async fn update_schema_for_db(
        &self,
        global_state: GlobalDbState,
        database: &str,
        generation: u64,
        cx: &mut AsyncApp,
    ) {
        if !self.is_context_generation_current(generation) {
            return;
        }

        let connection_id = self.connection_id.clone();
        let editor = self.editor.clone();

        // For Oracle (uses_schema_as_database), the database parameter is actually the schema name
        let (db, selected_schema) = if self.uses_schema_as_database {
            (String::new(), Some(database.to_string()))
        } else if self.supports_schema {
            let schema = self
                .schema_select
                .read_with(cx, |state, _cx| state.selected_value().cloned());
            (database.to_string(), schema)
        } else {
            (database.to_string(), None)
        };

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
        if !self.is_context_generation_current(generation) {
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
        if !self.is_context_generation_current(generation) {
            return;
        }

        let mut schema = SqlSchema::default();

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

        // Load columns for each table
        for table in &tables {
            if let Ok(columns) = global_state
                .list_columns(
                    cx,
                    connection_id.clone(),
                    db.clone(),
                    selected_schema.clone(),
                    table.name.clone(),
                )
                .await
            {
                if !self.is_context_generation_current(generation) {
                    return;
                }
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
            }
        }

        let functions = global_state
            .list_functions(cx, connection_id.clone(), db.clone())
            .await;
        if !self.is_context_generation_current(generation) {
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

        // Update editor with schema and database-specific completion info
        if !self.is_context_generation_current(generation) {
            return;
        }
        _ = editor.update(cx, |e, cx| {
            e.set_db_completion_info(db_completion_info, schema, cx);
        });
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

        if self.transaction_mode == SqlTransactionMode::Manual {
            self.execute_manual_sql_text(sql, scope, window, cx);
            return;
        }

        self.run_auto_sql_text(sql, scope, window, cx);
    }

    fn run_auto_sql_text(
        &self,
        sql: String,
        scope: SqlExecutionScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let connection_id = self.connection_id.clone();
        let sql_result_tab_container = self.sql_result_tab_container.clone();
        sql_result_tab_container.update(cx, |container, cx| {
            container.handle_run_query(
                sql,
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
        scope: SqlExecutionScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !supports_manual_transactions(&self.database_type) {
            window.push_notification(t!("Query.transaction_not_supported").to_string(), cx);
            return;
        }

        if let Some(session) = &self.manual_transaction {
            if !session.matches_execution_scope(&scope) {
                window.push_notification(t!("Query.transaction_scope_changed").to_string(), cx);
                return;
            }
            self.run_manual_sql_on_session(sql, session.session_id().to_string(), scope, cx);
            return;
        }

        self.start_manual_transaction_and_run(sql, scope, cx);
    }

    fn run_manual_sql_on_session(
        &self,
        sql: String,
        session_id: String,
        scope: SqlExecutionScope,
        cx: &mut App,
    ) {
        let request = SessionSqlRun {
            sql,
            session_id,
            connection_id: self.connection_id.clone(),
            database: scope.database,
            schema: scope.schema,
            database_type: self.database_type.clone(),
        };
        self.sql_result_tab_container.update(cx, |container, cx| {
            container.handle_run_query_with_session(request, cx);
        });
    }

    fn start_manual_transaction_and_run(
        &self,
        sql: String,
        scope: SqlExecutionScope,
        cx: &mut Context<Self>,
    ) {
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
                    Self::notify_async(
                        cx,
                        t!("Query.transaction_start_failed", error = error.to_string()).to_string(),
                    );
                    return;
                }
            };

            let prepare = ManualTransactionPrepare {
                database_type: &database_type,
                scope: &scope,
                session_id: &session_id,
            };
            if let Err(error) =
                Self::prepare_manual_transaction_session(&global_state, prepare).await
            {
                let _ = global_state.close_session(cx, session_id).await;
                Self::notify_async(
                    cx,
                    t!("Query.transaction_start_failed", error = error.to_string()).to_string(),
                );
                return;
            }

            let _ = entity.update(cx, |this, cx| {
                this.manual_transaction = Some(ManualTransactionSession::new(
                    session_id.clone(),
                    scope.database.clone(),
                    scope.schema.clone(),
                ));
                this.run_manual_sql_on_session(sql.clone(), session_id.clone(), scope.clone(), cx);
                cx.notify();
            });
            Self::notify_async(cx, t!("Query.transaction_started").to_string());
        })
        .detach();
    }

    async fn prepare_manual_transaction_session(
        global_state: &GlobalDbState,
        prepare: ManualTransactionPrepare<'_>,
    ) -> anyhow::Result<()> {
        if let Some(schema) = &prepare.scope.schema {
            global_state
                .switch_session_schema(prepare.session_id.to_string(), schema.clone())
                .await?;
        }
        if let Some(begin_sql) =
            manual_transaction_control_sql(prepare.database_type, ManualTransactionAction::Begin)
        {
            let result = global_state
                .execute_session(
                    prepare.session_id.to_string(),
                    begin_sql.to_string(),
                    Some(manual_transaction_control_options()),
                )
                .await;
            if transaction_control_failed(&result) {
                return Err(anyhow::anyhow!("BEGIN failed"));
            }
        }
        Ok(())
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

        let global_state = cx.global::<GlobalDbState>().clone();
        cx.spawn(async move |entity: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = global_state
                .execute_session(
                    session.session_id().to_string(),
                    sql.to_string(),
                    Some(manual_transaction_control_options()),
                )
                .await;
            if transaction_control_failed(&result) {
                Self::notify_async(cx, t!("Query.transaction_control_failed").to_string());
                return;
            }

            let _ = global_state
                .close_session(cx, session.session_id().to_string())
                .await;
            let _ = entity.update(cx, |this, cx| {
                this.manual_transaction = None;
                cx.notify();
            });
            let message = match action {
                ManualTransactionAction::Commit => t!("Query.transaction_committed").to_string(),
                ManualTransactionAction::Rollback => {
                    t!("Query.transaction_rolled_back").to_string()
                }
                ManualTransactionAction::Begin => t!("Query.transaction_started").to_string(),
            };
            Self::notify_async(cx, message);
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

    fn handle_run_current_query_action(
        &mut self,
        _: &RunCurrentQuery,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected_text = self.editor.read(cx).get_selected_text(cx);
        let cursor_offset = self.editor.read(cx).cursor_offset(cx);
        let sql = sql_text_for_run_current(
            &self.get_sql_text(cx),
            &selected_text,
            cursor_offset,
            self.database_type.clone(),
        );
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

    fn handle_run_cursor_statement_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cursor_offset = self.editor.read(cx).cursor_offset(cx);
        let sql = sql_text_for_run_cursor_statement(
            &self.get_sql_text(cx),
            cursor_offset,
            self.database_type.clone(),
        );
        if sql.trim().is_empty() {
            window.push_notification(t!("Query.query_content_empty").to_string(), cx);
            return;
        }
        self.execute_sql_text(sql, window, cx);
    }

    fn handle_format_query(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.get_sql_text(cx);
        if text.trim().is_empty() {
            window.push_notification(t!("Query.no_sql_to_format").to_string(), cx);
            return;
        }
        let window_option = cx.active_window();
        cx.spawn(async move |entity: WeakEntity<Self>, cx: &mut AsyncApp| {
            entity
                .update(cx, |this, cx| {
                    let formatted = format_sql(&text);
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

            dialog
                .title(t!("Query.save_query_title").to_string())
                .w(px(380.0))
                .overlay_closable(false)
                .close_button(false)
                .footer(move |_ok, _cancel, _window, _cx| {
                    let input_for_save = input_for_save.clone();
                    let view_for_save = view_for_save.clone();
                    let tx_cancel = tx_cancel.clone();
                    let tx_discard = tx_discard.clone();
                    let tx_save = tx_save.clone();

                    vec![
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
                                let saved = view_for_save.update(cx, |view, cx| {
                                    view.save_named_query(&name, window, cx)
                                });
                                if saved {
                                    window.close_dialog(cx);
                                    if let Some(tx) = tx_save.lock().take() {
                                        let _ = tx.send(true);
                                    }
                                }
                            })
                            .into_any_element(),
                    ]
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

        let connection_id = self.connection_id.clone();
        let sql_result_tab_container = self.sql_result_tab_container.clone();

        sql_result_tab_container.update(cx, |container, cx| {
            container.handle_run_query(
                explain_sql,
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
        let connection_select = self.connection_select.clone();
        let database_select = self.database_select.clone();
        let schema_select = self.schema_select.clone();
        let transaction_mode_select = self.transaction_mode_select.clone();
        let supports_schema = self.supports_schema;
        let uses_schema_as_database = self.uses_schema_as_database;
        let supports_transactions = supports_manual_transactions(&self.database_type);
        let is_manual_mode = self.transaction_mode == SqlTransactionMode::Manual;
        let has_manual_transaction = self.manual_transaction.is_some();

        // Check if there are any results and if the panel is visible
        let has_results = self.sql_result_tab_container.read(cx).has_results(cx);
        let results_visible = self.sql_result_tab_container.read(cx).is_visible(cx);
        let is_query_executing = self.sql_result_tab_container.read(cx).is_executing(cx);

        // Check if there is selected text in the editor
        let has_selection = !self.editor.read(cx).get_selected_text(cx).trim().is_empty();

        v_flex()
            .size_full()
            .gap_2()
            .child(
                // Toolbar
                h_flex()
                    .gap_2()
                    .p_2()
                    .bg(cx.theme().muted)
                    .rounded_md()
                    .items_center()
                    .w_full()
                    .child(
                        Select::new(&connection_select)
                            .with_size(Size::Small)
                            .placeholder(t!("Query.select_connection"))
                            .search_placeholder(t!("Query.search_connection"))
                            .disabled(is_query_executing || has_manual_transaction)
                            .w(px(220.)),
                    )
                    .when(!uses_schema_as_database, |this| {
                        this.child(
                            // Database selector (for non-Oracle databases)
                            Select::new(&database_select)
                                .with_size(Size::Small)
                                .placeholder(t!("Query.select_database"))
                                .disabled(has_manual_transaction)
                                .w(px(200.)),
                        )
                    })
                    .when(
                        should_render_schema_select(supports_schema, uses_schema_as_database),
                        |this| {
                            this.child(
                                // Schema selector for PostgreSQL
                                Select::new(&schema_select)
                                    .with_size(Size::Small)
                                    .placeholder(t!("Query.select_schema"))
                                    .disabled(has_manual_transaction)
                                    .w(if uses_schema_as_database {
                                        px(200.)
                                    } else {
                                        px(150.)
                                    }),
                            )
                        },
                    )
                    .when(supports_transactions, |this| {
                        this.child(
                            Select::new(&transaction_mode_select)
                                .with_size(Size::Small)
                                .title_prefix(t!("Query.transaction_mode_prefix"))
                                .disabled(is_query_executing || has_manual_transaction)
                                .w(px(128.)),
                        )
                    })
                    .when(is_manual_mode, |this| {
                        this.child(
                            Button::new("transaction-commit")
                                .with_size(Size::Small)
                                .ghost()
                                .disabled(is_query_executing || !has_manual_transaction)
                                .label(t!("Query.transaction_commit"))
                                .icon(IconName::Check)
                                .on_click(cx.listener(Self::handle_commit_transaction)),
                        )
                        .child(
                            Button::new("transaction-rollback")
                                .with_size(Size::Small)
                                .ghost()
                                .disabled(is_query_executing || !has_manual_transaction)
                                .label(t!("Query.transaction_rollback"))
                                .icon(IconName::Undo)
                                .on_click(cx.listener(Self::handle_rollback_transaction)),
                        )
                    })
                    .child(
                        Button::new("run-query")
                            .with_size(Size::Small)
                            .primary()
                            .loading(is_query_executing)
                            .label(if is_query_executing {
                                t!("Query.running")
                            } else if has_selection {
                                t!("Query.run_selected")
                            } else {
                                t!("Query.run")
                            })
                            .icon(IconName::ArrowRight)
                            .on_click(cx.listener(Self::handle_run_query)),
                    )
                    .child(
                        Button::new("explain-sql")
                            .with_size(Size::Small)
                            .ghost()
                            .disabled(is_query_executing)
                            .label(t!("Query.explain"))
                            .on_click(cx.listener(Self::handle_explain_sql)),
                    )
                    .child(
                        Button::new("format-query")
                            .with_size(Size::Small)
                            .ghost()
                            .label(t!("Query.format"))
                            .icon(IconName::Star)
                            .on_click(cx.listener(Self::handle_format_query)),
                    )
                    .child(
                        Button::new("save-query")
                            .with_size(Size::Small)
                            .ghost()
                            .label(t!("Query.save"))
                            .icon(IconName::Plus)
                            .on_click(cx.listener(Self::handle_save_query)),
                    ),
            )
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
            auto_save_seq: self.auto_save_seq.clone(),
            is_dirty: self.is_dirty.clone(),
            context_generation: self.context_generation.clone(),
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

    fn try_close(
        &mut self,
        _tab_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        if self.manual_transaction.is_some() {
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
        ManualTransactionAction, ManualTransactionSession, QueryFileNameError,
        RUN_ALL_QUERY_KEY_BINDINGS, RUN_CURRENT_QUERY_KEY_BINDINGS, RunCurrentQuery,
        SQL_EDITOR_CONTEXT, SQL_EDITOR_INPUT_CONTEXT, ToggleLineComment,
        can_switch_query_connection, initial_database_select_value,
        is_current_query_context_generation, manual_transaction_control_sql,
        query_connection_context_label, query_connection_ids, query_file_path_for_name,
        should_render_schema_select, sql_text_for_run_all, sql_text_for_run_current,
        sql_text_for_run_cursor_statement, sql_text_for_toolbar_run, supports_manual_transactions,
        toggle_sql_line_comments, write_new_sql_file, write_sql_file,
    };
    use db::DbManager;
    use gpui::{KeyBinding, KeyContext, Keymap, Keystroke};
    use gpui_component::input;
    use one_core::storage::DatabaseType;
    use std::path::PathBuf;

    const WIRE_PREFIX: &str = "/*onetcli-ipc-wire*/ ";

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
    fn stale_query_context_generation_is_rejected() {
        assert!(is_current_query_context_generation(3, 3));
        assert!(!is_current_query_context_generation(2, 3));
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
        let actual = sql_text_for_run_current(
            "select * from users;",
            "select id from users;",
            0,
            DatabaseType::MySQL,
        );

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
        let actual = sql_text_for_run_current(sql, "   ", cursor_offset, DatabaseType::MySQL);

        assert_eq!("select * from orders", actual);
    }

    #[test]
    fn run_query_text_uses_full_multiline_statement_when_cursor_is_inside() {
        let sql = "select * from users;\nselect id,\n       name\nfrom orders\nwhere active = 1;\nselect * from products;";
        let cursor_offset = sql.find("name").expect("statement exists") + "na".len();
        let actual = sql_text_for_run_current(sql, "", cursor_offset, DatabaseType::MySQL);

        assert_eq!(
            "select id,\n       name\nfrom orders\nwhere active = 1",
            actual
        );
    }

    #[test]
    fn run_query_text_ignores_semicolon_inside_string() {
        let sql = "select 1;\nselect ';not delimiter' as value;\nselect 3;";
        let cursor_offset = sql.find("value").expect("statement exists") + "value".len();
        let actual = sql_text_for_run_current(sql, "", cursor_offset, DatabaseType::MySQL);

        assert_eq!("select ';not delimiter' as value", actual);
    }

    #[test]
    fn run_all_query_text_uses_editor_sql_even_with_selection() {
        let sql = "select * from users;\nselect * from orders;";
        let actual = sql_text_for_run_all(sql, "select * from users;");

        assert_eq!(sql, actual);
    }

    #[test]
    fn run_cursor_statement_text_uses_cursor_statement() {
        let sql = "select 1;\n  select * from 用户表;  \nselect 3;";
        let cursor_offset = sql.find("用户表").expect("line exists") + "用户".len();
        let actual = sql_text_for_run_cursor_statement(sql, cursor_offset, DatabaseType::MySQL);

        assert_eq!("select * from 用户表", actual);
    }

    #[test]
    fn run_cursor_statement_text_uses_full_multiline_statement() {
        let sql = "select * from users;\nselect id,\n       name\nfrom orders\nwhere active = 1;\nselect * from products;";
        let cursor_offset = sql.find("name").expect("statement exists") + "na".len();
        let actual = sql_text_for_run_cursor_statement(sql, cursor_offset, DatabaseType::MySQL);

        assert_eq!(
            "select id,\n       name\nfrom orders\nwhere active = 1",
            actual
        );
    }

    #[test]
    fn run_cursor_statement_text_handles_last_statement() {
        let sql = "select 1;\nselect 2";
        let actual = sql_text_for_run_cursor_statement(sql, sql.len(), DatabaseType::MySQL);

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
                input::Enter { secondary: true },
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
}
