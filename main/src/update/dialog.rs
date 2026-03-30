use std::path::PathBuf;

use gpui::prelude::FluentBuilder;
use gpui::{AppContext, Context, IntoElement, ParentElement, Render, Styled, Window, div, px};
use gpui_component::{
    ActiveTheme, WindowExt, dialog::DialogButtonProps, progress::Progress, v_flex,
};
use rust_i18n::t;

use super::UpdateDialogInfo;
use super::download::{build_download_path, download_update_file};
use super::install::start_install_update;
use super::util::{UpdateInstallAction, format_bytes};

pub(super) fn show_update_dialog(window: &mut Window, info: UpdateDialogInfo, cx: &mut gpui::App) {
    let view = cx.new(|_cx| UpdateDialogView::new(info));
    let view_for_ok = view.clone();
    let view_for_cancel = view.clone();

    window.open_dialog(cx, move |dialog, _window, cx| {
        let view_for_ok = view_for_ok.clone();
        let view_for_cancel = view_for_cancel.clone();
        let ok_text = {
            let state = view.read(cx);
            if state.applying {
                t!("Update.action_applying")
            } else if state.downloading {
                t!("Update.action_downloading")
            } else if state.completed {
                t!("Update.action_install")
            } else {
                t!("Update.action_download")
            }
        };
        dialog
            .title(t!("Update.title").to_string())
            .width(px(460.))
            .child(view.clone())
            .confirm()
            .button_props(
                DialogButtonProps::default()
                    .ok_text(ok_text)
                    .cancel_text(t!("Update.later")),
            )
            .on_ok(move |_, window, cx| {
                view_for_ok
                    .clone()
                    .update(cx, |view: &mut UpdateDialogView, cx| {
                        view.on_ok_action(window, cx);
                    });
                false
            })
            .on_cancel(move |_, window, cx| {
                let state = view_for_cancel.clone().read(cx);
                if state.downloading || state.applying {
                    window.push_notification(t!("Update.downloading_blocked").to_string(), cx);
                    return false;
                }
                true
            })
    });
}

struct UpdateDialogView {
    info: UpdateDialogInfo,
    downloading: bool,
    applying: bool,
    completed: bool,
    progress: f32,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    downloaded_path: Option<PathBuf>,
    status_message: String,
    error_message: Option<String>,
}

impl UpdateDialogView {
    fn new(info: UpdateDialogInfo) -> Self {
        Self {
            info,
            downloading: false,
            applying: false,
            completed: false,
            progress: 0.0,
            downloaded_bytes: 0,
            total_bytes: None,
            downloaded_path: None,
            status_message: t!("Update.ready").to_string(),
            error_message: None,
        }
    }

    fn on_ok_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.downloading {
            window.push_notification(t!("Update.downloading_blocked").to_string(), cx);
            return;
        }

        if self.completed {
            self.apply_downloaded_update(cx);
            return;
        }

        self.start_download(cx);
    }

    fn start_download(&mut self, cx: &mut Context<Self>) {
        if self.downloading || self.completed || self.applying {
            return;
        }

        let Some(download_url) = self.info.download_url.clone() else {
            self.error_message = Some(t!("Update.missing_download_url").to_string());
            self.status_message = t!("Update.download_failed").to_string();
            cx.notify();
            return;
        };

        let download_path = match build_download_path(&self.info.latest_version, &download_url) {
            Ok(path) => path,
            Err(err) => {
                self.error_message = Some(err);
                self.status_message = t!("Update.download_failed").to_string();
                cx.notify();
                return;
            }
        };

        self.downloading = true;
        self.completed = false;
        self.progress = 0.0;
        self.downloaded_bytes = 0;
        self.total_bytes = None;
        self.downloaded_path = None;
        self.error_message = None;
        self.status_message = t!("Update.downloading").to_string();
        cx.notify();

        let http_client = cx.http_client();

        cx.spawn(async move |this, cx| {
            let download_result = download_update_file(
                http_client,
                &download_url,
                &download_path,
                |downloaded, total| {
                    let _ = this.update(cx, |view, cx| {
                        view.update_progress(downloaded, total, cx);
                    });
                },
            )
            .await;

            match download_result {
                Ok(()) => {
                    let _ = this.update(cx, |view, cx| {
                        view.completed = true;
                        view.downloading = false;
                        view.applying = false;
                        view.progress = 100.0;
                        view.downloaded_path = Some(download_path.clone());
                        view.status_message = t!("Update.download_complete").to_string();
                        cx.notify();
                    });
                }
                Err(err) => {
                    let _ = this.update(cx, |view, cx| {
                        view.downloading = false;
                        view.applying = false;
                        view.downloaded_path = None;
                        view.error_message = Some(err);
                        view.status_message = t!("Update.download_failed").to_string();
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    fn apply_downloaded_update(&mut self, cx: &mut Context<Self>) {
        if self.downloading || self.applying {
            return;
        }

        let Some(download_path) = self.downloaded_path.clone() else {
            self.error_message = Some(t!("Update.missing_download_file").to_string());
            self.status_message = t!("Update.apply_failed").to_string();
            self.completed = false;
            self.downloaded_path = None;
            cx.notify();
            return;
        };

        self.applying = true;
        self.error_message = None;
        self.status_message = t!("Update.applying").to_string();
        cx.notify();

        cx.spawn(
            async move |this, cx| match start_install_update(download_path) {
                Ok(UpdateInstallAction::Quit) => {
                    let _ = cx.update(|cx| {
                        cx.quit();
                    });
                }
                Ok(UpdateInstallAction::Noop) => {
                    let _ = this.update(cx, |view, cx| {
                        view.applying = false;
                        view.status_message = t!("Update.download_complete").to_string();
                        cx.notify();
                    });
                }
                Err(err) => {
                    let _ = this.update(cx, |view, cx| {
                        view.applying = false;
                        view.error_message = Some(err);
                        view.status_message = t!("Update.apply_failed").to_string();
                        cx.notify();
                    });
                }
            },
        )
        .detach();
    }

    fn update_progress(&mut self, downloaded: u64, total: Option<u64>, cx: &mut Context<Self>) {
        self.downloaded_bytes = downloaded;
        self.total_bytes = total;
        if let Some(total) = total
            && total > 0
        {
            self.progress = ((downloaded as f32 / total as f32) * 100.0).min(100.0);
        }
        cx.notify();
    }

    fn progress_value(&self) -> f32 {
        if self.total_bytes.is_some() {
            self.progress
        } else {
            -1.0
        }
    }

    fn progress_label(&self) -> String {
        match self.total_bytes {
            Some(total) if total > 0 => format!(
                "{} / {}",
                format_bytes(self.downloaded_bytes),
                format_bytes(total)
            ),
            _ => format_bytes(self.downloaded_bytes),
        }
    }
}

impl Render for UpdateDialogView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let message = t!(
            "Update.message",
            latest = self.info.latest_version,
            current = self.info.current_version
        )
        .to_string();

        let release_notes = self
            .info
            .release_notes
            .clone()
            .filter(|notes| !notes.trim().is_empty());

        let show_progress = self.downloading || self.completed;
        let status_message = if let Some(error) = self.error_message.as_ref() {
            format!("{}: {}", t!("Update.error_prefix"), error)
        } else {
            self.status_message.clone()
        };

        v_flex()
            .gap_3()
            .p_4()
            .child(
                div()
                    .text_base()
                    .text_color(cx.theme().foreground)
                    .child(message),
            )
            .when(show_progress, |this| {
                this.child(
                    v_flex()
                        .gap_2()
                        .child(Progress::new("update-progress").value(self.progress_value()))
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(self.progress_label()),
                        ),
                )
            })
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(status_message),
            )
            .when(release_notes.is_some(), |this| {
                let notes = release_notes.clone().unwrap_or_default();
                this.child(
                    v_flex()
                        .gap_2()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(t!("Update.release_notes").to_string()),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(notes),
                        ),
                )
            })
    }
}
