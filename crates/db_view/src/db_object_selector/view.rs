use extension_component::DbSelectorKind;
use gpui::{Context, Div, IntoElement, ParentElement, Styled, div, prelude::FluentBuilder, px};
use gpui_component::{ActiveTheme, Sizable, select::Select, v_flex};
use rust_i18n::t;

use crate::compare::window_ui::{connection_select_row, section_title};
use crate::db_object_selector::parts::selector_includes;
use crate::db_object_selector::state::{DbObjectSelectorControls, StringSelect};

pub(crate) fn db_object_selector_panel<T: 'static>(
    title: impl Into<String>,
    kind: DbSelectorKind,
    controls: DbObjectSelectorControls,
    cx: &mut Context<T>,
) -> Div {
    let policy = controls.policy;
    let database = visible_control(&kind, DbSelectorKind::Database, controls.database);
    let schema = visible_schema_control(&kind, policy, controls.schema);
    let table = visible_control(&kind, DbSelectorKind::Table, controls.table);
    let column = visible_control(&kind, DbSelectorKind::Column, controls.column);

    v_flex()
        .gap_2()
        .p_3()
        .border_1()
        .border_color(cx.theme().border)
        .rounded_md()
        .child(section_title(title.into()))
        .child(connection_select_row(
            t!("DbObjectSelector.connection").to_string(),
            &controls.connection.select,
        ))
        .when_some(database, |this, control| {
            this.child(string_select_row(database_label(policy), &control.select))
        })
        .when_some(schema, |this, control| {
            this.child(string_select_row(
                label(DbSelectorKind::Schema),
                &control.select,
            ))
        })
        .when_some(table, |this, control| {
            this.child(string_select_row(
                label(DbSelectorKind::Table),
                &control.select,
            ))
        })
        .when_some(column, |this, control| {
            this.child(string_select_row(
                label(DbSelectorKind::Column),
                &control.select,
            ))
        })
}

fn visible_control<T>(
    target: &DbSelectorKind,
    part: DbSelectorKind,
    control: Option<T>,
) -> Option<T> {
    selector_includes(target, part).then_some(control).flatten()
}

fn visible_schema_control(
    target: &DbSelectorKind,
    policy: crate::db_object_selector::DbObjectSelectorPolicy,
    schema: Option<crate::db_object_selector::TargetStringControls>,
) -> Option<crate::db_object_selector::TargetStringControls> {
    if !policy.show_schema {
        return None;
    }
    visible_control(target, DbSelectorKind::Schema, schema)
}

fn database_label(policy: crate::db_object_selector::DbObjectSelectorPolicy) -> String {
    if policy.schema_as_database {
        label(DbSelectorKind::Schema)
    } else {
        label(DbSelectorKind::Database)
    }
}

fn label(kind: DbSelectorKind) -> String {
    match kind {
        DbSelectorKind::Connection => t!("DbObjectSelector.connection").to_string(),
        DbSelectorKind::Database => t!("DbObjectSelector.database").to_string(),
        DbSelectorKind::Schema => t!("DbObjectSelector.schema").to_string(),
        DbSelectorKind::Table => t!("DbObjectSelector.table").to_string(),
        DbSelectorKind::Column => t!("DbObjectSelector.column").to_string(),
    }
}

fn string_select_row(label: String, select: &StringSelect) -> impl IntoElement {
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
