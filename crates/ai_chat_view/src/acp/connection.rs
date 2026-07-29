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
use crate::acp::permission::AcpPermissionProvider;
use crate::acp::state::{AcpConnectionPhase, AcpSessionState};
use crate::acp::turn::AcpTurnTracker;
use crate::acp::{AcpError, AcpErrorKind};

use lifecycle::AcpConnectionLifecycle;
pub use pending::AcpPendingConnection;
pub use prompt::AcpPromptStartError;

pub enum AcpConnectOutcome {
    Ready(Box<AcpConnection>),
    AuthenticationRequired(Box<AcpPendingConnection>),
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

    pub async fn connect_with_permission_provider(
        config: &AcpAgentConfig,
        permission_provider: AcpPermissionProvider,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<AcpConnectOutcome> {
        runner::connect_with_permission_provider(config, permission_provider, cx).await
    }

    #[doc(hidden)]
    pub async fn connect_with_runtime(
        config: &AcpAgentConfig,
        handle: tokio::runtime::Handle,
    ) -> anyhow::Result<AcpConnectOutcome> {
        runner::connect_with_runtime(config, handle).await
    }

    #[doc(hidden)]
    pub async fn connect_with_runtime_and_permission_provider(
        config: &AcpAgentConfig,
        handle: tokio::runtime::Handle,
        permission_provider: AcpPermissionProvider,
    ) -> anyhow::Result<AcpConnectOutcome> {
        runner::connect_with_runtime_and_permission_provider(config, handle, permission_provider)
            .await
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.events_tx.subscribe()
    }

    pub fn session_id(&self) -> SessionId {
        self.session_id.clone()
    }

    pub fn phase(&self) -> AcpConnectionPhase {
        self.state
            .lock()
            .map(|state| state.phase().clone())
            .unwrap_or(AcpConnectionPhase::Closed)
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

pub(super) enum PromptCompletionClaim {
    Ready(AcpTurnTracker),
    Failed { turn_id: TurnId, error: AcpError },
}

pub(super) fn claim_prompt_completion(
    active_turn: &Arc<Mutex<Option<AcpTurnTracker>>>,
    state: &Arc<Mutex<AcpSessionState>>,
    expected_turn_id: &TurnId,
    closed_error: &AcpError,
) -> Option<PromptCompletionClaim> {
    // All paths that need both mutexes use this order. Keeping the tracker
    // claimed together with the phase transition prevents prompt completion,
    // connection failure, and lifecycle shutdown from publishing competing
    // terminal events for the same turn.
    let mut active = active_turn.lock().ok()?;
    if !active
        .as_ref()
        .is_some_and(|tracker| tracker.turn_id() == expected_turn_id)
    {
        return None;
    }
    let mut state = state.lock().ok()?;
    match state.phase() {
        AcpConnectionPhase::RunningTurn { turn_id } if turn_id == expected_turn_id => {
            if let Err(error) = state.transition(AcpConnectionPhase::Ready) {
                tracing::warn!(%error, "failed to finish ACP prompt phase");
                return None;
            }
            active.take().map(PromptCompletionClaim::Ready)
        }
        AcpConnectionPhase::Failed { error } => {
            let error = error.clone();
            active.take().map(|tracker| PromptCompletionClaim::Failed {
                turn_id: tracker.turn_id().clone(),
                error,
            })
        }
        AcpConnectionPhase::Closed => active.take().map(|tracker| PromptCompletionClaim::Failed {
            turn_id: tracker.turn_id().clone(),
            error: closed_error.clone(),
        }),
        phase => {
            tracing::warn!(
                ?phase,
                expected_turn_id = %expected_turn_id,
                "ACP prompt completed outside its running phase"
            );
            None
        }
    }
}

pub(super) fn fail_connection_and_take_active_turn(
    active_turn: &Arc<Mutex<Option<AcpTurnTracker>>>,
    state: &Arc<Mutex<AcpSessionState>>,
    error: AcpError,
) -> Option<TurnId> {
    let mut active = active_turn.lock().ok()?;
    let mut state = state.lock().ok()?;
    if let Err(transition_error) = state.transition(AcpConnectionPhase::Failed { error }) {
        tracing::warn!(%transition_error, "failed to mark ACP connection as failed");
        return None;
    }
    active.take().map(|tracker| tracker.turn_id().clone())
}

pub(super) fn close_connection_and_take_active_turn(
    active_turn: &Arc<Mutex<Option<AcpTurnTracker>>>,
    state: &Arc<Mutex<AcpSessionState>>,
) -> Option<TurnId> {
    let mut active = active_turn.lock().ok()?;
    let mut state = state.lock().ok()?;
    if !matches!(state.phase(), AcpConnectionPhase::Closed)
        && let Err(error) = state.transition(AcpConnectionPhase::Closed)
    {
        tracing::warn!(%error, "failed to close ACP connection phase");
        return None;
    }
    active.take().map(|tracker| tracker.turn_id().clone())
}

pub(super) fn connection_closed_error(
    agent_id: &str,
    agent_name: &str,
    detail: Option<&str>,
) -> AcpError {
    let error = AcpError::new(
        AcpErrorKind::ConnectionClosed,
        agent_id,
        agent_name,
        rust_i18n::t!("AgentUi.acp_connection_closed").to_string(),
    );
    match detail {
        Some(detail) => error.with_detail(detail),
        None => error,
    }
}
