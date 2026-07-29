use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_runtime::{RuntimeEvent, SessionId};
use tokio::sync::broadcast;
use tokio::sync::oneshot;

use crate::acp::AcpError;
use crate::acp::state::AcpSessionState;
use crate::acp::turn::AcpTurnTracker;

use super::close_connection_and_take_active_turn;

const SHUTDOWN_ABORT_GRACE: Duration = Duration::from_secs(2);

pub(super) struct AcpConnectionLifecycle {
    pub(super) handle: tokio::runtime::Handle,
    pub(super) join: tokio::task::JoinHandle<()>,
    pub(super) shutdown: Option<oneshot::Sender<()>>,
    pub(super) state: Arc<Mutex<AcpSessionState>>,
    pub(super) active_turn: Arc<Mutex<Option<AcpTurnTracker>>>,
    pub(super) events_tx: broadcast::Sender<RuntimeEvent>,
    pub(super) session_id: SessionId,
    pub(super) closed_error: AcpError,
}

impl Drop for AcpConnectionLifecycle {
    fn drop(&mut self) {
        let terminal_turn = close_connection_and_take_active_turn(&self.active_turn, &self.state);
        if let Some(turn_id) = terminal_turn {
            let _ = self.events_tx.send(RuntimeEvent::TurnFailed {
                session_id: self.session_id.clone(),
                turn_id,
                reason: self.closed_error.to_string(),
            });
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
    use crate::acp::state::AcpConnectionPhase;
    use agent_runtime::TurnId;

    #[tokio::test]
    async fn dropping_connection_lifecycle_signals_shutdown_before_abort() {
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let (observed_tx, observed_rx) = oneshot::channel();
        let (events_tx, _events_rx) = broadcast::channel(4);
        let join = tokio::spawn(async move {
            let graceful = shutdown_rx.await.is_ok();
            let _ = observed_tx.send(graceful);
            tokio::time::sleep(Duration::from_secs(60)).await;
        });

        drop(AcpConnectionLifecycle {
            handle: tokio::runtime::Handle::current(),
            join,
            shutdown: Some(shutdown_tx),
            state: ready_state(),
            active_turn: Arc::new(Mutex::new(None)),
            events_tx,
            session_id: SessionId::from_string("session"),
            closed_error: closed_error(),
        });

        let observed = tokio::time::timeout(Duration::from_millis(100), observed_rx)
            .await
            .expect("connection task should observe lifecycle drop")
            .expect("connection task should report shutdown reason");
        assert!(observed, "drop should send shutdown before aborting task");
    }

    #[tokio::test]
    async fn dropping_connection_lifecycle_fails_active_turn_once() {
        let (shutdown_tx, _shutdown_rx) = oneshot::channel();
        let join = tokio::spawn(std::future::pending::<()>());
        let (events_tx, mut events_rx) = broadcast::channel(4);
        let session_id = SessionId::from_string("session");
        let turn_id = TurnId::from_string("turn");
        let state = ready_state();
        state
            .lock()
            .expect("state lock")
            .transition(AcpConnectionPhase::RunningTurn {
                turn_id: turn_id.clone(),
            })
            .expect("running turn");
        let active_turn = Arc::new(Mutex::new(Some(AcpTurnTracker::new(turn_id.clone()))));

        drop(AcpConnectionLifecycle {
            handle: tokio::runtime::Handle::current(),
            join,
            shutdown: Some(shutdown_tx),
            state: state.clone(),
            active_turn: active_turn.clone(),
            events_tx,
            session_id: session_id.clone(),
            closed_error: closed_error(),
        });

        let event = events_rx.recv().await.expect("terminal event");
        assert!(matches!(
            event,
            RuntimeEvent::TurnFailed {
                session_id: event_session,
                turn_id: event_turn,
                ..
            } if event_session == session_id && event_turn == turn_id
        ));
        assert!(active_turn.lock().expect("active turn lock").is_none());
        assert_eq!(
            &AcpConnectionPhase::Closed,
            state.lock().expect("state lock").phase()
        );
        match tokio::time::timeout(Duration::from_millis(10), events_rx.recv()).await {
            Err(_) | Ok(Err(broadcast::error::RecvError::Closed)) => {}
            Ok(Err(broadcast::error::RecvError::Lagged(skipped))) => {
                panic!("unexpected lag while checking terminal events: skipped {skipped}")
            }
            Ok(Ok(event)) => panic!("unexpected duplicate terminal event: {event:?}"),
        }
    }

    #[tokio::test]
    async fn dropping_starting_connection_lifecycle_closes_state() {
        let (shutdown_tx, _shutdown_rx) = oneshot::channel();
        let join = tokio::spawn(std::future::pending::<()>());
        let (events_tx, mut events_rx) = broadcast::channel(4);
        let state = Arc::new(Mutex::new(AcpSessionState::default()));

        drop(AcpConnectionLifecycle {
            handle: tokio::runtime::Handle::current(),
            join,
            shutdown: Some(shutdown_tx),
            state: state.clone(),
            active_turn: Arc::new(Mutex::new(None)),
            events_tx,
            session_id: SessionId::from_string("session"),
            closed_error: closed_error(),
        });

        assert_eq!(
            &AcpConnectionPhase::Closed,
            state.lock().expect("state lock").phase()
        );
        match tokio::time::timeout(Duration::from_millis(10), events_rx.recv()).await {
            Err(_) | Ok(Err(broadcast::error::RecvError::Closed)) => {}
            Ok(Err(broadcast::error::RecvError::Lagged(skipped))) => {
                panic!("unexpected lag while checking terminal events: skipped {skipped}")
            }
            Ok(Ok(event)) => panic!("startup close must not emit a turn event: {event:?}"),
        }
    }

    fn ready_state() -> Arc<Mutex<AcpSessionState>> {
        let state = Arc::new(Mutex::new(AcpSessionState::default()));
        {
            let mut state = state.lock().expect("state lock");
            state
                .transition(AcpConnectionPhase::Initializing)
                .expect("initialize");
            state
                .transition(AcpConnectionPhase::CreatingSession)
                .expect("create session");
            state.transition(AcpConnectionPhase::Ready).expect("ready");
        }
        state
    }

    fn closed_error() -> AcpError {
        AcpError::new(
            crate::acp::AcpErrorKind::ConnectionClosed,
            "agent",
            "Agent",
            "connection closed",
        )
    }
}
