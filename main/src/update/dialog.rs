use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, AsyncApp, Context, FocusHandle, Focusable, IntoElement, ParentElement, Render,
    Styled, WeakEntity, Window, div,
};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, TitleBar, WindowExt,
    button::{Button, ButtonVariants as _},
    clipboard::Clipboard,
    h_flex,
    progress::Progress,
    v_flex,
};
use one_core::gpui_tokio::Tokio;
use one_core::popup_window::{PopupWindowOptions, open_popup_window};
use rust_i18n::t;

use super::UpdateDialogInfo;
use super::download::{build_download_path, download_update_file, verify_sha256};
use super::install::start_install_update;
use super::util::{UpdateInstallAction, format_bytes};

const DOWNLOAD_PROGRESS_POLL_INTERVAL: Duration = Duration::from_millis(100);

pub(super) fn show_update_dialog(info: UpdateDialogInfo, cx: &mut App) {
    open_popup_window(
        PopupWindowOptions::new(t!("Update.title").to_string()).size(480.0, 300.0),
        move |_window, cx| cx.new(|cx| UpdateDialogView::new(info, cx)),
        cx,
    );
}

struct UpdateDialogView {
    focus_handle: FocusHandle,
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
    expected_sha256: Option<String>,
}

#[derive(Clone, Copy, Default)]
struct DownloadProgressSnapshot {
    downloaded: u64,
    total: Option<u64>,
}

impl UpdateDialogView {
    fn new(info: UpdateDialogInfo, cx: &mut Context<Self>) -> Self {
        let expected_sha256 = info.expected_sha256.clone();
        Self {
            focus_handle: cx.focus_handle(),
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
            expected_sha256,
        }
    }

    fn on_ok_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.downloading || self.applying {
            window.push_notification(t!("Update.downloading_blocked").to_string(), cx);
            return;
        }

        if self.completed {
            self.apply_downloaded_update(cx);
            return;
        }

        self.start_download(cx);
    }

    fn on_cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.downloading || self.applying {
            window.push_notification(t!("Update.downloading_blocked").to_string(), cx);
            return;
        }

        window.remove_window();
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
        let download_path_for_task = download_path.clone();
        let progress_state = Arc::new(Mutex::new(DownloadProgressSnapshot::default()));
        let progress_state_for_task = Arc::clone(&progress_state);
        let progress_finished = Arc::new(AtomicBool::new(false));
        let progress_finished_for_watcher = Arc::clone(&progress_finished);
        let view = cx.entity().downgrade();
        let expected_sha256 = self.expected_sha256.clone();

        cx.spawn(async move |_this: WeakEntity<Self>, cx: &mut AsyncApp| {
            loop {
                if progress_finished_for_watcher.load(Ordering::Relaxed) {
                    break;
                }

                cx.background_executor()
                    .timer(DOWNLOAD_PROGRESS_POLL_INTERVAL)
                    .await;
                sync_download_progress(&view, &progress_state, cx);
            }

            sync_download_progress(&view, &progress_state, cx);
        })
        .detach();

        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let download_task = Tokio::spawn(cx, async move {
                download_update_file(
                    http_client,
                    &download_url,
                    &download_path_for_task,
                    move |downloaded, total| {
                        if let Ok(mut progress) = progress_state_for_task.lock() {
                            *progress = DownloadProgressSnapshot { downloaded, total };
                        }
                    },
                )
                .await
            });

            let download_result = match download_task.await {
                Ok(result) => result,
                Err(err) => Err(format!("下载任务执行失败: {}", err)),
            };
            progress_finished.store(true, Ordering::Relaxed);

            match download_result {
                Ok(()) => {
                    // SHA256 完整性校验
                    if let Some(expected) = &expected_sha256 {
                        if let Err(err) = verify_sha256(&download_path, expected) {
                            let _ = std::fs::remove_file(&download_path);
                            let _ = this.update(cx, |view, cx| {
                                view.downloading = false;
                                view.applying = false;
                                view.downloaded_path = None;
                                view.error_message = Some(err);
                                view.status_message = t!("Update.download_failed").to_string();
                                cx.notify();
                            });
                            return;
                        }
                    }

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

fn sync_download_progress(
    view: &WeakEntity<UpdateDialogView>,
    progress_state: &Arc<Mutex<DownloadProgressSnapshot>>,
    cx: &mut AsyncApp,
) {
    let snapshot = match progress_state.lock() {
        Ok(progress) => *progress,
        Err(_) => return,
    };

    let _ = view.update(cx, |view, cx| {
        view.update_progress(snapshot.downloaded, snapshot.total, cx);
    });
}

impl Focusable for UpdateDialogView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
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

        let show_progress = self.downloading || self.completed;
        let status_message = if let Some(error) = self.error_message.as_ref() {
            format!("{}: {}", t!("Update.error_prefix"), error)
        } else {
            self.status_message.clone()
        };
        let action_text = if self.applying {
            t!("Update.action_applying").to_string()
        } else if self.downloading {
            t!("Update.action_downloading").to_string()
        } else if self.completed {
            t!("Update.action_install").to_string()
        } else {
            t!("Update.action_download").to_string()
        };

        let release_page_url = self.info.release_page_url.clone();

        v_flex()
            .gap_3()
            .size_full()
            .bg(cx.theme().background)
            .child(
                TitleBar::new().child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .flex_1()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child(t!("Update.title").to_string()),
                ),
            )
            .child(
                div().flex_1().p_4().child(
                    v_flex()
                        .gap_3()
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
                                    .child(
                                        Progress::new("update-progress")
                                            .value(self.progress_value()),
                                    )
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
                        .when(release_page_url.is_some(), |this| {
                            let url = release_page_url.clone().unwrap_or_default();
                            let url_for_open = url.clone();
                            this.child(
                                v_flex()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(t!("Update.open_release_page").to_string()),
                                    )
                                    .child(
                                        h_flex()
                                            .items_center()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(cx.theme().link)
                                                    .child(url.clone()),
                                            )
                                            .child(Clipboard::new("release-url-copy").value(url))
                                            .child(
                                                Button::new("release-url-open")
                                                    .xsmall()
                                                    .ghost()
                                                    .icon(gpui_component::IconName::ExternalLink)
                                                    .on_click(move |_, _, cx| {
                                                        cx.open_url(&url_for_open);
                                                    }),
                                            ),
                                    ),
                            )
                        }),
                ),
            )
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .p_4()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        Button::new("update-later")
                            .small()
                            .label(t!("Update.later").to_string())
                            .disabled(self.downloading || self.applying)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_cancel(window, cx);
                            })),
                    )
                    .child(
                        Button::new("update-action")
                            .small()
                            .primary()
                            .label(action_text)
                            .disabled(self.downloading || self.applying)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_ok_action(window, cx);
                            })),
                    ),
            )
    }
}
