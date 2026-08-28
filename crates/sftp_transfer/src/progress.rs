use std::sync::{Arc, Mutex};

use gpui::{AsyncApp, Task};
use sftp::{ProgressCallback, TransferProgress};

use super::{SftpTransferExecutor, SftpTransferId};

const PROGRESS_BRIDGE_INTERVAL_MS: u64 = 50;

pub(super) struct TransferProgressBridge {
    pub callback: ProgressCallback,
    pub task: Task<()>,
}

pub(super) fn create_progress_bridge(
    id: SftpTransferId,
    cx: &mut gpui::Context<SftpTransferExecutor>,
) -> TransferProgressBridge {
    let latest = Arc::new(Mutex::new(None));
    let (wakeup, mut receiver) = tokio::sync::mpsc::channel(1);
    let callback_latest = latest.clone();
    let callback = Box::new(move |progress| {
        let Ok(mut latest) = callback_latest.lock() else {
            return;
        };
        *latest = Some(progress);
        drop(latest);
        let _ = wakeup.try_send(());
    });
    let task = cx.spawn(async move |executor, cx| {
        while receiver.recv().await.is_some() {
            coalesce_progress(cx, &mut receiver).await;
            let progress = latest.lock().ok().and_then(|mut value| value.take());
            let Some(progress) = progress else {
                continue;
            };
            if executor
                .update(cx, |executor, cx| {
                    executor.update_progress(id, progress, cx);
                })
                .is_err()
            {
                break;
            }
        }
    });
    TransferProgressBridge { callback, task }
}

async fn coalesce_progress(cx: &mut AsyncApp, receiver: &mut tokio::sync::mpsc::Receiver<()>) {
    cx.background_executor()
        .timer(std::time::Duration::from_millis(
            PROGRESS_BRIDGE_INTERVAL_MS,
        ))
        .await;
    while receiver.try_recv().is_ok() {}
}

pub(super) fn progress_detail(progress: &TransferProgress) -> Option<gpui::SharedString> {
    progress.current_file.clone().map(Into::into)
}

pub(super) fn progress_speed(progress: &TransferProgress) -> Option<gpui::SharedString> {
    (progress.speed > 0.0).then(|| format_transfer_speed(progress.speed).into())
}

fn format_transfer_speed(bytes_per_second: f64) -> String {
    if bytes_per_second >= 1024.0 * 1024.0 {
        format!("{:.1} MB/s", bytes_per_second / (1024.0 * 1024.0))
    } else if bytes_per_second >= 1024.0 {
        format!("{:.1} KB/s", bytes_per_second / 1024.0)
    } else {
        format!("{bytes_per_second:.0} B/s")
    }
}
