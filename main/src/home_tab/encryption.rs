use super::*;

impl HomePage {
    pub(crate) fn is_master_key_ready_for_new_connection(&self) -> bool {
        crypto::has_master_key()
    }

    pub(super) fn ensure_master_key_ready_for_saved_connections(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.saved_connections_locked() {
            return true;
        }

        self.show_encryption_key_dialog(window, cx);
        false
    }

    pub(super) fn saved_connections_locked(&self) -> bool {
        crypto::has_repo_password_set() && !crypto::has_master_key()
    }

    pub(crate) fn startup_master_key_lock_active(&self, cx: &App) -> bool {
        AppSettings::current(cx).master_key_on_startup_required() && self.saved_connections_locked()
    }

    pub(crate) fn show_pending_master_key_prompt(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.master_key_unlock_prompt_pending || !self.saved_connections_locked() {
            return;
        }
        self.master_key_unlock_prompt_pending = false;
        self.show_encryption_key_dialog(window, cx);
    }

    pub(super) fn show_encryption_key_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.master_key_dialog_open {
            return;
        }

        self.master_key_dialog_open = true;

        let view = cx.entity();
        let has_password_set = crypto::has_repo_password_set();
        let is_first_setup = !has_password_set;
        let require_master_key_on_startup =
            AppSettings::current(cx).master_key_on_startup_required();
        let startup_lock =
            require_master_key_on_startup && has_password_set && !crypto::has_master_key();
        let initial_master_key = (!startup_lock)
            .then(|| {
                crypto::get_raw_master_key().or_else(|| {
                    let storage = key_storage::get_key_storage();
                    storage.load()
                })
            })
            .flatten();

        let key_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("Encryption.repo_password_placeholder"))
                .masked(true);

            if let Some(ref value) = initial_master_key {
                state = state.default_value(value);
            }

            state
        });

        let error_message = cx.new(|_| Option::<String>::None);

        let key_input_for_ok = key_input.clone();
        let error_msg_for_ok = error_message.clone();

        let key_input_for_render = key_input.clone();
        let error_msg_for_render = error_message.clone();

        let dialog_title = if is_first_setup {
            t!("Encryption.set_repo_password")
        } else {
            t!("Encryption.unlock_repo_password")
        };

        window.open_dialog(cx, move |dialog, _window, cx| {
            let key_input_ok = key_input_for_ok.clone();
            let error_msg_ok = error_msg_for_ok.clone();

            dialog
                .title(dialog_title.to_string())
                .width(px(520.))
                .confirm()
                .overlay_closable(!startup_lock)
                .close_button(!startup_lock)
                .on_cancel(move |_, _, _| !startup_lock)
                .on_ok(move |_, _window, cx: &mut App| {
                    let input_key = key_input_ok.read(cx).text().to_string();

                    if input_key.is_empty() {
                        error_msg_ok.update(cx, |msg, cx| {
                            *msg = Some(t!("Encryption.key_empty").to_string());
                            cx.notify();
                        });
                        return false;
                    }

                    if is_first_setup {
                        let result = if require_master_key_on_startup {
                            crypto::set_master_key_for_session(&input_key)
                        } else {
                            crypto::set_master_key(&input_key)
                        };
                        return match result {
                            Ok(()) => {
                                // 旧版固定盐安装：首次设置不会触发，这里保持对称调用
                                crate::master_key_flow::run_kdf_upgrade_after_unlock(cx);
                                true
                            }
                            Err(error) => {
                                tracing::error!("设置主密钥失败: {error}");
                                error_msg_ok.update(cx, |msg, cx| {
                                    *msg = Some(master_key_error_message(&error));
                                    cx.notify();
                                });
                                false
                            }
                        };
                    }

                    let result = if require_master_key_on_startup {
                        crypto::verify_and_set_master_key_for_session(&input_key)
                    } else {
                        crypto::verify_and_set_master_key(&input_key)
                    };
                    match result {
                        Ok(()) => {
                            // 解锁旧数据后把 KDF 迁移到 Argon2id（后台完成，失败可重试）
                            crate::master_key_flow::run_kdf_upgrade_after_unlock(cx);
                            true
                        }
                        Err(error) => {
                            tracing::error!("解锁主密钥失败: {error}");
                            error_msg_ok.update(cx, |msg, cx| {
                                *msg = Some(master_key_error_message(&error));
                                cx.notify();
                            });
                            false
                        }
                    }
                })
                .on_close({
                    let view_for_sync = view.clone();
                    move |_window, _result, cx| {
                        view_for_sync.update(cx, |this, cx| {
                            this.master_key_dialog_open = false;
                            if crypto::has_master_key() {
                                // 密钥已就绪后刷新连接列表，修复启动时序导致的空密码回显
                                this.load_connections(cx);
                                // 把当前密钥版本同步到云端，其他设备据此识别
                                // "云端密钥被改过"而不是"自己输错了"
                                crate::master_key_flow::report_key_version_to_cloud(cx);
                                if should_auto_onet_cloud_sync(cx, this.current_user.is_some()) {
                                    tracing::info!("密钥设置/解锁成功，自动触发云同步");
                                    this.trigger_sync(cx);
                                }
                            }
                        });
                    }
                })
                .child(
                    v_flex()
                        .gap_4()
                        .p_4()
                        .when(startup_lock, |content| {
                            content.child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(t!("Encryption.startup_lock_notice").to_string()),
                            )
                        })
                        .child(
                            h_flex()
                                .items_center()
                                .gap_3()
                                .child(
                                    div()
                                        .text_sm()
                                        .flex_shrink_0()
                                        .w(px(80.))
                                        .child(t!("Encryption.repo_password_label").to_string()),
                                )
                                .child(Input::new(&key_input_for_render).mask_toggle().w_full()),
                        )
                        .child(
                            v_flex()
                                .gap_3()
                                .child(
                                    div().text_base().font_weight(FontWeight::SEMIBOLD).child(
                                        t!("Encryption.remember_password_title").to_string(),
                                    ),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(
                                            t!("Encryption.remember_password_detail_local")
                                                .to_string(),
                                        ),
                                )
                                .child(
                                    v_flex()
                                        .gap_1()
                                        .p_3()
                                        .rounded_md()
                                        .border_1()
                                        .border_color(cx.theme().border)
                                        .bg(cx.theme().muted)
                                        .child(
                                            div()
                                                .text_sm()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child(
                                                    t!("Encryption.sync_info_title").to_string(),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(
                                                    t!("Encryption.remember_password_detail_cloud")
                                                        .to_string(),
                                                ),
                                        ),
                                )
                                .child(
                                    div().text_sm().text_color(cx.theme().warning).child(
                                        t!("Encryption.master_key_loss_warning").to_string(),
                                    ),
                                ),
                        )
                        .when_some(error_msg_for_render.read(cx).clone(), |this, msg| {
                            this.child(div().text_sm().text_color(cx.theme().danger).child(msg))
                        }),
                )
        });
        key_input.update(cx, |input, cx| input.focus(window, cx));
    }

    /// 修改主密钥。成功后刷新连接列表并触发全量同步，
    /// 让其他设备尽快看到"云端密钥已被更换"。
    pub(crate) fn show_change_master_key_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.entity().downgrade();
        crate::master_key_dialogs::show_change_master_key_dialog(
            window,
            cx,
            Some(Box::new(move |cx: &mut App| {
                // 视图可能已释放（例如弹窗期间窗口关闭），拿不到就无需刷新
                let _ = view.update(cx, |home, cx| {
                    home.load_connections(cx);
                    if should_auto_onet_cloud_sync(cx, home.current_user.is_some()) {
                        home.trigger_sync(cx);
                    }
                });
            })),
        );
    }

    /// 重置主密钥。成功后本地回到全新安装状态，刷新连接列表即可看到空列表。
    pub(crate) fn show_reset_master_key_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.entity().downgrade();
        crate::master_key_dialogs::show_reset_master_key_dialog(
            window,
            cx,
            Some(Box::new(move |cx: &mut App| {
                let _ = view.update(cx, |home, cx| home.load_connections(cx));
            })),
        );
    }
}

fn master_key_error_message(error: &crypto::CryptoError) -> String {
    match error {
        crypto::CryptoError::InvalidOldPassword => t!("Encryption.password_incorrect").to_string(),
        _ => t!("Encryption.master_key_persistence_failed").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use crate::master_key_flow::RESET_CONFIRM_WORD;

    #[test]
    fn startup_lock_dialog_cannot_be_dismissed_and_does_not_prefill_the_key() {
        let source = include_str!("encryption.rs");
        let dialog = source
            .split("pub(super) fn show_encryption_key_dialog(")
            .nth(1)
            .and_then(|source| {
                source
                    .split("pub(crate) fn show_change_master_key_dialog(")
                    .next()
            })
            .expect("show_encryption_key_dialog source");

        assert!(dialog.contains("let startup_lock ="));
        assert!(dialog.contains("let initial_master_key = (!startup_lock)"));
        assert!(dialog.contains(".overlay_closable(!startup_lock)"));
        assert!(dialog.contains(".close_button(!startup_lock)"));
        assert!(dialog.contains(".on_cancel(move |_, _, _| !startup_lock)"));
    }

    #[test]
    fn startup_lock_keeps_the_unlocked_key_in_memory_only() {
        let source = include_str!("encryption.rs");
        let flow = include_str!("../../../crates/app_kit/src/vault/master_key_flow.rs");
        let dialog = source
            .split("pub(super) fn show_encryption_key_dialog(")
            .nth(1)
            .and_then(|source| {
                source
                    .split("pub(crate) fn show_change_master_key_dialog(")
                    .next()
            })
            .expect("show_encryption_key_dialog source");

        assert!(dialog.contains("crypto::set_master_key_for_session"));
        assert!(dialog.contains("crypto::verify_and_set_master_key_for_session"));
        // 启动锁下改密钥同样只留在内存：编排里按 master_key_on_startup_required 分流
        assert!(flow.contains("crypto::change_master_key_for_session"));
    }

    #[test]
    fn master_key_change_rotates_connections_and_keychain_entries_together() {
        let flow = include_str!("../../../crates/app_kit/src/vault/master_key_flow.rs");

        assert!(flow.contains("one_core::storage::re_encrypt_secrets("));
        assert!(flow.contains("stats.connections"));
        assert!(flow.contains("stats.credentials"));
        assert!(flow.contains("stats.team_key_caches"));
        assert!(!flow.contains("fn re_encrypt_all_connections("));
    }

    #[test]
    fn master_key_setup_and_unlock_surface_persistence_failures() {
        let source = include_str!("encryption.rs");
        let dialog = source
            .split("pub(super) fn show_encryption_key_dialog(")
            .nth(1)
            .and_then(|source| {
                source
                    .split("pub(crate) fn show_change_master_key_dialog(")
                    .next()
            })
            .expect("show_encryption_key_dialog source");

        assert!(dialog.contains("master_key_error_message(&error)"));
        assert!(source.contains("Encryption.master_key_persistence_failed"));
    }

    #[test]
    fn personal_key_entry_keeps_its_own_dialog() {
        // 「个人密钥」入口只做首次设置/解锁；已解锁时也不能转交给修改弹窗，
        // 否则它会与独立的「修改主密钥」入口弹出同一个表单。
        let source = include_str!("encryption.rs");
        let dialog = source
            .split("pub(super) fn show_encryption_key_dialog(")
            .nth(1)
            .and_then(|source| {
                source
                    .split("pub(crate) fn show_change_master_key_dialog(")
                    .next()
            })
            .expect("show_encryption_key_dialog source");

        assert!(!dialog.contains("self.show_change_master_key_dialog(window, cx)"));
        assert!(RESET_CONFIRM_WORD == "RESET");
    }
}
