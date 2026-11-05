use std::collections::BTreeMap;

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
    /// 提供者运行时 ID。声明了 `terminal` 的连接不需要(也不允许)提供者进程。
    #[serde(default, rename = "runtimeId")]
    pub runtime_id: String,
    /// 传给提供者 `resource/open` 的资源类型。声明了 `terminal` 时不适用。
    #[serde(default, rename = "resourceType")]
    pub resource_type: String,
    /// 打开这条连接时 app 的初始路由（如 `containers`）。
    #[serde(default)]
    pub route: Option<String>,
    /// 终端型连接:打开时由宿主直接以本地进程启动一个终端标签,不需要提供者进程。
    #[serde(default)]
    pub terminal: Option<ResourceConnectionTerminal>,
    #[serde(default)]
    pub form: ResourceConnectionForm,
}

/// 终端型连接的启动声明。
///
/// `args` / `env` / `title` 中的 `{{字段ID}}` 由宿主按连接表单里保存的值替换。
/// `command` 固定写死、不允许占位符:要执行什么程序只由扩展作者决定,
/// 连接数据(可能来自云同步 / 团队共享)无法改变它。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceConnectionTerminal {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// 标签标题模板;缺省使用连接名称。
    #[serde(default)]
    pub title: Option<String>,
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
    /// TextArea 行数;缺省 5。
    #[serde(default)]
    pub rows: Option<usize>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum ResourceConnectionFieldType {
    Text,
    Number,
    Password,
    TextArea,
    Select,
    Checkbox,
    /// 本地文件路径(文本输入 + 浏览按钮)。
    FilePath,
    /// 认证复合组件:用户名 + 密码 + 钥匙串引用。
    /// 选中钥匙串引用后隐藏手动用户名/密码。
    /// 收集产物:`config[id] = {"credential_reference": {...}}` 或 `{"username": "..."}`,
    /// 手动密码进入 secrets,键为 `{id}.password`。无需 `secret: true`。
    Auth,
    /// 连接图标选择器(自动 / 内置图标 / 本地图片),与 SSH 连接一致。
    /// 收集产物:`config[id] = {"icon": "<内置图标 ID>"}` 或 `{"file": "<本地图片路径>"}`,
    /// 保持「自动」时不写入。每个连接至多声明一个;不能 `secret`、不能有 `defaultValue` / `options`。
    /// 宿主在首页 / 列表中据此渲染该连接的图标(未选择时回退到 `icon` 声明的扩展图标)。
    Icon,
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
