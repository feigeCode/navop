use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use gpui::Context;
use one_core::background_tasks::{
    self, BackgroundTaskCancellation, BackgroundTaskHandle, BackgroundTaskOutcome,
    BackgroundTaskProgressUnit, BackgroundTaskSpec,
};
use sftp::{TransferCancelled, TransferProgress};
use tokio_util::sync::CancellationToken;

use super::{
    SftpTransferExecutor, SftpTransferId, SftpTransferSnapshot, SftpTransferState,
    operation::{TransferExecution, TransferRequest},
    progress::{progress_detail, progress_speed},
};

pub(super) struct TransferRecord {
    pub request: TransferRequest,
    pub snapshot: SftpTransferSnapshot,
    pub cancelled: Arc<AtomicBool>,
    pub cancellation_token: CancellationToken,
    pub cancellation_watcher_done: CancellationToken,
    pub background_task: BackgroundTaskHandle,
}

impl TransferRecord {
    pub(super) fn new(
        id: SftpTransferId,
        request: TransferRequest,
        cancellation_token: CancellationToken,
        background_task: BackgroundTaskHandle,
    ) -> Self {
        Self {
            snapshot: initial_snapshot(id, &request),
            request,
            cancelled: Arc::new(AtomicBool::new(false)),
            cancellation_token,
            cancellation_watcher_done: CancellationToken::new(),
            background_task,
        }
    }

    pub(super) fn execution(&self, id: SftpTransferId) -> TransferExecution {
        self.request.execution(id, self.cancelled.clone())
    }

    pub(super) fn update_progress(
        &mut self,
        progress: &TransferProgress,
        cx: &mut Context<SftpTransferExecutor>,
    ) {
        self.snapshot.transferred = progress.transferred;
        self.snapshot.total = Some(progress.total);
        self.snapshot.speed = progress.speed;
        self.snapshot.current_file = progress.current_file.clone();
        self.background_task.update_progress(
            progress.transferred,
            Some(progress.total),
            progress_detail(progress),
            progress_speed(progress),
            cx,
        );
    }

    pub(super) fn finish(
        &mut self,
        result: Result<anyhow::Result<()>, one_core::gpui_tokio::JoinError>,
    ) -> BackgroundTaskOutcome {
        if self.cancellation_wins(&result) {
            self.snapshot.state = SftpTransferState::Cancelled;
            self.snapshot.error = None;
            return BackgroundTaskOutcome::Cancelled(Some("Cancelled".into()));
        }
        match result {
            Ok(Ok(())) => {
                self.snapshot.state = SftpTransferState::Succeeded;
                BackgroundTaskOutcome::Succeeded(None)
            }
            Ok(Err(error)) => self.fail(format!("{error:#}")),
            Err(error) => self.fail(error.to_string()),
        }
    }

    fn cancellation_wins(
        &self,
        result: &Result<anyhow::Result<()>, one_core::gpui_tokio::JoinError>,
    ) -> bool {
        self.cancellation_token.is_cancelled()
            || self.cancelled.load(Ordering::Relaxed)
            || matches!(result, Ok(Err(error)) if error.downcast_ref::<TransferCancelled>().is_some())
    }

    fn fail(&mut self, error: String) -> BackgroundTaskOutcome {
        self.snapshot.state = SftpTransferState::Failed;
        self.snapshot.error = Some(error.clone());
        BackgroundTaskOutcome::Failed(error)
    }
}

pub(super) fn register_background_task(
    request: &TransferRequest,
    token: &CancellationToken,
    cx: &mut Context<SftpTransferExecutor>,
) -> BackgroundTaskHandle {
    let manager = background_tasks::global(cx);
    let mut spec = BackgroundTaskSpec::new(request.background_kind(), request.title())
        .detail(request.background_detail())
        .progress_unit(BackgroundTaskProgressUnit::Bytes);
    if let Some(group) = request.task_group() {
        spec = spec.group(group);
    }
    if let Some(key) = request.task_key() {
        spec = spec.key(key);
    }
    if let Some(path) = request.open_folder() {
        spec = spec.open_folder(path);
    }
    let id = manager.update(cx, |manager, cx| {
        let id = manager.register(spec, cx);
        manager.set_cancellation(id, BackgroundTaskCancellation::token(token.clone()), cx);
        id
    });
    BackgroundTaskHandle::new(manager.downgrade(), id)
}

fn initial_snapshot(id: SftpTransferId, request: &TransferRequest) -> SftpTransferSnapshot {
    SftpTransferSnapshot {
        id,
        operation: request.operation(),
        connection: request.connection().clone(),
        local_path: request.local_path().to_path_buf(),
        remote_path: request.remote_path().to_string(),
        display_name: request.display_name().to_string(),
        state: SftpTransferState::Queued,
        transferred: 0,
        total: None,
        speed: 0.0,
        current_file: None,
        error: None,
    }
}
