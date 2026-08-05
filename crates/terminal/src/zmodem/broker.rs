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
    event_tx: Option<UnboundedSender<TerminalEvent>>,
}

#[derive(Default)]
struct ZmodemState {
    next_id: u64,
    pending: Option<ZmodemPending>,
}

struct ZmodemPending {
    request: ZmodemPickerRequest,
    response_tx: Option<oneshot::Sender<ZmodemPickerResponse>>,
}

struct PendingGuard {
    responder: ZmodemResponder,
    request_id: u64,
}

impl ZmodemResponder {
    pub(crate) fn new(event_tx: UnboundedSender<TerminalEvent>) -> Self {
        Self {
            state: Arc::new(StdMutex::new(ZmodemState::default())),
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

    pub(crate) fn submit(&self, response: ZmodemPickerResponse) -> bool {
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
        self.state
            .lock()
            .ok()
            .and_then(|mut state| state.pending.take())
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
}
