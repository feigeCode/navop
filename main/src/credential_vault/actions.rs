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
use rust_i18n::t;

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
                Notification::error(t!("CredentialVault.not_found").to_string()).autohide(true),
                cx,
            ),
            Err(error) => window.push_notification(
                Notification::error(t!("CredentialVault.open_failed", error = error).to_string())
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
                t!("CredentialForm.edit_title").to_string()
            } else {
                t!("CredentialForm.create_title").to_string()
            })
            .size(700.0, 650.0)
            .min_width(560.0)
            .min_height(480.0),
            move |window, cx| {
                cx.new(|cx| CredentialFormWindow::new(existing, storage_manager, view, window, cx))
            },
            Some(_window),
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
                .title(t!("CredentialVault.delete_title").to_string())
                .description(t!("CredentialVault.delete_description", name = name).to_string())
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("CredentialVault.delete").to_string())
                        .ok_variant(ButtonVariant::Danger)
                        .cancel_text(t!("CredentialForm.cancel").to_string())
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx: &mut App| {
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
        .ok_or_else(|| t!("CredentialVault.repository_unavailable").to_string())
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
            window.push_notification(
                Notification::success(t!("CredentialVault.deleted").to_string()).autohide(true),
                cx,
            );
            true
        }
        Ok((DeleteCredentialOutcome::NotFound, _)) => {
            _ = view.update(cx, |view, cx| view.reload(cx));
            window.push_notification(
                Notification::warning(t!("CredentialVault.already_deleted").to_string())
                    .autohide(true),
                cx,
            );
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
                Notification::error(t!("CredentialVault.delete_failed", error = error).to_string())
                    .autohide(true),
                cx,
            );
            false
        }
    }
}

fn format_reference_hits(hits: &[CredentialReferenceHit]) -> String {
    let mut lines = vec![t!("CredentialVault.reference_header").to_string()];
    lines.extend(hits.iter().take(5).map(|hit| {
        let via = hit
            .via_ssh_connection_id
            .map(|id| t!("CredentialVault.reference_via_ssh", id = id).to_string())
            .unwrap_or_default();
        t!(
            "CredentialVault.reference_item",
            name = hit.connection_name,
            connection_type = format!("{:?}", hit.connection_type),
            location = format!("{:?}", hit.location),
            via = via
        )
        .to_string()
    }));
    if hits.len() > 5 {
        lines.push(t!("CredentialVault.reference_more", count = hits.len() - 5).to_string());
    }
    lines.join("\n")
}
