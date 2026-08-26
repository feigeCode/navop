use super::zmodem_progress::format_zmodem_bytes;
use super::*;
use one_core::background_tasks::{
    self, BackgroundTaskCancellation, BackgroundTaskProgressUnit, BackgroundTaskSpec,
};
use terminal::zmodem::{ZmodemTransferDirection, ZmodemTransferOutcome};
use tokio_util::sync::CancellationToken;

impl TerminalView {
    /// 将 ZMODEM 传输进度同步到全局后台任务面板。
    pub(super) fn sync_zmodem_background_task(&mut self, cx: &mut Context<Self>) {
        let Some(progress) = self.terminal.read(cx).zmodem_transfer_progress() else {
            return;
        };
        let direction = progress.direction();
        let entity_id = self.terminal.entity_id().as_u64();
        let key = SharedString::from(format!("zmodem-{}:{entity_id}", direction.as_str()));
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

        let Some(manager) = background_tasks::try_global(cx) else {
            return;
        };

        let existing_id = self
            .zmodem_background_task_id
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
            let cancellation = CancellationToken::new();
            let id = manager.update(cx, |manager, cx| {
                let id = manager.ensure_by_key(spec, key, cx);
                manager.set_cancellation(
                    id,
                    BackgroundTaskCancellation::token(cancellation.clone()),
                    cx,
                );
                id
            });
            self.zmodem_background_task_id = Some(id);
            self.start_zmodem_cancel_watch(cancellation, cx);
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

    fn start_zmodem_cancel_watch(&mut self, token: CancellationToken, cx: &mut Context<Self>) {
        self.zmodem_background_cancel_watch = Some(cx.spawn(async move |this, cx| {
            token.cancelled().await;
            let _ = this.update(cx, |this, cx| {
                this.terminal.update(cx, |terminal, _| {
                    terminal.cancel_zmodem_transfer();
                });
            });
        }));
    }

    /// 根据协议返回的真实终态更新全局后台任务。
    pub(super) fn finish_zmodem_background_task(
        &mut self,
        outcome: &ZmodemTransferOutcome,
        cx: &mut Context<Self>,
    ) {
        self.zmodem_background_cancel_watch = None;
        let Some(id) = self.zmodem_background_task_id.take() else {
            return;
        };
        let Some(manager) = background_tasks::try_global(cx) else {
            return;
        };
        manager.update(cx, |manager, cx| match outcome {
            ZmodemTransferOutcome::Succeeded => manager.succeed(id, None, cx),
            ZmodemTransferOutcome::Cancelled => manager.cancel_confirmed(id, None, cx),
            ZmodemTransferOutcome::Failed(error) => manager.fail(id, error.clone(), cx),
        });
    }
}
