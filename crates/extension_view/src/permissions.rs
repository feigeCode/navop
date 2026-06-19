use std::path::PathBuf;

use gpui::{App, Entity, IntoElement, ParentElement, Styled, Window, div};
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
                self.refresh_after_extension_change(cx);
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
        window.open_dialog(cx, move |dialog, _window, cx| {
            let entity_for_ok = entity.clone();
            let entity_for_cancel = entity.clone();
            let ok_staging = staging_for_ok.clone();
            let cancel_staging = staging_for_cancel.clone();
            dialog
                .title(t!("Extension.confirm_install", name = entry_name.clone()).to_string())
                .width(gpui::px(520.0))
                .child(permission_review_body(&downloaded.review, cx))
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("Extension.allow_and_install").to_string())
                        .cancel_text(t!("Common.cancel").to_string()),
                )
                .on_ok(move |_, window, cx| {
                    entity_for_ok.update(cx, |view: &mut ExtensionManagerView, cx| {
                        view.install_confirmed_staging(ok_staging.clone(), window, cx);
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

    fn install_confirmed_staging(&mut self, staging: PathBuf, window: &mut Window, cx: &mut App) {
        match self.host.install_confirmed_staging(staging) {
            Ok(summary) => {
                self.status = t!("Extension.installed_name", name = summary.name.clone())
                    .to_string()
                    .into();
                self.refresh_after_extension_change(cx);
                window.push_notification(
                    Notification::success(t!("Extension.install_complete").to_string()),
                    cx,
                );
            }
            Err(err) => {
                self.busy = None;
                let message =
                    format_notification_error(&t!("Extension.install_failed").to_string(), &err);
                self.status =
                    format_status_error(&t!("Extension.install_failed_short").to_string(), &err)
                        .into();
                window.push_notification(Notification::error(message).autohide(false), cx);
            }
        }
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
