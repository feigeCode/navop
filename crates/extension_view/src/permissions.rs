use std::path::PathBuf;

use gpui::{App, AppContext, ClickEvent, Entity, IntoElement, ParentElement, Styled, Window, div};
use gpui_component::{
    ActiveTheme, WindowExt, dialog::DialogButtonProps, notification::Notification, v_flex,
};
use rust_i18n::t;

use crate::{
    DownloadedMarketplaceExtension, ExtensionManagerView, MarketplaceInstallOutcome,
    PermissionReviewModel,
    status_message::{format_notification_error, format_status_error},
};

impl ExtensionManagerView {
    pub(crate) fn finish_marketplace_outcome(
        &mut self,
        outcome: MarketplaceInstallOutcome,
        entity: Entity<ExtensionManagerView>,
        window: &mut Window,
        cx: &mut App,
    ) {
        match outcome {
            MarketplaceInstallOutcome::Installed(summary) => {
                self.status = t!("Extension.installed_name", name = summary.name.clone())
                    .to_string()
                    .into();
                self.refresh_after_extension_change(summary.kind, cx);
                window.push_notification(
                    Notification::success(t!("Extension.install_complete").to_string()),
                    cx,
                );
            }
            MarketplaceInstallOutcome::NeedsPermission(downloaded) => {
                self.status = t!(
                    "Extension.permission_required",
                    name = downloaded.entry.name.clone()
                )
                .to_string()
                .into();
                self.open_permission_dialog(downloaded, entity, window, cx);
            }
        }
    }

    fn open_permission_dialog(
        &mut self,
        downloaded: DownloadedMarketplaceExtension,
        entity: Entity<ExtensionManagerView>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let staging_for_ok = downloaded.staging.clone();
        let staging_for_cancel = downloaded.staging.clone();
        let entry_name = downloaded.entry.name.clone();
        let target_extension_id = downloaded.target_extension_id.clone();
        window.open_dialog(cx, move |dialog, _window, cx| {
            let entity_for_ok = entity.clone();
            let entity_for_cancel = entity.clone();
            let ok_staging = staging_for_ok.clone();
            let ok_extension_id = target_extension_id.clone();
            let cancel_staging = staging_for_cancel.clone();
            dialog
                .title(t!("Extension.confirm_install", name = entry_name.clone()).to_string())
                .width(gpui::px(520.0))
                .child(permission_review_body(&downloaded.review, cx))
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("Extension.allow_and_install").to_string())
                        .cancel_text(t!("Common.cancel").to_string())
                        .show_cancel(true),
                )
                .on_ok(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    entity_for_ok.update(cx, |view: &mut ExtensionManagerView, cx| {
                        view.install_confirmed_staging(
                            ok_staging.clone(),
                            ok_extension_id.clone(),
                            entity_for_ok.clone(),
                            window,
                            cx,
                        );
                    });
                    true
                })
                .on_cancel(move |_, _, cx| {
                    cleanup_staging(cancel_staging.clone());
                    entity_for_cancel.update(cx, |view: &mut ExtensionManagerView, cx| {
                        view.busy = None;
                        view.status = t!("Extension.install_cancelled").to_string().into();
                        cx.notify();
                    });
                    true
                })
        });
    }

    fn install_confirmed_staging(
        &mut self,
        staging: PathBuf,
        extension_id: String,
        entity: Entity<ExtensionManagerView>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let close_task = crate::shell::close_shell_extension(&extension_id, window, cx);
        let host = self.host.clone();
        let refresh_host = host.clone();
        let entity = entity.downgrade();
        let window_handle = window.window_handle();
        let gate_id = extension_id.clone();
        cx.spawn(async move |cx: &mut gpui::AsyncApp| {
            if !close_task.await {
                let _ = cx.update(|cx| {
                    crate::shell::finish_shell_extension(&gate_id, cx);
                    cleanup_staging(staging);
                });
                return;
            }
            let install =
                cx.background_spawn(async move { host.install_confirmed_staging(staging) });
            let outcome = install.await;
            let mut view_alive = false;
            let updated = cx.update_window(window_handle, |_, window, cx| {
                let Some(entity) = entity.upgrade() else {
                    return;
                };
                view_alive = true;
                entity.update(cx, |view, cx| match outcome {
                    Ok(summary) => {
                        view.status = t!("Extension.installed_name", name = summary.name.clone())
                            .to_string()
                            .into();
                        view.refresh_after_extension_change(summary.kind, cx);
                        window.push_notification(
                            Notification::success(t!("Extension.install_complete").to_string()),
                            cx,
                        );
                    }
                    Err(err) => {
                        view.busy = None;
                        let message = format_notification_error(
                            &t!("Extension.install_failed").to_string(),
                            &err,
                        );
                        view.status = format_status_error(
                            &t!("Extension.install_failed_short").to_string(),
                            &err,
                        )
                        .into();
                        window.push_notification(Notification::error(message).autohide(false), cx);
                    }
                });
            });
            if updated.is_err() || !view_alive {
                let _ = cx.update(|cx| {
                    refresh_host
                        .refresh_after_extension_change(crate::ExtensionKind::Composite, cx);
                    crate::shell::finish_shell_extension(&gate_id, cx);
                });
            } else {
                let _ = cx.update(|cx| crate::shell::finish_shell_extension(&gate_id, cx));
            }
        })
        .detach();
    }
}

fn cleanup_staging(staging: PathBuf) {
    let _ = std::fs::remove_dir_all(staging);
}

fn permission_review_body(review: &PermissionReviewModel, cx: &App) -> impl IntoElement {
    v_flex()
        .gap_3()
        .p_4()
        .child(
            div().text_sm().text_color(cx.theme().foreground).child(
                t!(
                    "Extension.high_risk_permission_summary",
                    count = review.high_risk_count
                )
                .to_string(),
            ),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(review.summary.clone()),
        )
}
