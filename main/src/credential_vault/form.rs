use std::collections::BTreeSet;

use gpui::{App, AppContext, Context, Entity, FocusHandle, Focusable, SharedString, Window};
use gpui_component::input::InputState;
use one_core::storage::{CredentialEntry, SshAccountExpect, TerminalExpectSend};
use regex::Regex;

pub(super) const CREDENTIAL_KIND_OPTIONS: [(&str, &str); 8] = [
    ("通用", "可用于任意支持凭据引用的连接"),
    ("SSH", "SSH、SFTP 和终端连接"),
    ("数据库", "MySQL、PostgreSQL、SQLite 等数据库连接"),
    ("Redis", "Redis 与兼容协议连接"),
    ("MongoDB", "MongoDB 数据库连接"),
    ("RDP/VNC", "远程桌面连接"),
    ("代理", "代理服务器认证"),
    ("跳板机", "堡垒机与跳板机认证"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CredentialFormValues {
    pub name: String,
    pub kinds: Vec<String>,
    pub username: String,
    pub password: String,
    pub private_key_path: String,
    pub private_key_content: String,
    pub passphrase: String,
    pub ssh_expect: SshAccountExpect,
    pub sync_enabled: bool,
}

pub(crate) struct CredentialForm {
    focus_handle: FocusHandle,
    existing: Option<CredentialEntry>,
    pub(super) active_tab: usize,
    pub(super) name_input: Entity<InputState>,
    pub(super) selected_kinds: BTreeSet<String>,
    pub(super) kind_picker_open: bool,
    pub(super) username_input: Entity<InputState>,
    pub(super) password_input: Entity<InputState>,
    pub(super) private_key_path_input: Entity<InputState>,
    pub(super) private_key_content_input: Entity<InputState>,
    pub(super) passphrase_input: Entity<InputState>,
    pub(super) username_expect_input: Entity<InputState>,
    pub(super) username_send_input: Entity<InputState>,
    pub(super) password_expect_input: Entity<InputState>,
    pub(super) password_send_input: Entity<InputState>,
    pub(super) sync_enabled: bool,
}

impl CredentialForm {
    pub(crate) fn new(
        existing: Option<CredentialEntry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name_input = text_input(
            existing.as_ref().map(|entry| entry.name.as_str()),
            "例如：生产环境通用账号",
            false,
            window,
            cx,
        );
        let selected_kinds = existing
            .as_ref()
            .map(|entry| parse_credential_kinds(&entry.kind))
            .unwrap_or_else(|| BTreeSet::from(["通用".to_string()]));
        let username_input = text_input(
            existing
                .as_ref()
                .and_then(|entry| entry.username.as_deref()),
            "可选用户名",
            false,
            window,
            cx,
        );
        let password_input = text_input(
            existing
                .as_ref()
                .and_then(|entry| entry.password.as_deref()),
            "可选密码",
            true,
            window,
            cx,
        );
        let private_key_path_input = text_input(
            existing
                .as_ref()
                .and_then(|entry| entry.private_key_path.as_deref()),
            "例如：~/.ssh/id_ed25519（仅本机使用）",
            false,
            window,
            cx,
        );
        let private_key_content_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder("可选，粘贴 PEM/OpenSSH 私钥内容")
                .multi_line(true)
                .rows(5);
            if let Some(value) = existing
                .as_ref()
                .and_then(|entry| entry.private_key_content.as_deref())
            {
                state = state.default_value(value);
            }
            state
        });
        let passphrase_input = text_input(
            existing
                .as_ref()
                .and_then(|entry| entry.passphrase.as_deref()),
            "可选私钥密码",
            true,
            window,
            cx,
        );
        let username_expect_input = text_input(
            existing
                .as_ref()
                .map(|entry| entry.ssh_expect.username.expect.as_str()),
            "如 (?i)(?:login|username)\\s*:",
            false,
            window,
            cx,
        );
        let username_send_input = text_input(
            existing
                .as_ref()
                .map(|entry| entry.ssh_expect.username.send.as_str()),
            "留空时使用运行时用户名",
            false,
            window,
            cx,
        );
        let password_expect_input = text_input(
            existing
                .as_ref()
                .map(|entry| entry.ssh_expect.password.expect.as_str()),
            "如 (?i)password\\s*:",
            false,
            window,
            cx,
        );
        let password_send_input = text_input(
            existing
                .as_ref()
                .map(|entry| entry.ssh_expect.password.send.as_str()),
            "留空时使用运行时密码",
            true,
            window,
            cx,
        );
        let sync_enabled = existing.as_ref().is_some_and(|entry| entry.sync_enabled);

        Self {
            focus_handle: cx.focus_handle(),
            existing,
            active_tab: 0,
            name_input,
            selected_kinds,
            kind_picker_open: false,
            username_input,
            password_input,
            private_key_path_input,
            private_key_content_input,
            passphrase_input,
            username_expect_input,
            username_send_input,
            password_expect_input,
            password_send_input,
            sync_enabled,
        }
    }

    pub(crate) fn build_entry(&self, cx: &App) -> Result<CredentialEntry, String> {
        build_entry(self.existing.clone(), self.values(cx))
    }

    pub(super) fn is_editing(&self) -> bool {
        self.existing.is_some()
    }

    fn values(&self, cx: &App) -> CredentialFormValues {
        CredentialFormValues {
            name: input_value(&self.name_input, cx),
            kinds: ordered_credential_kinds(&self.selected_kinds),
            username: input_value(&self.username_input, cx),
            password: input_value(&self.password_input, cx),
            private_key_path: input_value(&self.private_key_path_input, cx),
            private_key_content: input_value(&self.private_key_content_input, cx),
            passphrase: input_value(&self.passphrase_input, cx),
            ssh_expect: SshAccountExpect {
                username: TerminalExpectSend {
                    expect: input_value(&self.username_expect_input, cx),
                    send: input_value(&self.username_send_input, cx),
                },
                password: TerminalExpectSend {
                    expect: input_value(&self.password_expect_input, cx),
                    send: input_value(&self.password_send_input, cx),
                },
            },
            sync_enabled: self.sync_enabled,
        }
    }
}

impl Focusable for CredentialForm {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

fn text_input(
    value: Option<&str>,
    placeholder: impl Into<SharedString>,
    masked: bool,
    window: &mut Window,
    cx: &mut Context<CredentialForm>,
) -> Entity<InputState> {
    cx.new(|cx| {
        let mut state = InputState::new(window, cx)
            .placeholder(placeholder)
            .masked(masked);
        if let Some(value) = value {
            state = state.default_value(value);
        }
        state
    })
}

fn input_value(input: &Entity<InputState>, cx: &App) -> String {
    input.read(cx).value().to_string()
}

pub(super) fn build_entry(
    existing: Option<CredentialEntry>,
    values: CredentialFormValues,
) -> Result<CredentialEntry, String> {
    let name = values.name.trim().to_string();
    if name.is_empty() {
        return Err("凭据名称不能为空".to_string());
    }
    let kind = serialize_credential_kinds(values.kinds);
    let mut entry = existing.unwrap_or_else(|| CredentialEntry::new(&name, &kind));
    entry.name = name;
    entry.kind = kind;
    entry.username = optional_trimmed(values.username);
    entry.password = optional_trimmed(values.password);
    entry.private_key_path = optional_trimmed(values.private_key_path);
    entry.private_key_content = optional_trimmed(values.private_key_content);
    entry.passphrase = optional_trimmed(values.passphrase);
    entry.ssh_expect = normalize_ssh_expect(values.ssh_expect)?;
    entry.sync_enabled = values.sync_enabled;
    Ok(entry)
}

fn normalize_ssh_expect(value: SshAccountExpect) -> Result<SshAccountExpect, String> {
    Ok(SshAccountExpect {
        username: normalize_expect_step(value.username, "用户名")?,
        password: normalize_expect_step(value.password, "密码")?,
    })
}

fn normalize_expect_step(
    value: TerminalExpectSend,
    label: &str,
) -> Result<TerminalExpectSend, String> {
    let expect = value.expect.trim().to_string();
    let send = value.send;

    if expect.is_empty() {
        if !send.trim().is_empty() {
            return Err(format!("{label}发送内容必须先配置 Expect 正则"));
        }
        return Ok(TerminalExpectSend::default());
    }

    let regex = Regex::new(&expect).map_err(|error| format!("{label} Expect 正则无效：{error}"))?;
    if regex.is_match("") {
        return Err(format!("{label} Expect 正则不能匹配空字符串"));
    }

    Ok(TerminalExpectSend { expect, send })
}

fn optional_trimmed(value: String) -> Option<String> {
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}

pub(super) fn credential_kind_values(value: &str) -> Vec<String> {
    let mut values = Vec::new();
    for value in value.split(['、', ',', '，', ';', '；']) {
        let value = value.trim();
        if !value.is_empty() && !values.iter().any(|current| current == value) {
            values.push(value.to_string());
        }
    }
    values
}

fn parse_credential_kinds(value: &str) -> BTreeSet<String> {
    let values = credential_kind_values(value);
    if values.is_empty() {
        BTreeSet::from(["通用".to_string()])
    } else {
        values.into_iter().collect()
    }
}

pub(super) fn ordered_credential_kinds(selected: &BTreeSet<String>) -> Vec<String> {
    let mut values = CREDENTIAL_KIND_OPTIONS
        .iter()
        .map(|(kind, _)| *kind)
        .filter(|kind| selected.contains(*kind))
        .map(str::to_string)
        .collect::<Vec<_>>();
    values.extend(
        selected
            .iter()
            .filter(|kind| {
                !CREDENTIAL_KIND_OPTIONS
                    .iter()
                    .any(|(option, _)| option == &kind.as_str())
            })
            .cloned(),
    );
    values
}

fn serialize_credential_kinds(kinds: Vec<String>) -> String {
    let mut selected = BTreeSet::new();
    for kind in kinds {
        let kind = kind.trim();
        if !kind.is_empty() {
            selected.insert(kind.to_string());
        }
    }
    let kinds = ordered_credential_kinds(&selected);
    if kinds.is_empty() {
        "通用".to_string()
    } else {
        kinds.join("、")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values() -> CredentialFormValues {
        CredentialFormValues {
            name: "  Production  ".into(),
            kinds: vec![" 数据库 ".into(), " SSH ".into(), "SSH".into()],
            username: " root ".into(),
            password: " secret ".into(),
            private_key_path: "   ".into(),
            private_key_content: " key ".into(),
            passphrase: " phrase ".into(),
            ssh_expect: SshAccountExpect {
                username: TerminalExpectSend {
                    expect: " login: ".into(),
                    send: " root ".into(),
                },
                password: TerminalExpectSend {
                    expect: " Password: ".into(),
                    send: " secret ".into(),
                },
            },
            sync_enabled: true,
        }
    }

    #[test]
    fn rejects_empty_name() {
        let mut values = values();
        values.name = "   ".into();
        assert_eq!(build_entry(None, values).unwrap_err(), "凭据名称不能为空");
    }

    #[test]
    fn normalizes_optional_values() {
        let entry = build_entry(None, values()).unwrap();
        assert_eq!(entry.name, "Production");
        assert_eq!(entry.kind, "SSH、数据库");
        assert_eq!(entry.username.as_deref(), Some("root"));
        assert_eq!(entry.private_key_path, None);
        assert_eq!(entry.private_key_content.as_deref(), Some("key"));
        assert!(entry.sync_enabled);
        assert_eq!(entry.ssh_expect.username.expect, "login:");
        assert_eq!(entry.ssh_expect.username.send, " root ");
    }

    #[test]
    fn allows_password_only_credentials() {
        let mut values = values();
        values.username.clear();
        values.private_key_content.clear();
        values.passphrase.clear();

        let entry = build_entry(None, values).unwrap();

        assert_eq!(entry.username, None);
        assert_eq!(entry.password.as_deref(), Some("secret"));
    }

    #[test]
    fn allows_password_only_expect_rules() {
        let mut values = values();
        values.username.clear();
        values.ssh_expect.username = TerminalExpectSend::default();
        values.ssh_expect.password = TerminalExpectSend {
            expect: "Password:".into(),
            send: String::new(),
        };

        let entry = build_entry(None, values).unwrap();

        assert!(entry.ssh_expect.username.is_empty());
        assert_eq!(entry.ssh_expect.password.expect, "Password:");
        assert!(entry.ssh_expect.password.send.is_empty());
    }

    #[test]
    fn rejects_invalid_expect_rules() {
        let mut values = values();
        values.ssh_expect.username.expect = ".*".into();
        assert_eq!(
            build_entry(None, values).unwrap_err(),
            "用户名 Expect 正则不能匹配空字符串"
        );
    }

    #[test]
    fn credential_kinds_support_multiple_values_and_legacy_strings() {
        assert_eq!(
            credential_kind_values("SSH、数据库, Redis；SSH"),
            vec!["SSH", "数据库", "Redis"]
        );
        assert_eq!(
            ordered_credential_kinds(&parse_credential_kinds("Redis、SSH、自定义")),
            vec!["SSH", "Redis", "自定义"]
        );

        let mut values = values();
        values.kinds.clear();
        assert_eq!(build_entry(None, values).unwrap().kind, "通用");
    }

    #[test]
    fn editing_preserves_repository_metadata_and_can_clear_secrets() {
        let mut existing = CredentialEntry::new("old", "database");
        existing.id = Some(7);
        existing.password = Some("old-secret".into());
        existing.cloud_id = Some("cloud-1".into());
        existing.last_synced_at = Some(10);
        existing.team_id = Some("team-1".into());
        existing.owner_id = Some("owner-1".into());
        existing.created_at = Some(20);
        existing.updated_at = Some(30);
        let mut values = values();
        values.password.clear();

        let entry = build_entry(Some(existing), values).unwrap();
        assert_eq!(entry.id, Some(7));
        assert_eq!(entry.password, None);
        assert_eq!(entry.cloud_id.as_deref(), Some("cloud-1"));
        assert_eq!(entry.last_synced_at, Some(10));
        assert_eq!(entry.team_id.as_deref(), Some("team-1"));
        assert_eq!(entry.owner_id.as_deref(), Some("owner-1"));
        assert_eq!(entry.created_at, Some(20));
        assert_eq!(entry.updated_at, Some(30));
    }
}
