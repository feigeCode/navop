use std::fmt;

use agent_client_protocol::schema::{CancelNotification, ContentBlock, PromptRequest, TextContent};
use agent_runtime::{RuntimeEvent, TurnId};
use rust_i18n::t;

use crate::acp::error::extract_rpc_error_detail;
use crate::acp::state::AcpConnectionPhase;
use crate::acp::turn::{AcpTurnTracker, TurnOutcome};
use crate::acp::{AcpError, AcpErrorKind, AcpRecoveryAction};

use super::{
    AcpConnection, PromptCompletionClaim, claim_prompt_completion, connection_closed_error,
    new_acp_turn_id,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcpPromptStartError {
    AlreadyRunning,
    NotReady,
    ImageUnsupported,
}

impl fmt::Display for AcpPromptStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyRunning => {
                write!(formatter, "{}", t!("AgentUi.acp_turn_already_running"))
            }
            Self::NotReady => write!(formatter, "{}", t!("AgentUi.acp_not_ready")),
            Self::ImageUnsupported => {
                write!(formatter, "{}", t!("AgentUi.acp_image_not_supported"))
            }
        }
    }
}

impl std::error::Error for AcpPromptStartError {}

impl AcpConnection {
    pub fn prompt(&self, text: String) -> Result<TurnId, AcpPromptStartError> {
        self.try_prompt(vec![ContentBlock::Text(TextContent::new(text))])
    }

    pub fn try_prompt(&self, prompt: Vec<ContentBlock>) -> Result<TurnId, AcpPromptStartError> {
        self.validate_prompt_capabilities(&prompt)?;
        let turn_id = new_acp_turn_id();
        self.register_turn(turn_id.clone())?;
        self.emit_turn_started(turn_id.clone());
        let request = PromptRequest::new(self.acp_session_id.clone(), prompt);
        let connection = self.conn.clone();
        let acp_session_id = self.acp_session_id.clone();
        let timeout = self.prompt_timeout;
        let events = self.events_tx.clone();
        let session_id = self.session_id.clone();
        let active_turn = self.active_turn.clone();
        let state = self.state.clone();
        let agent_id = self.agent_id.clone();
        let agent_name = self.agent_name.clone();
        let expected_turn_id = turn_id.clone();
        self.handle.spawn(async move {
            let future = connection.send_request(request).block_task();
            let result = tokio::time::timeout(timeout, future).await;
            finish_prompt(
                PromptContext {
                    connection,
                    acp_session_id,
                    events,
                    session_id,
                    active_turn,
                    state,
                    agent_id,
                    agent_name,
                    expected_turn_id,
                },
                result,
            )
            .await;
        });
        Ok(turn_id)
    }

    pub fn cancel(&self) {
        let connection = self.conn.clone();
        let session_id = self.acp_session_id.clone();
        self.handle.spawn(async move {
            let _ = connection.send_notification(CancelNotification::new(session_id));
        });
    }

    fn validate_prompt_capabilities(
        &self,
        prompt: &[ContentBlock],
    ) -> Result<(), AcpPromptStartError> {
        let includes_image = prompt
            .iter()
            .any(|block| matches!(block, ContentBlock::Image(_)));
        if !includes_image {
            return Ok(());
        }
        let state = self
            .state
            .lock()
            .map_err(|_| AcpPromptStartError::NotReady)?;
        if !state.agent_capabilities().prompt_capabilities.image {
            return Err(AcpPromptStartError::ImageUnsupported);
        }
        Ok(())
    }

    fn register_turn(&self, turn_id: TurnId) -> Result<(), AcpPromptStartError> {
        let mut active = self
            .active_turn
            .lock()
            .map_err(|_| AcpPromptStartError::NotReady)?;
        if active.is_some() {
            return Err(AcpPromptStartError::AlreadyRunning);
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| AcpPromptStartError::NotReady)?;
        if !matches!(state.phase(), AcpConnectionPhase::Ready) {
            return Err(AcpPromptStartError::NotReady);
        };
        state
            .transition(AcpConnectionPhase::RunningTurn {
                turn_id: turn_id.clone(),
            })
            .map_err(|_| AcpPromptStartError::NotReady)?;
        *active = Some(AcpTurnTracker::new(turn_id.clone()));
        Ok(())
    }

    fn emit_turn_started(&self, turn_id: TurnId) {
        let _ = self.events_tx.send(RuntimeEvent::TurnStarted {
            session_id: self.session_id.clone(),
            turn_id: turn_id.clone(),
        });
        let _ = self.events_tx.send(RuntimeEvent::Status {
            session_id: self.session_id.clone(),
            turn_id,
            title: responding_status_title(&self.agent_name),
            is_done: false,
        });
    }
}

fn responding_status_title(agent_name: &str) -> String {
    t!("AgentUi.acp_responding", name = agent_name).to_string()
}

struct PromptContext {
    connection: agent_client_protocol::ConnectionTo<agent_client_protocol::Agent>,
    acp_session_id: agent_client_protocol::schema::SessionId,
    events: tokio::sync::broadcast::Sender<RuntimeEvent>,
    session_id: agent_runtime::SessionId,
    active_turn: std::sync::Arc<std::sync::Mutex<Option<AcpTurnTracker>>>,
    state: std::sync::Arc<std::sync::Mutex<crate::acp::AcpSessionState>>,
    agent_id: String,
    agent_name: String,
    expected_turn_id: TurnId,
}

async fn finish_prompt(
    context: PromptContext,
    result: Result<
        Result<agent_client_protocol::schema::PromptResponse, agent_client_protocol::Error>,
        tokio::time::error::Elapsed,
    >,
) {
    let closed_error = connection_closed_error(&context.agent_id, &context.agent_name, None);
    let claim = claim_prompt_completion(
        &context.active_turn,
        &context.state,
        &context.expected_turn_id,
        &closed_error,
    );
    let Some(claim) = claim else {
        return;
    };
    let tracker = match claim {
        PromptCompletionClaim::Ready(tracker) => tracker,
        PromptCompletionClaim::Failed { turn_id, error } => {
            emit_failed(&context, turn_id, error);
            return;
        }
    };
    let turn_id = tracker.turn_id().clone();
    match result {
        Ok(Ok(response)) => emit_success(&context, turn_id, tracker, response.stop_reason),
        Ok(Err(error)) => emit_protocol_error(&context, turn_id, error),
        Err(_) => emit_timeout(&context, turn_id).await,
    }
}

fn emit_success(
    context: &PromptContext,
    turn_id: TurnId,
    tracker: AcpTurnTracker,
    stop_reason: agent_client_protocol::schema::StopReason,
) {
    match tracker.finish_success(stop_reason) {
        TurnOutcome::Completed => emit_completed(context, turn_id),
        TurnOutcome::Cancelled => emit_cancelled(context, turn_id),
        TurnOutcome::EmptyResponse => {
            let error = AcpError::empty_response(&context.agent_id, &context.agent_name);
            emit_failed(context, turn_id, error);
        }
    }
}

fn emit_completed(context: &PromptContext, turn_id: TurnId) {
    let _ = context.events.send(RuntimeEvent::TurnCompleted {
        session_id: context.session_id.clone(),
        turn_id,
        answer: None,
    });
}

fn emit_cancelled(context: &PromptContext, turn_id: TurnId) {
    let _ = context.events.send(RuntimeEvent::TurnCancelled {
        session_id: context.session_id.clone(),
        turn_id,
    });
}

fn emit_protocol_error(
    context: &PromptContext,
    turn_id: TurnId,
    protocol: agent_client_protocol::Error,
) {
    let detail = extract_rpc_error_detail(&protocol.message, protocol.data.as_ref());
    let error = AcpError::new(
        AcpErrorKind::PromptFailed,
        &context.agent_id,
        &context.agent_name,
        t!("AgentUi.acp_prompt_failed").to_string(),
    )
    .with_detail(detail)
    .with_recovery(AcpRecoveryAction::Retry);
    emit_failed(context, turn_id, error);
}

async fn emit_timeout(context: &PromptContext, turn_id: TurnId) {
    let _ = context
        .connection
        .send_notification(CancelNotification::new(context.acp_session_id.clone()));
    let error = AcpError::new(
        AcpErrorKind::PromptTimeout,
        &context.agent_id,
        &context.agent_name,
        t!("AgentUi.acp_prompt_timeout").to_string(),
    )
    .with_recovery(AcpRecoveryAction::Retry);
    emit_failed(context, turn_id, error);
}

fn emit_failed(context: &PromptContext, turn_id: TurnId, error: AcpError) {
    tracing::warn!(kind = ?error.kind, detail = %error.detail, "ACP prompt failed");
    let _ = context.events.send(RuntimeEvent::TurnFailed {
        session_id: context.session_id.clone(),
        turn_id,
        reason: error.to_string(),
    });
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use agent_runtime::TurnId;

    use crate::acp::state::{AcpConnectionPhase, AcpSessionState};
    use crate::acp::turn::AcpTurnTracker;
    use crate::acp::{AcpError, AcpErrorKind};

    #[test]
    fn responding_status_uses_visible_agent_name() {
        assert_eq!(
            rust_i18n::t!("AgentUi.acp_responding", name = "Claude Code"),
            super::responding_status_title("Claude Code")
        );
    }

    #[test]
    fn active_turn_is_only_taken_when_the_expected_id_matches() {
        let current_turn = TurnId::from_string("current");
        let stale_turn = TurnId::from_string("stale");
        let active_turn = Arc::new(Mutex::new(Some(AcpTurnTracker::new(current_turn.clone()))));

        let state = running_state(&current_turn);
        let closed_error = closed_error();
        assert!(
            super::claim_prompt_completion(&active_turn, &state, &stale_turn, &closed_error)
                .is_none()
        );
        assert_eq!(
            Some(current_turn.clone()),
            active_turn
                .lock()
                .expect("active turn lock")
                .as_ref()
                .map(|tracker| tracker.turn_id().clone())
        );

        let super::PromptCompletionClaim::Ready(tracker) =
            super::claim_prompt_completion(&active_turn, &state, &current_turn, &closed_error)
                .expect("matching active turn should be taken")
        else {
            panic!("running prompt should complete as ready");
        };
        assert_eq!(&current_turn, tracker.turn_id());
        assert!(active_turn.lock().expect("active turn lock").is_none());
    }

    #[test]
    fn terminal_events_are_published_only_after_connection_is_ready() {
        let turn_id = TurnId::from_string("turn");
        let state = running_state(&turn_id);
        let active_turn = Arc::new(Mutex::new(Some(AcpTurnTracker::new(turn_id.clone()))));

        let claim = super::claim_prompt_completion(&active_turn, &state, &turn_id, &closed_error());

        assert!(matches!(
            claim,
            Some(super::PromptCompletionClaim::Ready(_))
        ));
        assert_eq!(
            &AcpConnectionPhase::Ready,
            state.lock().expect("state lock").phase()
        );
    }

    #[test]
    fn failed_connection_wins_prompt_completion_without_duplicate_success() {
        let turn_id = TurnId::from_string("turn");
        let state = running_state(&turn_id);
        let active_turn = Arc::new(Mutex::new(Some(AcpTurnTracker::new(turn_id.clone()))));
        let error = AcpError::new(
            AcpErrorKind::ConnectionClosed,
            "agent",
            "Agent",
            "connection failed",
        );
        state
            .lock()
            .expect("state lock")
            .transition(AcpConnectionPhase::Failed {
                error: error.clone(),
            })
            .expect("connection failure");

        let claim = super::claim_prompt_completion(&active_turn, &state, &turn_id, &closed_error());

        assert!(matches!(
            claim,
            Some(super::PromptCompletionClaim::Failed {
                turn_id: claimed_turn,
                error: claimed_error,
            }) if claimed_turn == turn_id && claimed_error == error
        ));
        assert!(active_turn.lock().expect("active turn lock").is_none());
        assert!(
            super::claim_prompt_completion(&active_turn, &state, &turn_id, &closed_error())
                .is_none(),
            "a terminal turn can only be claimed once"
        );
    }

    #[test]
    fn closed_connection_converts_late_prompt_success_into_failure() {
        let turn_id = TurnId::from_string("turn");
        let state = running_state(&turn_id);
        let active_turn = Arc::new(Mutex::new(Some(AcpTurnTracker::new(turn_id.clone()))));
        state
            .lock()
            .expect("state lock")
            .transition(AcpConnectionPhase::Closed)
            .expect("connection close");
        let closed_error = closed_error();

        let claim = super::claim_prompt_completion(&active_turn, &state, &turn_id, &closed_error);

        assert!(matches!(
            claim,
            Some(super::PromptCompletionClaim::Failed {
                turn_id: claimed_turn,
                error,
            }) if claimed_turn == turn_id && error == closed_error
        ));
        assert!(active_turn.lock().expect("active turn lock").is_none());
    }

    fn running_state(turn_id: &TurnId) -> Arc<Mutex<AcpSessionState>> {
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
            state
                .transition(AcpConnectionPhase::RunningTurn {
                    turn_id: turn_id.clone(),
                })
                .expect("run turn");
        }
        state
    }

    fn closed_error() -> AcpError {
        AcpError::new(
            AcpErrorKind::ConnectionClosed,
            "agent",
            "Agent",
            "connection closed",
        )
    }
}
