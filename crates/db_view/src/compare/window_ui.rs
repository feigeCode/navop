use std::fmt::Display;
use std::sync::Arc;

use db::GlobalDbState;
use gpui::{
    App, AppContext, AsyncApp, Context, Entity, Hsla, IntoElement, ParentElement, Styled, Window,
    div, prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme, IndexPath, Sizable, StyledExt, WindowExt,
    button::Button,
    checkbox::Checkbox,
    clipboard::Clipboard,
    dialog::DialogButtonProps,
    h_flex,
    highlighter::Language,
    input::{Input, InputState},
    progress::Progress,
    scroll::ScrollableElement,
    select::{SearchableVec, Select, SelectItem, SelectState},
    v_flex,
};
use one_core::storage::{
    ConnectionRepository, ConnectionType, GlobalStorageState, StoredConnection, traits::Repository,
};
use rust_i18n::t;

use crate::compare::{CompareProgress, CompareTargetScope, execute_sync_sql};

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
    status: Entity<String>,
    is_executing: Entity<bool>,
    execution_log: Entity<Vec<SyncSqlExecutionLogEntry>>,
    window: &mut Window,
    cx: &mut Context<T>,
) {
    let Some(target) = target else {
        let message = t!("Compare.sync_sql_compare_first").to_string();
        set_status(&status, message.clone(), cx);
        append_sync_sql_execution_log(&execution_log, SyncSqlExecutionLogEntry::error(message), cx);
        return;
    };
    if sql.trim().is_empty() {
        let message = t!("Compare.sync_sql_empty").to_string();
        set_status(&status, message.clone(), cx);
        append_sync_sql_execution_log(&execution_log, SyncSqlExecutionLogEntry::error(message), cx);
        return;
    }

    if contains_destructive_sync_sql(&sql) {
        open_destructive_sync_sql_dialog(
            target,
            sql,
            status,
            is_executing,
            execution_log,
            window,
            cx,
        );
        return;
    }

    execute_sync_sql_now(target, sql, status, is_executing, execution_log, cx);
}

fn open_destructive_sync_sql_dialog<T: 'static>(
    target: CompareTargetScope,
    sql: String,
    status: Entity<String>,
    is_executing: Entity<bool>,
    execution_log: Entity<Vec<SyncSqlExecutionLogEntry>>,
    window: &mut Window,
    cx: &mut Context<T>,
) {
    let view = cx.entity().clone();
    window.open_dialog(cx, move |dialog, _window, _cx| {
        let target = target.clone();
        let sql = sql.clone();
        let status = status.clone();
        let is_executing = is_executing.clone();
        let execution_log = execution_log.clone();
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
                let status = status.clone();
                let is_executing = is_executing.clone();
                let execution_log = execution_log.clone();
                view.update(cx, |_, cx| {
                    execute_sync_sql_now(target, sql, status, is_executing, execution_log, cx);
                });
                true
            })
    });
}

fn execute_sync_sql_now<T: 'static>(
    target: CompareTargetScope,
    sql: String,
    status: Entity<String>,
    is_executing: Entity<bool>,
    execution_log: Entity<Vec<SyncSqlExecutionLogEntry>>,
    cx: &mut Context<T>,
) {
    let db_state = Arc::new(cx.global::<GlobalDbState>().clone());
    is_executing.update(cx, |executing, cx| {
        *executing = true;
        cx.notify();
    });
    let executing_message = t!("Compare.sync_sql_executing").to_string();
    set_status(&status, executing_message.clone(), cx);
    append_sync_sql_execution_log(
        &execution_log,
        SyncSqlExecutionLogEntry::info(executing_message),
        cx,
    );

    cx.spawn(async move |_, cx: &mut AsyncApp| {
        let result = execute_sync_sql(target, sql, db_state, cx).await;
        let _ = cx.update(|cx| {
            is_executing.update(cx, |executing, cx| {
                *executing = false;
                cx.notify();
            });
            match result {
                Ok(count) => {
                    let entry = sync_sql_execution_success_log_entry(count);
                    set_status_app(&status, entry.message.clone(), cx);
                    append_sync_sql_execution_log_app(&execution_log, entry, cx);
                }
                Err(error) => {
                    let entry = sync_sql_execution_error_log_entry(error);
                    set_status_app(&status, entry.message.clone(), cx);
                    append_sync_sql_execution_log_app(&execution_log, entry, cx);
                }
            }
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

pub(crate) fn sync_sql_execution_error_log_entry(error: impl Display) -> SyncSqlExecutionLogEntry {
    SyncSqlExecutionLogEntry::error(
        t!("Compare.execution_failed", error = error.to_string()).to_string(),
    )
}

pub(super) fn reset_sync_sql_execution_log<T: 'static>(
    execution_log: &Entity<Vec<SyncSqlExecutionLogEntry>>,
    entries: Vec<SyncSqlExecutionLogEntry>,
    cx: &mut Context<T>,
) {
    execution_log.update(cx, |log, cx| {
        *log = entries;
        cx.notify();
    });
}

pub(super) fn clear_sync_sql_execution_log<T: 'static>(
    execution_log: &Entity<Vec<SyncSqlExecutionLogEntry>>,
    cx: &mut Context<T>,
) {
    reset_sync_sql_execution_log(execution_log, Vec::new(), cx);
}

fn append_sync_sql_execution_log<T: 'static>(
    execution_log: &Entity<Vec<SyncSqlExecutionLogEntry>>,
    entry: SyncSqlExecutionLogEntry,
    cx: &mut Context<T>,
) {
    execution_log.update(cx, |log, cx| {
        log.push(entry);
        cx.notify();
    });
}

fn append_sync_sql_execution_log_app(
    execution_log: &Entity<Vec<SyncSqlExecutionLogEntry>>,
    entry: SyncSqlExecutionLogEntry,
    cx: &mut App,
) {
    execution_log.update(cx, |log, cx| {
        log.push(entry);
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

fn set_status<T>(status: &Entity<String>, message: impl Into<String>, cx: &mut Context<T>) {
    status.update(cx, |value, cx| {
        *value = message.into();
        cx.notify();
    });
}

fn set_status_app(status: &Entity<String>, message: impl Into<String>, cx: &mut App) {
    status.update(cx, |value, cx| {
        *value = message.into();
        cx.notify();
    });
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
        .gap_2()
        .child(div().w(px(120.0)).text_sm().child(label))
        .child(Input::new(input).small().w_full())
}

pub(crate) fn connection_select_row(
    label: impl Into<String>,
    select: &Entity<SelectState<SearchableVec<ConnectionSelectItem>>>,
) -> impl IntoElement {
    let label = label.into();
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(div().w(px(120.0)).text_sm().child(label.clone()))
        .child(
            Select::new(select)
                .small()
                .search_placeholder(t!("DbObjectSelector.search", item = label).to_string())
                .w_full(),
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

pub(super) fn sync_sql_execution_log_panel(
    execution_log: &Entity<Vec<SyncSqlExecutionLogEntry>>,
    is_executing: bool,
    cx: &App,
) -> impl IntoElement {
    let entries = execution_log.read(cx).clone();

    v_flex()
        .size_full()
        .min_h_0()
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
                .min_h_0()
                .border_1()
                .border_color(cx.theme().border)
                .rounded_md()
                .bg(cx.theme().background)
                .overflow_y_scrollbar()
                .child(
                    v_flex().p_2().gap_1().children(
                        entries
                            .into_iter()
                            .enumerate()
                            .map(|(index, entry)| sync_sql_execution_log_row(index, entry, cx)),
                    ),
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
    use gpui::{AppContext, Context, Entity, IntoElement, Render, TestAppContext, Window, div};
    use gpui_component::{
        input::InputState,
        select::{SearchableVec, SelectState},
    };

    use super::{ConnectionSelectItem, connection_select_state, selected_connection_id};

    struct ConnectionSelectTestRoot {
        select: Entity<SelectState<SearchableVec<ConnectionSelectItem>>>,
        fallback: Entity<InputState>,
    }

    impl Render for ConnectionSelectTestRoot {
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
