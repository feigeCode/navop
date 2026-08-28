use super::{
    TransferProgressGuard, ZmodemProgressState, ZmodemTransferDirection, ZmodemTransferId,
    ZmodemTransferOutcome, ZmodemTransferProgress,
};
use crate::TerminalEvent;
use anyhow::{Context as _, Result, bail};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex},
};
use tokio::sync::{mpsc::UnboundedSender, oneshot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZmodemPickerKind {
    UploadFiles,
    DownloadDirectory,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ZmodemPickerRequest {
    id: u64,
    kind: ZmodemPickerKind,
}

impl ZmodemPickerRequest {
    pub fn id(self) -> u64 {
        self.id
    }

    pub fn kind(self) -> ZmodemPickerKind {
        self.kind
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ZmodemPickerResponse {
    UploadFiles(Vec<PathBuf>),
    DownloadDirectory(PathBuf),
    Cancel,
}

#[derive(Clone, Default)]
pub(crate) struct ZmodemResponder {
    state: Arc<StdMutex<ZmodemState>>,
    progress: ZmodemProgressState,
    event_tx: Option<UnboundedSender<TerminalEvent>>,
}

#[derive(Default)]
struct ZmodemState {
    next_id: u64,
    pending: Option<ZmodemPending>,
    picker_claim: Option<u64>,
}

struct ZmodemPending {
    request: ZmodemPickerRequest,
    response_tx: Option<oneshot::Sender<ZmodemPickerResponse>>,
}

struct PendingGuard {
    responder: ZmodemResponder,
    request_id: u64,
}

/// Exclusive ownership of the system picker for one pending ZMODEM request.
///
/// A `Terminal` can have more than one `TerminalView`, so the ownership must
/// live alongside the shared pending request rather than in a view-local
/// field. Dropping this claim leaves the request available for another view
/// to present.
pub struct ZmodemPickerClaim {
    responder: ZmodemResponder,
    request_id: u64,
    submitted: bool,
}

impl ZmodemResponder {
    pub(crate) fn new(event_tx: UnboundedSender<TerminalEvent>) -> Self {
        Self {
            state: Arc::new(StdMutex::new(ZmodemState::default())),
            progress: ZmodemProgressState::new(event_tx.clone()),
            event_tx: Some(event_tx),
        }
    }

    pub(crate) fn pending_request(&self) -> Option<ZmodemPickerRequest> {
        self.state
            .lock()
            .ok()?
            .pending
            .as_ref()
            .map(|pending| pending.request)
    }

    pub(crate) fn transfer_progress(&self) -> Option<ZmodemTransferProgress> {
        self.progress.snapshot()
    }

    pub(crate) fn begin_transfer(&self, direction: ZmodemTransferDirection) -> ZmodemTransferId {
        self.progress.begin_transfer(direction)
    }

    pub(crate) fn begin_upload(
        &self,
        transfer_id: ZmodemTransferId,
        progress: ZmodemTransferProgress,
    ) -> TransferProgressGuard {
        self.progress.begin(transfer_id, progress)
    }

    pub(crate) fn update_upload(
        &self,
        transfer_id: ZmodemTransferId,
        progress: ZmodemTransferProgress,
    ) {
        self.progress.update(transfer_id, progress);
    }

    pub(crate) fn begin_download(
        &self,
        transfer_id: ZmodemTransferId,
        progress: ZmodemTransferProgress,
    ) {
        self.progress.start(transfer_id, progress);
    }

    pub(crate) fn update_download(
        &self,
        transfer_id: ZmodemTransferId,
        progress: ZmodemTransferProgress,
    ) {
        self.progress.update(transfer_id, progress);
    }

    pub(crate) fn finish_transfer(
        &self,
        transfer_id: ZmodemTransferId,
        outcome: ZmodemTransferOutcome,
    ) {
        self.progress.finish(transfer_id, outcome);
    }

    pub(crate) fn submit(&self, response: ZmodemPickerResponse) -> bool {
        self.submit_pending(response)
    }

    pub(crate) fn try_claim_picker(&self, request_id: u64) -> Option<ZmodemPickerClaim> {
        let mut state = self.state.lock().ok()?;
        let matches_pending = state
            .pending
            .as_ref()
            .is_some_and(|pending| pending.request.id == request_id);
        if !matches_pending || state.picker_claim.is_some() {
            return None;
        }
        state.picker_claim = Some(request_id);
        Some(ZmodemPickerClaim {
            responder: self.clone(),
            request_id,
            submitted: false,
        })
    }

    fn submit_pending(&self, response: ZmodemPickerResponse) -> bool {
        let Some(mut pending) = self.take_pending() else {
            return false;
        };
        let sent = pending
            .response_tx
            .take()
            .is_some_and(|tx| tx.send(response).is_ok());
        self.notify_changed();
        sent
    }

    fn submit_claim(&self, request_id: u64, response: ZmodemPickerResponse) -> bool {
        let Some(mut pending) = self.take_claimed_pending(request_id) else {
            return false;
        };
        let sent = pending
            .response_tx
            .take()
            .is_some_and(|tx| tx.send(response).is_ok());
        self.notify_changed();
        sent
    }

    pub(crate) fn cancel(&self) -> bool {
        let cleared = self.take_pending().is_some();
        if cleared {
            self.notify_changed();
        }
        cleared
    }

    pub(crate) async fn request(&self, kind: ZmodemPickerKind) -> Result<ZmodemPickerResponse> {
        let (response_tx, response_rx) = oneshot::channel();
        let request_id = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| anyhow::anyhow!("ZMODEM picker state is poisoned"))?;
            if state.pending.is_some() {
                bail!("a ZMODEM picker request is already pending");
            }
            state.next_id = state.next_id.wrapping_add(1).max(1);
            let request = ZmodemPickerRequest {
                id: state.next_id,
                kind,
            };
            state.pending = Some(ZmodemPending {
                request,
                response_tx: Some(response_tx),
            });
            state.picker_claim = None;
            request.id
        };

        self.notify_changed();
        let _guard = PendingGuard {
            responder: self.clone(),
            request_id,
        };
        response_rx
            .await
            .context("ZMODEM picker request was cancelled")
    }

    fn take_pending(&self) -> Option<ZmodemPending> {
        self.state.lock().ok().and_then(|mut state| {
            state.picker_claim = None;
            state.pending.take()
        })
    }

    fn take_claimed_pending(&self, request_id: u64) -> Option<ZmodemPending> {
        self.state.lock().ok().and_then(|mut state| {
            if state.picker_claim != Some(request_id) {
                return None;
            }
            let pending = state.pending.as_ref()?;
            if pending.request.id != request_id {
                return None;
            }
            state.picker_claim = None;
            state.pending.take()
        })
    }

    fn release_picker_claim(&self, request_id: u64) {
        let released = self.state.lock().ok().is_some_and(|mut state| {
            if state.picker_claim != Some(request_id) {
                return false;
            }
            state.picker_claim = None;
            true
        });
        if released {
            self.notify_changed();
        }
    }

    fn clear_request(&self, request_id: u64) {
        let cleared = self
            .state
            .lock()
            .ok()
            .and_then(|mut state| {
                let is_match = state
                    .pending
                    .as_ref()
                    .is_some_and(|pending| pending.request.id == request_id);
                if is_match {
                    state.picker_claim = None;
                }
                is_match.then(|| state.pending.take()).flatten()
            })
            .is_some();
        if cleared {
            self.notify_changed();
        }
    }

    fn notify_changed(&self) {
        if let Some(event_tx) = &self.event_tx {
            let _ = event_tx.send(TerminalEvent::ZmodemRequestChanged);
        }
    }
}

impl ZmodemPickerClaim {
    pub fn submit(mut self, response: ZmodemPickerResponse) -> bool {
        let sent = self.responder.submit_claim(self.request_id, response);
        self.submitted = sent;
        sent
    }
}

impl Drop for ZmodemPickerClaim {
    fn drop(&mut self) {
        if !self.submitted {
            self.responder.release_picker_claim(self.request_id);
        }
    }
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.responder.clear_request(self.request_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc::unbounded_channel;

    #[tokio::test]
    async fn request_publishes_picker_request() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let responder = ZmodemResponder::new(event_tx);
        let task_responder = responder.clone();
        let task =
            tokio::spawn(
                async move { task_responder.request(ZmodemPickerKind::UploadFiles).await },
            );

        let event = event_rx.recv().await.expect("picker event");
        assert!(matches!(event, TerminalEvent::ZmodemRequestChanged));
        let request = responder.pending_request().expect("pending request");
        assert_eq!(request.kind(), ZmodemPickerKind::UploadFiles);

        assert!(responder.submit(ZmodemPickerResponse::Cancel));
        assert_eq!(task.await.unwrap().unwrap(), ZmodemPickerResponse::Cancel);
    }

    #[tokio::test]
    async fn submit_resolves_and_clears_pending_request() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let responder = ZmodemResponder::new(event_tx);
        let task_responder = responder.clone();
        let task = tokio::spawn(async move {
            task_responder
                .request(ZmodemPickerKind::DownloadDirectory)
                .await
        });
        event_rx.recv().await.expect("request event");

        let directory = PathBuf::from("/tmp/downloads");
        assert!(responder.submit(ZmodemPickerResponse::DownloadDirectory(directory.clone())));
        assert!(responder.pending_request().is_none());
        assert_eq!(
            task.await.unwrap().unwrap(),
            ZmodemPickerResponse::DownloadDirectory(directory)
        );
        assert!(matches!(
            event_rx.recv().await,
            Some(TerminalEvent::ZmodemRequestChanged)
        ));
    }

    #[tokio::test]
    async fn cancel_releases_waiter_and_notifies() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let responder = ZmodemResponder::new(event_tx);
        let task_responder = responder.clone();
        let task =
            tokio::spawn(
                async move { task_responder.request(ZmodemPickerKind::UploadFiles).await },
            );
        event_rx.recv().await.expect("request event");

        assert!(responder.cancel());
        assert!(task.await.unwrap().is_err());
        assert!(responder.pending_request().is_none());
        assert!(matches!(
            event_rx.recv().await,
            Some(TerminalEvent::ZmodemRequestChanged)
        ));
    }

    #[tokio::test]
    async fn second_request_is_rejected_while_picker_is_pending() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let responder = ZmodemResponder::new(event_tx);
        let task_responder = responder.clone();
        let task =
            tokio::spawn(
                async move { task_responder.request(ZmodemPickerKind::UploadFiles).await },
            );
        event_rx.recv().await.expect("request event");

        let error = responder
            .request(ZmodemPickerKind::DownloadDirectory)
            .await
            .expect_err("second request should fail");
        assert!(error.to_string().contains("already pending"));

        responder.cancel();
        let _ = task.await;
    }

    #[tokio::test]
    async fn only_one_picker_claim_can_own_a_request() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let responder = ZmodemResponder::new(event_tx);
        let task_responder = responder.clone();
        let task = tokio::spawn(async move {
            task_responder
                .request(ZmodemPickerKind::DownloadDirectory)
                .await
        });
        event_rx.recv().await.expect("request event");
        let request_id = responder.pending_request().unwrap().id();

        let claim = responder.try_claim_picker(request_id).expect("first claim");
        assert!(responder.clone().try_claim_picker(request_id).is_none());
        assert!(responder.pending_request().is_some());

        drop(claim);
        assert!(responder.clone().try_claim_picker(request_id).is_some());

        assert!(responder.cancel());
        assert!(task.await.unwrap().is_err());
    }

    #[tokio::test]
    async fn picker_claim_submit_is_bound_to_its_request() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let responder = ZmodemResponder::new(event_tx);
        let task_responder = responder.clone();
        let task =
            tokio::spawn(
                async move { task_responder.request(ZmodemPickerKind::UploadFiles).await },
            );
        event_rx.recv().await.expect("request event");
        let request_id = responder.pending_request().unwrap().id();
        let claim = responder.try_claim_picker(request_id).unwrap();

        assert!(claim.submit(ZmodemPickerResponse::Cancel));
        assert_eq!(task.await.unwrap().unwrap(), ZmodemPickerResponse::Cancel);
        assert!(responder.pending_request().is_none());
        assert!(responder.try_claim_picker(request_id).is_none());
    }

    #[tokio::test]
    async fn stale_picker_claim_cannot_affect_a_new_request() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let responder = ZmodemResponder::new(event_tx);
        let first_responder = responder.clone();
        let first =
            tokio::spawn(
                async move { first_responder.request(ZmodemPickerKind::UploadFiles).await },
            );
        event_rx.recv().await.expect("first request event");
        let first_id = responder.pending_request().unwrap().id();
        let stale_claim = responder.try_claim_picker(first_id).unwrap();

        assert!(responder.cancel());
        assert!(first.await.unwrap().is_err());
        event_rx.recv().await.expect("first clear event");

        let second_responder = responder.clone();
        let second = tokio::spawn(async move {
            second_responder
                .request(ZmodemPickerKind::DownloadDirectory)
                .await
        });
        event_rx.recv().await.expect("second request event");
        let second_id = responder.pending_request().unwrap().id();
        assert_ne!(first_id, second_id);
        drop(stale_claim);
        assert!(responder.pending_request().is_some());
        let claim = responder.try_claim_picker(second_id).unwrap();
        assert!(claim.submit(ZmodemPickerResponse::Cancel));
        assert!(second.await.unwrap().is_ok());
    }

    #[tokio::test]
    async fn dropping_stale_picker_claim_does_not_release_new_claim() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let responder = ZmodemResponder::new(event_tx);
        let first_responder = responder.clone();
        let first =
            tokio::spawn(
                async move { first_responder.request(ZmodemPickerKind::UploadFiles).await },
            );
        event_rx.recv().await.expect("first request event");
        let first_id = responder.pending_request().unwrap().id();
        let stale_claim = responder.try_claim_picker(first_id).unwrap();

        assert!(responder.cancel());
        assert!(first.await.unwrap().is_err());
        event_rx.recv().await.expect("first clear event");

        let second_responder = responder.clone();
        let second = tokio::spawn(async move {
            second_responder
                .request(ZmodemPickerKind::DownloadDirectory)
                .await
        });
        event_rx.recv().await.expect("second request event");
        let second_id = responder.pending_request().unwrap().id();
        let second_claim = responder.try_claim_picker(second_id).unwrap();

        drop(stale_claim);

        assert!(second_claim.submit(ZmodemPickerResponse::Cancel));
        assert!(second.await.unwrap().is_ok());
    }
}
