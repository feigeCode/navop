use gpui::{
    App, AppContext, ClipboardItem, Context, Entity, ExternalPaths, FocusHandle, Focusable,
    PathPromptOptions, SharedString, Window,
};
use gpui_component::{
    IndexPath, WindowExt,
    input::{InputEvent, InputState, TextareaState},
    notification::Notification,
    select::SelectState,
};
use one_core::storage::{
    CredentialEntry, GeneratedSshKey, SshAccountExpect, SshKeyAlgorithm, TerminalExpectSend,
};
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
    pub forward_to_agent: bool,
}

pub(crate) struct CredentialForm {
    focus_handle: FocusHandle,
    existing: Option<CredentialEntry>,
    pub(super) active_tab: usize,
    pub(super) name_input: Entity<InputState>,
    pub(super) username_input: Entity<InputState>,
    pub(super) password_input: Entity<InputState>,
    pub(super) private_key_path_input: Entity<InputState>,
    pub(super) private_key_content_input: Entity<TextareaState>,
    pub(super) passphrase_input: Entity<InputState>,
    pub(super) username_expect_input: Entity<InputState>,
    pub(super) username_send_input: Entity<InputState>,
    pub(super) password_expect_input: Entity<InputState>,
    pub(super) password_send_input: Entity<InputState>,
    pub(super) sync_enabled: bool,
    pub(super) forward_to_agent: bool,
    /// 「生成密钥」的算法下拉框，选项顺序与 [`SshKeyAlgorithm::ALL`] 一致。
    pub(super) algorithm_select: Entity<SelectState<Vec<String>>>,
    /// 后台正在生成密钥：期间禁用按钮，避免重复生成把表单覆盖掉。
    pub(super) generating_key: bool,
    /// 最近一次生成的 OpenSSH 公钥：留在表单里展示，避免只复制到剪贴板后找不回来（#558）。
    pub(super) generated_public_key: Option<String>,
    /// 文件选择器挑中的私钥路径，等下一次渲染再写进输入框（写输入态需要 Window）。
    pending_private_key_path: Option<String>,
}

impl CredentialForm {
    /// 按下拉框选中的算法生成密钥：私钥写进表单，公钥复制到剪贴板并提示指纹。
    ///
    /// 使用凭据名称作为密钥注释，方便在 `authorized_keys` 里辨认来源。RSA 生成
    /// 在慢机器上要几秒钟，所以统一放后台线程，避免卡住对话框。
    pub(super) fn generate_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.generating_key {
            return;
        }
        let comment = self.name_input.read(cx).value().trim().to_string();
        let algorithm = self.selected_key_algorithm(cx);
        self.generating_key = true;

        let task =
            cx.background_spawn(
                async move { one_core::storage::generate_ssh_key(algorithm, &comment) },
            );
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |form, window, cx| {
                form.apply_generated_key(result, window, cx);
            });
        })
        .detach();
        cx.notify();
    }

    /// 生成结束：把密钥写进表单并恢复按钮；失败时只弹提示，表单保持原样。
    fn apply_generated_key(
        &mut self,
        result: Result<GeneratedSshKey, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.generating_key = false;
        match result {
            Ok(key) => {
                self.private_key_content_input.update(cx, |state, cx| {
                    state.set_value(key.private_key_openssh.clone(), window, cx);
                });
                self.generated_public_key = Some(key.public_key_openssh.clone());
                cx.write_to_clipboard(ClipboardItem::new_string(key.public_key_openssh));
                window.push_notification(
                    Notification::success(
                        t!(
                            "CredentialForm.generate_key_success",
                            fingerprint = key.fingerprint
                        )
                        .to_string(),
                    )
                    .autohide(true),
                    cx,
                );
            }
            Err(error) => {
                window.push_notification(
                    Notification::error(
                        t!("CredentialForm.generate_key_failed", error = error).to_string(),
                    )
                    .autohide(true),
                    cx,
                );
            }
        }
        cx.notify();
    }

    /// 重新复制刚生成的公钥：生成时虽然已自动复制，但用户可能先复制了别的内容。
    pub(super) fn copy_generated_public_key(&mut self, cx: &mut Context<Self>) {
        if let Some(public_key) = self.generated_public_key.clone() {
            cx.write_to_clipboard(ClipboardItem::new_string(public_key));
        }
    }

    /// 下拉框当前选中项对应的算法；越界时回退到默认的 Ed25519。
    fn selected_key_algorithm(&self, cx: &App) -> SshKeyAlgorithm {
        let row = self
            .algorithm_select
            .read(cx)
            .selected_index(cx)
            .map_or(0, |index| index.row);
        SshKeyAlgorithm::ALL
            .get(row)
            .copied()
            .unwrap_or(SshKeyAlgorithm::Ed25519)
    }

    /// 打开系统文件选择器挑选本地私钥文件，选中后写回「私钥路径」。
    pub(super) fn browse_private_key_path(&mut self, cx: &mut Context<Self>) {
        let future = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(
                t!("CredentialForm.private_key_path_select")
                    .to_string()
                    .into(),
            ),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = future.await else {
                return;
            };
            let Some(path) = paths.first() else {
                return;
            };
            let path = path.to_string_lossy().to_string();
            let _ = this.update(cx, |form, cx| {
                form.pending_private_key_path = Some(path);
                cx.notify();
            });
        })
        .detach();
    }

    /// 渲染前应用浏览结果：写输入态必须拿到 `Window`，所以推迟到这里。
    pub(super) fn apply_pending_private_key_path(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self.pending_private_key_path.take() else {
            return;
        };
        self.private_key_path_input.update(cx, |state, cx| {
            state.set_value(path, window, cx);
        });
    }

    /// 拖入私钥文件时，把路径写进「私钥路径」输入框。
    ///
    /// 与浏览按钮共用同一份输入态；拖放只是省去一次文件选择对话框。
    pub(super) fn accept_dropped_private_key_path(
        &mut self,
        paths: &ExternalPaths,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = paths.paths().first() else {
            return;
        };
        let path = path.to_string_lossy().to_string();
        self.private_key_path_input.update(cx, |state, cx| {
            state.set_value(path, window, cx);
        });
        cx.notify();
    }

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
        // 用户名同时出现在「基础信息」与「SSH 密钥」两页，这里订阅变化以便密钥页的提示实时刷新。
        cx.subscribe(&username_input, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
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
            let mut state = TextareaState::new(window, cx)
                .placeholder(t!("CredentialForm.private_key_content_placeholder").to_string())
                .auto_grow(5, 5);
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
        let forward_to_agent = existing
            .as_ref()
            .is_some_and(|entry| entry.forward_to_agent);
        let algorithm_items: Vec<String> = SshKeyAlgorithm::ALL
            .iter()
            .map(|algorithm| generate_key_algorithm_label(*algorithm))
            .collect();
        let algorithm_select =
            cx.new(|cx| SelectState::new(algorithm_items, Some(IndexPath::new(0)), window, cx));

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
            forward_to_agent,
            algorithm_select,
            generating_key: false,
            generated_public_key: None,
            pending_private_key_path: None,
        }
    }

    pub(crate) fn build_entry(&self, cx: &App) -> Result<CredentialEntry, String> {
        build_entry(self.existing.clone(), self.values(cx))
    }

    /// 保存已经落地：把表单切到「已保存」状态（记住刚保存的条目，含 id）。
    ///
    /// 下一轮保存会按已有条目走更新而不是新建 —— 窗口因为隐藏失败留在屏幕上时
    /// （关闭漏斗返回 `Retained`）用户可能再点一次「保存」。
    pub(crate) fn mark_saved(&mut self, entry: CredentialEntry) {
        self.existing = Some(entry);
    }

    fn values(&self, cx: &App) -> CredentialFormValues {
        CredentialFormValues {
            name: input_value(&self.name_input, cx),
            username: input_value(&self.username_input, cx),
            password: input_value(&self.password_input, cx),
            private_key_path: input_value(&self.private_key_path_input, cx),
            private_key_content: self.private_key_content_input.read(cx).value().to_string(),
            passphrase: input_value(&self.passphrase_input, cx),
            ssh_expect: SshAccountExpect {
                elevation: Default::default(),
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
            forward_to_agent: self.forward_to_agent,
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

/// 「生成密钥」下拉框里展示的算法名称。
fn generate_key_algorithm_label(algorithm: SshKeyAlgorithm) -> String {
    match algorithm {
        SshKeyAlgorithm::Ed25519 => t!("CredentialForm.generate_key_algorithm_ed25519").to_string(),
        SshKeyAlgorithm::EcdsaP256 => {
            t!("CredentialForm.generate_key_algorithm_ecdsa_p256").to_string()
        }
        SshKeyAlgorithm::EcdsaP384 => {
            t!("CredentialForm.generate_key_algorithm_ecdsa_p384").to_string()
        }
        SshKeyAlgorithm::Rsa3072 => t!("CredentialForm.generate_key_algorithm_rsa3072").to_string(),
    }
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
    entry.forward_to_agent = values.forward_to_agent;
    Ok(entry)
}

fn normalize_ssh_expect(value: SshAccountExpect) -> Result<SshAccountExpect, String> {
    Ok(SshAccountExpect {
        elevation: Default::default(),
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
                elevation: Default::default(),
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
            forward_to_agent: false,
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
    fn key_algorithm_labels_cover_every_generation_option() {
        let labels: Vec<String> = SshKeyAlgorithm::ALL
            .iter()
            .map(|algorithm| generate_key_algorithm_label(*algorithm))
            .collect();

        assert_eq!(labels.len(), SshKeyAlgorithm::ALL.len());
        assert!(labels.iter().all(|label| !label.trim().is_empty()));

        let mut unique = labels.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), labels.len());
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
