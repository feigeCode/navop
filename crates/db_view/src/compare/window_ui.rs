use std::sync::Arc;

use db::GlobalDbState;
use db::compare::{DataCompareResult, SchemaCompareResult};
use gpui::{
    App, AppContext, AsyncApp, Context, Entity, Hsla, IntoElement, ParentElement, Styled, div, px,
};
use gpui_component::{
    IndexPath, Sizable, StyledExt,
    button::Button,
    clipboard::Clipboard,
    h_flex,
    input::{Input, InputState},
    scroll::ScrollableElement,
    select::{Select, SelectItem, SelectState},
    v_flex,
};
use one_core::storage::{
    ConnectionRepository, ConnectionType, GlobalStorageState, StoredConnection, traits::Repository,
};

use crate::compare::{CompareTargetScope, execute_sync_sql};

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

pub(super) fn data_summary(result: &DataCompareResult) -> String {
    format!(
        "Rows: +{} / -{} / ~{}{}{}",
        result.added.len(),
        result.removed.len(),
        result.modified.len(),
        if result.source_truncated {
            " | source truncated"
        } else {
            ""
        },
        if result.target_truncated {
            " | target truncated"
        } else {
            ""
        },
    )
}

pub(super) fn schema_summary(result: &SchemaCompareResult) -> String {
    format!(
        "Tables: +{} / -{} / ~{} ({} diff item(s))",
        result.added_count,
        result.removed_count,
        result.modified_count,
        result.table_diffs.len()
    )
}

pub(super) fn start_sync_sql_execution<T: 'static>(
    target: Option<CompareTargetScope>,
    sql: String,
    status: Entity<String>,
    is_executing: Entity<bool>,
    cx: &mut Context<T>,
) {
    let Some(target) = target else {
        set_status(&status, "Run compare before executing sync SQL", cx);
        return;
    };
    if sql.trim().is_empty() {
        set_status(&status, "No selected sync SQL to execute", cx);
        return;
    }

    let db_state = Arc::new(cx.global::<GlobalDbState>().clone());
    is_executing.update(cx, |executing, cx| {
        *executing = true;
        cx.notify();
    });
    set_status(&status, "Executing selected sync SQL...", cx);

    cx.spawn(async move |_, cx: &mut AsyncApp| {
        let result = execute_sync_sql(target, sql, db_state, cx).await;
        let _ = cx.update(|cx| {
            is_executing.update(cx, |executing, cx| {
                *executing = false;
                cx.notify();
            });
            match result {
                Ok(count) => {
                    set_status_app(&status, format!("Sync SQL executed: {count} result(s)"), cx)
                }
                Err(error) => set_status_app(&status, format!("Execute failed: {error}"), cx),
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

pub(super) fn sql_preview(
    copy_id: &'static str,
    sql: String,
    border_color: Hsla,
) -> impl IntoElement {
    let sql_for_copy = sql.clone();
    v_flex()
        .gap_1()
        .child(
            h_flex()
                .justify_between()
                .child(div().text_sm().font_semibold().child("Sync SQL"))
                .child(Clipboard::new(copy_id).value(sql_for_copy)),
        )
        .child(
            div()
                .max_h(px(220.0))
                .overflow_y_scrollbar()
                .p_3()
                .border_1()
                .border_color(border_color)
                .rounded_md()
                .text_sm()
                .child(sql),
        )
}
