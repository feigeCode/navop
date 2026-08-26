use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

#[cfg(test)]
use std::sync::atomic::AtomicUsize;

use gpui::{Context, EventEmitter};
use one_core::gpui_tokio::Tokio;
use sftp::TransferProgress;
use tokio_util::sync::CancellationToken;

#[cfg(test)]
use super::cancellation_watcher::CancellationWatcherGuard;
use super::{
    SftpConnectionIdentity, SftpDeleteRemoteRequest, SftpDownloadRequest, SftpTransferEvent,
    SftpTransferId, SftpTransferProvider, SftpTransferSnapshot, SftpTransferState,
    SftpUploadRequest,
    history::CompletedTransferHistory,
    operation::{TransferRequest, execute_transfer},
    progress::create_progress_bridge,
    record::{TransferRecord, register_background_task},
    scheduler::ConnectionLanes,
};

const DEFAULT_COMPLETED_HISTORY_LIMIT: usize = 200;
static NEXT_EXECUTOR_INSTANCE_ID: AtomicU64 = AtomicU64::new(0);

#[must_use = "a reserved SFTP transfer must be committed or explicitly dropped"]
pub struct SftpTransferReservation {
    executor_instance_id: u64,
    id: SftpTransferId,
    request: TransferRequest,
}

impl SftpTransferReservation {
    pub fn id(&self) -> SftpTransferId {
        self.id
    }
}

pub struct SftpTransferExecutor {
    instance_id: u64,
    provider: Arc<dyn SftpTransferProvider>,
    next_transfer_id: u64,
    next_runtime_connection: u64,
    lanes: ConnectionLanes,
    active_transfers: HashMap<SftpTransferId, TransferRecord>,
    completed_transfers: CompletedTransferHistory,
    #[cfg(test)]
    active_cancellation_watchers: Arc<AtomicUsize>,
}

impl EventEmitter<SftpTransferEvent> for SftpTransferExecutor {}

impl SftpTransferExecutor {
    pub fn new(provider: Arc<dyn SftpTransferProvider>) -> Self {
        Self::with_completed_history_limit(provider, DEFAULT_COMPLETED_HISTORY_LIMIT)
    }

    fn with_completed_history_limit(
        provider: Arc<dyn SftpTransferProvider>,
        completed_history_limit: usize,
    ) -> Self {
        Self {
            instance_id: NEXT_EXECUTOR_INSTANCE_ID
                .fetch_add(1, Ordering::Relaxed)
                .wrapping_add(1)
                .max(1),
            provider,
            next_transfer_id: 0,
            next_runtime_connection: 0,
            lanes: ConnectionLanes::default(),
            active_transfers: HashMap::new(),
            completed_transfers: CompletedTransferHistory::new(completed_history_limit),
            #[cfg(test)]
            active_cancellation_watchers: Arc::new(AtomicUsize::new(0)),
        }
    }

    #[cfg(test)]
    pub(super) fn new_with_completed_history_limit(
        provider: Arc<dyn SftpTransferProvider>,
        completed_history_limit: usize,
    ) -> Self {
        Self::with_completed_history_limit(provider, completed_history_limit)
    }

    #[cfg(test)]
    pub(super) fn active_cancellation_watcher_count(&self) -> usize {
        self.active_cancellation_watchers.load(Ordering::Relaxed)
    }

    pub fn allocate_runtime_connection(&mut self) -> SftpConnectionIdentity {
        self.next_runtime_connection = self.next_runtime_connection.wrapping_add(1).max(1);
        SftpConnectionIdentity::Runtime(self.next_runtime_connection)
    }

    pub fn submit(&mut self, request: SftpUploadRequest, cx: &mut Context<Self>) -> SftpTransferId {
        let reservation = self.reserve(request);
        self.commit_own_reservation(reservation, cx)
    }

    pub fn reserve(&mut self, request: SftpUploadRequest) -> SftpTransferReservation {
        self.reserve_request(TransferRequest::Upload(request))
    }

    pub fn submit_download(
        &mut self,
        request: SftpDownloadRequest,
        cx: &mut Context<Self>,
    ) -> SftpTransferId {
        let reservation = self.reserve_download(request);
        self.commit_own_reservation(reservation, cx)
    }

    pub fn reserve_download(&mut self, request: SftpDownloadRequest) -> SftpTransferReservation {
        self.reserve_request(TransferRequest::Download(request))
    }

    pub fn submit_delete_remote(
        &mut self,
        request: SftpDeleteRemoteRequest,
        cx: &mut Context<Self>,
    ) -> SftpTransferId {
        let reservation = self.reserve_delete_remote(request);
        self.commit_own_reservation(reservation, cx)
    }

    pub fn reserve_delete_remote(
        &mut self,
        request: SftpDeleteRemoteRequest,
    ) -> SftpTransferReservation {
        self.reserve_request(TransferRequest::DeleteRemote(request))
    }

    pub fn commit_reserved(
        &mut self,
        reservation: SftpTransferReservation,
        cx: &mut Context<Self>,
    ) -> Result<SftpTransferId, SftpTransferReservation> {
        if reservation.executor_instance_id != self.instance_id {
            return Err(reservation);
        }
        Ok(self.commit_request(reservation.id, reservation.request, cx))
    }

    fn reserve_request(&mut self, request: TransferRequest) -> SftpTransferReservation {
        let id = self.allocate_transfer_id();
        SftpTransferReservation {
            executor_instance_id: self.instance_id,
            id,
            request,
        }
    }

    fn commit_own_reservation(
        &mut self,
        reservation: SftpTransferReservation,
        cx: &mut Context<Self>,
    ) -> SftpTransferId {
        match self.commit_reserved(reservation, cx) {
            Ok(id) => id,
            Err(_) => unreachable!("executor must accept its own transfer reservation"),
        }
    }

    fn commit_request(
        &mut self,
        id: SftpTransferId,
        request: TransferRequest,
        cx: &mut Context<Self>,
    ) -> SftpTransferId {
        let token = CancellationToken::new();
        let background_task = register_background_task(&request, &token, cx);
        let connection = request.connection().clone();
        let record = TransferRecord::new(id, request, token.clone(), background_task);
        let watcher_done = record.cancellation_watcher_done.clone();
        self.active_transfers.insert(id, record);
        self.lanes.enqueue(connection.clone(), id);
        self.watch_cancellation(id, token, watcher_done, cx);
        cx.emit(SftpTransferEvent::Added(id));
        cx.notify();
        self.start_connection_if_idle(&connection, cx);
        id
    }

    pub fn cancel(&mut self, id: SftpTransferId, cx: &mut Context<Self>) -> bool {
        self.active_transfers
            .get(&id)
            .is_some_and(|record| record.background_task.request_cancel(cx))
    }

    pub fn snapshot(&self, id: SftpTransferId) -> Option<SftpTransferSnapshot> {
        self.active_transfers
            .get(&id)
            .map(|record| record.snapshot.clone())
            .or_else(|| self.completed_transfers.get(id))
    }

    pub fn active_for_connection(
        &self,
        connection: &SftpConnectionIdentity,
    ) -> Option<SftpTransferSnapshot> {
        self.active_transfers
            .values()
            .filter(|record| {
                record.snapshot.connection == *connection && record.snapshot.state.is_active()
            })
            .min_by_key(|record| record.snapshot.id.as_u64())
            .map(|record| record.snapshot.clone())
    }

    pub fn pending_count(&self, connection: &SftpConnectionIdentity) -> usize {
        self.active_transfers
            .values()
            .filter(|record| {
                record.snapshot.connection == *connection
                    && record.snapshot.state == SftpTransferState::Queued
            })
            .count()
    }

    fn allocate_transfer_id(&mut self) -> SftpTransferId {
        self.next_transfer_id = self.next_transfer_id.wrapping_add(1).max(1);
        SftpTransferId::new(self.next_transfer_id)
    }

    fn watch_cancellation(
        &self,
        id: SftpTransferId,
        token: CancellationToken,
        watcher_done: CancellationToken,
        cx: &mut Context<Self>,
    ) {
        #[cfg(test)]
        let active_watchers = self.active_cancellation_watchers.clone();
        cx.spawn(async move |executor, cx| {
            #[cfg(test)]
            let _watcher_guard = CancellationWatcherGuard::new(active_watchers);
            tokio::select! {
                _ = token.cancelled() => {
                    let _ = executor.update(cx, |executor, cx| {
                        executor.apply_cancellation(id, cx);
                    });
                }
                _ = watcher_done.cancelled() => {}
            }
        })
        .detach();
    }

    fn apply_cancellation(&mut self, id: SftpTransferId, cx: &mut Context<Self>) {
        let Some(record) = self.active_transfers.get_mut(&id) else {
            return;
        };
        record.cancelled.store(true, Ordering::Relaxed);
        match record.snapshot.state {
            SftpTransferState::Queued => self.cancel_queued(id, cx),
            SftpTransferState::Running => {
                record.snapshot.state = SftpTransferState::Cancelling;
                cx.emit(SftpTransferEvent::Updated(id));
                cx.notify();
            }
            _ => {}
        }
    }

    fn cancel_queued(&mut self, id: SftpTransferId, cx: &mut Context<Self>) {
        let Some(connection) = self
            .active_transfers
            .get(&id)
            .map(|record| record.snapshot.connection.clone())
        else {
            return;
        };
        if !self.lanes.remove_pending(&connection, id) {
            return;
        }
        let Some(mut record) = self.active_transfers.remove(&id) else {
            return;
        };
        record.snapshot.state = SftpTransferState::Cancelled;
        record
            .background_task
            .cancel_confirmed(Some("Cancelled".into()), cx);
        self.publish_finished(record, cx);
    }

    fn start_connection_if_idle(
        &mut self,
        connection: &SftpConnectionIdentity,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.lanes.take_startable(connection) else {
            return;
        };
        self.start_transfer(id, cx);
    }

    fn start_transfer(&mut self, id: SftpTransferId, cx: &mut Context<Self>) {
        let Some(record) = self.active_transfers.get_mut(&id) else {
            return;
        };
        if record.cancellation_token.is_cancelled() {
            self.finish_transfer(id, Ok(Ok(())), cx);
            return;
        }
        record.snapshot.state = SftpTransferState::Running;
        record.background_task.mark_running(cx);
        let execution = record.execution(id);
        let provider = self.provider.clone();
        let progress = create_progress_bridge(id, cx);
        progress.task.detach();
        let task = Tokio::spawn(cx, execute_transfer(provider, execution, progress.callback));
        cx.emit(SftpTransferEvent::Updated(id));
        cx.notify();
        cx.spawn(async move |executor, cx| {
            let result = task.await;
            let _ = executor.update(cx, |executor, cx| {
                executor.finish_transfer(id, result, cx);
            });
        })
        .detach();
    }

    pub(super) fn update_progress(
        &mut self,
        id: SftpTransferId,
        progress: TransferProgress,
        cx: &mut Context<Self>,
    ) {
        let Some(record) = self.active_transfers.get_mut(&id) else {
            return;
        };
        if !record.snapshot.state.is_active() {
            return;
        }
        record.update_progress(&progress, cx);
        cx.emit(SftpTransferEvent::Updated(id));
        cx.notify();
    }

    fn finish_transfer(
        &mut self,
        id: SftpTransferId,
        result: Result<anyhow::Result<()>, one_core::gpui_tokio::JoinError>,
        cx: &mut Context<Self>,
    ) {
        let Some(mut record) = self.active_transfers.remove(&id) else {
            return;
        };
        let connection = record.snapshot.connection.clone();
        let outcome = record.finish(result);
        record.background_task.finish(outcome, cx);
        self.publish_finished(record, cx);
        if let Some(next) = self.lanes.complete(&connection, id) {
            self.start_transfer(next, cx);
        }
    }

    fn publish_finished(&mut self, record: TransferRecord, cx: &mut Context<Self>) {
        let id = record.snapshot.id;
        record.cancellation_watcher_done.cancel();
        self.completed_transfers.push(record.snapshot);
        cx.emit(SftpTransferEvent::Finished(id));
        cx.notify();
    }
}
