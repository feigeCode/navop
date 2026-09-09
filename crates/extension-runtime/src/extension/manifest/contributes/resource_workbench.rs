use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchContrib {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    pub id: String,
    pub title: String,
    #[serde(rename = "connectionIds")]
    pub connection_ids: Vec<String>,
    #[serde(rename = "runtimeId")]
    pub runtime_id: String,
    #[serde(rename = "resourceType")]
    pub resource_type: String,
    #[serde(rename = "defaultPage")]
    pub default_page: String,
    pub operations: BTreeMap<String, ResourceWorkbenchOperation>,
    #[serde(default)]
    pub navigation: Vec<ResourceWorkbenchNavigation>,
    #[serde(default)]
    pub tree: Vec<ResourceWorkbenchTree>,
    pub pages: Vec<ResourceWorkbenchPage>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchOperation {
    pub mode: ResourceWorkbenchOperationMode,
    pub method: String,
    #[serde(default)]
    pub requires: Vec<String>,
    pub effect: ResourceWorkbenchEffect,
    #[serde(default)]
    pub params: BTreeMap<String, ResourceWorkbenchBinding>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ResourceWorkbenchOperationMode {
    Invoke,
    Job,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ResourceWorkbenchEffect {
    Read,
    Write,
    Destructive,
    Unknown,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchBinding {
    pub source: ResourceWorkbenchBindingSource,
    #[serde(default)]
    pub path: String,
    #[serde(rename = "type")]
    pub value_type: ResourceWorkbenchValueType,
    #[serde(default)]
    pub value: Option<Value>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ResourceWorkbenchBindingSource {
    Literal,
    Input,
    Route,
    Selection,
    Paging,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ResourceWorkbenchValueType {
    String,
    Number,
    Boolean,
    Json,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchNavigation {
    #[serde(rename = "pageId")]
    pub page_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchTree {
    pub id: String,
    pub title: String,
    #[serde(rename = "pageId")]
    pub page_id: String,
    #[serde(default)]
    pub children: Option<ResourceWorkbenchTreeChildren>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchTreeChildren {
    pub operation: String,
    #[serde(rename = "itemsPath")]
    pub items_path: String,
    #[serde(rename = "keyPaths")]
    pub key_paths: Vec<String>,
    #[serde(rename = "labelPath")]
    pub label_path: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchPage {
    pub id: String,
    pub title: String,
    pub template: ResourceWorkbenchTemplate,
    pub renderer: ResourceWorkbenchRenderer,
    #[serde(default)]
    pub load: Option<ResourceWorkbenchAction>,
    #[serde(default)]
    pub execute: Option<ResourceWorkbenchAction>,
    #[serde(default)]
    pub collection: Option<ResourceWorkbenchCollection>,
    #[serde(default)]
    pub inputs: Vec<ResourceWorkbenchInput>,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ResourceWorkbenchTemplate {
    Overview,
    Collection,
    Detail,
    Query,
    Json,
    Events,
    Tasks,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchRenderer {
    pub kind: ResourceWorkbenchRendererKind,
    #[serde(default)]
    pub view_id: Option<String>,
    #[serde(default)]
    pub fallback: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ResourceWorkbenchRendererKind {
    Native,
    Shell,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchAction {
    pub operation: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchCollection {
    #[serde(rename = "itemsPath")]
    pub items_path: String,
    #[serde(rename = "keyPaths")]
    pub key_paths: Vec<String>,
    pub pagination: ResourceWorkbenchPagination,
    pub columns: Vec<ResourceWorkbenchColumn>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchPagination {
    pub kind: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchColumn {
    pub id: String,
    pub title: String,
    pub path: String,
    #[serde(rename = "type")]
    pub value_type: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchInput {
    pub id: String,
    #[serde(rename = "type")]
    pub value_type: String,
    pub editor: String,
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub required: bool,
}
