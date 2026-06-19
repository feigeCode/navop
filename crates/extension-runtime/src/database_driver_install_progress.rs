use crate::extension_downloader::{DownloadProgress, DownloadProgressCallback};
use gpui::{
    App, AsyncApp, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled,
    WeakEntity, Window, div, px,
};
use gpui_component::{ActiveTheme, Sizable, WindowExt, progress::Progress, v_flex};
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
            .title("正在安装数据库驱动")
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
            status: "准备下载驱动...".into(),
            progress_value: 0.0,
        }
    }

    fn apply_download_progress(&mut self, progress: DownloadProgress) {
        match progress {
            DownloadProgress::Started { .. } => {
                self.status = "正在连接下载源...".into();
                self.progress_value = START_PROGRESS;
            }
            DownloadProgress::Bytes { downloaded, total } => {
                self.progress_value = byte_progress_value(downloaded, total);
                self.status = format!("正在下载驱动 {:.0}%", self.progress_value).into();
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
        self.status = "正在安装驱动...".into();
        self.progress_value = INSTALLING_PROGRESS;
    }

    fn mark_finished(&mut self) {
        self.status = "安装完成，正在打开连接...".into();
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
                    .child(format!(
                        "连接「{}」需要安装「{}」数据库驱动。",
                        self.state.connection_name, self.state.driver_id
                    )),
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

fn failed_source_status(error: &str, retrying: bool) -> &'static str {
    match (
        error.contains("sha256 mismatch") || error.contains("verify sha256"),
        retrying,
    ) {
        (true, true) => "当前下载源校验失败，正在切换到下一个源...",
        (true, false) => "下载源校验失败，请稍后重试。",
        (false, true) => "当前下载源失败，正在切换到下一个源...",
        (false, false) => "下载源失败，请稍后重试。",
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
        assert_eq!("正在连接下载源...", state.status());
        assert_eq!(5.0, state.progress_value());

        state.apply_download_progress(DownloadProgress::Bytes {
            downloaded: 25,
            total: Some(100),
        });
        assert_eq!("正在下载驱动 25%", state.status());
        assert_eq!(25.0, state.progress_value());

        state.mark_installing();
        assert_eq!("正在安装驱动...", state.status());
        assert_eq!(95.0, state.progress_value());

        state.mark_finished();
        assert_eq!("安装完成，正在打开连接...", state.status());
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
        assert_eq!("当前下载源校验失败，正在切换到下一个源...", state.status());
        assert_eq!(5.0, state.progress_value());

        state.apply_download_progress(DownloadProgress::Started {
            url: "https://github.example.test/duckdb.tar.gz".to_string(),
        });
        assert_eq!("正在连接下载源...", state.status());
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
