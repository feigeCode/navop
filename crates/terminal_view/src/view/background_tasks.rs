use super::*;
use one_core::background_tasks::{
    self, BackgroundTaskCancellation, BackgroundTaskProgressUnit, BackgroundTaskSpec,
};
use terminal::zmodem::{ZmodemTransferDirection, ZmodemTransferId, ZmodemTransferOutcome};

impl TerminalView {
    /// 将 ZMODEM 传输进度同步到全局后台任务面板。
    pub(super) fn sync_zmodem_background_task(
        &mut self,
        progress: Option<terminal::zmodem::ZmodemTransferProgress>,
        cx: &mut Context<Self>,
    ) {
        let Some(progress) = progress.or_else(|| self.terminal.read(cx).zmodem_transfer_progress())
        else {
            return;
        };
        let direction = progress.direction();
        let key = self.zmodem_background_task_key(&progress);
        let title = progress.file_name().to_string();
        let file_number = progress.file_index().saturating_add(1);
        let file_count = progress.file_count();
        let file_progress = if file_count == 0 {
            format!("File {file_number}")
        } else {
            format!("{file_number} / {file_count}")
        };
        let total = progress.total();
        let (progress_current, progress_total) = zmodem_progress_values(
            progress.transferred(),
            total,
            progress.current_file_transferred(),
            progress.current_file_total(),
        );
        let byte_progress = if total == 0 && progress.current_file_total() > 0 {
            format!(
                "{} / {}",
                format_zmodem_bytes(progress.current_file_transferred()),
                format_zmodem_bytes(progress.current_file_total())
            )
        } else if total == 0 {
            format_zmodem_bytes(progress.transferred())
        } else {
            format!(
                "{} / {}",
                format_zmodem_bytes(progress.transferred()),
                format_zmodem_bytes(total)
            )
        };
        let detail = format!("{title} · {file_progress} · {byte_progress}");

        let manager = background_tasks::global(cx);

        let active_id = manager.read(cx).find_by_key(&key);
        let existing_id = self
            .zmodem_background_tasks
            .get(&progress.transfer_id())
            .copied()
            .filter(|id| active_id == Some(*id))
            .or(active_id);
        let id = if let Some(id) = existing_id {
            id
        } else {
            let title_prefix = match direction {
                ZmodemTransferDirection::Upload => t!("TerminalZmodem.upload_title"),
                ZmodemTransferDirection::Download => t!("TerminalZmodem.download_title"),
            };
            let title = format!("{} · {title_prefix}", title);
            let spec = BackgroundTaskSpec::new("zmodem-transfer", title)
                .detail(detail.clone())
                .key(key.clone())
                .progress_unit(BackgroundTaskProgressUnit::Bytes);
            let cancellation = self.terminal.read(cx).zmodem_transfer_cancel_handle();
            let id = manager.update(cx, |manager, cx| {
                let id = manager.ensure_by_key(spec, key, cx);
                if let Some(cancellation) = cancellation {
                    manager.set_cancellation(
                        id,
                        BackgroundTaskCancellation::callback_with_result(move || {
                            cancellation.cancel()
                        }),
                        cx,
                    );
                }
                id
            });
            self.zmodem_background_tasks
                .insert(progress.transfer_id(), id);
            id
        };

        manager.update(cx, |manager, cx| {
            manager.mark_running(id, cx);
            manager.update_progress(
                id,
                progress_current,
                progress_total,
                Some(detail.into()),
                None,
                cx,
            );
        });
    }

    /// 根据协议返回的真实终态更新全局后台任务。
    pub(super) fn finish_zmodem_background_task(
        &mut self,
        transfer_id: ZmodemTransferId,
        outcome: &ZmodemTransferOutcome,
        progress: Option<terminal::zmodem::ZmodemTransferProgress>,
        cx: &mut Context<Self>,
    ) {
        let manager = background_tasks::global(cx);
        let mut id = self.zmodem_background_tasks.remove(&transfer_id);
        if id.is_none() {
            if let Some(progress) = progress.as_ref() {
                let key = self.zmodem_background_task_key(progress);
                id = manager.read(cx).find_latest_by_key(&key);
            }
        }
        if id.is_none() {
            if let Some(progress) = progress {
                self.sync_zmodem_background_task(Some(progress), cx);
                id = self.zmodem_background_tasks.remove(&transfer_id);
            }
        }
        let Some(id) = id else {
            return;
        };
        manager.update(cx, |manager, cx| match outcome {
            ZmodemTransferOutcome::Succeeded => manager.succeed(id, None, cx),
            ZmodemTransferOutcome::Cancelled => manager.cancel_confirmed(id, None, cx),
            ZmodemTransferOutcome::Failed(error) => manager.fail(id, error.clone(), cx),
        });
    }

    fn zmodem_background_task_key(
        &self,
        progress: &terminal::zmodem::ZmodemTransferProgress,
    ) -> SharedString {
        let entity_id = self.terminal.entity_id().as_u64();
        SharedString::from(format!(
            "zmodem-{}:{entity_id}:{}",
            progress.direction().as_str(),
            progress.transfer_id().as_u64()
        ))
    }

    pub(super) fn cancel_zmodem_background_tasks(&mut self, cx: &mut App) {
        self.terminal.read(cx).cancel_zmodem_transfer();
        let transfer_tasks = std::mem::take(&mut self.zmodem_background_tasks);
        if transfer_tasks.is_empty() {
            return;
        }
        let manager = background_tasks::global(cx);
        manager.update(cx, |manager, cx| {
            for task_id in transfer_tasks.into_values() {
                manager.cancel_confirmed(task_id, None, cx);
            }
        });
    }
}

fn format_zmodem_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

fn zmodem_progress_values(
    transferred: u64,
    total: u64,
    current_file_transferred: u64,
    current_file_total: u64,
) -> (u64, Option<u64>) {
    if total > 0 {
        (transferred.min(total), Some(total))
    } else if current_file_total > 0 {
        (
            current_file_transferred.min(current_file_total),
            Some(current_file_total),
        )
    } else {
        (transferred, None)
    }
}

#[cfg(test)]
mod tests {
    use super::{format_zmodem_bytes, zmodem_progress_values};
    use std::collections::HashMap;
    use terminal::zmodem::ZmodemTransferId;

    #[test]
    fn formats_zmodem_byte_counts() {
        assert_eq!("0 B", format_zmodem_bytes(0));
        assert_eq!("1.0 KB", format_zmodem_bytes(1024));
        assert_eq!("1.0 MB", format_zmodem_bytes(1024 * 1024));
    }

    #[test]
    fn download_uses_current_file_total_when_batch_total_is_unknown() {
        assert_eq!((512, Some(1024)), zmodem_progress_values(512, 0, 512, 1024));
    }

    #[test]
    fn upload_prefers_known_batch_total() {
        assert_eq!(
            (1536, Some(4096)),
            zmodem_progress_values(1536, 4096, 512, 1024)
        );
    }

    #[test]
    fn stale_zmodem_finish_does_not_take_current_background_task() {
        let stale_id = ZmodemTransferId::from(1);
        let current_id = ZmodemTransferId::from(2);
        let task_id = 7_u64;
        let mut tasks = HashMap::from([(current_id, task_id)]);

        assert_eq!(None, tasks.remove(&stale_id));
        assert_eq!(Some(&task_id), tasks.get(&current_id));
        assert_eq!(Some(task_id), tasks.remove(&current_id));
        assert!(tasks.is_empty());
    }
}
