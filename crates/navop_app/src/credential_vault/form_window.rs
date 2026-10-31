use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement, Render,
    Styled, Window, div,
};
use gpui_component::{
    ActiveTheme, Sizable, WindowExt,
    button::{Button, ButtonVariants as _},
    h_flex,
    notification::Notification,
    v_flex,
};
use one_core::connection_notifier::{ConnectionDataEvent, emit_connection_event};
use one_core::storage::{
    CredentialEntry, CredentialRepository, StorageManager, traits::Repository,
};
use rust_i18n::t;

use super::{CredentialVaultView, form::CredentialForm};

pub(super) struct CredentialFormWindow {
    focus_handle: FocusHandle,
    form: Entity<CredentialForm>,
    storage_manager: StorageManager,
    vault_view: Entity<CredentialVaultView>,
    editing: bool,
}

impl CredentialFormWindow {
    pub(super) fn new(
        existing: Option<CredentialEntry>,
        storage_manager: StorageManager,
        vault_view: Entity<CredentialVaultView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editing = existing.is_some();
        let form = cx.new(|cx| CredentialForm::new(existing, window, cx));
        Self {
            focus_handle: cx.focus_handle(),
            form,
            storage_manager,
            vault_view,
            editing,
        }
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 事件类型（新建/更新）和提示词都按「这一轮开始时」的状态算：保存成功后表单会
        // 切成编辑态（见下面），读得太晚新建也会被当成更新。
        let was_editing = self.editing;
        let result = self.form.read(cx).build_entry(cx).and_then(|mut entry| {
            let repository = self
                .storage_manager
                .get::<CredentialRepository>()
                .ok_or_else(|| t!("CredentialVault.repository_unavailable").to_string())?;
            let credential_id = if was_editing {
                let id = entry
                    .id
                    .ok_or_else(|| t!("CredentialForm.error_missing_local_id").to_string())?;
                repository
                    .update(&entry)
                    .map(|_| id)
                    .map_err(|error| error.to_string())?
            } else {
                repository
                    .insert(&mut entry)
                    .map_err(|error| error.to_string())?
            };
            entry.id = Some(credential_id);
            Ok((credential_id, entry))
        });

        match result {
            Ok((credential_id, saved_entry)) => {
                // 保存已经落地：把表单切到「已保存」状态并带上 id —— 窗口因为隐藏失败
                // 留在屏幕上时（关闭漏斗返回 `Retained`）用户可能再点一次「保存」，
                // 没有这一步会再插一条凭据。
                self.editing = true;
                self.form.update(cx, |form, _| form.mark_saved(saved_entry));
                emit_connection_event(
                    if was_editing {
                        ConnectionDataEvent::CredentialUpdated { credential_id }
                    } else {
                        ConnectionDataEvent::CredentialCreated { credential_id }
                    },
                    cx,
                );
                _ = self
                    .vault_view
                    .update(cx, |vault_view, cx| vault_view.reload(cx));
                window.push_notification(
                    Notification::success(if was_editing {
                        t!("CredentialForm.updated").to_string()
                    } else {
                        t!("CredentialForm.created").to_string()
                    })
                    .autohide(true),
                    cx,
                );
                let _ = one_core::window_close::close_window_after_save(window, cx);
            }
            Err(error) => {
                window.push_notification(
                    Notification::error(
                        t!("CredentialForm.save_failed", error = error).to_string(),
                    )
                    .autohide(true),
                    cx,
                );
            }
        }
    }
}

impl Focusable for CredentialFormWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for CredentialFormWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(self.form.clone()),
            )
            .child(
                h_flex()
                    .flex_shrink_0()
                    .justify_end()
                    .gap_2()
                    .p_4()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        Button::new("credential-form-cancel")
                            .small()
                            .label(t!("CredentialForm.cancel").to_string())
                            .on_click(|_, window, cx| {
                                let _ = one_core::window_close::close_window_for_reuse(window, cx);
                            }),
                    )
                    .child(
                        Button::new("credential-form-save")
                            .small()
                            .primary()
                            .label(t!("CredentialForm.save").to_string())
                            .on_click(cx.listener(|form_window, _, window, cx| {
                                form_window.save(window, cx);
                            })),
                    ),
            )
    }
}
