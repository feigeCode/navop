use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::oneshot;

use crate::acp::state::{AcpConnectionPhase, AcpSessionState};

const SHUTDOWN_ABORT_GRACE: Duration = Duration::from_secs(2);

pub(super) struct AcpConnectionLifecycle {
    pub(super) handle: tokio::runtime::Handle,
    pub(super) join: tokio::task::JoinHandle<()>,
    pub(super) shutdown: Option<oneshot::Sender<()>>,
    pub(super) state: Arc<Mutex<AcpSessionState>>,
}

impl Drop for AcpConnectionLifecycle {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.lock() {
            let _ = state.transition(AcpConnectionPhase::Closed);
        }
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        let abort = self.join.abort_handle();
        self.handle.spawn(async move {
            tokio::time::sleep(SHUTDOWN_ABORT_GRACE).await;
            abort.abort();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dropping_connection_lifecycle_signals_shutdown_before_abort() {
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let (observed_tx, observed_rx) = oneshot::channel();
        let join = tokio::spawn(async move {
            let graceful = shutdown_rx.await.is_ok();
            let _ = observed_tx.send(graceful);
            tokio::time::sleep(Duration::from_secs(60)).await;
        });

        drop(AcpConnectionLifecycle {
            handle: tokio::runtime::Handle::current(),
            join,
            shutdown: Some(shutdown_tx),
            state: Arc::new(Mutex::new(AcpSessionState::default())),
        });

        let observed = tokio::time::timeout(Duration::from_millis(100), observed_rx)
            .await
            .expect("connection task should observe lifecycle drop")
            .expect("connection task should report shutdown reason");
        assert!(observed, "drop should send shutdown before aborting task");
    }
}
