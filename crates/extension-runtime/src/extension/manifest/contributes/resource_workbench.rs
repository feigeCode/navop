use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 资源工作台贡献(schemaVersion 2)。
///
/// v2 用单一 `layout` 声明取代 v1 的 `navigation`/`tree`/`statusBar`/
/// `pages[].tabs`;区域内容源要么是 native 声明式模板,要么是 JS Shell 视图,
/// 含整个工作台主体(`layout.renderer`)。规范见
/// `docs/superpowers/specs/2026-09-14-resource-workbench-layout-v2.md`。
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
    /// 工作台布局声明;缺省等价 `{left: list, center: pages}`。
    #[serde(default)]
    pub layout: Option<ResourceWorkbenchLayout>,
    pub pages: Vec<ResourceWorkbenchPage>,
}

/// 工作台布局:root Shell 覆盖与四个区域槽。
///
/// `renderer` 存在时不得再声明任何区域(注册期校验互斥);
/// 宿主永不可覆盖部分(连接 Header、session 所有权、权限、任务入口、关闭守卫)
/// 不在 layout 表达范围内。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchLayout {
    /// 整个工作台主体交给 JS Shell 视图;无 native fallback。
    #[serde(default)]
    pub renderer: Option<ResourceWorkbenchRenderer>,
    #[serde(default)]
    pub left: Option<ResourceWorkbenchLeftRegion>,
    #[serde(default)]
    pub center: Option<ResourceWorkbenchCenterRegion>,
    #[serde(default)]
    pub right: Option<ResourceWorkbenchSideRegion>,
    #[serde(default)]
    pub bottom: Option<ResourceWorkbenchBottomRegion>,
}

/// 左侧导航区域(list/tree/shell/none)。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchLeftRegion {
    /// 像素宽;缺省 208。
    #[serde(default)]
    pub width: Option<u32>,
    /// 预留;首版不做拖拽调宽。
    #[serde(default)]
    pub resizable: bool,
    pub source: ResourceWorkbenchNavSource,
}

/// 右侧区域(首版仅 shell/none;DTO 开放 native 演进空间)。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchSideRegion {
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub resizable: bool,
    pub source: ResourceWorkbenchSideSource,
}

/// 中央区域(pages/shell)。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchCenterRegion {
    pub source: ResourceWorkbenchCenterSource,
}

/// 底部区域(status/shell/none)。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchBottomRegion {
    /// 像素高;缺省 28。
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub resizable: bool,
    pub source: ResourceWorkbenchBottomSource,
}

/// left/right 区域内容源。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum ResourceWorkbenchNavSource {
    /// 扁平页面列表(v1 `navigation` 的等价物):显式声明条目与顺序。
    List {
        items: Vec<ResourceWorkbenchNavEntry>,
    },
    /// 树:静态根 + 声明式 lazy children(v1 `tree` 的转正)。
    Tree {
        roots: Vec<ResourceWorkbenchTreeRoot>,
    },
    Shell(ResourceWorkbenchShellSource),
    None,
}

/// 列表导航条目:引用 pages 中的页面 id。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchNavEntry {
    #[serde(rename = "pageId")]
    pub page_id: String,
}

/// right 区域内容源。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum ResourceWorkbenchSideSource {
    Shell(ResourceWorkbenchShellSource),
    None,
}

/// center 区域内容源。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum ResourceWorkbenchCenterSource {
    /// 声明式页面区 + 共享 tab 组。
    Pages {
        #[serde(default, rename = "tabGroups")]
        tab_groups: Vec<ResourceWorkbenchTabGroup>,
    },
    Shell(ResourceWorkbenchShellSource),
}

/// bottom 区域内容源。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum ResourceWorkbenchBottomSource {
    /// 单一操作驱动的 items 状态栏。
    Status {
        operation: String,
        items: Vec<ResourceWorkbenchStatusItem>,
    },
    Shell(ResourceWorkbenchShellSource),
    None,
}

/// Shell 视图源:挂载同扩展 shellViews 中的嵌入视图。
///
/// 区域级 fallback 只能 `none`(显示占位空态);root 级(`layout.renderer`)
/// 无 fallback,viewId 不可用时渲染错误态说明。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchShellSource {
    #[serde(rename = "viewId")]
    pub view_id: String,
    /// 仅接受 `"none"`;缺省即 none。
    #[serde(default)]
    pub fallback: Option<String>,
}

/// 树根节点:静态声明,lazy children 展开时按 operation 拉取。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchTreeRoot {
    pub id: String,
    pub title: String,
    #[serde(rename = "pageId")]
    pub page_id: String,
    #[serde(default)]
    pub children: Option<ResourceWorkbenchTreeChildren>,
}

/// 子节点拉取声明,可递归嵌套形成多级 lazy 树。
///
/// 每一层的 `operation` 参数可用 `parent` 绑定源引用父节点行数据
/// (如 K8s namespace → pods 传 `{"source": "parent", "path": "/name"}`)。
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
    /// 子节点点击跳转声明;缺省回退根 pageId + 行数据作 route。
    #[serde(default)]
    pub open: Option<ResourceWorkbenchOpen>,
    /// 下一级子节点声明;缺省为叶子。
    #[serde(default)]
    pub children: Option<Box<ResourceWorkbenchTreeChildren>>,
}

/// 共享 tab 组:组内页面经 `pages[].tabGroupId` 引用,
/// 取代 v1 每页复制完整 tabs 列表的做法。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchTabGroup {
    pub id: String,
    #[serde(default)]
    pub tabs: Vec<ResourceWorkbenchTab>,
}

/// 状态栏条目:JSON Pointer 取值 + 声明式格式化。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchStatusItem {
    pub path: String,
    /// `pair` 格式要求同时提供 `otherPath`。
    #[serde(default, rename = "otherPath")]
    pub other_path: Option<String>,
    pub label: String,
    #[serde(default)]
    pub format: ResourceWorkbenchStatusFormat,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ResourceWorkbenchStatusFormat {
    #[default]
    Raw,
    Number,
    Bytes,
    Percent,
    Version,
    /// 绿/红状态点 + label。
    BooleanUp,
    /// `x/y`,需配 `otherPath`。
    Pair,
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
    /// 树 lazy 展开时的父节点行数据;非树上下文下为空。
    Parent,
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
pub struct ResourceWorkbenchPage {
    pub id: String,
    pub title: String,
    pub template: ResourceWorkbenchTemplate,
    pub renderer: ResourceWorkbenchRenderer,
    /// 所属 tab 组(layout.center.tabGroups 中声明);缺省无 strip。
    #[serde(default, rename = "tabGroupId")]
    pub tab_group_id: Option<String>,
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
    pub kind: ResourceWorkbenchPaginationKind,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ResourceWorkbenchPaginationKind {
    None,
    /// page/limit 页码式。
    Page,
    /// nextCursor 游标式。
    Cursor,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchColumn {
    pub id: String,
    pub title: String,
    pub path: String,
    #[serde(rename = "type")]
    pub value_type: String,
    #[serde(default)]
    pub style: ResourceWorkbenchColumnStyle,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ResourceWorkbenchColumnStyle {
    #[default]
    Plain,
    /// 状态徽章(如容器 state)。
    Badge,
    /// 等宽(id、镜像名、路径)。
    Mono,
    /// 次要文本(时间、描述)。
    Muted,
}

/// query 页面的输入字段。`select` 需配 `options`,`checkbox` 提交 "true"/"false"。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchInput {
    pub id: String,
    #[serde(rename = "type")]
    pub value_type: String,
    #[serde(default)]
    pub editor: ResourceWorkbenchInputEditor,
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub required: bool,
    /// 显示名;缺省用 id。
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub placeholder: Option<String>,
    /// 字段下方的提示文案。
    #[serde(default)]
    pub description: Option<String>,
    /// `select` 编辑器的候选项。
    #[serde(default)]
    pub options: Vec<ResourceWorkbenchInputOption>,
    /// `textarea` 行数;缺省 4。
    #[serde(default)]
    pub rows: Option<usize>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ResourceWorkbenchInputEditor {
    #[default]
    Text,
    Textarea,
    Password,
    Number,
    Select,
    Checkbox,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchInputOption {
    pub value: String,
    pub label: String,
}
