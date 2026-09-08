//! 中间件表单共享的 SSH 隧道 / 备注标签页配置
//!
//! 字段结构统一使用 `connection_form::declarative` 的 `DeclarativeFormField` /
//! `DeclarativeFormTab`;中间件表单直接声明声明式配置,不再经过过渡层翻译。

use rust_i18n::t;

use crate::declarative::{DeclarativeFieldType, DeclarativeFormField, DeclarativeFormTab};
use crate::ssh_auth::{SshAuthOption, normalize_ssh_auth_type};

/// 共享 SSH 隧道标签页(声明式字段,与数据库表单的 SSH 页一致)
///
/// 约定字段名以 `ssh_` 开头;引擎在 tab id == "ssh" 时使用自定义渲染
/// (启用开关 + 引用已有 SSH 连接下拉 + 按认证类型显示字段)。
pub fn ssh_tab_group() -> DeclarativeFormTab {
    DeclarativeFormTab::new("ssh", "SSH").fields(vec![
        DeclarativeFormField::new(
            "ssh_tunnel_enabled",
            t!("ConnectionForm.ssh_tunnel_enabled"),
            DeclarativeFieldType::Checkbox,
        )
        .optional()
        .default("false"),
        DeclarativeFormField::new(
            "ssh_connection_id",
            t!("ConnectionForm.ssh_connection_id"),
            DeclarativeFieldType::Text,
        )
        .optional(),
        DeclarativeFormField::new(
            "ssh_host",
            t!("ConnectionForm.ssh_host"),
            DeclarativeFieldType::Text,
        )
        .optional()
        .placeholder("jump.example.com"),
        DeclarativeFormField::new(
            "ssh_port",
            t!("ConnectionForm.ssh_port"),
            DeclarativeFieldType::Number,
        )
        .optional()
        .default("22")
        .placeholder("22"),
        DeclarativeFormField::new(
            "ssh_username",
            t!("ConnectionForm.ssh_username"),
            DeclarativeFieldType::Text,
        )
        .optional()
        .placeholder("root"),
        DeclarativeFormField::new(
            "ssh_auth_type",
            t!("ConnectionForm.ssh_auth_type"),
            DeclarativeFieldType::Select,
        )
        .optional()
        .default("password")
        .options(
            SshAuthOption::ALL
                .iter()
                .map(|option| (option.value().to_string(), option.label())),
        ),
        DeclarativeFormField::new(
            "ssh_password",
            t!("ConnectionForm.ssh_password"),
            DeclarativeFieldType::Password,
        )
        .optional()
        .placeholder(t!("ConnectionForm.enter_password")),
        DeclarativeFormField::new(
            "ssh_private_key_path",
            t!("ConnectionForm.ssh_private_key_path"),
            DeclarativeFieldType::Text,
        )
        .optional()
        .placeholder("~/.ssh/id_rsa"),
        DeclarativeFormField::new(
            "ssh_private_key_content",
            t!("ConnectionForm.ssh_private_key_content"),
            DeclarativeFieldType::TextArea,
        )
        .rows(5)
        .optional()
        .placeholder(t!("ConnectionForm.ssh_private_key_content_placeholder")),
        DeclarativeFormField::new(
            "ssh_private_key_passphrase",
            t!("ConnectionForm.ssh_private_key_passphrase"),
            DeclarativeFieldType::Password,
        )
        .optional()
        .placeholder(t!("ConnectionForm.enter_passphrase")),
        DeclarativeFormField::new(
            "ssh_target_host",
            t!("ConnectionForm.ssh_target_host"),
            DeclarativeFieldType::Text,
        )
        .optional()
        .placeholder("127.0.0.1"),
        DeclarativeFormField::new(
            "ssh_target_port",
            t!("ConnectionForm.ssh_target_port"),
            DeclarativeFieldType::Number,
        )
        .optional()
        .placeholder("1883"),
    ])
}

/// 共享备注标签页
pub fn notes_tab_group() -> DeclarativeFormTab {
    DeclarativeFormTab::new("notes", t!("ConnectionForm.tab_notes")).fields(vec![
        DeclarativeFormField::new(
            "remark",
            t!("ConnectionForm.remark"),
            DeclarativeFieldType::TextArea,
        )
        .rows(14)
        .optional()
        .placeholder(t!("ConnectionForm.remark_placeholder")),
    ])
}

/// 归一化 SSH 认证类型(未知值回退密码认证)
pub(crate) fn normalized_ssh_auth_type_or_default(auth_type: &str) -> &str {
    normalize_ssh_auth_type(auth_type)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declarative::DeclarativeVisibilityRule;

    #[test]
    fn ssh_tab_group_uses_conventional_field_names() {
        let group = ssh_tab_group();
        let names: Vec<&str> = group.fields.iter().map(|f| f.id.as_str()).collect();

        assert_eq!(group.id, "ssh");
        assert!(names.contains(&"ssh_tunnel_enabled"));
        assert!(names.contains(&"ssh_connection_id"));
        assert!(names.contains(&"ssh_target_port"));
        // 全部字段均为可选,由引擎按启用状态校验必填
        assert!(group.fields.iter().all(|field| !field.required));
    }

    #[test]
    fn ssh_auth_type_declares_select_options() {
        let group = ssh_tab_group();
        let auth_type = group
            .fields
            .iter()
            .find(|field| field.id == "ssh_auth_type")
            .expect("ssh_auth_type 字段应存在");
        assert_eq!(DeclarativeFieldType::Select, auth_type.field_type);
        assert!(!auth_type.options.is_empty());
    }

    #[test]
    fn visibility_rule_matches_equality_and_missing() {
        assert!(DeclarativeVisibilityRule::field_equals("use_tls", "true").matches(Some("true")));
        assert!(!DeclarativeVisibilityRule::field_equals("use_tls", "true").matches(Some("false")));
        assert!(!DeclarativeVisibilityRule::field_equals("use_tls", "true").matches(None));

        assert!(DeclarativeVisibilityRule::field_missing("use_tls").matches(None));
        assert!(DeclarativeVisibilityRule::field_missing("use_tls").matches(Some("  ")));
        assert!(!DeclarativeVisibilityRule::field_missing("use_tls").matches(Some("true")));
    }
}
