//! ACP 连接句柄与生命周期入口。

mod lifecycle;
mod notifications;
mod outcome;
mod pending;
mod prompt;
mod runner;
mod session;
mod setup;

use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::SessionId as AcpSessionId;
use agent_client_protocol::{Agent, ConnectionTo};
use agent_runtime::{RuntimeEvent, SessionId, TurnId};
use gpui::AsyncApp;
use tokio::sync::broadcast;

use crate::acp::config::AcpAgentConfig;
use crate::acp::state::{AcpConnectionPhase, AcpSessionState};
use crate::acp::turn::AcpTurnTracker;

use lifecycle::AcpConnectionLifecycle;
pub use pending::AcpPendingConnection;

pub enum AcpConnectOutcome {
    Ready(AcpConnection),
    AuthenticationRequired(AcpPendingConnection),
}

pub struct AcpConnection {
    pub(super) handle: tokio::runtime::Handle,
    pub(super) conn: ConnectionTo<Agent>,
    pub(super) acp_session_id: AcpSessionId,
    pub(super) session_id: SessionId,
    pub(super) events_tx: broadcast::Sender<RuntimeEvent>,
    pub(super) state: Arc<Mutex<AcpSessionState>>,
    pub(super) active_turn: Arc<Mutex<Option<AcpTurnTracker>>>,
    pub(super) prompt_timeout: std::time::Duration,
    pub(super) agent_id: String,
    pub(super) agent_name: String,
    _lifecycle: AcpConnectionLifecycle,
}

impl AcpConnection {
    pub async fn connect(
        config: &AcpAgentConfig,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<AcpConnectOutcome> {
        runner::connect(config, cx).await
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.events_tx.subscribe()
    }

    pub fn session_id(&self) -> SessionId {
        self.session_id.clone()
    }

    pub(crate) fn state(&self) -> AcpSessionState {
        self.state
            .lock()
            .map(|state| state.clone())
            .unwrap_or_default()
    }
}

fn new_acp_turn_id() -> TurnId {
    TurnId::from_string(format!("acp-turn:{}", uuid::Uuid::new_v4()))
}

fn transition_state(state: &Arc<Mutex<AcpSessionState>>, phase: AcpConnectionPhase) {
    if let Ok(mut state) = state.lock()
        && let Err(error) = state.transition(phase)
    {
        tracing::warn!(%error, "invalid ACP phase transition");
    }
}

fn take_active_turn_id(active: &Arc<Mutex<Option<AcpTurnTracker>>>) -> Option<TurnId> {
    active
        .lock()
        .ok()
        .and_then(|mut active| active.take())
        .map(|tracker| tracker.turn_id().clone())
}
