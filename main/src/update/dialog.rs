use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use super::UpdateDialogInfo;
use super::download::{
    DOWNLOAD_CANCELLED_ERROR, build_download_path, download_update_file_from_sources_cancellable,
    verify_sha256,
};
use super::install::start_install_update;
use super::util::{UpdateInstallAction, format_bytes};
use crate::onetcli_app::shutdown_ssh_sessions_and_quit;
use crate::setting_tab::AppSettings;
use crate::update::github_release::GITHUB_LATEST_RELEASE_URL;
use gpui::{
    App, AppContext, AsyncApp, Context, FocusHandle, Focusable, WeakEntity, Window, px, size,
};
use gpui_component::WindowExt;
use one_core::gpui_tokio::Tokio;
use one_core::popup_window::{PopupWindowOptions, open_popup_window};
use rust_i18n::t;

#[path = "dialog_render.rs"]
mod dialog_render;

const DOWNLOAD_PROGRESS_POLL_INTERVAL: Duration = Duration::from_millis(100);
const AVAILABLE_WINDOW_WIDTH: f32 = 560.0;
const AVAILABLE_WINDOW_HEIGHT: f32 = 450.0;
const DOWNLOAD_WINDOW_WIDTH: f32 = 440.0;
const DOWNLOAD_WINDOW_HEIGHT: f32 = 170.0;

pub(super) fn show_update_dialog(info: UpdateDialogInfo, cx: &mut App) {
    open_popup_window(
        PopupWindowOptions::new(t!("Update.title").to_string())
            .size(AVAILABLE_WINDOW_WIDTH, AVAILABLE_WINDOW_HEIGHT),
        move |_window, cx| cx.new(|cx| UpdateDialogView::new(info, cx)),
        cx,
    );
}

struct UpdateDialogView {
    focus_handle: FocusHandle,
    info: UpdateDialogInfo,
    downloading: bool,
    cancelling: bool,
    cancelled: bool,
    applying: bool,
    completed: bool,
    auto_update: bool,
    progress: f32,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    downloaded_path: Option<PathBuf>,
    status_message: String,
    error_message: Option<String>,
    expected_sha256: Option<String>,
    cancel_requested: Option<Arc<AtomicBool>>,
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
            cancelling: false,
            cancelled: false,
            applying: false,
            completed: false,
            auto_update: AppSettings::global(cx).auto_update,
            progress: 0.0,
            downloaded_bytes: 0,
            total_bytes: None,
            downloaded_path: None,
            status_message: t!("Update.ready").to_string(),
            error_message: None,
            expected_sha256,
            cancel_requested: None,
        }
    }

    fn on_ok_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.downloading || self.applying {
            window.push_notification(t!("Update.downloading_blocked").to_string(), cx);
            return;
        }

        if self.completed && self.info.is_local_simulation {
            window.remove_window();
            return;
        }

        if one_core::app_paths::is_portable() && !self.info.is_local_simulation {
            cx.open_url(GITHUB_LATEST_RELEASE_URL);
            return;
        }

        if self.completed {
            self.apply_downloaded_update(cx);
            return;
        }

        self.start_download(window, cx);
    }

    fn on_cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.downloading {
            if let Some(cancel_requested) = self.cancel_requested.as_ref() {
                cancel_requested.store(true, Ordering::Relaxed);
                self.cancelling = true;
                self.status_message = t!("Update.cancelling").to_string();
                cx.notify();
            }
            return;
        }

        if self.applying {
            window.push_notification(t!("Update.downloading_blocked").to_string(), cx);
            return;
        }

        window.remove_window();
    }

    fn start_download(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.downloading || self.completed || self.applying {
            return;
        }

        let download_urls = self.info.download_urls();
        let Some(download_url) = download_urls.first() else {
            self.error_message = Some(t!("Update.missing_download_url").to_string());
            self.status_message = t!("Update.download_failed").to_string();
            cx.notify();
            return;
        };

        let download_path = match build_download_path(&self.info.latest_version, download_url) {
            Ok(path) => path,
            Err(err) => {
                self.error_message = Some(err);
                self.status_message = t!("Update.download_failed").to_string();
                cx.notify();
                return;
            }
        };

        self.downloading = true;
        self.cancelling = false;
        self.cancelled = false;
        self.completed = false;
        self.progress = 0.0;
        self.downloaded_bytes = 0;
        self.total_bytes = None;
        self.downloaded_path = None;
        self.error_message = None;
        self.status_message = t!("Update.downloading").to_string();
        window.resize(size(px(DOWNLOAD_WINDOW_WIDTH), px(DOWNLOAD_WINDOW_HEIGHT)));
        let window_title = if self.info.is_local_simulation {
            t!("Update.simulation_title")
        } else {
            t!("Update.downloading_title")
        };
        window.set_window_title(&window_title);
        cx.notify();

        let http_client = cx.http_client();
        let download_path_for_task = download_path.clone();
        let progress_state = Arc::new(Mutex::new(DownloadProgressSnapshot::default()));
        let progress_state_for_task = Arc::clone(&progress_state);
        let progress_finished = Arc::new(AtomicBool::new(false));
        let progress_finished_for_watcher = Arc::clone(&progress_finished);
        let view = cx.entity().downgrade();
        let expected_sha256 = self.expected_sha256.clone();
        let cancel_requested = Arc::new(AtomicBool::new(false));
        let cancel_requested_for_task = Arc::clone(&cancel_requested);
        self.cancel_requested = Some(cancel_requested);

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
                download_update_file_from_sources_cancellable(
                    http_client,
                    &download_urls,
                    &download_path_for_task,
                    cancel_requested_for_task,
                    move |downloaded, total| {
                        if let Ok(mut progress) = progress_state_for_task.lock() {
                            *progress = DownloadProgressSnapshot { downloaded, total };
                        }
                    },
                )
                .await
            });

            let download_result = download_task
                .await
                .unwrap_or_else(|err| Err(format!("下载任务执行失败: {}", err)));
            progress_finished.store(true, Ordering::Relaxed);

            match download_result {
                Ok(()) => {
                    // SHA256 完整性校验
                    if let Some(expected) = &expected_sha256 {
                        if let Err(err) = verify_sha256(&download_path, expected) {
                            let _ = std::fs::remove_file(&download_path);
                            let _ = this.update(cx, |view, cx| {
                                view.downloading = false;
                                view.cancelling = false;
                                view.cancelled = false;
                                view.applying = false;
                                view.cancel_requested = None;
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
                        view.cancelling = false;
                        view.cancelled = false;
                        view.applying = false;
                        view.cancel_requested = None;
                        view.progress = 100.0;
                        view.downloaded_path = Some(download_path.clone());
                        view.status_message = if view.info.is_local_simulation {
                            t!("Update.simulation_complete").to_string()
                        } else {
                            t!("Update.download_complete").to_string()
                        };
                        cx.notify();
                    });
                }
                Err(err) if err == DOWNLOAD_CANCELLED_ERROR => {
                    let _ = this.update(cx, |view, cx| {
                        view.downloading = false;
                        view.cancelling = false;
                        view.cancelled = true;
                        view.applying = false;
                        view.cancel_requested = None;
                        view.downloaded_path = None;
                        view.error_message = None;
                        view.status_message = t!("Update.cancelled").to_string();
                        cx.notify();
                    });
                }
                Err(err) => {
                    let _ = this.update(cx, |view, cx| {
                        view.downloading = false;
                        view.cancelling = false;
                        view.cancelled = false;
                        view.applying = false;
                        view.cancel_requested = None;
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

        let Some(download_path) = self.downloaded_path.clone().filter(|path| path.is_file()) else {
            self.error_message = Some(t!("Update.missing_download_file").to_string());
            self.status_message = t!("Update.apply_failed").to_string();
            self.completed = false;
            self.downloaded_path = None;
            cx.notify();
            return;
        };

        self.applying = true;
        self.cancelling = false;
        self.error_message = None;
        self.status_message = t!("Update.applying").to_string();
        cx.notify();

        cx.spawn(
            async move |this, cx| match start_install_update(download_path) {
                Ok(UpdateInstallAction::Quit) => {
                    let _ = cx.update(|cx| {
                        shutdown_ssh_sessions_and_quit(cx, "update installation");
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

    fn set_auto_update(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.auto_update = enabled;
        AppSettings::update_and_save(cx, |settings| {
            settings.auto_update = enabled;
        });
        cx.notify();
    }

    fn skip_version(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let latest_version = self.info.latest_version.clone();
        AppSettings::update_and_save(cx, |settings| {
            settings.skipped_update_version = Some(latest_version);
        });
        window.remove_window();
    }

    fn status_message(&self) -> String {
        if let Some(error) = self.error_message.as_ref() {
            format!("{}: {}", t!("Update.error_prefix"), error)
        } else {
            self.status_message.clone()
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
