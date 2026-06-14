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
pub struct ViewActionEvent {
    pub view_id: String,
    pub action_id: String,
    pub fields: Vec<FieldValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldValue {
    pub id: String,
    pub value: String,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialog_form_spec_can_describe_database_selectors() {
        let spec = ViewSpec::dialog(
            "backup-form",
            "备份数据库",
            vec![UiNode::form(vec![
                UiField::db_select("connection", "连接", DbSelectorKind::Connection),
                UiField::db_select("database", "数据库", DbSelectorKind::Database),
                UiField::db_select("schema", "Schema", DbSelectorKind::Schema),
                UiField::db_select("table", "表", DbSelectorKind::Table),
                UiField::db_select("column", "字段", DbSelectorKind::Column),
            ])],
            vec![UiAction::primary("run", "执行")],
        );

        assert_eq!(ViewMode::Dialog, spec.mode);
        assert_eq!("backup-form", spec.id);
        assert_eq!(1, spec.nodes.len());
        assert_eq!(1, spec.actions.len());
    }

    #[test]
    fn dialog_spec_can_define_window_size() {
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
        assert_eq!(Some(480.0), window.min_width);
        assert_eq!(Some(360.0), window.min_height);
    }

    #[test]
    fn view_action_event_carries_typed_form_values() {
        let event = ViewActionEvent {
            view_id: "full-search".to_string(),
            action_id: "run".to_string(),
            fields: vec![FieldValue {
                id: "database".to_string(),
                value: "app".to_string(),
            }],
        };

        assert_eq!("full-search", event.view_id);
        assert_eq!("run", event.action_id);
        assert_eq!("database", event.fields[0].id);
        assert_eq!("app", event.fields[0].value);
    }
}
