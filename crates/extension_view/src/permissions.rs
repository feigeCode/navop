use std::path::PathBuf;

use gpui::{App, Entity, IntoElement, ParentElement, Styled, Window, div};
use gpui_component::{
    ActiveTheme, WindowExt, dialog::DialogButtonProps, notification::Notification, v_flex,
};

use crate::{
    DownloadedMarketplaceExtension, ExtensionManagerView, MarketplaceInstallOutcome,
    PermissionReviewModel,
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
                self.status = format!("已安装 {}", summary.name).into();
                self.refresh_after_extension_change(cx);
                window.push_notification(Notification::success("扩展安装完成"), cx);
            }
            MarketplaceInstallOutcome::NeedsPermission(downloaded) => {
                self.status = format!("{} 需要权限确认", downloaded.entry.name).into();
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
                .title(format!("确认安装 {entry_name}"))
                .width(gpui::px(520.0))
                .child(permission_review_body(&downloaded.review, cx))
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("允许并安装")
                        .cancel_text("取消"),
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
                        view.status = "已取消安装".into();
                        cx.notify();
                    });
                    true
                })
        });
    }

    fn install_confirmed_staging(&mut self, staging: PathBuf, window: &mut Window, cx: &mut App) {
        match self.host.install_confirmed_staging(staging) {
            Ok(summary) => {
                self.status = format!("已安装 {}", summary.name).into();
                self.refresh_after_extension_change(cx);
                window.push_notification(Notification::success("扩展安装完成"), cx);
            }
            Err(err) => {
                self.busy = None;
                self.status = format!("安装失败: {err:?}").into();
                window.push_notification(Notification::error("扩展安装失败"), cx);
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
            div()
                .text_sm()
                .text_color(cx.theme().foreground)
                .child(format!(
                    "该扩展声明了 {} 个高危权限。请确认你信任该扩展来源。",
                    review.high_risk_count
                )),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(review.summary.clone()),
        )
}
