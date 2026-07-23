use super::*;

impl HomePage {
    pub(crate) fn ensure_master_key_ready_for_new_connection(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.is_master_key_ready_for_new_connection() {
            return true;
        }

        self.show_encryption_key_dialog(window, cx);
        false
    }

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
        one_core::app_paths::master_key_on_startup_required(
            AppSettings::current(cx).require_master_key_on_startup,
        ) && self.saved_connections_locked()
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
        let has_key_in_memory = crypto::has_master_key();
        let is_first_setup = !has_password_set;
        let is_change_mode = has_password_set && has_key_in_memory;
        let require_master_key_on_startup = one_core::app_paths::master_key_on_startup_required(
            AppSettings::current(cx).require_master_key_on_startup,
        );
        let startup_lock = require_master_key_on_startup && has_password_set && !has_key_in_memory;
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
        } else if is_change_mode {
            t!("Encryption.change_repo_password")
        } else {
            t!("Encryption.unlock_repo_password")
        };

        window.open_dialog(cx, move |dialog, _window, cx| {
            let key_input_ok = key_input_for_ok.clone();
            let error_msg_ok = error_msg_for_ok.clone();

            dialog
                .title(dialog_title.to_string())
                .width(px(450.))
                .confirm()
                .overlay_closable(!startup_lock)
                .close_button(!startup_lock)
                .on_cancel(move |_, _, _| !startup_lock)
                .on_ok(move |_, _window, cx| {
                    let input_key = key_input_ok.read(cx).text().to_string();

                    if input_key.is_empty() {
                        error_msg_ok.update(cx, |msg, cx| {
                            *msg = Some(t!("Encryption.key_empty").to_string());
                            cx.notify();
                        });
                        return false;
                    }

                    if is_first_setup {
                        if require_master_key_on_startup {
                            crypto::set_master_key_for_session(&input_key);
                        } else {
                            crypto::set_master_key(&input_key);
                        }
                        return true;
                    }

                    if is_change_mode {
                        let old_key = match crypto::get_raw_master_key() {
                            Some(key) if !key.is_empty() => key,
                            _ => {
                                error_msg_ok.update(cx, |msg, cx| {
                                    *msg = Some(t!("Encryption.password_incorrect").to_string());
                                    cx.notify();
                                });
                                return false;
                            }
                        };

                        if input_key != old_key {
                            let result = if require_master_key_on_startup {
                                crypto::change_master_key_for_session(
                                    &old_key, &input_key, &input_key,
                                )
                            } else {
                                crypto::change_master_key(&old_key, &input_key, &input_key)
                            };
                            match result {
                                Ok(()) => {
                                    let storage = cx.global::<GlobalStorageState>().storage.clone();
                                    match re_encrypt_all_connections(&storage) {
                                        Ok(count) => {
                                            tracing::info!(
                                                "主密钥修改成功，已重新加密 {} 个本地连接",
                                                count
                                            );
                                        }
                                        Err(e) => {
                                            tracing::error!("重新加密本地连接失败: {}", e);
                                            error_msg_ok.update(cx, |msg, cx| {
                                                *msg = Some(e.to_string());
                                                cx.notify();
                                            });
                                            return false;
                                        }
                                    }
                                }
                                Err(e) => {
                                    error_msg_ok.update(cx, |msg, cx| {
                                        *msg = Some(e.to_string());
                                        cx.notify();
                                    });
                                    return false;
                                }
                            }
                        }

                        return true;
                    }

                    let result = if require_master_key_on_startup {
                        crypto::verify_and_set_master_key_for_session(&input_key)
                    } else {
                        crypto::verify_and_set_master_key(&input_key)
                    };
                    match result {
                        Ok(()) => true,
                        Err(_) => {
                            error_msg_ok.update(cx, |msg, cx| {
                                *msg = Some(t!("Encryption.password_incorrect").to_string());
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
                                    .text_color(cx.theme().warning)
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
                                .gap_2()
                                .child(
                                    div().text_base().font_weight(FontWeight::SEMIBOLD).child(
                                        t!("Encryption.remember_password_title").to_string(),
                                    ),
                                )
                                .child(div().text_sm().child(
                                    t!("Encryption.remember_password_detail_local").to_string(),
                                ))
                                .child(div().text_sm().text_color(cx.theme().warning).child(
                                    t!("Encryption.remember_password_detail_cloud").to_string(),
                                )),
                        )
                        .when_some(error_msg_for_render.read(cx).clone(), |this, msg| {
                            this.child(div().text_sm().text_color(cx.theme().danger).child(msg))
                        }),
                )
        });
        key_input.update(cx, |input, cx| input.focus(window, cx));
    }

    pub(super) fn team_management_url(&self) -> Result<String, String> {
        let Some((access_token, refresh_token, _, _)) = load_auth_data() else {
            return Err(t!("Home.cloud_need_login").to_string());
        };

        let template = team_management_url_template();
        let template = template.trim();
        let url = build_team_management_url(template, &access_token, &refresh_token);
        Ok(resolve_team_management_url(
            &url,
            website_base_url().as_deref(),
        ))
    }

    pub(crate) fn open_team_management(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.team_management_url() {
            Ok(url) => cx.open_url(&url),
            Err(message) => window.push_notification(message, cx),
        }
    }
}

/// 使用当前主密钥重新加密并保存所有连接。
pub(super) fn re_encrypt_all_connections(
    storage: &one_core::storage::StorageManager,
) -> anyhow::Result<usize> {
    let conn_repo = storage
        .get::<ConnectionRepository>()
        .ok_or_else(|| anyhow::anyhow!("ConnectionRepository not found"))?;

    let connections = conn_repo.list()?;
    let mut count = 0;

    for conn in connections {
        conn_repo.update(&conn)?;
        count += 1;
    }

    Ok(count)
}

#[cfg(test)]
mod tests {
    #[test]
    fn startup_lock_dialog_cannot_be_dismissed_and_does_not_prefill_the_key() {
        let source = include_str!("encryption.rs");
        let dialog = source
            .split("pub(super) fn show_encryption_key_dialog(")
            .nth(1)
            .and_then(|source| source.split("pub(super) fn team_management_url").next())
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
        let dialog = source
            .split("pub(super) fn show_encryption_key_dialog(")
            .nth(1)
            .and_then(|source| source.split("pub(super) fn team_management_url").next())
            .expect("show_encryption_key_dialog source");

        assert!(dialog.contains("crypto::set_master_key_for_session"));
        assert!(dialog.contains("crypto::verify_and_set_master_key_for_session"));
        assert!(dialog.contains("crypto::change_master_key_for_session"));
    }
}
