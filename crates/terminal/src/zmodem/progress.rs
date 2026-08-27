use crate::TerminalEvent;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZmodemTransferProgress {
    pub(crate) transfer_id: ZmodemTransferId,
    pub(crate) direction: ZmodemTransferDirection,
    pub(crate) file_name: String,
    pub(crate) file_index: usize,
    pub(crate) file_count: usize,
    pub(crate) current_file_transferred: u64,
    pub(crate) current_file_total: u64,
    pub(crate) transferred: u64,
    pub(crate) total: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ZmodemTransferId(u64);

impl ZmodemTransferId {
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl From<u64> for ZmodemTransferId {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

/// ZMODEM 传输结束原因，供全局后台任务面板准确标记终态。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ZmodemTransferOutcome {
    Succeeded,
    Cancelled,
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZmodemTransferDirection {
    Upload,
    Download,
}

impl ZmodemTransferDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Upload => "upload",
            Self::Download => "download",
        }
    }
}

impl ZmodemTransferProgress {
    pub fn transfer_id(&self) -> ZmodemTransferId {
        self.transfer_id
    }

    pub fn file_name(&self) -> &str {
        &self.file_name
    }

    pub fn file_index(&self) -> usize {
        self.file_index
    }

    pub fn file_count(&self) -> usize {
        self.file_count
    }

    pub fn current_file_transferred(&self) -> u64 {
        self.current_file_transferred
    }

    pub fn current_file_total(&self) -> u64 {
        self.current_file_total
    }

    pub fn transferred(&self) -> u64 {
        self.transferred
    }

    pub fn total(&self) -> u64 {
        self.total
    }

    pub fn direction(&self) -> ZmodemTransferDirection {
        self.direction
    }

    pub fn percent(&self) -> u32 {
        if self.total == 0 {
            return 0;
        }
        ((u128::from(self.transferred) * 100 / u128::from(self.total)).min(100)) as u32
    }
}

#[derive(Clone, Default)]
pub(crate) struct ZmodemProgressState {
    state: Arc<StdMutex<ProgressState>>,
    event_tx: Option<UnboundedSender<TerminalEvent>>,
}

#[derive(Default)]
struct ProgressState {
    next_id: u64,
    active_id: Option<ZmodemTransferId>,
    snapshot: Option<ZmodemTransferProgress>,
}

pub(crate) struct TransferProgressGuard {
    progress: ZmodemProgressState,
    transfer_id: ZmodemTransferId,
}

impl ZmodemProgressState {
    pub(crate) fn new(event_tx: UnboundedSender<TerminalEvent>) -> Self {
        Self {
            state: Arc::new(StdMutex::new(ProgressState::default())),
            event_tx: Some(event_tx),
        }
    }

    pub(crate) fn snapshot(&self) -> Option<ZmodemTransferProgress> {
        self.state.lock().ok()?.snapshot.clone()
    }

    pub(crate) fn begin_transfer(&self, _direction: ZmodemTransferDirection) -> ZmodemTransferId {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.next_id = state.next_id.wrapping_add(1).max(1);
        let transfer_id = ZmodemTransferId(state.next_id);
        state.active_id = Some(transfer_id);
        state.snapshot = None;
        transfer_id
    }

    pub(crate) fn begin(
        &self,
        transfer_id: ZmodemTransferId,
        progress: ZmodemTransferProgress,
    ) -> TransferProgressGuard {
        self.set(transfer_id, progress, true);
        TransferProgressGuard {
            progress: self.clone(),
            transfer_id,
        }
    }

    pub(crate) fn start(&self, transfer_id: ZmodemTransferId, progress: ZmodemTransferProgress) {
        // Downloads are executor-driven and keep their snapshot until the
        // transfer reaches a terminal outcome.
        self.set(transfer_id, progress, true);
    }

    pub(crate) fn update(&self, transfer_id: ZmodemTransferId, progress: ZmodemTransferProgress) {
        self.set(transfer_id, progress, false);
    }

    pub(crate) fn finish(&self, transfer_id: ZmodemTransferId, outcome: ZmodemTransferOutcome) {
        self.finish_inner(transfer_id, outcome);
    }

    fn set(
        &self,
        transfer_id: ZmodemTransferId,
        mut progress: ZmodemTransferProgress,
        force: bool,
    ) {
        let changed = self.state.lock().ok().is_some_and(|mut state| {
            if state.active_id != Some(transfer_id) {
                return false;
            }
            progress.transfer_id = transfer_id;
            let changed = force || state.snapshot.as_ref() != Some(&progress);
            state.snapshot = Some(progress);
            changed
        });
        if changed {
            self.notify(transfer_id);
        }
    }

    fn clear(&self, transfer_id: ZmodemTransferId) {
        let cleared = self.state.lock().ok().is_some_and(|mut state| {
            if state.active_id != Some(transfer_id) {
                return false;
            }
            state.snapshot.take().is_some()
        });
        if cleared {
            self.notify(transfer_id);
        }
    }

    fn finish_inner(&self, transfer_id: ZmodemTransferId, outcome: ZmodemTransferOutcome) {
        let should_notify = self.state.lock().ok().is_some_and(|mut state| {
            if state.active_id != Some(transfer_id) {
                return false;
            }
            state.snapshot.take();
            state.active_id = None;
            true
        });
        if should_notify {
            self.notify_finished(transfer_id, outcome);
        }
    }

    fn notify(&self, transfer_id: ZmodemTransferId) {
        if let Some(event_tx) = &self.event_tx {
            let _ = event_tx.send(TerminalEvent::ZmodemProgressChanged(transfer_id));
        }
    }

    fn notify_finished(&self, transfer_id: ZmodemTransferId, outcome: ZmodemTransferOutcome) {
        if let Some(event_tx) = &self.event_tx {
            let _ = event_tx.send(TerminalEvent::ZmodemTransferFinished {
                transfer_id,
                outcome,
            });
        }
    }
}

impl Drop for TransferProgressGuard {
    fn drop(&mut self) {
        self.progress.clear(self.transfer_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc::unbounded_channel;

    #[test]
    fn transfer_progress_uses_total_bytes_and_clamps_percent() {
        let progress = upload_progress(150, 100);
        assert_eq!(100, progress.percent());
        assert_eq!("archive.tar", progress.file_name());
        assert_eq!(0, progress.file_index());
        assert_eq!(1, progress.file_count());
        assert_eq!(150, progress.current_file_transferred());
        assert_eq!(100, progress.current_file_total());
        assert_eq!(150, progress.transferred());
        assert_eq!(100, progress.total());
    }

    #[tokio::test]
    async fn upload_progress_notifies_on_transferred_change_and_clear() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let progress_state = ZmodemProgressState::new(event_tx);
        let transfer_id = progress_state.begin_transfer(ZmodemTransferDirection::Upload);
        let guard = progress_state.begin(transfer_id, upload_progress(0, 1_000));
        assert_progress_event_for(&mut event_rx, transfer_id).await;

        progress_state.update(transfer_id, upload_progress(5, 1_000));
        assert_progress_event_for(&mut event_rx, transfer_id).await;
        progress_state.update(transfer_id, upload_progress(5, 1_000));
        assert!(event_rx.try_recv().is_err());
        progress_state.update(transfer_id, upload_progress(10, 1_000));
        assert_progress_event_for(&mut event_rx, transfer_id).await;
        assert_eq!(10, progress_state.snapshot().unwrap().transferred());

        drop(guard);
        assert!(progress_state.snapshot().is_none());
        assert_progress_event_for(&mut event_rx, transfer_id).await;
    }

    #[tokio::test]
    async fn stale_upload_guard_does_not_clear_new_transfer() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let progress_state = ZmodemProgressState::new(event_tx);
        let stale_id = progress_state.begin_transfer(ZmodemTransferDirection::Upload);
        let stale_guard = progress_state.begin(stale_id, upload_progress(1, 1_000));
        assert_progress_event_for(&mut event_rx, stale_id).await;

        let current_id = progress_state.begin_transfer(ZmodemTransferDirection::Upload);
        let _current_guard = progress_state.begin(current_id, upload_progress(2, 1_000));
        assert_progress_event_for(&mut event_rx, current_id).await;

        drop(stale_guard);

        assert_eq!(
            Some(current_id),
            progress_state
                .snapshot()
                .map(|progress| progress.transfer_id())
        );
        assert!(event_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn stale_finish_does_not_finish_new_transfer() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let progress_state = ZmodemProgressState::new(event_tx);
        let stale_id = progress_state.begin_transfer(ZmodemTransferDirection::Upload);
        let _stale_guard = progress_state.begin(stale_id, upload_progress(1, 1_000));
        assert_progress_event_for(&mut event_rx, stale_id).await;

        let current_id = progress_state.begin_transfer(ZmodemTransferDirection::Download);
        progress_state.start(current_id, download_progress(2));
        assert_progress_event_for(&mut event_rx, current_id).await;

        progress_state.finish(stale_id, ZmodemTransferOutcome::Succeeded);

        assert_eq!(
            Some(current_id),
            progress_state
                .snapshot()
                .map(|progress| progress.transfer_id())
        );
        assert!(event_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn explicit_progress_finish_does_not_emit_a_duplicate_cancel() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let progress_state = ZmodemProgressState::new(event_tx);
        let transfer_id = progress_state.begin_transfer(ZmodemTransferDirection::Upload);
        let guard = progress_state.begin(transfer_id, upload_progress(1_000, 1_000));
        assert_progress_event_for(&mut event_rx, transfer_id).await;

        progress_state.finish(transfer_id, ZmodemTransferOutcome::Succeeded);
        assert!(matches!(
            event_rx.recv().await,
            Some(TerminalEvent::ZmodemTransferFinished {
                transfer_id: id,
                outcome: ZmodemTransferOutcome::Succeeded,
            }) if id == transfer_id
        ));
        drop(guard);
        assert!(event_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn explicit_finish_still_notifies_after_guard_clears_snapshot() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let progress_state = ZmodemProgressState::new(event_tx);
        let transfer_id = progress_state.begin_transfer(ZmodemTransferDirection::Upload);
        let guard = progress_state.begin(transfer_id, upload_progress(1, 1_000));
        assert_progress_event_for(&mut event_rx, transfer_id).await;
        drop(guard);
        assert_progress_event_for(&mut event_rx, transfer_id).await;

        progress_state.finish(transfer_id, ZmodemTransferOutcome::Succeeded);
        assert!(matches!(
            event_rx.recv().await,
            Some(TerminalEvent::ZmodemTransferFinished {
                transfer_id: id,
                outcome: ZmodemTransferOutcome::Succeeded,
            }) if id == transfer_id
        ));
    }

    #[tokio::test]
    async fn finish_before_guard_drop_does_not_emit_a_duplicate_terminal_event() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let progress_state = ZmodemProgressState::new(event_tx);
        let transfer_id = progress_state.begin_transfer(ZmodemTransferDirection::Upload);
        let guard = progress_state.begin(transfer_id, upload_progress(1_000, 1_000));
        assert_progress_event_for(&mut event_rx, transfer_id).await;

        progress_state.finish(transfer_id, ZmodemTransferOutcome::Cancelled);
        assert!(matches!(
            event_rx.recv().await,
            Some(TerminalEvent::ZmodemTransferFinished {
                transfer_id: id,
                outcome: ZmodemTransferOutcome::Cancelled,
            }) if id == transfer_id
        ));
        drop(guard);
        assert!(event_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn explicit_failed_finish_does_not_emit_a_cancel_on_drop() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let progress_state = ZmodemProgressState::new(event_tx);
        let transfer_id = progress_state.begin_transfer(ZmodemTransferDirection::Upload);
        let guard = progress_state.begin(transfer_id, upload_progress(1, 1_000));
        assert_progress_event_for(&mut event_rx, transfer_id).await;

        progress_state.finish(
            transfer_id,
            ZmodemTransferOutcome::Failed("boom".to_string()),
        );
        assert!(matches!(
            event_rx.recv().await,
            Some(TerminalEvent::ZmodemTransferFinished {
                transfer_id: id,
                outcome: ZmodemTransferOutcome::Failed(error),
            }) if id == transfer_id && error == "boom"
        ));
        drop(guard);
        assert!(event_rx.try_recv().is_err());
    }

    #[test]
    fn download_start_keeps_snapshot_active_without_a_guard() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let progress_state = ZmodemProgressState::new(event_tx);
        let transfer_id = progress_state.begin_transfer(ZmodemTransferDirection::Download);

        progress_state.start(transfer_id, download_progress(0));
        assert_eq!(
            ZmodemTransferDirection::Download,
            progress_state.snapshot().unwrap().direction()
        );
        assert!(matches!(
            event_rx.try_recv(),
            Ok(TerminalEvent::ZmodemProgressChanged(id)) if id == transfer_id
        ));
        assert!(event_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn unknown_total_progress_notifies_on_transferred_change() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let progress_state = ZmodemProgressState::new(event_tx);
        let transfer_id = progress_state.begin_transfer(ZmodemTransferDirection::Download);
        progress_state.start(transfer_id, download_progress(0));
        assert_progress_event_for(&mut event_rx, transfer_id).await;

        progress_state.update(transfer_id, download_progress(1));
        assert_progress_event_for(&mut event_rx, transfer_id).await;
        assert_eq!(1, progress_state.snapshot().unwrap().transferred());

        progress_state.update(transfer_id, download_progress(2));
        assert_progress_event_for(&mut event_rx, transfer_id).await;
        assert_eq!(2, progress_state.snapshot().unwrap().transferred());

        progress_state.finish(transfer_id, ZmodemTransferOutcome::Succeeded);
        assert!(matches!(
            event_rx.recv().await,
            Some(TerminalEvent::ZmodemTransferFinished {
                transfer_id: id,
                outcome: ZmodemTransferOutcome::Succeeded,
            }) if id == transfer_id
        ));
        assert!(progress_state.snapshot().is_none());
    }

    #[test]
    fn begin_transfer_resets_the_previous_snapshot_without_notifying() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let progress_state = ZmodemProgressState::new(event_tx);
        let upload_id = progress_state.begin_transfer(ZmodemTransferDirection::Upload);
        let guard = progress_state.begin(upload_id, upload_progress(1, 1_000));
        assert!(event_rx.try_recv().is_ok());

        let download_id = progress_state.begin_transfer(ZmodemTransferDirection::Download);
        assert_ne!(upload_id, download_id);
        assert!(progress_state.snapshot().is_none());
        assert!(event_rx.try_recv().is_err());
        drop(guard);
        assert!(event_rx.try_recv().is_err());
    }

    async fn assert_progress_event_for(
        event_rx: &mut tokio::sync::mpsc::UnboundedReceiver<TerminalEvent>,
        transfer_id: ZmodemTransferId,
    ) {
        assert!(matches!(
            event_rx.recv().await,
            Some(TerminalEvent::ZmodemProgressChanged(id)) if id == transfer_id
        ));
    }

    fn upload_progress(transferred: u64, total: u64) -> ZmodemTransferProgress {
        ZmodemTransferProgress {
            transfer_id: ZmodemTransferId::default(),
            direction: ZmodemTransferDirection::Upload,
            file_name: "archive.tar".to_string(),
            file_index: 0,
            file_count: 1,
            current_file_transferred: transferred,
            current_file_total: total,
            transferred,
            total,
        }
    }

    fn download_progress(transferred: u64) -> ZmodemTransferProgress {
        ZmodemTransferProgress {
            transfer_id: ZmodemTransferId::default(),
            direction: ZmodemTransferDirection::Download,
            file_name: "remote.tar".to_string(),
            file_index: 0,
            file_count: 0,
            current_file_transferred: 0,
            current_file_total: 0,
            transferred,
            total: 0,
        }
    }
}
