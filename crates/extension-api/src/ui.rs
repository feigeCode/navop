use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewSpec {
    pub id: String,
    pub title: String,
    pub mode: ViewMode,
    pub nodes: Vec<UiNode>,
    pub actions: Vec<UiAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<ViewWindowOptions>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewWindowOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_width: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_height: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionContext {
    pub extension_id: String,
    pub command_id: String,
    pub node_id: String,
    pub node_name: String,
    pub node_type: String,
    pub database_type: String,
    pub connection_id: String,
}

impl ViewSpec {
    pub fn dialog(
        id: impl Into<String>,
        title: impl Into<String>,
        nodes: Vec<UiNode>,
        actions: Vec<UiAction>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            mode: ViewMode::Dialog,
            nodes,
            actions,
            window: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewMode {
    Dialog,
    Window,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UiNode {
    Text { text: String },
    Form { fields: Vec<UiField> },
}

impl UiNode {
    pub fn form(fields: Vec<UiField>) -> Self {
        Self::Form { fields }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiField {
    pub id: String,
    pub label: String,
    pub kind: UiFieldKind,
    pub required: bool,
    pub value: Option<String>,
    pub source: Option<FieldSource>,
}

impl UiField {
    pub fn db_select(
        id: impl Into<String>,
        label: impl Into<String>,
        selector: DbSelectorKind,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            kind: UiFieldKind::Select,
            required: true,
            value: None,
            source: Some(FieldSource::DbSelector(DbSelectorSource {
                kind: selector,
                query: DbSelectorQuery::default(),
            })),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UiFieldKind {
    Text,
    TextArea,
    Password,
    Checkbox,
    Select,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldSource {
    StaticOptions(Vec<SelectOption>),
    DbSelector(DbSelectorSource),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectOption {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbSelectorSource {
    pub kind: DbSelectorKind,
    pub query: DbSelectorQuery,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DbSelectorKind {
    Connection,
    Database,
    Schema,
    Table,
    Column,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbSelectorQuery {
    pub connection_id: Option<String>,
    pub database: Option<String>,
    pub schema: Option<String>,
    pub table: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiAction {
    pub id: String,
    pub label: String,
    pub style: UiActionStyle,
}

impl UiAction {
    pub fn primary(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            style: UiActionStyle::Primary,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UiActionStyle {
    Primary,
    Secondary,
    Danger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotificationLevel {
    Info,
    Success,
    Warning,
    Error,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_level_has_info_variant() {
        assert_eq!(NotificationLevel::Info, NotificationLevel::Info);
    }

    #[test]
    fn view_spec_can_describe_database_selector_form() {
        let spec = ViewSpec::dialog(
            "backup",
            "备份",
            vec![UiNode::form(vec![UiField::db_select(
                "connection",
                "连接",
                DbSelectorKind::Connection,
            )])],
            vec![UiAction::primary("run", "执行")],
        );

        assert_eq!(ViewMode::Dialog, spec.mode);
        assert_eq!(1, spec.actions.len());
    }

    #[test]
    fn view_spec_can_serialize_optional_window_size() {
        let mut spec = ViewSpec::dialog("sized", "Sized", Vec::new(), Vec::new());
        spec.window = Some(ViewWindowOptions {
            width: Some(720.0),
            height: Some(640.0),
            min_width: Some(480.0),
            min_height: Some(360.0),
        });

        let window = spec.window.as_ref().unwrap();

        assert_eq!(Some(720.0), window.width);
        assert_eq!(Some(640.0), window.height);
    }

    #[test]
    fn wit_ui_interface_declares_view_spec_types() {
        let wit = include_str!("../wit/ui.wit");

        assert!(wit.contains("record action-context"));
        assert!(wit.contains("record view-spec"));
        assert!(wit.contains("record view-window-options"));
        assert!(wit.contains("variant ui-node"));
        assert!(wit.contains("enum db-selector-kind"));
        assert!(wit.contains("record db-selector-query"));
    }

    #[test]
    fn wit_extension_world_exports_run_action() {
        let wit = include_str!("../wit/extension.wit");

        assert!(wit.contains("export run-action: func();"));
        assert!(wit.contains("export handle-view-action: func(event: view-action-event);"));
        let ui_wit = include_str!("../wit/ui.wit");
        assert!(ui_wit.contains("record field-value"));
        assert!(ui_wit.contains("record view-action-event"));
        assert!(ui_wit.contains("current-action-context: func() -> option<action-context>;"));
        assert!(ui_wit.contains("open-view: func(view: view-spec);"));
    }
}
