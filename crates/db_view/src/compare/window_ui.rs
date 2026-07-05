use std::fmt::Display;
use std::sync::Arc;
use std::time::Instant;

use db::{GlobalDbState, SqlResult, StreamingProgress};
use gpui::{
    App, AppContext, AsyncApp, Context, Entity, Hsla, InteractiveElement, IntoElement,
    ParentElement, ScrollHandle, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme, Disableable, IndexPath, Sizable, StyledExt, WindowExt,
    button::Button,
    checkbox::Checkbox,
    clipboard::Clipboard,
    dialog::DialogButtonProps,
    h_flex,
    highlighter::Language,
    input::{Input, InputState},
    progress::Progress,
    scroll::Scrollbar,
    select::{SearchableVec, Select, SelectItem, SelectState},
    switch::Switch,
    v_flex,
};
use one_core::storage::{
    ConnectionRepository, ConnectionType, GlobalStorageState, StoredConnection, traits::Repository,
};
use rust_i18n::t;
use tokio::sync::mpsc;

use crate::compare::{
    CompareProgress, CompareSyncExecutionOptions, CompareTargetScope, execute_sync_sql,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompareStep {
    Objects,
    SqlPreview,
    SqlExecute,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SyncSqlExecutionLogEntry {
    pub message: String,
    pub is_error: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SyncSqlExecutionSummary {
    success_count: usize,
    error_count: usize,
    stopped_on_error: bool,
}

impl SyncSqlExecutionSummary {
    fn total_count(&self) -> usize {
        self.success_count + self.error_count
    }
}

#[derive(Clone)]
struct SyncSqlExecutionRuntime {
    options: CompareSyncExecutionOptions,
    status: Entity<String>,
    execution_log: Entity<Vec<SyncSqlExecutionLogEntry>>,
    execution_log_scroll: ScrollHandle,
}

impl SyncSqlExecutionRuntime {
    async fn consume_progress(
        &self,
        rx: &mut mpsc::Receiver<StreamingProgress>,
        cx: &mut AsyncApp,
    ) -> SyncSqlExecutionSummary {
        let mut summary = SyncSqlExecutionSummary::default();
        let mut last_progress_log = Instant::now();
        while let Some(progress) = rx.recv().await {
            if self.handle_progress(progress, &mut summary, &mut last_progress_log, cx) {
                break;
            }
        }
        summary
    }

    fn handle_progress(
        &self,
        progress: StreamingProgress,
        summary: &mut SyncSqlExecutionSummary,
        last_progress_log: &mut Instant,
        cx: &mut AsyncApp,
    ) -> bool {
        match &progress.result {
            SqlResult::Error(error) => {
                summary.error_count += 1;
                self.append_async(
                    sync_sql_progress_error_log_entry(progress.current, &error.message),
                    cx,
                );
                summary.stopped_on_error = !self.options.continue_on_error;
                summary.stopped_on_error
            }
            _ => {
                summary.success_count += 1;
                self.handle_success_progress(progress, last_progress_log, cx)
            }
        }
    }

    fn handle_success_progress(
        &self,
        progress: StreamingProgress,
        last_progress_log: &mut Instant,
        cx: &mut AsyncApp,
    ) -> bool {
        let should_log =
            progress.current == progress.total || last_progress_log.elapsed().as_millis() >= 200;
        if should_log {
            *last_progress_log = Instant::now();
            self.append_async(
                sync_sql_statement_progress_log_entry(
                    progress.current,
                    progress.progress_percent(),
                ),
                cx,
            );
        }
        false
    }

    fn fail_start(&self, is_executing: &Entity<bool>, error: impl Display, cx: &mut App) {
        self.set_executing(is_executing, false, cx);
        let entry = sync_sql_execution_error_log_entry(error);
        self.set_status(entry.message.clone(), cx);
        self.append_app(entry, cx);
    }

    fn finish(&self, is_executing: &Entity<bool>, summary: SyncSqlExecutionSummary, cx: &mut App) {
        self.set_executing(is_executing, false, cx);
        let entry = sync_sql_execution_summary_log_entry(&summary);
        self.set_status(entry.message.clone(), cx);
        self.append_app(entry, cx);
    }

    fn set_executing(&self, is_executing: &Entity<bool>, value: bool, cx: &mut App) {
        is_executing.update(cx, |executing, cx| {
            *executing = value;
            cx.notify();
        });
    }

    fn append_app(&self, entry: SyncSqlExecutionLogEntry, cx: &mut App) {
        append_sync_sql_execution_log_app(
            &self.execution_log,
            &self.execution_log_scroll,
            entry,
            cx,
        );
    }

    fn set_status(&self, message: String, cx: &mut App) {
        set_sync_sql_execution_status_app(&self.status, message, cx);
    }

    fn append_async(&self, entry: SyncSqlExecutionLogEntry, cx: &mut AsyncApp) {
        let _ = cx.update(|cx| {
            self.append_app(entry, cx);
        });
    }
}

impl SyncSqlExecutionLogEntry {
    fn info(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            is_error: false,
        }
    }

    fn error(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            is_error: true,
        }
    }
}

impl CompareStep {
    #[cfg(test)]
    pub(crate) fn next(self) -> Option<Self> {
        match self {
            Self::Objects => Some(Self::SqlPreview),
            Self::SqlPreview => Some(Self::SqlExecute),
            Self::SqlExecute => None,
        }
    }

    pub(crate) fn previous(self) -> Option<Self> {
        match self {
            Self::Objects => None,
            Self::SqlPreview => Some(Self::Objects),
            Self::SqlExecute => Some(Self::SqlPreview),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ConnectionSelectItem {
    pub id: String,
    pub label: String,
}

impl SelectItem for ConnectionSelectItem {
    type Value = String;

    fn title(&self) -> gpui::SharedString {
        self.label.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }
}

impl ConnectionSelectItem {
    fn from_connection(connection: StoredConnection) -> Option<Self> {
        if connection.connection_type != ConnectionType::Database {
            return None;
        }
        let id = connection.id?.to_string();
        let host = connection
            .to_db_connection()
            .ok()
            .map(|config| config.host)
            .filter(|host| !host.trim().is_empty());
        let label = match host {
            Some(host) => format!("{} ({host})", connection.name),
            None => connection.name,
        };
        Some(Self { label, id })
    }
}

pub(super) fn connection_select_state(
    current_connection_id: &str,
    window: &mut gpui::Window,
    cx: &mut App,
) -> Entity<SelectState<SearchableVec<ConnectionSelectItem>>> {
    let mut items = connection_select_items(cx);
    if !current_connection_id.trim().is_empty()
        && !items.iter().any(|item| item.id == current_connection_id)
    {
        items.push(ConnectionSelectItem {
            id: current_connection_id.to_string(),
            label: current_connection_id.to_string(),
        });
    }
    let selected_index = items
        .iter()
        .position(|item| item.id == current_connection_id)
        .map(IndexPath::new);
    cx.new(|cx| {
        SelectState::new(SearchableVec::new(items), selected_index, window, cx).searchable(true)
    })
}

fn connection_select_items(cx: &mut App) -> Vec<ConnectionSelectItem> {
    let Some(storage_state) = cx.try_global::<GlobalStorageState>() else {
        return Vec::new();
    };
    let Some(repo) = storage_state.storage.get::<ConnectionRepository>() else {
        return Vec::new();
    };
    repo.list()
        .unwrap_or_default()
        .into_iter()
        .filter_map(ConnectionSelectItem::from_connection)
        .collect()
}

pub(super) fn selected_connection_id(
    select: &Entity<SelectState<SearchableVec<ConnectionSelectItem>>>,
    fallback: &Entity<InputState>,
    cx: &App,
) -> String {
    select
        .read(cx)
        .selected_value()
        .cloned()
        .unwrap_or_else(|| fallback.read(cx).text().to_string())
}

pub(crate) fn register_connection_for_compare<T>(connection_id: &str, cx: &mut Context<T>) {
    let Some(storage_state) = cx.try_global::<GlobalStorageState>() else {
        return;
    };
    let Some(repo) = storage_state.storage.get::<ConnectionRepository>() else {
        return;
    };
    let Ok(id) = connection_id.parse::<i64>() else {
        return;
    };
    let Ok(Some(connection)) = repo.get(id) else {
        return;
    };
    let Ok(config) = connection.to_db_connection() else {
        return;
    };
    let mut db_state = cx.global::<GlobalDbState>().clone();
    db_state.register_connection(config);
}

pub(super) fn data_truncation_note(
    source_truncated: bool,
    target_truncated: bool,
) -> Option<String> {
    match (source_truncated, target_truncated) {
        (true, true) => Some(t!("Compare.data_truncated_both").to_string()),
        (true, false) => Some(t!("Compare.data_truncated_source").to_string()),
        (false, true) => Some(t!("Compare.data_truncated_target").to_string()),
        (false, false) => None,
    }
}

pub(super) fn ignore_identifier_case_option<T: 'static>(
    checkbox_id: &'static str,
    checked: Entity<bool>,
    cx: &mut Context<T>,
) -> impl IntoElement {
    let is_checked = *checked.read(cx);
    h_flex()
        .gap_2()
        .items_center()
        .child(
            Checkbox::new(checkbox_id)
                .checked(is_checked)
                .on_click(move |_, _, cx| {
                    checked.update(cx, |value, cx| {
                        *value = !*value;
                        cx.notify();
                    });
                }),
        )
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().foreground)
                .child(t!("Compare.ignore_identifier_case").to_string()),
        )
}

/// 比较结果统计卡片(新增/删除/修改)
pub(super) fn stat_cards_row(
    added: usize,
    removed: usize,
    modified: usize,
    cx: &App,
) -> impl IntoElement {
    h_flex()
        .gap_2()
        .child(stat_card(
            "+",
            added,
            t!("Compare.added").to_string(),
            cx.theme().success,
            cx,
        ))
        .child(stat_card(
            "−",
            removed,
            t!("Compare.removed").to_string(),
            cx.theme().danger,
            cx,
        ))
        .child(stat_card(
            "~",
            modified,
            t!("Compare.modified").to_string(),
            cx.theme().warning,
            cx,
        ))
}

fn stat_card(sign: &str, count: usize, label: String, color: Hsla, cx: &App) -> impl IntoElement {
    v_flex()
        .flex_1()
        .px_3()
        .py_2()
        .gap_1()
        .border_1()
        .border_color(cx.theme().border)
        .rounded_md()
        .bg(cx.theme().secondary)
        .child(
            div()
                .text_color(color)
                .font_semibold()
                .child(format!("{sign}{count}")),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
}

/// 进度视图:阶段文本 +(确定型时)进度条
pub(super) fn compare_progress_view(progress: &CompareProgress, cx: &App) -> impl IntoElement {
    let mut col = v_flex().gap_1().child(
        div()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(progress.label()),
    );
    if let Some(value) = progress.percentage() {
        col = col.child(Progress::new("compare-progress").value(value));
    }
    col
}

pub(super) fn start_sync_sql_execution<T: 'static>(
    target: Option<CompareTargetScope>,
    sql: String,
    options: CompareSyncExecutionOptions,
    status: Entity<String>,
    is_executing: Entity<bool>,
    execution_log: Entity<Vec<SyncSqlExecutionLogEntry>>,
    execution_log_scroll: ScrollHandle,
    window: &mut Window,
    cx: &mut Context<T>,
) {
    let Some(target) = target else {
        let message = t!("Compare.sync_sql_compare_first").to_string();
        set_sync_sql_execution_status(&status, message.clone(), cx);
        append_sync_sql_execution_log(
            &execution_log,
            &execution_log_scroll,
            SyncSqlExecutionLogEntry::error(message),
            cx,
        );
        return;
    };
    if sql.trim().is_empty() {
        let message = t!("Compare.sync_sql_empty").to_string();
        set_sync_sql_execution_status(&status, message.clone(), cx);
        append_sync_sql_execution_log(
            &execution_log,
            &execution_log_scroll,
            SyncSqlExecutionLogEntry::error(message),
            cx,
        );
        return;
    }

    if contains_destructive_sync_sql(&sql) {
        open_destructive_sync_sql_dialog(
            target,
            sql,
            options,
            status,
            is_executing,
            execution_log,
            execution_log_scroll,
            window,
            cx,
        );
        return;
    }

    execute_sync_sql_now(
        target,
        sql,
        options,
        status,
        is_executing,
        execution_log,
        execution_log_scroll,
        cx,
    );
}

fn open_destructive_sync_sql_dialog<T: 'static>(
    target: CompareTargetScope,
    sql: String,
    options: CompareSyncExecutionOptions,
    status: Entity<String>,
    is_executing: Entity<bool>,
    execution_log: Entity<Vec<SyncSqlExecutionLogEntry>>,
    execution_log_scroll: ScrollHandle,
    window: &mut Window,
    cx: &mut Context<T>,
) {
    let view = cx.entity().clone();
    window.open_dialog(cx, move |dialog, _window, _cx| {
        let target = target.clone();
        let sql = sql.clone();
        let options = options;
        let status = status.clone();
        let is_executing = is_executing.clone();
        let execution_log = execution_log.clone();
        let execution_log_scroll = execution_log_scroll.clone();
        let view = view.clone();

        dialog
            .title(t!("Compare.destructive_sql_confirm_title").to_string())
            .confirm()
            .overlay(false)
            .button_props(
                DialogButtonProps::default()
                    .ok_text(t!("Compare.destructive_sql_confirm_execute").to_string())
                    .cancel_text(t!("Common.cancel").to_string()),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(t!("Compare.destructive_sql_confirm_message").to_string())
                    .child(t!("Compare.destructive_sql_confirm_desc").to_string())
                    .child(t!("Common.irreversible").to_string()),
            )
            .on_ok(move |_, _, cx| {
                let target = target.clone();
                let sql = sql.clone();
                let options = options;
                let status = status.clone();
                let is_executing = is_executing.clone();
                let execution_log = execution_log.clone();
                let execution_log_scroll = execution_log_scroll.clone();
                view.update(cx, |_, cx| {
                    execute_sync_sql_now(
                        target,
                        sql,
                        options,
                        status,
                        is_executing,
                        execution_log,
                        execution_log_scroll,
                        cx,
                    );
                });
                true
            })
    });
}

fn execute_sync_sql_now<T: 'static>(
    target: CompareTargetScope,
    sql: String,
    options: CompareSyncExecutionOptions,
    status: Entity<String>,
    is_executing: Entity<bool>,
    execution_log: Entity<Vec<SyncSqlExecutionLogEntry>>,
    execution_log_scroll: ScrollHandle,
    cx: &mut Context<T>,
) {
    let db_state = Arc::new(cx.global::<GlobalDbState>().clone());
    is_executing.update(cx, |executing, cx| {
        *executing = true;
        cx.notify();
    });
    let executing_message = t!("Compare.sync_sql_executing").to_string();
    set_sync_sql_execution_status(&status, executing_message.clone(), cx);
    append_sync_sql_execution_log(
        &execution_log,
        &execution_log_scroll,
        SyncSqlExecutionLogEntry::info(executing_message),
        cx,
    );

    cx.spawn(async move |_, cx: &mut AsyncApp| {
        let runtime = SyncSqlExecutionRuntime {
            options,
            status,
            execution_log,
            execution_log_scroll,
        };
        let mut rx = match execute_sync_sql(target, sql, db_state, options, cx) {
            Ok(rx) => rx,
            Err(error) => {
                let _ = cx.update(|cx| {
                    runtime.fail_start(&is_executing, error, cx);
                });
                return;
            }
        };
        let summary = runtime.consume_progress(&mut rx, cx).await;
        let _ = cx.update(|cx| {
            runtime.finish(&is_executing, summary, cx);
        });
    })
    .detach();
}

pub(crate) fn sync_sql_execution_start_log_entries(sql: &str) -> Vec<SyncSqlExecutionLogEntry> {
    vec![SyncSqlExecutionLogEntry::info(
        t!(
            "Compare.sync_sql_execution_ready",
            count = sync_sql_statement_count(sql)
        )
        .to_string(),
    )]
}

pub(crate) fn sync_sql_execution_success_log_entry(count: usize) -> SyncSqlExecutionLogEntry {
    SyncSqlExecutionLogEntry::info(t!("Compare.sync_sql_executed", count = count).to_string())
}

fn sync_sql_execution_summary_log_entry(
    summary: &SyncSqlExecutionSummary,
) -> SyncSqlExecutionLogEntry {
    if summary.stopped_on_error {
        return SyncSqlExecutionLogEntry::error(
            t!(
                "Compare.sync_sql_stopped_on_error",
                success = summary.success_count,
                error = summary.error_count
            )
            .to_string(),
        );
    }
    if summary.error_count > 0 {
        return SyncSqlExecutionLogEntry::error(
            t!(
                "Compare.sync_sql_finished_with_errors",
                success = summary.success_count,
                error = summary.error_count
            )
            .to_string(),
        );
    }
    sync_sql_execution_success_log_entry(summary.total_count())
}

pub(crate) fn sync_sql_execution_error_log_entry(error: impl Display) -> SyncSqlExecutionLogEntry {
    SyncSqlExecutionLogEntry::error(
        t!("Compare.execution_failed", error = error.to_string()).to_string(),
    )
}

fn sync_sql_statement_progress_log_entry(
    statement: usize,
    progress: f32,
) -> SyncSqlExecutionLogEntry {
    SyncSqlExecutionLogEntry::info(
        t!(
            "Compare.sync_sql_statement_progress",
            statement = statement,
            progress = format!("{progress:.1}")
        )
        .to_string(),
    )
}

fn sync_sql_statement_failed_log_entry(
    statement: usize,
    error: impl Display,
) -> SyncSqlExecutionLogEntry {
    SyncSqlExecutionLogEntry::error(
        t!(
            "Compare.sync_sql_statement_failed",
            statement = statement,
            error = error.to_string()
        )
        .to_string(),
    )
}

fn sync_sql_progress_error_log_entry(
    statement: usize,
    error: impl Display,
) -> SyncSqlExecutionLogEntry {
    if statement == 0 {
        sync_sql_execution_error_log_entry(error)
    } else {
        sync_sql_statement_failed_log_entry(statement, error)
    }
}

pub(super) fn reset_sync_sql_execution_log<T: 'static>(
    execution_log: &Entity<Vec<SyncSqlExecutionLogEntry>>,
    execution_log_scroll: &ScrollHandle,
    entries: Vec<SyncSqlExecutionLogEntry>,
    cx: &mut Context<T>,
) {
    execution_log.update(cx, |log, cx| {
        *log = entries;
        cx.notify();
    });
    execution_log_scroll.scroll_to_bottom();
}

pub(super) fn clear_sync_sql_execution_log<T: 'static>(
    execution_log: &Entity<Vec<SyncSqlExecutionLogEntry>>,
    execution_log_scroll: &ScrollHandle,
    cx: &mut Context<T>,
) {
    reset_sync_sql_execution_log(execution_log, execution_log_scroll, Vec::new(), cx);
}

fn append_sync_sql_execution_log<T: 'static>(
    execution_log: &Entity<Vec<SyncSqlExecutionLogEntry>>,
    execution_log_scroll: &ScrollHandle,
    entry: SyncSqlExecutionLogEntry,
    cx: &mut Context<T>,
) {
    execution_log.update(cx, |log, cx| {
        log.push(entry);
        cx.notify();
    });
    execution_log_scroll.scroll_to_bottom();
}

fn append_sync_sql_execution_log_app(
    execution_log: &Entity<Vec<SyncSqlExecutionLogEntry>>,
    execution_log_scroll: &ScrollHandle,
    entry: SyncSqlExecutionLogEntry,
    cx: &mut App,
) {
    execution_log.update(cx, |log, cx| {
        log.push(entry);
        cx.notify();
    });
    execution_log_scroll.scroll_to_bottom();
}

fn set_sync_sql_execution_status<T: 'static>(
    status: &Entity<String>,
    message: String,
    cx: &mut Context<T>,
) {
    status.update(cx, |status, cx| {
        *status = message;
        cx.notify();
    });
}

fn set_sync_sql_execution_status_app(status: &Entity<String>, message: String, cx: &mut App) {
    status.update(cx, |status, cx| {
        *status = message;
        cx.notify();
    });
}

fn sync_sql_statement_count(sql: &str) -> usize {
    sql.split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
        .count()
}

fn contains_destructive_sync_sql(sql: &str) -> bool {
    let normalized_sql = strip_single_quoted_literals(sql)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("--"))
        .collect::<Vec<_>>()
        .join(" ")
        .to_uppercase();
    [
        "DELETE FROM",
        "TRUNCATE TABLE",
        "DROP TABLE",
        "DROP INDEX",
        "DROP COLUMN",
        "DROP CONSTRAINT",
        "DROP DATABASE",
        "DROP SCHEMA",
        "DROP VIEW",
    ]
    .iter()
    .any(|keyword| normalized_sql.contains(keyword))
}

fn strip_single_quoted_literals(sql: &str) -> String {
    let mut output = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    let mut in_string = false;
    while let Some(ch) = chars.next() {
        if ch == '\'' {
            if in_string && chars.peek() == Some(&'\'') {
                let _ = chars.next();
                continue;
            }
            in_string = !in_string;
            output.push(' ');
        } else if !in_string {
            output.push(ch);
        }
    }
    output
}

pub(crate) fn section_title(title: impl IntoElement) -> impl IntoElement {
    div().font_semibold().child(title)
}

pub(super) fn close_button() -> impl IntoElement {
    Button::new("close")
        .child("Close")
        .on_click(|_, window, _| {
            window.remove_window();
        })
}

pub(super) fn input_row(label: impl Into<String>, input: &Entity<InputState>) -> impl IntoElement {
    let label = label.into();
    div()
        .flex()
        .items_center()
        .min_w_0()
        .gap_2()
        .child(div().w(px(120.0)).flex_none().text_sm().child(label))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(Input::new(input).small().w_full()),
        )
}

pub(crate) fn connection_select_row(
    label: impl Into<String>,
    select: &Entity<SelectState<SearchableVec<ConnectionSelectItem>>>,
) -> impl IntoElement {
    let label = label.into();
    div()
        .flex()
        .items_center()
        .min_w_0()
        .gap_2()
        .child(
            div()
                .w(px(120.0))
                .flex_none()
                .text_sm()
                .child(label.clone()),
        )
        .child(
            div().flex_1().min_w_0().child(
                Select::new(select)
                    .small()
                    .search_placeholder(t!("DbObjectSelector.search", item = label).to_string())
                    .w_full(),
            ),
        )
}

/// 创建用于「同步 SQL」的代码编辑器(SQL 语法高亮 + 行号,可编辑)
pub(super) fn sync_sql_editor_state(window: &mut Window, cx: &mut App) -> Entity<InputState> {
    cx.new(|cx| {
        InputState::new(window, cx)
            .code_editor(Language::from_str("sql"))
            .line_number(true)
            .multi_line(true)
            .soft_wrap(false)
            .placeholder(t!("Compare.sync_sql_placeholder").to_string())
    })
}

/// 同步 SQL 编辑器面板:标题 + 复制按钮 + 可编辑代码编辑器(填满所在列并内部滚动)
pub(super) fn sql_editor_panel(
    copy_id: &'static str,
    editor: &Entity<InputState>,
    copy_value: String,
    cx: &App,
) -> impl IntoElement {
    v_flex()
        .size_full()
        .min_h_0()
        .gap_1()
        .child(
            h_flex()
                .justify_between()
                .child(
                    div()
                        .text_sm()
                        .font_semibold()
                        .child(t!("Compare.sync_sql").to_string()),
                )
                .child(Clipboard::new(copy_id).value(copy_value)),
        )
        .child(
            div()
                .flex_1()
                .min_h_0()
                .border_1()
                .border_color(cx.theme().border)
                .rounded_md()
                .child(Input::new(editor).size_full()),
        )
}

pub(super) fn sync_sql_execution_options_row<T: 'static>(
    use_transaction: Entity<bool>,
    continue_on_error: Entity<bool>,
    is_executing: bool,
    cx: &mut Context<T>,
) -> impl IntoElement {
    let transaction_checked = *use_transaction.read(cx);
    let continue_checked = *continue_on_error.read(cx);
    let transaction_toggle = use_transaction.clone();
    let transaction_continue = continue_on_error.clone();
    let continue_toggle = continue_on_error.clone();
    let continue_transaction = use_transaction.clone();
    h_flex()
        .gap_4()
        .items_center()
        .child(
            Switch::new("compare-sync-use-transaction")
                .small()
                .checked(transaction_checked)
                .disabled(is_executing)
                .label(t!("Compare.use_transaction").to_string())
                .on_click(move |checked, _, cx| {
                    transaction_toggle.update(cx, |value, cx| {
                        *value = *checked;
                        if *value {
                            transaction_continue.update(cx, |continue_value, cx| {
                                *continue_value = false;
                                cx.notify();
                            });
                        }
                        cx.notify();
                    });
                }),
        )
        .child(
            Switch::new("compare-sync-continue-on-error")
                .small()
                .checked(continue_checked)
                .disabled(is_executing)
                .label(t!("Compare.continue_on_error").to_string())
                .on_click(move |checked, _, cx| {
                    continue_toggle.update(cx, |value, cx| {
                        *value = *checked;
                        if *value {
                            continue_transaction.update(cx, |transaction_value, cx| {
                                *transaction_value = false;
                                cx.notify();
                            });
                        }
                        cx.notify();
                    });
                }),
        )
}

pub(super) fn sync_sql_execution_continue_on_error_row<T: 'static>(
    continue_on_error: Entity<bool>,
    is_executing: bool,
    cx: &mut Context<T>,
) -> impl IntoElement {
    let continue_checked = *continue_on_error.read(cx);
    let continue_toggle = continue_on_error.clone();
    h_flex().gap_4().items_center().child(
        Switch::new("compare-sync-continue-on-error")
            .small()
            .checked(continue_checked)
            .disabled(is_executing)
            .label(t!("Compare.continue_on_error").to_string())
            .on_click(move |checked, _, cx| {
                continue_toggle.update(cx, |value, cx| {
                    *value = *checked;
                    cx.notify();
                });
            }),
    )
}

pub(super) fn sync_sql_execution_log_panel(
    execution_log: &Entity<Vec<SyncSqlExecutionLogEntry>>,
    execution_log_scroll: &ScrollHandle,
    is_executing: bool,
    cx: &App,
) -> impl IntoElement {
    let entries = execution_log.read(cx).clone();

    v_flex()
        .size_full()
        .min_h_0()
        .overflow_hidden()
        .gap_2()
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .child(section_title(
                    t!("Compare.sync_sql_execution_log").to_string(),
                ))
                .when(is_executing, |this| {
                    this.child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("Compare.sync_sql_executing").to_string()),
                    )
                }),
        )
        .child(
            div()
                .flex_1()
                .h_full()
                .min_h_0()
                .border_1()
                .border_color(cx.theme().border)
                .rounded_md()
                .bg(cx.theme().background)
                .relative()
                .overflow_hidden()
                .child(
                    div()
                        .id("compare-sync-sql-execution-log-scroll")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(execution_log_scroll)
                        .child(
                            v_flex().w_full().p_2().pr_4().gap_1().children(
                                entries.into_iter().enumerate().map(|(index, entry)| {
                                    sync_sql_execution_log_row(index, entry, cx)
                                }),
                            ),
                        ),
                )
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .right_0()
                        .bottom_0()
                        .w(px(16.0))
                        .child(Scrollbar::vertical(execution_log_scroll)),
                ),
        )
}

fn sync_sql_execution_log_row(
    index: usize,
    entry: SyncSqlExecutionLogEntry,
    cx: &App,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .items_start()
        .gap_2()
        .text_xs()
        .child(
            div()
                .w(px(36.0))
                .text_color(cx.theme().muted_foreground)
                .child(format!("{:>3}", index + 1)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_color(if entry.is_error {
                    cx.theme().danger
                } else {
                    cx.theme().foreground
                })
                .child(entry.message),
        )
}

#[cfg(test)]
mod tests {
    use gpui::{
        AppContext, Context, Entity, IntoElement, Render, ScrollHandle, TestAppContext, Window, div,
    };
    use gpui_component::{
        input::InputState,
        select::{SearchableVec, SelectState},
    };

    use super::{
        CompareSyncExecutionOptions, ConnectionSelectItem, SyncSqlExecutionLogEntry,
        SyncSqlExecutionSummary, connection_select_state, selected_connection_id,
        start_sync_sql_execution, sync_sql_execution_summary_log_entry,
        sync_sql_progress_error_log_entry,
    };

    struct ConnectionSelectTestRoot {
        select: Entity<SelectState<SearchableVec<ConnectionSelectItem>>>,
        fallback: Entity<InputState>,
    }

    struct SyncSqlExecutionTestRoot {
        status: Entity<String>,
        is_executing: Entity<bool>,
        execution_log: Entity<Vec<SyncSqlExecutionLogEntry>>,
        execution_log_scroll: ScrollHandle,
    }

    impl Render for ConnectionSelectTestRoot {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    impl Render for SyncSqlExecutionTestRoot {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    #[gpui::test]
    fn connection_select_state_keeps_current_connection_when_items_are_missing(
        cx: &mut TestAppContext,
    ) {
        let (root, cx) = cx.add_window_view(|window, cx| {
            let fallback = cx.new(|cx| InputState::new(window, cx).default_value("conn-missing"));
            ConnectionSelectTestRoot {
                select: connection_select_state("conn-missing", window, cx),
                fallback,
            }
        });

        let selected = root.read_with(cx, |root, cx| {
            root.select.read(cx).selected_value().cloned()
        });
        assert_eq!(Some("conn-missing".to_string()), selected);
    }

    #[gpui::test]
    fn selected_connection_id_uses_fallback_when_select_is_empty(cx: &mut TestAppContext) {
        let (root, cx) = cx.add_window_view(|window, cx| {
            let fallback = cx.new(|cx| InputState::new(window, cx).default_value("conn-fallback"));
            ConnectionSelectTestRoot {
                select: connection_select_state("", window, cx),
                fallback,
            }
        });

        let selected = root.read_with(cx, |root, cx| {
            selected_connection_id(&root.select, &root.fallback, cx)
        });
        assert_eq!("conn-fallback", selected);
    }

    #[gpui::test]
    fn sync_sql_execution_error_updates_footer_status(cx: &mut TestAppContext) {
        let (root, cx) = cx.add_window_view(|_, cx| SyncSqlExecutionTestRoot {
            status: cx.new(|_| "compare complete".to_string()),
            is_executing: cx.new(|_| false),
            execution_log: cx.new(|_| Vec::new()),
            execution_log_scroll: ScrollHandle::new(),
        });

        root.update_in(cx, |root, window, cx| {
            start_sync_sql_execution(
                None,
                "SELECT 1;".to_string(),
                CompareSyncExecutionOptions::default(),
                root.status.clone(),
                root.is_executing.clone(),
                root.execution_log.clone(),
                root.execution_log_scroll.clone(),
                window,
                cx,
            );
        });

        let (status, logs) = root.read_with(cx, |root, cx| {
            (
                root.status.read(cx).clone(),
                root.execution_log.read(cx).clone(),
            )
        });
        assert_eq!(1, logs.len());
        assert!(logs[0].is_error);
        assert_eq!(logs[0].message, status);
    }

    #[test]
    fn sync_sql_execution_summary_distinguishes_success_continue_and_stop() {
        let success = sync_sql_execution_summary_log_entry(&SyncSqlExecutionSummary {
            success_count: 3,
            error_count: 0,
            stopped_on_error: false,
        });
        assert!(!success.is_error);
        assert!(success.message.contains('3'));

        let continued = sync_sql_execution_summary_log_entry(&SyncSqlExecutionSummary {
            success_count: 2,
            error_count: 4,
            stopped_on_error: false,
        });
        assert!(continued.is_error);
        assert!(continued.message.contains('2'));
        assert!(continued.message.contains('4'));

        let stopped = sync_sql_execution_summary_log_entry(&SyncSqlExecutionSummary {
            success_count: 1,
            error_count: 1,
            stopped_on_error: true,
        });
        assert!(stopped.is_error);
        assert!(stopped.message.contains('1'));
    }

    #[test]
    fn sync_sql_zero_statement_error_uses_execution_failure_message() {
        let entry = sync_sql_progress_error_log_entry(0, "session failed");

        assert!(entry.is_error);
        assert!(entry.message.contains("session failed"));
        assert!(!entry.message.contains(" 0 "));
    }

    #[test]
    fn destructive_sync_sql_detection_catches_dangerous_statements() {
        assert!(super::contains_destructive_sync_sql(
            "DELETE FROM users WHERE id = 1;"
        ));
        assert!(super::contains_destructive_sync_sql("TRUNCATE TABLE logs;"));
        assert!(super::contains_destructive_sync_sql("DROP TABLE users;"));
    }

    #[test]
    fn destructive_sync_sql_detection_ignores_comments_and_safe_sql() {
        assert!(!super::contains_destructive_sync_sql(
            "-- DELETE FROM users;\nINSERT INTO users (id) VALUES (1);"
        ));
        assert!(!super::contains_destructive_sync_sql(
            "UPDATE users SET name = 'drop table note' WHERE id = 1;"
        ));
    }
}
