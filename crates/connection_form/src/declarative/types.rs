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
    /// TextArea 行数;其他字段类型忽略。
    pub rows: usize,
}

/// TextArea 缺省行数,与 `middleware_form::FormField` 对齐。
pub const DECLARATIVE_TEXTAREA_DEFAULT_ROWS: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeclarativeFieldType {
    Text,
    Number,
    Password,
    TextArea,
    Select,
    Checkbox,
    /// 文件路径选择(当前渲染为文本输入,后续接浏览按钮)
    FilePath,
    /// 认证复合组件:钥匙串引用下拉 + 手动用户名/密码;
    /// 选中钥匙串引用后隐藏手动 username/password。
    /// 收集产物:`config[id] = {"credential_reference": {...}}` 或 `{"username": "..."}`,
    /// 手动密码进入 secrets,键为 `{id}.password`。
    Auth,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeclarativeSelectOption {
    pub value: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeclarativeVisibilityRule {
    pub field: String,
    /// `Some(v)`:字段值等于 `v` 时可见;`None`:字段缺失或为空时可见。
    pub equals: Option<String>,
}

impl DeclarativeVisibilityRule {
    pub fn field_equals(field: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            equals: Some(value.into()),
        }
    }

    pub fn field_missing(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            equals: None,
        }
    }
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
