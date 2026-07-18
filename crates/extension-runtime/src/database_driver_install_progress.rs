use crate::extension_downloader::{DownloadProgress, DownloadProgressCallback};
use gpui::{
    App, AsyncApp, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled,
    WeakEntity, Window, div, px,
};
use gpui_component::{ActiveTheme, Sizable, WindowExt, progress::Progress, v_flex};
use rust_i18n::t;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

const START_PROGRESS: f32 = 5.0;
const INSTALLING_PROGRESS: f32 = 95.0;
const FINISHED_PROGRESS: f32 = 100.0;
const DRIVER_INSTALL_PROGRESS_POLL_INTERVAL: Duration = Duration::from_millis(100);

pub(super) struct DriverInstallProgressView {
    state: DriverInstallProgressState,
}

pub(super) struct DriverInstallProgressState {
    driver_id: SharedString,
    connection_name: SharedString,
    status: SharedString,
    progress_value: f32,
}

#[derive(Default)]
pub(super) struct DriverInstallProgressSnapshot {
    progress: VecDeque<DownloadProgress>,
}

impl DriverInstallProgressView {
    pub(super) fn new(driver_id: &str, connection_name: &str) -> Self {
        Self {
            state: DriverInstallProgressState::new(driver_id, connection_name),
        }
    }

    pub(super) fn apply_download_progress(&mut self, progress: DownloadProgress) {
        self.state.apply_download_progress(progress);
    }

    pub(super) fn mark_finished(&mut self) {
        self.state.mark_finished();
    }
}

impl DriverInstallProgressSnapshot {
    pub(super) fn set(&mut self, progress: DownloadProgress) {
        if matches!(progress, DownloadProgress::Bytes { .. })
            && matches!(self.progress.back(), Some(DownloadProgress::Bytes { .. }))
        {
            if let Some(last_progress) = self.progress.back_mut() {
                *last_progress = progress;
            }
            return;
        }
        self.progress.push_back(progress);
    }

    fn take(&mut self) -> Option<DownloadProgress> {
        self.progress.pop_front()
    }
}

pub(super) fn open_driver_install_progress_dialog(
    view: Entity<DriverInstallProgressView>,
    window: &mut Window,
    cx: &mut App,
) {
    let dialog_view = view.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title(t!("DriverInstall.title").to_string())
            .child(dialog_view.clone())
            .w(px(420.))
            .close_button(false)
            .overlay_closable(false)
            .keyboard(false)
    });
}

pub(super) fn sync_driver_install_progress(
    view: &WeakEntity<DriverInstallProgressView>,
    snapshot: &Arc<Mutex<DriverInstallProgressSnapshot>>,
    cx: &mut AsyncApp,
) {
    let Some(progress) = snapshot
        .lock()
        .ok()
        .and_then(|mut snapshot| snapshot.take())
    else {
        return;
    };
    let _ = view.update(cx, |view, cx| {
        view.apply_download_progress(progress);
        cx.notify();
    });
}

pub(super) fn mark_driver_install_finished(
    view: &WeakEntity<DriverInstallProgressView>,
    cx: &mut AsyncApp,
) {
    let _ = view.update(cx, |view, cx| {
        view.mark_finished();
        cx.notify();
    });
}

pub(super) fn watch_driver_install_progress<T>(
    progress_view: WeakEntity<DriverInstallProgressView>,
    progress_snapshot: Arc<Mutex<DriverInstallProgressSnapshot>>,
    progress_finished: Arc<AtomicBool>,
    cx: &mut Context<T>,
) where
    T: 'static,
{
    cx.spawn(async move |_: WeakEntity<T>, cx| {
        loop {
            if progress_finished.load(Ordering::Relaxed) {
                break;
            }
            cx.background_executor()
                .timer(DRIVER_INSTALL_PROGRESS_POLL_INTERVAL)
                .await;
            sync_driver_install_progress(&progress_view, &progress_snapshot, cx);
        }
        sync_driver_install_progress(&progress_view, &progress_snapshot, cx);
    })
    .detach();
}

pub(super) fn driver_install_progress_callback(
    progress_snapshot: Arc<Mutex<DriverInstallProgressSnapshot>>,
) -> DownloadProgressCallback {
    Arc::new(move |progress| {
        if let Ok(mut snapshot) = progress_snapshot.lock() {
            snapshot.set(progress);
        }
    })
}

impl DriverInstallProgressState {
    fn new(driver_id: &str, connection_name: &str) -> Self {
        Self {
            driver_id: driver_id.to_string().into(),
            connection_name: connection_name.to_string().into(),
            status: t!("DriverInstall.preparing").to_string().into(),
            progress_value: 0.0,
        }
    }

    fn apply_download_progress(&mut self, progress: DownloadProgress) {
        match progress {
            DownloadProgress::Started { .. } => {
                self.status = t!("DriverInstall.connecting_source").to_string().into();
                self.progress_value = START_PROGRESS;
            }
            DownloadProgress::Bytes { downloaded, total } => {
                self.progress_value = byte_progress_value(downloaded, total);
                self.status = t!(
                    "DriverInstall.downloading",
                    progress = format!("{:.0}", self.progress_value)
                )
                .to_string()
                .into();
            }
            DownloadProgress::Failed {
                error, retrying, ..
            } => {
                self.status = failed_source_status(&error, retrying).into();
                self.progress_value = START_PROGRESS;
            }
            DownloadProgress::Finished => self.mark_installing(),
        }
    }

    fn mark_installing(&mut self) {
        self.status = t!("DriverInstall.installing").to_string().into();
        self.progress_value = INSTALLING_PROGRESS;
    }

    fn mark_finished(&mut self) {
        self.status = t!("DriverInstall.finished").to_string().into();
        self.progress_value = FINISHED_PROGRESS;
    }

    #[cfg(test)]
    fn status(&self) -> &str {
        self.status.as_ref()
    }

    #[cfg(test)]
    fn progress_value(&self) -> f32 {
        self.progress_value
    }
}

impl Render for DriverInstallProgressView {
    fn render(&mut self, _window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        t!(
                            "DriverInstall.description",
                            connection = self.state.connection_name,
                            driver = self.state.driver_id
                        )
                        .to_string(),
                    ),
            )
            .child(
                Progress::new("database-driver-install-progress")
                    .value(self.state.progress_value)
                    .small(),
            )
            .child(div().text_sm().child(self.state.status.clone()))
    }
}

fn byte_progress_value(downloaded: u64, total: Option<u64>) -> f32 {
    let Some(total) = total.filter(|total| *total > 0) else {
        return START_PROGRESS;
    };
    ((downloaded as f32 / total as f32) * 100.0).clamp(START_PROGRESS, 90.0)
}

fn failed_source_status(error: &str, retrying: bool) -> String {
    match (
        error.contains("sha256 mismatch") || error.contains("verify sha256"),
        retrying,
    ) {
        (true, true) => t!("DriverInstall.checksum_failed_retrying").to_string(),
        (true, false) => t!("DriverInstall.checksum_failed").to_string(),
        (false, true) => t!("DriverInstall.source_failed_retrying").to_string(),
        (false, false) => t!("DriverInstall.source_failed").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_state_maps_download_events_to_dialog_values() {
        let mut state = DriverInstallProgressState::new("duckdb", "Local DuckDB");

        state.apply_download_progress(DownloadProgress::Started {
            url: "https://example.test/duckdb.tar.gz".to_string(),
        });
        assert_eq!(t!("DriverInstall.connecting_source"), state.status());
        assert_eq!(5.0, state.progress_value());

        state.apply_download_progress(DownloadProgress::Bytes {
            downloaded: 25,
            total: Some(100),
        });
        assert_eq!(
            t!("DriverInstall.downloading", progress = 25),
            state.status()
        );
        assert_eq!(25.0, state.progress_value());

        state.mark_installing();
        assert_eq!(t!("DriverInstall.installing"), state.status());
        assert_eq!(95.0, state.progress_value());

        state.mark_finished();
        assert_eq!(t!("DriverInstall.finished"), state.status());
        assert_eq!(100.0, state.progress_value());
    }

    #[test]
    fn progress_state_reports_failed_source_and_resets_for_retry() {
        let mut state = DriverInstallProgressState::new("duckdb", "Local DuckDB");

        state.apply_download_progress(DownloadProgress::Bytes {
            downloaded: 80,
            total: Some(100),
        });
        assert_eq!(80.0, state.progress_value());

        state.apply_download_progress(DownloadProgress::Failed {
            url: "https://onetcli.test.cn/extensions/duckdb.tar.gz".to_string(),
            error: "sha256 mismatch".to_string(),
            retrying: true,
        });
        assert_eq!(t!("DriverInstall.checksum_failed_retrying"), state.status());
        assert_eq!(5.0, state.progress_value());

        state.apply_download_progress(DownloadProgress::Started {
            url: "https://github.example.test/duckdb.tar.gz".to_string(),
        });
        assert_eq!(t!("DriverInstall.connecting_source"), state.status());
        assert_eq!(5.0, state.progress_value());
    }

    #[test]
    fn progress_snapshot_preserves_key_events_and_coalesces_byte_updates() {
        let mut snapshot = DriverInstallProgressSnapshot::default();

        snapshot.set(DownloadProgress::Started {
            url: "https://onetcli.test.cn/extensions/duckdb.tar.gz".to_string(),
        });
        snapshot.set(DownloadProgress::Bytes {
            downloaded: 20,
            total: Some(100),
        });
        snapshot.set(DownloadProgress::Bytes {
            downloaded: 40,
            total: Some(100),
        });
        snapshot.set(DownloadProgress::Failed {
            url: "https://onetcli.test.cn/extensions/duckdb.tar.gz".to_string(),
            error: "sha256 mismatch".to_string(),
            retrying: true,
        });
        snapshot.set(DownloadProgress::Started {
            url: "https://github.example.test/duckdb.tar.gz".to_string(),
        });

        assert!(matches!(
            snapshot.take(),
            Some(DownloadProgress::Started { url })
                if url == "https://onetcli.test.cn/extensions/duckdb.tar.gz"
        ));
        assert!(matches!(
            snapshot.take(),
            Some(DownloadProgress::Bytes {
                downloaded: 40,
                total: Some(100),
            })
        ));
        assert!(matches!(
            snapshot.take(),
            Some(DownloadProgress::Failed { retrying: true, .. })
        ));
        assert!(matches!(
            snapshot.take(),
            Some(DownloadProgress::Started { url })
                if url == "https://github.example.test/duckdb.tar.gz"
        ));
        assert_eq!(None, snapshot.take());
    }
}
