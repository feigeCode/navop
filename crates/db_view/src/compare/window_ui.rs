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
    select::{Select, SelectItem, SelectState},
    v_flex,
};
use one_core::storage::{
    ConnectionRepository, ConnectionType, GlobalStorageState, StoredConnection, traits::Repository,
};

use crate::compare::{CompareProgress, CompareTargetScope, execute_sync_sql};

#[derive(Clone, Debug)]
pub(super) struct ConnectionSelectItem {
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
        Some(Self {
            label: format!("{} ({id})", connection.name),
            id,
        })
    }
}

pub(super) fn connection_select_state(
    current_connection_id: &str,
    window: &mut gpui::Window,
    cx: &mut App,
) -> Entity<SelectState<Vec<ConnectionSelectItem>>> {
    let items = connection_select_items(cx);
    let selected_index = items
        .iter()
        .position(|item| item.id == current_connection_id)
        .map(IndexPath::new);
    cx.new(|cx| SelectState::new(items, selected_index, window, cx))
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
    select: &Entity<SelectState<Vec<ConnectionSelectItem>>>,
    fallback: &Entity<InputState>,
    cx: &App,
) -> String {
    select
        .read(cx)
        .selected_value()
        .cloned()
        .unwrap_or_else(|| fallback.read(cx).text().to_string())
}

pub(super) fn data_truncation_note(result: &DataCompareResult) -> Option<String> {
    match (result.source_truncated, result.target_truncated) {
        (true, true) => Some("源/目标数据均已截断,仅比较前若干行".to_string()),
        (true, false) => Some("源数据已截断,仅比较前若干行".to_string()),
        (false, true) => Some("目标数据已截断,仅比较前若干行".to_string()),
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
        .child(stat_card("+", added, "新增", cx.theme().success, cx))
        .child(stat_card("−", removed, "删除", cx.theme().danger, cx))
        .child(stat_card("~", modified, "修改", cx.theme().warning, cx))
}

fn stat_card(sign: &str, count: usize, label: &str, color: Hsla, cx: &App) -> impl IntoElement {
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
                .child(label.to_string()),
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
        set_status(&status, "请先执行比较,再执行同步 SQL", cx);
        return;
    };
    if sql.trim().is_empty() {
        set_status(&status, "没有可执行的同步 SQL", cx);
        return;
    }

    let db_state = Arc::new(cx.global::<GlobalDbState>().clone());
    is_executing.update(cx, |executing, cx| {
        *executing = true;
        cx.notify();
    });
    set_status(&status, "正在执行同步 SQL…", cx);

    cx.spawn(async move |_, cx: &mut AsyncApp| {
        let result = execute_sync_sql(target, sql, db_state, cx).await;
        let _ = cx.update(|cx| {
            is_executing.update(cx, |executing, cx| {
                *executing = false;
                cx.notify();
            });
            match result {
                Ok(count) => {
                    set_status_app(&status, format!("同步 SQL 执行完成:{count} 条结果"), cx)
                }
                Err(error) => set_status_app(&status, format!("执行失败:{error}"), cx),
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

pub(super) fn section_title(title: &'static str) -> impl IntoElement {
    div().font_semibold().child(title)
}

pub(super) fn close_button() -> impl IntoElement {
    Button::new("close")
        .child("Close")
        .on_click(|_, window, _| {
            window.remove_window();
        })
}

pub(super) fn input_row(label: &'static str, input: &Entity<InputState>) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(div().w(px(120.0)).text_sm().child(label))
        .child(Input::new(input).small().w_full())
}

pub(super) fn connection_select_row(
    label: &'static str,
    select: &Entity<SelectState<Vec<ConnectionSelectItem>>>,
) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(div().w(px(120.0)).text_sm().child(label))
        .child(Select::new(select).small().w_full())
}

pub(super) fn detail_row(label: &'static str, value: String) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(div().w(px(120.0)).text_sm().child(label))
        .child(div().text_sm().child(if value.is_empty() {
            "-".to_string()
        } else {
            value
        }))
}

/// 创建用于「同步 SQL」的代码编辑器(SQL 语法高亮 + 行号,可编辑)
pub(super) fn sync_sql_editor_state(window: &mut Window, cx: &mut App) -> Entity<InputState> {
    cx.new(|cx| {
        InputState::new(window, cx)
            .code_editor(Language::from_str("sql"))
            .line_number(true)
            .multi_line(true)
            .soft_wrap(false)
            .placeholder("比较完成后,选中的同步语句将在此生成,可手动编辑后执行")
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
                .child(div().text_sm().font_semibold().child("同步 SQL"))
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
