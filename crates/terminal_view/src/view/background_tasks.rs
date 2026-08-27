use super::*;
use one_core::background_tasks::{
    self, BackgroundTaskCancellation, BackgroundTaskProgressUnit, BackgroundTaskSpec,
};
use terminal::zmodem::{ZmodemTransferDirection, ZmodemTransferId, ZmodemTransferOutcome};

impl TerminalView {
    /// 将 ZMODEM 传输进度同步到全局后台任务面板。
    pub(super) fn sync_zmodem_background_task(
        &mut self,
        expected_transfer_id: Option<ZmodemTransferId>,
        cx: &mut Context<Self>,
    ) {
        let Some(progress) = self.terminal.read(cx).zmodem_transfer_progress() else {
            return;
        };
        if expected_transfer_id.is_some_and(|id| id != progress.transfer_id()) {
            return;
        }
        let direction = progress.direction();
        let entity_id = self.terminal.entity_id().as_u64();
        let key = SharedString::from(format!(
            "zmodem-{}:{entity_id}:{}",
            direction.as_str(),
            progress.transfer_id().as_u64()
        ));
        let title = progress.file_name().to_string();
        let file_number = progress.file_index().saturating_add(1);
        let file_count = progress.file_count();
        let file_progress = if file_count == 0 {
            format!("File {file_number}")
        } else {
            format!("{file_number} / {file_count}")
        };
        let total = progress.total();
        let byte_progress = if total == 0 {
            format_zmodem_bytes(progress.transferred())
        } else {
            format!(
                "{} / {}",
                format_zmodem_bytes(progress.transferred()),
                format_zmodem_bytes(total)
            )
        };
        let detail = format!("{file_progress} · {byte_progress}");

        let manager = background_tasks::global(cx);

        let existing_id = self
            .zmodem_background_tasks
            .get(&progress.transfer_id())
            .copied()
            .filter(|id| manager.read(cx).find_by_key(&key) == Some(*id));
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
                        BackgroundTaskCancellation::callback(move || {
                            cancellation.cancel();
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
                progress.transferred(),
                (total > 0).then_some(total),
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
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.zmodem_background_tasks.remove(&transfer_id) else {
            return;
        };
        let manager = background_tasks::global(cx);
        manager.update(cx, |manager, cx| match outcome {
            ZmodemTransferOutcome::Succeeded => manager.succeed(id, None, cx),
            ZmodemTransferOutcome::Cancelled => manager.cancel_confirmed(id, None, cx),
            ZmodemTransferOutcome::Failed(error) => manager.fail(id, error.clone(), cx),
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

#[cfg(test)]
mod tests {
    use super::format_zmodem_bytes;
    use std::collections::HashMap;
    use terminal::zmodem::ZmodemTransferId;

    #[test]
    fn formats_zmodem_byte_counts() {
        assert_eq!("0 B", format_zmodem_bytes(0));
        assert_eq!("1.0 KB", format_zmodem_bytes(1024));
        assert_eq!("1.0 MB", format_zmodem_bytes(1024 * 1024));
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
