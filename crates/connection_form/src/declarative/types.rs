use gpui::SharedString;
use gpui_component::select::SelectItem;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeclarativeFormConfig {
    pub tabs: Vec<DeclarativeFormTab>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeclarativeFormTab {
    pub id: String,
    pub label: String,
    pub fields: Vec<DeclarativeFormField>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeclarativeFormField {
    pub id: String,
    pub label: String,
    pub field_type: DeclarativeFieldType,
    pub required: bool,
    pub default_value: Option<String>,
    pub placeholder: Option<String>,
    pub secret: bool,
    pub options: Vec<DeclarativeSelectOption>,
    pub visible_when: Vec<DeclarativeVisibilityRule>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeclarativeFieldType {
    Text,
    Number,
    Password,
    TextArea,
    Select,
    Checkbox,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeclarativeSelectOption {
    pub value: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeclarativeVisibilityRule {
    pub field: String,
    pub equals: String,
}

#[derive(Clone, Debug)]
pub(crate) struct FormSelectItem {
    pub(super) value: String,
    pub(super) label: SharedString,
}

impl SelectItem for FormSelectItem {
    type Value = String;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}
