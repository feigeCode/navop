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

impl DeclarativeFormTab {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            fields: Vec::new(),
        }
    }

    pub fn field(mut self, field: DeclarativeFormField) -> Self {
        self.fields.push(field);
        self
    }

    pub fn fields(mut self, fields: Vec<DeclarativeFormField>) -> Self {
        self.fields = fields;
        self
    }
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

impl DeclarativeFormField {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        field_type: DeclarativeFieldType,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            field_type,
            required: true,
            default_value: None,
            placeholder: None,
            secret: false,
            options: Vec::new(),
            visible_when: Vec::new(),
            rows: DECLARATIVE_TEXTAREA_DEFAULT_ROWS,
        }
    }

    pub fn optional(mut self) -> Self {
        self.required = false;
        self
    }

    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    pub fn default(mut self, value: impl Into<String>) -> Self {
        self.default_value = Some(value.into());
        self
    }

    pub fn options<I, V, L>(mut self, options: I) -> Self
    where
        I: IntoIterator<Item = (V, L)>,
        V: Into<String>,
        L: Into<String>,
    {
        self.options = options
            .into_iter()
            .map(|(value, label)| DeclarativeSelectOption {
                value: value.into(),
                label: label.into(),
            })
            .collect();
        self
    }

    pub fn rows(mut self, rows: usize) -> Self {
        self.rows = rows;
        self
    }

    pub fn visible_when(mut self, rule: DeclarativeVisibilityRule) -> Self {
        self.visible_when.push(rule);
        self
    }
}

/// TextArea 缺省行数,与声明式引擎默认一致。
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

    pub fn matches(&self, value: Option<&str>) -> bool {
        match &self.equals {
            Some(expected) => value == Some(expected.as_str()),
            None => value.is_none_or(|value| value.trim().is_empty()),
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
