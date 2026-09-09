use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceConnectionContrib {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(rename = "runtimeId")]
    pub runtime_id: String,
    #[serde(rename = "resourceType")]
    pub resource_type: String,
    #[serde(default, rename = "shellViewId")]
    pub shell_view_id: Option<String>,
    #[serde(default)]
    pub form: ResourceConnectionForm,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceConnectionForm {
    #[serde(default)]
    pub tabs: Vec<ResourceConnectionFormTab>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceConnectionFormTab {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub fields: Vec<ResourceConnectionFormField>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceConnectionFormField {
    pub id: String,
    pub label: String,
    #[serde(rename = "fieldType")]
    pub field_type: ResourceConnectionFieldType,
    #[serde(default)]
    pub required: bool,
    #[serde(default, rename = "defaultValue")]
    pub default_value: Option<String>,
    #[serde(default)]
    pub placeholder: Option<String>,
    #[serde(default)]
    pub secret: bool,
    #[serde(default)]
    pub options: Vec<ResourceConnectionSelectOption>,
    #[serde(default, rename = "visibleWhen")]
    pub visible_when: Vec<ResourceConnectionVisibilityRule>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum ResourceConnectionFieldType {
    Text,
    Number,
    Password,
    TextArea,
    Select,
    Checkbox,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceConnectionSelectOption {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceConnectionVisibilityRule {
    pub field: String,
    pub equals: String,
}
