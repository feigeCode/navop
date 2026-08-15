use gpui::{App, AppContext, Context, Entity, Window};
use gpui_component::{
    WindowExt, button::ButtonVariant, dialog::DialogButtonProps, notification::Notification,
};
use one_core::connection_notifier::{ConnectionDataEvent, emit_connection_event_from_app};
use one_core::popup_window::{PopupWindowOptions, open_popup_window};
use one_core::storage::{
    CredentialEntry, CredentialReferenceHit, CredentialRepository, DeleteCredentialOutcome,
    StorageManager,
};

use super::{CredentialVaultView, form_window::CredentialFormWindow};

impl CredentialVaultView {
    pub(super) fn open_create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_form(None, window, cx);
    }

    pub(super) fn open_edit(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let result = self
            .repository()
            .and_then(|repository| repository.get_plaintext(id).map_err(|e| e.to_string()));
        match result {
            Ok(Some(entry)) => self.open_form(Some(entry), window, cx),
            Ok(None) => window.push_notification(
                Notification::error("凭据不存在，可能已被删除").autohide(true),
                cx,
            ),
            Err(error) => window.push_notification(
                Notification::error(format!(
                    "无法打开凭据：{error}。若钥匙串已锁定，请先解锁主密钥。"
                ))
                .autohide(true),
                cx,
            ),
        }
    }

    fn open_form(
        &mut self,
        existing: Option<CredentialEntry>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editing = existing.is_some();
        let storage_manager = self.storage_manager.clone();
        let view = cx.entity();
        open_popup_window(
            PopupWindowOptions::new(if editing {
                "编辑凭据"
            } else {
                "新增凭据"
            })
            .size(700.0, 650.0)
            .min_width(560.0)
            .min_height(480.0),
            move |window, cx| {
                cx.new(|cx| CredentialFormWindow::new(existing, storage_manager, view, window, cx))
            },
            cx,
        );
    }

    pub(super) fn confirm_delete(
        &mut self,
        id: i64,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let storage_manager = self.storage_manager.clone();
        let view = cx.entity().clone();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let storage_manager = storage_manager.clone();
            let view = view.clone();
            alert
                .title("删除凭据")
                .description(format!(
                    "确定删除“{name}”吗？如果有连接仍在引用它，删除会被安全拒绝。"
                ))
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("删除")
                        .ok_variant(ButtonVariant::Danger)
                        .cancel_text("取消")
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx| {
                    delete_credential(id, &storage_manager, &view, window, cx)
                })
        });
    }
}

fn delete_credential(
    id: i64,
    storage: &StorageManager,
    view: &Entity<CredentialVaultView>,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let result = storage
        .get::<CredentialRepository>()
        .ok_or_else(|| "CredentialRepository 尚未注册".to_string())
        .and_then(|repo| {
            let cloud_id = repo
                .get_summary(id)
                .map_err(|error| error.to_string())?
                .and_then(|summary| summary.cloud_id);
            let outcome = repo.delete_checked(id).map_err(|error| error.to_string())?;
            Ok((outcome, cloud_id))
        });
    match result {
        Ok((DeleteCredentialOutcome::Deleted, cloud_id)) => {
            emit_connection_event_from_app(
                ConnectionDataEvent::CredentialDeleted {
                    credential_id: id,
                    cloud_id,
                },
                cx,
            );
            _ = view.update(cx, |view, cx| view.reload(cx));
            window.push_notification(Notification::success("凭据已删除").autohide(true), cx);
            true
        }
        Ok((DeleteCredentialOutcome::NotFound, _)) => {
            _ = view.update(cx, |view, cx| view.reload(cx));
            window.push_notification(Notification::warning("凭据已不存在").autohide(true), cx);
            true
        }
        Ok((DeleteCredentialOutcome::Referenced(hits), _)) => {
            window.push_notification(
                Notification::error(format_reference_hits(&hits)).autohide(false),
                cx,
            );
            false
        }
        Err(error) => {
            window.push_notification(
                Notification::error(format!("删除凭据失败：{error}")).autohide(true),
                cx,
            );
            false
        }
    }
}

fn format_reference_hits(hits: &[CredentialReferenceHit]) -> String {
    let mut lines = vec!["该凭据仍被以下连接引用，请先取消引用：".to_string()];
    lines.extend(hits.iter().take(5).map(|hit| {
        let via = hit
            .via_ssh_connection_id
            .map(|id| format!("，经 SSH 连接 #{id}"))
            .unwrap_or_default();
        format!(
            "• {}（{:?} / {:?}{}）",
            hit.connection_name, hit.connection_type, hit.location, via
        )
    }));
    if hits.len() > 5 {
        lines.push(format!("…以及另外 {} 个引用", hits.len() - 5));
    }
    lines.join("\n")
}
