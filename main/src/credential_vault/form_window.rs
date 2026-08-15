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
        let result = self.form.read(cx).build_entry(cx).and_then(|mut entry| {
            let repository = self
                .storage_manager
                .get::<CredentialRepository>()
                .ok_or_else(|| "CredentialRepository 尚未注册".to_string())?;
            if self.editing {
                let id = entry.id.ok_or_else(|| "编辑凭据缺少本地 ID".to_string())?;
                repository
                    .update(&entry)
                    .map(|_| id)
                    .map_err(|error| error.to_string())
            } else {
                repository
                    .insert(&mut entry)
                    .map_err(|error| error.to_string())
            }
        });

        match result {
            Ok(credential_id) => {
                emit_connection_event(
                    if self.editing {
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
                    Notification::success(if self.editing {
                        "凭据已更新"
                    } else {
                        "凭据已创建"
                    })
                    .autohide(true),
                    cx,
                );
                window.remove_window();
            }
            Err(error) => {
                window.push_notification(
                    Notification::error(format!("保存凭据失败：{error}")).autohide(true),
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
                            .label("取消")
                            .on_click(|_, window, _| window.remove_window()),
                    )
                    .child(
                        Button::new("credential-form-save")
                            .small()
                            .primary()
                            .label("保存")
                            .on_click(cx.listener(|form_window, _, window, cx| {
                                form_window.save(window, cx);
                            })),
                    ),
            )
    }
}
