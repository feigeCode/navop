use std::sync::Arc;

use db::GlobalDbState;
use db::compare::DataCompareResult;
use gpui::{
    App, AppContext, AsyncApp, Context, Entity, Hsla, IntoElement, ParentElement, Styled, Window,
    div, px,
};
use gpui_component::{
    ActiveTheme, IndexPath, Sizable, StyledExt,
    button::Button,
    clipboard::Clipboard,
    h_flex,
    highlighter::Language,
    input::{Input, InputState},
    progress::Progress,
    select::{SearchableVec, Select, SelectItem, SelectState},
    v_flex,
};
use one_core::storage::{
    ConnectionRepository, ConnectionType, GlobalStorageState, StoredConnection, traits::Repository,
};
use rust_i18n::t;

use crate::compare::{CompareProgress, CompareTargetScope, execute_sync_sql};

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
    let items = connection_select_items(cx);
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
    _fallback: &Entity<InputState>,
    cx: &App,
) -> String {
    select
        .read(cx)
        .selected_value()
        .cloned()
        .unwrap_or_default()
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

pub(super) fn data_truncation_note(result: &DataCompareResult) -> Option<String> {
    match (result.source_truncated, result.target_truncated) {
        (true, true) => Some(t!("Compare.data_truncated_both").to_string()),
        (true, false) => Some(t!("Compare.data_truncated_source").to_string()),
        (false, true) => Some(t!("Compare.data_truncated_target").to_string()),
        (false, false) => None,
    }
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
    cx: &mut Context<T>,
) {
    let Some(target) = target else {
        set_status(
            &status,
            t!("Compare.sync_sql_compare_first").to_string(),
            cx,
        );
        return;
    };
    if sql.trim().is_empty() {
        set_status(&status, t!("Compare.sync_sql_empty").to_string(), cx);
        return;
    }

    let db_state = Arc::new(cx.global::<GlobalDbState>().clone());
    is_executing.update(cx, |executing, cx| {
        *executing = true;
        cx.notify();
    });
    set_status(&status, t!("Compare.sync_sql_executing").to_string(), cx);

    cx.spawn(async move |_, cx: &mut AsyncApp| {
        let result = execute_sync_sql(target, sql, db_state, cx).await;
        let _ = cx.update(|cx| {
            is_executing.update(cx, |executing, cx| {
                *executing = false;
                cx.notify();
            });
            match result {
                Ok(count) => set_status_app(
                    &status,
                    t!("Compare.sync_sql_executed", count = count).to_string(),
                    cx,
                ),
                Err(error) => set_status_app(
                    &status,
                    t!("Compare.execution_failed", error = error.to_string()).to_string(),
                    cx,
                ),
            }
        });
    })
    .detach();
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
