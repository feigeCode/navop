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
    /// 工作台底部常驻状态栏声明(如 Engine 状态/资源占用)。
    #[serde(default, rename = "statusBar")]
    pub status_bar: Option<ResourceWorkbenchStatusBar>,
}

/// 工作台底部状态栏:由一个命名操作提供状态数据。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchStatusBar {
    /// 提供状态 JSON 的命名操作。
    pub operation: String,
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
    /// terminal 模板页面的终端声明。
    #[serde(default)]
    pub terminal: Option<ResourceWorkbenchTerminal>,
    /// 页面 tab 条声明:同一 tab 组的每个页面都声明完整列表,
    /// 渲染时按 `pageId == 当前页 id` 高亮当前项。
    #[serde(default)]
    pub tabs: Vec<ResourceWorkbenchTab>,
    /// detail/query 页面的路由参数声明(如 {"name": {"type": "string", "required": true}})。
    #[serde(default)]
    pub route: Option<BTreeMap<String, ResourceWorkbenchRouteParam>>,
    /// 页面内跳转链接(如 Index → Mapping)。
    #[serde(default)]
    pub links: Vec<ResourceWorkbenchLink>,
}

/// terminal 页面要启动的终端进程声明。
///
/// 宿主按此启动一个可嵌入的原生终端组件;`args` 支持
/// `{{route.xxx}}` 与 `{{xxx}}` 两种占位符,由宿主按当前路由插值。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchTerminal {
    /// 可执行程序(如 `docker`)。
    pub command: String,
    /// 参数列表。
    #[serde(default)]
    pub args: Vec<String>,
    /// 追加的环境变量。
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// 工作目录。
    #[serde(default, rename = "workingDir")]
    pub working_dir: Option<String>,
}

/// tab 条中的一项。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchTab {
    pub id: String,
    pub title: String,
    #[serde(rename = "pageId")]
    pub page_id: String,
    #[serde(default)]
    pub route: BTreeMap<String, ResourceWorkbenchBinding>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchRouteParam {
    #[serde(rename = "type")]
    pub value_type: String,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchLink {
    pub title: String,
    #[serde(rename = "pageId")]
    pub page_id: String,
    pub route: BTreeMap<String, ResourceWorkbenchBinding>,
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
    Terminal,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchRenderer {
    pub kind: ResourceWorkbenchRendererKind,
    #[serde(default, rename = "viewId")]
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
    /// 行点击跳转声明:按 selection 绑定构造目标页 route。
    #[serde(default)]
    pub open: Option<ResourceWorkbenchOpen>,
    /// 行内操作按钮:点击后以该行为 selection 执行命名操作。
    #[serde(default)]
    pub actions: Vec<ResourceWorkbenchRowAction>,
}

/// collection 行内操作:operation 的 params 通常以 selection 来源绑定行字段。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchRowAction {
    pub id: String,
    pub label: String,
    pub operation: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchOpen {
    #[serde(rename = "pageId")]
    pub page_id: String,
    pub route: BTreeMap<String, ResourceWorkbenchBinding>,
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
    /// 可选渲染样式:`badge` 按值渲染状态徽章(如容器 state)。
    #[serde(default)]
    pub style: Option<String>,
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
