use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 资源工作台贡献(schemaVersion 3)。
///
/// v3 用 `pages[].stack`(可组合原语的有序 v-stack)取代 v2 的闭合
/// `template` 枚举:每个页面由若干原语(table/form/viewer/stream/tasks/terminal)
/// 纵向堆叠而成,新资源类型组合原语即可,无需宿主改动。数据面:页面持有一个
/// `load` 操作产出结果,`form.submit` 可覆盖该结果,table/viewer/stream 消费结果。
/// 布局仍由 `layout`(left/center/right/bottom 四区域 + root Shell)声明。
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
    /// 扁平页面列表:显式声明条目与顺序。
    List {
        items: Vec<ResourceWorkbenchNavEntry>,
    },
    /// 树:静态根 + 声明式 lazy children。
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

/// 子节点来源。
///
/// `remote` 是原有形态:按 `operation` lazy 拉取集合。`static` 是增补形态:
/// 由宿主直接渲染的**功能子节点**(如「索引 → Mapping/设置/分片」),不发任何
/// provider 请求。两者共用同一个字段名,旧 manifest 不写 `kind` 即等价 `remote`,
/// 因此宿主向后兼容;反过来,旧宿主因为 `deny_unknown_fields` 会拒绝 `kind`/`items`,
/// 于是"新声明装进旧宿主"必定在解析期失败,而不是静默少渲染一层。
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ResourceWorkbenchTreeChildrenKind {
    #[default]
    Remote,
    Static,
}

/// 子节点拉取声明,可递归嵌套形成多级 lazy 树。
///
/// 每一层的 `operation` 参数可用 `parent` 绑定源引用父节点行数据
/// (如 K8s namespace → pods 传 `{"source": "parent", "path": "/name"}`)。
///
/// 两种形态互斥:`remote` 需要 `operation`/`itemsPath`/`labelPath`,`static`
/// 需要 `items`。注册期按 `kind` 分别校验,写错形态在安装时报错,不会到渲染期
/// 才表现为"点开是空的"。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchTreeChildren {
    /// 子节点来源;缺省 `remote`。
    #[serde(default, skip_serializing_if = "is_remote_children")]
    pub kind: ResourceWorkbenchTreeChildrenKind,
    // ---- remote 形态 ----
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    #[serde(default, rename = "itemsPath", skip_serializing_if = "Option::is_none")]
    pub items_path: Option<String>,
    #[serde(default, rename = "keyPaths", skip_serializing_if = "Vec::is_empty")]
    pub key_paths: Vec<String>,
    #[serde(default, rename = "labelPath", skip_serializing_if = "Option::is_none")]
    pub label_path: Option<String>,
    /// 点击该层任意子行的跳转声明;缺省回退根 pageId + 行数据作 route。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<ResourceWorkbenchOpen>,
    /// 远程形态的下一级子节点声明;缺省为叶子。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Box<ResourceWorkbenchTreeChildren>>,
    // ---- static 形态 ----
    /// 静态功能子节点。零请求展开,顺序即渲染顺序。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<ResourceWorkbenchStaticTreeItem>,
}

fn is_remote_children(kind: &ResourceWorkbenchTreeChildrenKind) -> bool {
    *kind == ResourceWorkbenchTreeChildrenKind::Remote
}

impl ResourceWorkbenchTreeChildren {
    /// 静态集合的子项;远程形态返回 `None`。
    ///
    /// 只看 `kind` 而不看 `items` 是否为空:形态由 `kind` 决定,`items` 存在
    /// 但 `kind` 不是 static 是非法声明(注册期拒绝),读取侧不该悄悄兼容它。
    pub fn static_items(&self) -> Option<&[ResourceWorkbenchStaticTreeItem]> {
        (self.kind == ResourceWorkbenchTreeChildrenKind::Static).then(|| self.items.as_slice())
    }

    /// 该层是否按 provider 请求 lazy 拉取(远程形态)。
    pub fn is_remote(&self) -> bool {
        self.kind == ResourceWorkbenchTreeChildrenKind::Remote
    }
}

/// 静态树项:宿主直接渲染的功能子节点,不是 provider 资源。
///
/// 身份只来自 `id`(同一层内唯一),不依赖 provider 行数据;点击目标由 `open`
/// 给定,其 `parent` 绑定源指向**直接父节点**的领域行 —— 例如「索引 → Mapping」
/// 的 parent 是索引行 `{name, ...}`,于是 `{"source":"parent","path":"/name"}`
/// 能拿到索引名。需要继承更远的祖先上下文时不要隐式穿透,应显式扩展契约。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchStaticTreeItem {
    pub id: String,
    pub title: String,
    /// 点击目标。静态项必须能导航:声明了却点不到目标页的节点在树里就是死节点,
    /// 所以这里不是 `Option`,注册期也不接受缺失。
    pub open: ResourceWorkbenchOpen,
    /// 该静态项自身的子节点声明(可再是 static 或 remote)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Box<ResourceWorkbenchTreeChildren>>,
}

/// 共享 tab 组:组内页面经 `pages[].tabGroupId` 引用。
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
    /// 连接配置字段(来自连接的保存配置)。
    Connection,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ResourceWorkbenchValueType {
    String,
    Number,
    Boolean,
    Json,
}

/// 页面:声明式渲染的最小单位,内容为原语 v-stack。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchPage {
    pub id: String,
    pub title: String,
    pub renderer: ResourceWorkbenchRenderer,
    /// 所属 tab 组(layout.center.tabGroups 中声明);缺省无 strip。
    #[serde(default, rename = "tabGroupId")]
    pub tab_group_id: Option<String>,
    /// 页面数据操作:打开/刷新/翻页时执行,结果供 table/viewer/stream 消费。
    #[serde(default)]
    pub load: Option<ResourceWorkbenchAction>,
    /// 页面内跳转链接(如 Index → Mapping)。
    #[serde(default)]
    pub links: Vec<ResourceWorkbenchLink>,
    /// detail/query 页面的路由参数声明(如 {"name": {"type": "string", "required": true}})。
    #[serde(default)]
    pub route: Option<BTreeMap<String, ResourceWorkbenchRouteParam>>,
    /// 页面内容原语。当前宿主渲染器一页只呈现一个原语,声明多个会被安装
    /// 校验拒绝(需要组合时拆成多个页面)。字段名保留 `stack` 以兼容线格式,
    /// 待真正的堆叠渲染落地后再放宽该约束。
    pub stack: Vec<ResourceWorkbenchPrimitive>,
}

/// 页面内容原语:页面 stack 中的一项。当前一页只允许一个原语。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum ResourceWorkbenchPrimitive {
    /// 表格:按 itemsPath 从页面结果投影行,声明列/分页/行操作。
    Table(ResourceWorkbenchTable),
    /// 表单:输入字段 + 提交操作;提交结果覆盖页面结果,由同页 table/viewer 呈现。
    Form(ResourceWorkbenchForm),
    /// 只读视图:json / text。
    Viewer(ResourceWorkbenchViewer),
    /// 事件流:job 操作返回 EventStream,持续追加事件。
    Stream,
    /// 宿主任务列表。
    Tasks,
    /// 终端:本地进程或 runtime operation 驱动。
    Terminal(ResourceWorkbenchTerminal),
}

/// table 原语:页面结果的表格投影。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchTable {
    #[serde(rename = "itemsPath")]
    pub items_path: String,
    #[serde(rename = "keyPaths")]
    pub key_paths: Vec<String>,
    #[serde(default)]
    pub pagination: ResourceWorkbenchPagination,
    pub columns: Vec<ResourceWorkbenchColumn>,
    /// 行点击跳转声明:按 selection 绑定构造目标页 route。
    #[serde(default)]
    pub open: Option<ResourceWorkbenchOpen>,
    /// 行内操作按钮:点击后以该行为 selection 执行命名操作。
    #[serde(default)]
    pub actions: Vec<ResourceWorkbenchRowAction>,
}

/// form 原语:输入字段 + 提交操作。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchForm {
    /// 提交操作:结果覆盖页面结果。
    pub submit: ResourceWorkbenchAction,
    #[serde(default)]
    pub inputs: Vec<ResourceWorkbenchInput>,
}

/// viewer 原语:页面结果的只读呈现。
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ResourceWorkbenchViewerFormat {
    #[default]
    Json,
    Text,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchViewer {
    #[serde(default)]
    pub format: ResourceWorkbenchViewerFormat,
}

/// 终端原语。
///
/// 目前只有 `command` 模式可用:由宿主启动可嵌入的本地终端进程。
/// `operation`(runtime pty 驱动)是**预留声明**,扩展协议还没有 provider
/// PTY 流式通道,宿主侧必定返回 not supported —— 因此注册期直接拒绝声明了
/// `operation` 的 manifest,让失败发生在安装时而不是用户点开时。
/// 字段仍保留在 schema 里,是为了给出明确的拒绝理由,而不是一个 serde 未知字段错误。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchTerminal {
    /// 本地进程可执行程序(如 `docker`);与 `operation` 二选一。
    #[serde(default)]
    pub command: Option<String>,
    /// 本地进程参数列表;`{{route.xxx}}` 与 `{{xxx}}` 由宿主按当前路由插值。
    #[serde(default)]
    pub args: Vec<String>,
    /// 追加的环境变量。
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// 工作目录。
    #[serde(default, rename = "workingDir")]
    pub working_dir: Option<String>,
    /// runtime pty operation;与 `command` 二选一。**尚未实现,注册期会拒绝**
    /// (见类型文档)。
    #[serde(default)]
    pub operation: Option<ResourceWorkbenchTerminalOperation>,
}

/// runtime 侧 pty 会话声明。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchTerminalOperation {
    pub operation: String,
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

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceWorkbenchPagination {
    #[serde(default)]
    pub kind: ResourceWorkbenchPaginationKind,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ResourceWorkbenchPaginationKind {
    #[default]
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
    #[serde(rename = "type", default)]
    pub value_type: ResourceWorkbenchColumnType,
    #[serde(default)]
    pub style: ResourceWorkbenchColumnStyle,
}

/// 列值类型:决定表格单元格的渲染与格式化。
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ResourceWorkbenchColumnType {
    #[default]
    Display,
    String,
    Number,
    Boolean,
    Json,
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
    #[serde(rename = "type", default)]
    pub value_type: ResourceWorkbenchInputType,
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

/// 输入值类型:提交时用于把文本强制转换为对应 JSON 类型。
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ResourceWorkbenchInputType {
    #[default]
    String,
    Number,
    Boolean,
    Json,
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
