use db::{GlobalDbState, TableInfo};
use gpui::{
    App, AppContext, AsyncApp, Context, Entity, IntoElement, ParentElement, Styled, Window, div, px,
};
use gpui_component::{
    IndexPath, Sizable,
    input::InputState,
    select::{SearchableVec, Select, SelectState},
};

use crate::compare::window_ui::{ConnectionSelectItem, register_connection_for_compare};

pub(super) type StringSelect = Entity<SelectState<SearchableVec<String>>>;

pub(super) fn string_select_state(
    initial_value: String,
    window: &mut Window,
    cx: &mut App,
) -> StringSelect {
    let selected = if initial_value.is_empty() {
        None
    } else {
        Some(IndexPath::new(0))
    };
    let items = if initial_value.is_empty() {
        Vec::new()
    } else {
        vec![initial_value]
    };
    cx.new(|cx| SelectState::new(SearchableVec::new(items), selected, window, cx).searchable(true))
}

pub(super) fn selected_string(
    select: &StringSelect,
    fallback: &Entity<InputState>,
    cx: &App,
) -> String {
    select
        .read(cx)
        .selected_value()
        .cloned()
        .unwrap_or_else(|| fallback.read(cx).text().to_string())
}

pub(super) fn set_connection_select<T>(
    select: &Entity<SelectState<SearchableVec<ConnectionSelectItem>>>,
    value: &str,
    window: &mut Window,
    cx: &mut Context<T>,
) {
    select.update(cx, |state, cx| {
        state.set_selected_value(&value.to_string(), window, cx);
    });
}

pub(super) fn set_string_select<T>(
    select: &StringSelect,
    fallback: &Entity<InputState>,
    value: String,
    window: &mut Window,
    cx: &mut Context<T>,
) {
    fallback.update(cx, |input, cx| {
        input.set_value(value.clone(), window, cx);
    });
    select.update(cx, |state, cx| {
        state.set_selected_value(&value, window, cx);
    });
}

pub(super) fn string_select_row(label: &'static str, select: &StringSelect) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(div().w(px(120.0)).text_sm().child(label))
        .child(
            Select::new(select)
                .small()
                .search_placeholder(format!("搜索{label}"))
                .w_full(),
        )
}

pub(super) fn load_databases<T: 'static>(
    connection_select: Entity<SelectState<SearchableVec<ConnectionSelectItem>>>,
    database_select: StringSelect,
    database_fallback: Entity<InputState>,
    status: Entity<String>,
    cx: &mut Context<T>,
) {
    let connection_id = selected_connection_value(&connection_select, cx);
    let preferred = selected_string(&database_select, &database_fallback, cx);
    clear_string_select(&database_select, cx);
    if connection_id.trim().is_empty() {
        set_status(&status, "请先选择连接", cx);
        return;
    }
    register_connection_for_compare(&connection_id, cx);
    let db_state = cx.global::<GlobalDbState>().clone();
    set_status(&status, "正在加载数据库…", cx);

    cx.spawn(async move |_, cx: &mut AsyncApp| {
        let result = db_state.list_databases(cx, connection_id).await;
        update_string_select_async(result, database_select, preferred, status, cx);
    })
    .detach();
}

pub(super) fn load_schemas<T: 'static>(
    connection: TargetConnectionControls,
    database: TargetStringControls,
    schema: TargetStringControls,
    status: Entity<String>,
    cx: &mut Context<T>,
) {
    let connection_id = selected_connection_value(&connection.select, cx);
    let database_name = selected_select_string(&database.select, cx);
    let preferred = selected_string(&schema.select, &schema.fallback, cx);
    clear_string_select(&schema.select, cx);
    if connection_id.trim().is_empty() || database_name.trim().is_empty() {
        set_status(&status, "请先选择连接和数据库", cx);
        return;
    }
    register_connection_for_compare(&connection_id, cx);
    let db_state = cx.global::<GlobalDbState>().clone();
    set_status(&status, "正在加载 Schema…", cx);

    cx.spawn(async move |_, cx: &mut AsyncApp| {
        let result = db_state
            .list_schemas(cx, connection_id, database_name)
            .await;
        update_string_select_async(result, schema.select, preferred, status, cx);
    })
    .detach();
}

pub(super) fn load_tables<T: 'static>(
    connection: TargetConnectionControls,
    database: TargetStringControls,
    schema: TargetStringControls,
    table: TargetStringControls,
    status: Entity<String>,
    cx: &mut Context<T>,
) {
    let connection_id = selected_connection_value(&connection.select, cx);
    let database_name = selected_select_string(&database.select, cx);
    let preferred = selected_string(&table.select, &table.fallback, cx);
    clear_string_select(&table.select, cx);
    if connection_id.trim().is_empty() || database_name.trim().is_empty() {
        set_status(&status, "请先选择连接和数据库", cx);
        return;
    }
    let schema_name = empty_to_none(selected_select_string(&schema.select, cx));
    register_connection_for_compare(&connection_id, cx);
    let db_state = cx.global::<GlobalDbState>().clone();
    set_status(&status, "正在加载表…", cx);

    cx.spawn(async move |_, cx: &mut AsyncApp| {
        let result = db_state
            .list_tables(cx, connection_id, database_name, schema_name)
            .await
            .map(table_names);
        update_string_select_async(result, table.select, preferred, status, cx);
    })
    .detach();
}

#[derive(Clone)]
pub(super) struct TargetConnectionControls {
    pub select: Entity<SelectState<SearchableVec<ConnectionSelectItem>>>,
}

#[derive(Clone)]
pub(super) struct TargetStringControls {
    pub select: StringSelect,
    pub fallback: Entity<InputState>,
}

fn table_names(tables: Vec<TableInfo>) -> Vec<String> {
    tables.into_iter().map(|table| table.name).collect()
}

fn empty_to_none(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub(super) fn clear_string_select<T: 'static>(select: &StringSelect, cx: &mut Context<T>) {
    let Some(window_id) = cx.active_window() else {
        return;
    };
    let select = select.clone();
    let _ = cx.update_window(window_id, |_, window, cx| {
        select.update(cx, |state, cx| {
            state.set_items(SearchableVec::new(Vec::new()), window, cx);
            state.set_selected_index(None, window, cx);
        });
    });
}

fn selected_connection_value<T>(
    select: &Entity<SelectState<SearchableVec<ConnectionSelectItem>>>,
    cx: &Context<T>,
) -> String {
    select
        .read(cx)
        .selected_value()
        .cloned()
        .unwrap_or_default()
}

fn selected_select_string<T>(select: &StringSelect, cx: &Context<T>) -> String {
    select
        .read(cx)
        .selected_value()
        .cloned()
        .unwrap_or_default()
}

fn update_string_select_async(
    result: anyhow::Result<Vec<String>>,
    select: StringSelect,
    preferred: String,
    status: Entity<String>,
    cx: &mut AsyncApp,
) {
    let message = match result {
        Ok(items) => update_select_items(select, items, preferred, cx),
        Err(error) => format!("加载失败:{error}"),
    };
    let _ = cx.update(|cx| {
        status.update(cx, |status, cx| {
            *status = message;
            cx.notify();
        });
    });
}

fn update_select_items(
    select: StringSelect,
    items: Vec<String>,
    preferred: String,
    cx: &mut AsyncApp,
) -> String {
    let count = items.len();
    let selected = preferred_index(&items, &preferred);
    let _ = cx.update(|cx| {
        if let Some(window_id) = cx.active_window() {
            let _ = cx.update_window(window_id, |_, window, cx| {
                select.update(cx, |state, cx| {
                    state.set_items(SearchableVec::new(items), window, cx);
                    state.set_selected_index(selected, window, cx);
                });
            });
        }
    });
    format!("已加载 {count} 项")
}

fn preferred_index(items: &[String], preferred: &str) -> Option<IndexPath> {
    items
        .iter()
        .position(|item| item == preferred)
        .or(if items.is_empty() { None } else { Some(0) })
        .map(IndexPath::new)
}

fn set_status<T>(status: &Entity<String>, message: impl Into<String>, cx: &mut Context<T>) {
    status.update(cx, |status, cx| {
        *status = message.into();
        cx.notify();
    });
}
