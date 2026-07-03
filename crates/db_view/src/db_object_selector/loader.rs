use db::GlobalDbState;
use gpui::{AppContext, AsyncApp, Context, Entity};
use gpui_component::{IndexPath, select::SearchableVec};
use rust_i18n::t;

use crate::compare::window_ui::register_connection_for_compare;
use crate::db_object_selector::state::{
    StringSelect, TargetConnectionControls, TargetStringControls, policy_for_connection,
    selected_string,
};

pub(crate) fn load_databases<T: 'static>(
    connection: TargetConnectionControls,
    database: TargetStringControls,
    status: Entity<String>,
    cx: &mut Context<T>,
) {
    let connection_id = selected_connection(&connection, cx);
    let policy = policy_for_connection(&connection, cx);
    let preferred = selected_string(&database.select, &database.fallback, cx);
    clear_string_select(&database.select, cx);
    if connection_id.trim().is_empty() {
        return set_status(
            &status,
            t!("DbObjectSelector.select_connection").to_string(),
            cx,
        );
    }
    let loading_key = if policy.schema_as_database {
        "DbObjectSelector.loading_schemas"
    } else {
        "DbObjectSelector.loading_databases"
    };
    prepare_load(&connection_id, &status, loading_key, cx);
    let db_state = cx.global::<GlobalDbState>().clone();
    cx.spawn(async move |_, cx: &mut AsyncApp| {
        let result = if policy.schema_as_database {
            db_state
                .list_schemas(cx, connection_id, String::new())
                .await
        } else {
            db_state.list_databases(cx, connection_id).await
        };
        update_string_select_async(result, database.select, preferred, status, cx);
    })
    .detach();
}

pub(crate) fn load_schemas<T: 'static>(
    connection: TargetConnectionControls,
    database: TargetStringControls,
    schema: TargetStringControls,
    status: Entity<String>,
    cx: &mut Context<T>,
) {
    let connection_id = selected_connection(&connection, cx);
    let policy = policy_for_connection(&connection, cx);
    let database_name = selected_select_string(&database.select, cx);
    let preferred = selected_string(&schema.select, &schema.fallback, cx);
    clear_string_select(&schema.select, cx);
    if !policy.show_schema {
        return;
    }
    if connection_id.trim().is_empty() || database_name.trim().is_empty() {
        return set_status(
            &status,
            t!("DbObjectSelector.select_connection_database").to_string(),
            cx,
        );
    }
    prepare_load(
        &connection_id,
        &status,
        "DbObjectSelector.loading_schemas",
        cx,
    );
    let db_state = cx.global::<GlobalDbState>().clone();
    cx.spawn(async move |_, cx: &mut AsyncApp| {
        let result = db_state
            .list_schemas(cx, connection_id, database_name)
            .await;
        update_string_select_async(result, schema.select, preferred, status, cx);
    })
    .detach();
}

pub(crate) fn clear_string_select<T: 'static>(select: &StringSelect, cx: &mut Context<T>) {
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

fn prepare_load<T>(
    connection_id: &str,
    status: &Entity<String>,
    message_key: &str,
    cx: &mut Context<T>,
) {
    register_connection_for_compare(connection_id, cx);
    set_status(status, t!(message_key).to_string(), cx);
}

fn selected_connection<T>(controls: &TargetConnectionControls, cx: &Context<T>) -> String {
    controls
        .select
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
        Err(error) => t!("DbObjectSelector.load_failed", error = error.to_string()).to_string(),
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
    t!("DbObjectSelector.loaded_count", count = count).to_string()
}

fn preferred_index(items: &[String], preferred: &str) -> Option<IndexPath> {
    items
        .iter()
        .position(|item| item == preferred)
        .or((!items.is_empty()).then_some(0))
        .map(IndexPath::new)
}

fn set_status<T>(status: &Entity<String>, message: String, cx: &mut Context<T>) {
    status.update(cx, |status, cx| {
        *status = message;
        cx.notify();
    });
}
