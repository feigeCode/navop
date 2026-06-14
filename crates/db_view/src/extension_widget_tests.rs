use std::collections::BTreeMap;

use extension_component::ui_protocol::{
    DbSelectorKind, SelectOption, UiAction, UiField, UiFieldKind, UiNode, ViewMode, ViewSpec,
};

use crate::extension_widget::{
    build_extension_widget_model, build_extension_widget_model_with_options, default_form_values,
    form_values_to_action_event,
};

#[test]
fn render_model_extracts_dialog_form_fields_and_actions() {
    let spec = ViewSpec::dialog(
        "full-search",
        "全库搜索",
        vec![UiNode::form(vec![
            UiField::db_select("connection", "连接", DbSelectorKind::Connection),
            UiField::db_select("database", "数据库", DbSelectorKind::Database),
        ])],
        vec![UiAction::primary("run", "搜索")],
    );

    let model = build_extension_widget_model(&spec).unwrap();
    assert_eq!(ViewMode::Dialog, model.mode);
    assert_eq!("全库搜索", model.title);
    assert_eq!(3, model.fields.len());
    assert_eq!("connection.connection_id", model.fields[0].id);
    assert_eq!("database.connection_id", model.fields[1].id);
    assert_eq!("database.database", model.fields[2].id);
    assert_eq!(1, model.actions.len());
}

#[test]
fn render_model_preserves_plain_field_kind() {
    let spec = ViewSpec::dialog(
        "coverage",
        "Coverage",
        vec![UiNode::form(vec![UiField {
            id: "keyword".to_string(),
            label: "Keyword".to_string(),
            kind: UiFieldKind::Text,
            required: false,
            value: None,
            source: None,
        }])],
        vec![],
    );

    let model = build_extension_widget_model(&spec).unwrap();
    assert_eq!(UiFieldKind::Text, model.fields[0].kind);
}

#[test]
fn default_form_values_include_checkbox_false() {
    let spec = ViewSpec::dialog(
        "coverage",
        "Coverage",
        vec![UiNode::form(vec![UiField {
            id: "dry_run".to_string(),
            label: "Dry run".to_string(),
            kind: UiFieldKind::Checkbox,
            required: false,
            value: None,
            source: None,
        }])],
        vec![],
    );
    let model = build_extension_widget_model(&spec).unwrap();
    let values = default_form_values(&model);
    assert_eq!(Some("false"), values.get("dry_run").map(String::as_str));
}

#[test]
fn render_model_attaches_loaded_selector_options_to_fields() {
    let spec = ViewSpec::dialog(
        "full-search",
        "全库搜索",
        vec![UiNode::form(vec![UiField::db_select(
            "database",
            "数据库",
            DbSelectorKind::Database,
        )])],
        vec![UiAction::primary("run", "搜索")],
    );
    let mut selector_options = BTreeMap::new();
    selector_options.insert(
        "database".to_string(),
        vec![SelectOption {
            value: "app".to_string(),
            label: "app".to_string(),
        }],
    );

    let model = build_extension_widget_model_with_options(&spec, selector_options).unwrap();
    let database = model
        .fields
        .iter()
        .find(|field| field.id == "database.database")
        .unwrap();
    assert_eq!(1, database.options.len());
    assert_eq!("app", database.options[0].value);
}

#[test]
fn db_table_selector_expands_to_composite_controls() {
    let spec = ViewSpec::dialog(
        "full-search",
        "全库搜索",
        vec![UiNode::form(vec![UiField::db_select(
            "target",
            "目标",
            DbSelectorKind::Table,
        )])],
        vec![UiAction::primary("run", "搜索")],
    );

    let model = build_extension_widget_model(&spec).unwrap();
    let field_ids = model
        .fields
        .iter()
        .map(|field| field.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        vec![
            "target.connection_id",
            "target.database",
            "target.schema",
            "target.table"
        ],
        field_ids
    );
}

#[test]
fn db_connection_selector_expands_to_connection_only() {
    let spec = ViewSpec::dialog(
        "connection-picker",
        "选择连接",
        vec![UiNode::form(vec![UiField::db_select(
            "target",
            "目标",
            DbSelectorKind::Connection,
        )])],
        vec![UiAction::primary("run", "执行")],
    );

    let model = build_extension_widget_model(&spec).unwrap();
    assert_eq!(1, model.fields.len());
    assert_eq!("target.connection_id", model.fields[0].id);
}

#[test]
fn db_column_selector_expands_to_full_composite_controls() {
    let spec = ViewSpec::dialog(
        "column-picker",
        "选择字段",
        vec![UiNode::form(vec![UiField::db_select(
            "target",
            "目标",
            DbSelectorKind::Column,
        )])],
        vec![UiAction::primary("run", "执行")],
    );

    let model = build_extension_widget_model(&spec).unwrap();
    let field_ids = model
        .fields
        .iter()
        .map(|field| field.id.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        vec![
            "target.connection_id",
            "target.database",
            "target.schema",
            "target.table",
            "target.column"
        ],
        field_ids
    );
}

#[test]
fn default_form_values_use_first_selector_option() {
    let spec = ViewSpec::dialog(
        "full-search",
        "全库搜索",
        vec![UiNode::form(vec![UiField::db_select(
            "database",
            "数据库",
            DbSelectorKind::Database,
        )])],
        vec![UiAction::primary("run", "搜索")],
    );
    let mut selector_options = BTreeMap::new();
    selector_options.insert(
        "database".to_string(),
        vec![SelectOption {
            value: "app".to_string(),
            label: "app".to_string(),
        }],
    );
    let model = build_extension_widget_model_with_options(&spec, selector_options).unwrap();

    let values = default_form_values(&model);

    assert_eq!(
        Some("app"),
        values.get("database.database").map(String::as_str)
    );
}

#[test]
fn explicit_field_value_overrides_first_option() {
    let mut field = UiField::db_select("database", "数据库", DbSelectorKind::Database);
    field.value = Some("warehouse".to_string());
    let spec = ViewSpec::dialog(
        "full-search",
        "全库搜索",
        vec![UiNode::form(vec![field])],
        vec![UiAction::primary("run", "搜索")],
    );
    let mut selector_options = BTreeMap::new();
    selector_options.insert(
        "database".to_string(),
        vec![SelectOption {
            value: "app".to_string(),
            label: "app".to_string(),
        }],
    );
    let model = build_extension_widget_model_with_options(&spec, selector_options).unwrap();

    let values = default_form_values(&model);

    assert_eq!(
        Some("warehouse"),
        values.get("database.database").map(String::as_str)
    );
}

#[test]
fn form_values_to_action_event_carries_stable_typed_fields() {
    let mut values = BTreeMap::new();
    values.insert("table".to_string(), "orders".to_string());
    values.insert("database".to_string(), "app".to_string());

    let event = form_values_to_action_event("full-search", "run", &values);

    assert_eq!("full-search", event.view_id);
    assert_eq!("run", event.action_id);
    assert_eq!(2, event.fields.len());
    assert_eq!("database", event.fields[0].id);
    assert_eq!("app", event.fields[0].value);
    assert_eq!("table", event.fields[1].id);
}

#[test]
fn form_values_to_action_event_aliases_composite_selector_deepest_value() {
    let mut values = BTreeMap::new();
    values.insert("target.connection_id".to_string(), "conn-1".to_string());
    values.insert("target.database".to_string(), "app".to_string());
    values.insert("target.schema".to_string(), "public".to_string());
    values.insert("target.table".to_string(), "orders".to_string());

    let event = form_values_to_action_event("full-search", "run", &values);

    assert_eq!("target", event.fields[0].id);
    assert_eq!("orders", event.fields[0].value);
    assert!(
        event
            .fields
            .iter()
            .any(|field| { field.id == "target.connection_id" && field.value == "conn-1" })
    );
}

#[test]
fn form_values_to_action_event_aliases_column_selector_to_column() {
    let mut values = BTreeMap::new();
    values.insert("target.connection_id".to_string(), "conn-1".to_string());
    values.insert("target.database".to_string(), "app".to_string());
    values.insert("target.schema".to_string(), "public".to_string());
    values.insert("target.table".to_string(), "orders".to_string());
    values.insert("target.column".to_string(), "amount".to_string());

    let event = form_values_to_action_event("column-picker", "run", &values);

    assert_eq!("target", event.fields[0].id);
    assert_eq!("amount", event.fields[0].value);
    assert!(
        event
            .fields
            .iter()
            .any(|field| { field.id == "target.column" && field.value == "amount" })
    );
}
