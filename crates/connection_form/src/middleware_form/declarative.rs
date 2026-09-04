//! 声明式表单配置结构
//!
//! API 与 `db_view` 的 `TabGroup`/`FormField` 同形,便于按数据库表单的
//! 心智模型编写中间件表单配置。结构体在此独立定义,避免 connection_form
//! 反向依赖 `db` crate(`FormField.visible_when` 在 db 侧绑定
//! `plugin_manifest::FormVisibilityRule`)。

use rust_i18n::t;

use crate::ssh_auth::{SshAuthOption, normalize_ssh_auth_type};

/// 表单字段控件类型
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormFieldType {
    Text,
    Number,
    Password,
    TextArea,
    Select,
    Checkbox,
    FilePath,
}

/// 字段可见性规则:另一字段取特定值时才显示(仅等值匹配)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormVisibilityRule {
    /// 依赖的字段名
    pub when_field: String,
    /// 期望取值;`None` 表示字段缺失或为空
    pub equals: Option<String>,
}

impl FormVisibilityRule {
    pub fn field_equals(field: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            when_field: field.into(),
            equals: Some(value.into()),
        }
    }

    pub fn field_missing(field: impl Into<String>) -> Self {
        Self {
            when_field: field.into(),
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

/// 声明式表单字段
#[derive(Clone, Debug)]
pub struct FormField {
    pub name: String,
    pub label: String,
    pub placeholder: String,
    pub field_type: FormFieldType,
    pub rows: usize,
    pub required: bool,
    pub default_value: String,
    pub options: Vec<(String, String)>,
    pub visible_when: Vec<FormVisibilityRule>,
}

impl FormField {
    pub fn new(
        name: impl Into<String>,
        label: impl Into<String>,
        field_type: FormFieldType,
    ) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            placeholder: String::new(),
            field_type,
            rows: 5,
            required: true,
            default_value: String::new(),
            options: Vec::new(),
            visible_when: Vec::new(),
        }
    }

    pub fn optional(mut self) -> Self {
        self.required = false;
        self
    }

    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn default(mut self, value: impl Into<String>) -> Self {
        self.default_value = value.into();
        self
    }

    pub fn options(mut self, options: Vec<(String, String)>) -> Self {
        self.options = options;
        self
    }

    pub fn rows(mut self, rows: usize) -> Self {
        self.rows = rows;
        self
    }

    pub fn visible_when(mut self, rule: FormVisibilityRule) -> Self {
        self.visible_when.push(rule);
        self
    }
}

/// 标签页分组
#[derive(Clone, Debug)]
pub struct TabGroup {
    pub name: String,
    pub label: String,
    pub fields: Vec<FormField>,
}

impl TabGroup {
    pub fn new(name: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            fields: Vec::new(),
        }
    }

    pub fn field(mut self, field: FormField) -> Self {
        self.fields.push(field);
        self
    }

    pub fn fields(mut self, fields: Vec<FormField>) -> Self {
        self.fields = fields;
        self
    }
}

/// 共享 SSH 隧道标签页(声明式字段,与数据库表单的 SSH 页一致)
///
/// 约定字段名以 `ssh_` 开头;引擎在 tab name == "ssh" 时使用自定义渲染
/// (启用开关 + 引用已有 SSH 连接下拉 + 按认证类型显示字段)。
pub fn ssh_tab_group() -> TabGroup {
    TabGroup::new("ssh", "SSH").fields(vec![
        FormField::new(
            "ssh_tunnel_enabled",
            t!("ConnectionForm.ssh_tunnel_enabled"),
            FormFieldType::Checkbox,
        )
        .optional()
        .default("false"),
        FormField::new(
            "ssh_connection_id",
            t!("ConnectionForm.ssh_connection_id"),
            FormFieldType::Text,
        )
        .optional(),
        FormField::new(
            "ssh_host",
            t!("ConnectionForm.ssh_host"),
            FormFieldType::Text,
        )
        .optional()
        .placeholder("jump.example.com"),
        FormField::new(
            "ssh_port",
            t!("ConnectionForm.ssh_port"),
            FormFieldType::Number,
        )
        .optional()
        .default("22")
        .placeholder("22"),
        FormField::new(
            "ssh_username",
            t!("ConnectionForm.ssh_username"),
            FormFieldType::Text,
        )
        .optional()
        .placeholder("root"),
        FormField::new(
            "ssh_auth_type",
            t!("ConnectionForm.ssh_auth_type"),
            FormFieldType::Select,
        )
        .optional()
        .default("password")
        .options(
            SshAuthOption::ALL
                .iter()
                .map(|option| (option.value().to_string(), option.label()))
                .collect(),
        ),
        FormField::new(
            "ssh_password",
            t!("ConnectionForm.ssh_password"),
            FormFieldType::Password,
        )
        .optional()
        .placeholder(t!("ConnectionForm.enter_password")),
        FormField::new(
            "ssh_private_key_path",
            t!("ConnectionForm.ssh_private_key_path"),
            FormFieldType::Text,
        )
        .optional()
        .placeholder("~/.ssh/id_rsa"),
        FormField::new(
            "ssh_private_key_content",
            t!("ConnectionForm.ssh_private_key_content"),
            FormFieldType::TextArea,
        )
        .rows(5)
        .optional()
        .placeholder(t!("ConnectionForm.ssh_private_key_content_placeholder")),
        FormField::new(
            "ssh_private_key_passphrase",
            t!("ConnectionForm.ssh_private_key_passphrase"),
            FormFieldType::Password,
        )
        .optional()
        .placeholder(t!("ConnectionForm.enter_passphrase")),
        FormField::new(
            "ssh_target_host",
            t!("ConnectionForm.ssh_target_host"),
            FormFieldType::Text,
        )
        .optional()
        .placeholder("127.0.0.1"),
        FormField::new(
            "ssh_target_port",
            t!("ConnectionForm.ssh_target_port"),
            FormFieldType::Number,
        )
        .optional()
        .placeholder("1883"),
    ])
}

/// 共享备注标签页
pub fn notes_tab_group() -> TabGroup {
    TabGroup::new("notes", t!("MiddlewareForm.tab_notes")).fields(vec![
        FormField::new(
            "remark",
            t!("MiddlewareForm.remark"),
            FormFieldType::TextArea,
        )
        .rows(14)
        .optional()
        .placeholder(t!("MiddlewareForm.remark_placeholder")),
    ])
}

/// 归一化 SSH 认证类型(未知值回退密码认证)
pub(crate) fn normalized_ssh_auth_type_or_default(auth_type: &str) -> &str {
    normalize_ssh_auth_type(auth_type)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_rule_matches_equality_and_missing() {
        assert!(FormVisibilityRule::field_equals("use_tls", "true").matches(Some("true")));
        assert!(!FormVisibilityRule::field_equals("use_tls", "true").matches(Some("false")));
        assert!(!FormVisibilityRule::field_equals("use_tls", "true").matches(None));

        assert!(FormVisibilityRule::field_missing("use_tls").matches(None));
        assert!(FormVisibilityRule::field_missing("use_tls").matches(Some("  ")));
        assert!(!FormVisibilityRule::field_missing("use_tls").matches(Some("true")));
    }

    #[test]
    fn ssh_tab_group_uses_conventional_field_names() {
        let group = ssh_tab_group();
        let names: Vec<&str> = group.fields.iter().map(|f| f.name.as_str()).collect();

        assert_eq!(group.name, "ssh");
        assert!(names.contains(&"ssh_tunnel_enabled"));
        assert!(names.contains(&"ssh_connection_id"));
        assert!(names.contains(&"ssh_target_port"));
        // 全部字段均为可选,由引擎按启用状态校验必填
        assert!(group.fields.iter().all(|field| !field.required));
    }
}
