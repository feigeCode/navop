use gpui::{App, AppContext, Context, Entity, FocusHandle, Focusable, SharedString, Window};
use gpui_component::input::InputState;
use one_core::storage::{CredentialEntry, SshAccountExpect, TerminalExpectSend};
use regex::Regex;
use rust_i18n::t;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CredentialFormValues {
    pub name: String,
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
            t!("CredentialForm.name_placeholder").to_string(),
            false,
            window,
            cx,
        );
        let username_input = text_input(
            existing
                .as_ref()
                .and_then(|entry| entry.username.as_deref()),
            t!("CredentialForm.username_placeholder").to_string(),
            false,
            window,
            cx,
        );
        let password_input = text_input(
            existing
                .as_ref()
                .and_then(|entry| entry.password.as_deref()),
            t!("CredentialForm.password_placeholder").to_string(),
            true,
            window,
            cx,
        );
        let private_key_path_input = text_input(
            existing
                .as_ref()
                .and_then(|entry| entry.private_key_path.as_deref()),
            t!("CredentialForm.private_key_path_placeholder").to_string(),
            false,
            window,
            cx,
        );
        let private_key_content_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("CredentialForm.private_key_content_placeholder").to_string())
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
            t!("CredentialForm.passphrase_placeholder").to_string(),
            true,
            window,
            cx,
        );
        let username_expect_input = text_input(
            existing
                .as_ref()
                .map(|entry| entry.ssh_expect.username.expect.as_str()),
            t!("CredentialForm.username_expect_placeholder").to_string(),
            false,
            window,
            cx,
        );
        let username_send_input = text_input(
            existing
                .as_ref()
                .map(|entry| entry.ssh_expect.username.send.as_str()),
            t!("CredentialForm.username_send_placeholder").to_string(),
            false,
            window,
            cx,
        );
        let password_expect_input = text_input(
            existing
                .as_ref()
                .map(|entry| entry.ssh_expect.password.expect.as_str()),
            t!("CredentialForm.password_expect_placeholder").to_string(),
            false,
            window,
            cx,
        );
        let password_send_input = text_input(
            existing
                .as_ref()
                .map(|entry| entry.ssh_expect.password.send.as_str()),
            t!("CredentialForm.password_send_placeholder").to_string(),
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

    fn values(&self, cx: &App) -> CredentialFormValues {
        CredentialFormValues {
            name: input_value(&self.name_input, cx),
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
        return Err(t!("CredentialForm.error_name_required").to_string());
    }
    let mut entry = existing.unwrap_or_else(|| CredentialEntry::new(&name));
    entry.name = name;
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
        username: normalize_expect_step(
            value.username,
            &t!("CredentialForm.username").to_string(),
        )?,
        password: normalize_expect_step(
            value.password,
            &t!("CredentialForm.password").to_string(),
        )?,
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
            return Err(t!(
                "CredentialForm.error_expect_send_requires_pattern",
                field = label
            )
            .to_string());
        }
        return Ok(TerminalExpectSend::default());
    }

    let regex = Regex::new(&expect).map_err(|error| {
        t!(
            "CredentialForm.error_expect_invalid",
            field = label,
            error = error
        )
        .to_string()
    })?;
    if regex.is_match("") {
        return Err(t!("CredentialForm.error_expect_matches_empty", field = label).to_string());
    }

    Ok(TerminalExpectSend { expect, send })
}

fn optional_trimmed(value: String) -> Option<String> {
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values() -> CredentialFormValues {
        CredentialFormValues {
            name: "  Production  ".into(),
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
        assert_eq!(
            build_entry(None, values).unwrap_err(),
            t!("CredentialForm.error_name_required").to_string()
        );
    }

    #[test]
    fn normalizes_optional_values() {
        let entry = build_entry(None, values()).unwrap();
        assert_eq!(entry.name, "Production");
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
            t!(
                "CredentialForm.error_expect_matches_empty",
                field = t!("CredentialForm.username")
            )
            .to_string()
        );
    }

    #[test]
    fn editing_preserves_repository_metadata_and_can_clear_secrets() {
        let mut existing = CredentialEntry::new("old");
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
