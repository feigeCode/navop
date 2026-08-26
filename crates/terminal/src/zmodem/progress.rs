use crate::TerminalEvent;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZmodemTransferProgress {
    pub(crate) direction: ZmodemTransferDirection,
    pub(crate) file_name: String,
    pub(crate) file_index: usize,
    pub(crate) file_count: usize,
    pub(crate) current_file_transferred: u64,
    pub(crate) current_file_total: u64,
    pub(crate) transferred: u64,
    pub(crate) total: u64,
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
    snapshot: Option<ZmodemTransferProgress>,
    notified_key: Option<(usize, u32)>,
    /// A transfer has started even if its visible snapshot was just cleared.
    transfer_active: bool,
}

pub(crate) struct TransferProgressGuard {
    progress: ZmodemProgressState,
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

    pub(crate) fn begin(&self, progress: ZmodemTransferProgress) -> TransferProgressGuard {
        self.mark_transfer_active();
        self.set(progress, true);
        TransferProgressGuard {
            progress: self.clone(),
        }
    }

    pub(crate) fn start(&self, progress: ZmodemTransferProgress) {
        // Downloads are executor-driven and keep their snapshot until the
        // transfer reaches a terminal outcome.
        self.mark_transfer_active();
        self.set(progress, true);
    }

    pub(crate) fn update(&self, progress: ZmodemTransferProgress) {
        self.set(progress, false);
    }

    pub(crate) fn set_direction(&self, direction: ZmodemTransferDirection) {
        let changed = self
            .state
            .lock()
            .ok()
            .and_then(|mut state| {
                state
                    .snapshot
                    .as_mut()
                    .map(|snapshot| snapshot.direction = direction)
            })
            .is_some();
        if changed {
            self.notify();
        }
    }

    pub(crate) fn finish(&self, outcome: ZmodemTransferOutcome) {
        self.finish_inner(outcome);
    }

    fn set(&self, progress: ZmodemTransferProgress, force: bool) {
        let key = (progress.file_index, progress.percent());
        let changed = self.state.lock().ok().is_some_and(|mut state| {
            let changed = force
                || state.notified_key != Some(key)
                || (progress.total == 0
                    && state
                        .snapshot
                        .as_ref()
                        .is_some_and(|old| old.transferred != progress.transferred));
            state.snapshot = Some(progress);
            if changed {
                state.notified_key = Some(key);
            }
            changed
        });
        if changed {
            self.notify();
        }
    }

    fn mark_transfer_active(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.transfer_active = true;
        }
    }

    fn clear(&self) {
        let cleared = self
            .state
            .lock()
            .ok()
            .is_some_and(|mut state| state.snapshot.take().is_some());
        if cleared {
            self.notify();
        }
    }

    fn finish_inner(&self, outcome: ZmodemTransferOutcome) {
        let should_notify = self.state.lock().ok().is_some_and(|mut state| {
            state.notified_key = None;
            state.snapshot.take();
            std::mem::replace(&mut state.transfer_active, false)
        });
        if should_notify {
            self.notify_finished(outcome);
        }
    }

    fn notify(&self) {
        if let Some(event_tx) = &self.event_tx {
            let _ = event_tx.send(TerminalEvent::ZmodemProgressChanged);
        }
    }

    fn notify_finished(&self, outcome: ZmodemTransferOutcome) {
        if let Some(event_tx) = &self.event_tx {
            let _ = event_tx.send(TerminalEvent::ZmodemTransferFinished(outcome));
        }
    }
}

impl Drop for TransferProgressGuard {
    fn drop(&mut self) {
        self.progress.clear();
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
    async fn upload_progress_notifies_on_percent_change_and_clear() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let progress_state = ZmodemProgressState::new(event_tx);
        let guard = progress_state.begin(upload_progress(0, 1_000));
        assert_progress_event(&mut event_rx).await;

        progress_state.update(upload_progress(5, 1_000));
        assert!(event_rx.try_recv().is_err());
        progress_state.update(upload_progress(10, 1_000));
        assert_progress_event(&mut event_rx).await;
        assert_eq!(10, progress_state.snapshot().unwrap().transferred());

        drop(guard);
        assert!(progress_state.snapshot().is_none());
        assert_progress_event(&mut event_rx).await;
    }

    #[tokio::test]
    async fn explicit_progress_finish_does_not_emit_a_duplicate_cancel() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let progress_state = ZmodemProgressState::new(event_tx);
        let guard = progress_state.begin(upload_progress(1_000, 1_000));
        assert_progress_event(&mut event_rx).await;

        progress_state.finish(ZmodemTransferOutcome::Succeeded);
        assert!(matches!(
            event_rx.recv().await,
            Some(TerminalEvent::ZmodemTransferFinished(
                ZmodemTransferOutcome::Succeeded
            ))
        ));
        drop(guard);
        assert!(event_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn explicit_finish_still_notifies_after_guard_clears_snapshot() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let progress_state = ZmodemProgressState::new(event_tx);
        let guard = progress_state.begin(upload_progress(1, 1_000));
        assert_progress_event(&mut event_rx).await;
        drop(guard);
        assert_progress_event(&mut event_rx).await;

        progress_state.finish(ZmodemTransferOutcome::Succeeded);
        assert!(matches!(
            event_rx.recv().await,
            Some(TerminalEvent::ZmodemTransferFinished(
                ZmodemTransferOutcome::Succeeded
            ))
        ));
    }

    #[tokio::test]
    async fn finish_before_guard_drop_does_not_emit_a_duplicate_terminal_event() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let progress_state = ZmodemProgressState::new(event_tx);
        let guard = progress_state.begin(upload_progress(1_000, 1_000));
        assert_progress_event(&mut event_rx).await;

        progress_state.finish(ZmodemTransferOutcome::Cancelled);
        assert!(matches!(
            event_rx.recv().await,
            Some(TerminalEvent::ZmodemTransferFinished(
                ZmodemTransferOutcome::Cancelled
            ))
        ));
        drop(guard);
        assert!(event_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn explicit_failed_finish_does_not_emit_a_cancel_on_drop() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let progress_state = ZmodemProgressState::new(event_tx);
        let guard = progress_state.begin(upload_progress(1, 1_000));
        assert_progress_event(&mut event_rx).await;

        progress_state.finish(ZmodemTransferOutcome::Failed("boom".to_string()));
        assert!(matches!(
            event_rx.recv().await,
            Some(TerminalEvent::ZmodemTransferFinished(
                ZmodemTransferOutcome::Failed(error)
            )) if error == "boom"
        ));
        drop(guard);
        assert!(event_rx.try_recv().is_err());
    }

    #[test]
    fn download_start_keeps_snapshot_active_without_a_guard() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let progress_state = ZmodemProgressState::new(event_tx);

        progress_state.start(download_progress(0));
        assert_eq!(
            ZmodemTransferDirection::Download,
            progress_state.snapshot().unwrap().direction()
        );
        assert!(matches!(
            event_rx.try_recv(),
            Ok(TerminalEvent::ZmodemProgressChanged)
        ));
        assert!(event_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn unknown_total_progress_notifies_on_transferred_change() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let progress_state = ZmodemProgressState::new(event_tx);
        progress_state.start(download_progress(0));
        assert_progress_event(&mut event_rx).await;

        progress_state.update(download_progress(1));
        assert_progress_event(&mut event_rx).await;
        assert_eq!(1, progress_state.snapshot().unwrap().transferred());

        progress_state.update(download_progress(2));
        assert_progress_event(&mut event_rx).await;
        assert_eq!(2, progress_state.snapshot().unwrap().transferred());

        progress_state.finish(ZmodemTransferOutcome::Succeeded);
        assert!(matches!(
            event_rx.recv().await,
            Some(TerminalEvent::ZmodemTransferFinished(
                ZmodemTransferOutcome::Succeeded
            ))
        ));
        assert!(progress_state.snapshot().is_none());
    }

    #[test]
    fn set_direction_only_notifies_when_transfer_is_visible() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let progress_state = ZmodemProgressState::new(event_tx);

        progress_state.set_direction(ZmodemTransferDirection::Download);
        assert!(event_rx.try_recv().is_err());

        let guard = progress_state.begin(upload_progress(1, 1_000));
        progress_state.set_direction(ZmodemTransferDirection::Download);
        assert!(matches!(
            event_rx.try_recv(),
            Ok(TerminalEvent::ZmodemProgressChanged)
        ));
        assert_eq!(
            ZmodemTransferDirection::Download,
            progress_state.snapshot().unwrap().direction()
        );
        drop(guard);
    }

    async fn assert_progress_event(
        event_rx: &mut tokio::sync::mpsc::UnboundedReceiver<TerminalEvent>,
    ) {
        assert!(matches!(
            event_rx.recv().await,
            Some(TerminalEvent::ZmodemProgressChanged)
        ));
    }

    fn upload_progress(transferred: u64, total: u64) -> ZmodemTransferProgress {
        ZmodemTransferProgress {
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
