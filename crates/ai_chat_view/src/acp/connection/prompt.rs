use agent_client_protocol::schema::{CancelNotification, ContentBlock, PromptRequest, TextContent};
use agent_runtime::{RuntimeEvent, TurnId};

use crate::acp::error::extract_rpc_error_detail;
use crate::acp::state::AcpConnectionPhase;
use crate::acp::turn::{AcpTurnTracker, TurnOutcome};
use crate::acp::{AcpError, AcpErrorKind, AcpRecoveryAction};

use super::{AcpConnection, new_acp_turn_id, transition_state};

impl AcpConnection {
    pub fn prompt(&self, text: String) {
        let turn_id = new_acp_turn_id();
        if !self.register_turn(turn_id.clone()) {
            self.emit_turn_failure(turn_id, "ACP Agent 已有一轮正在运行");
            return;
        }
        self.emit_turn_started(turn_id.clone());
        let request = PromptRequest::new(
            self.acp_session_id.clone(),
            vec![ContentBlock::Text(TextContent::new(text))],
        );
        let connection = self.conn.clone();
        let acp_session_id = self.acp_session_id.clone();
        let timeout = self.prompt_timeout;
        let events = self.events_tx.clone();
        let session_id = self.session_id.clone();
        let active_turn = self.active_turn.clone();
        let state = self.state.clone();
        let agent_id = self.agent_id.clone();
        let agent_name = self.agent_name.clone();
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
                },
                result,
            )
            .await;
        });
    }

    pub fn cancel(&self) {
        let connection = self.conn.clone();
        let session_id = self.acp_session_id.clone();
        self.handle.spawn(async move {
            let _ = connection.send_notification(CancelNotification::new(session_id));
        });
    }

    fn register_turn(&self, turn_id: TurnId) -> bool {
        let Ok(mut active) = self.active_turn.lock() else {
            return false;
        };
        if active.is_some() {
            return false;
        }
        *active = Some(AcpTurnTracker::new(turn_id.clone()));
        transition_state(&self.state, AcpConnectionPhase::RunningTurn { turn_id });
        true
    }

    fn emit_turn_started(&self, turn_id: TurnId) {
        let _ = self.events_tx.send(RuntimeEvent::TurnStarted {
            session_id: self.session_id.clone(),
            turn_id: turn_id.clone(),
        });
        let _ = self.events_tx.send(RuntimeEvent::Status {
            session_id: self.session_id.clone(),
            turn_id,
            title: "ACP 正在响应…".to_string(),
            is_done: false,
        });
    }

    fn emit_turn_failure(&self, turn_id: TurnId, reason: &str) {
        let _ = self.events_tx.send(RuntimeEvent::TurnFailed {
            session_id: self.session_id.clone(),
            turn_id,
            reason: reason.to_string(),
        });
    }
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
}

async fn finish_prompt(
    context: PromptContext,
    result: Result<
        Result<agent_client_protocol::schema::PromptResponse, agent_client_protocol::Error>,
        tokio::time::error::Elapsed,
    >,
) {
    let tracker = context
        .active_turn
        .lock()
        .ok()
        .and_then(|mut active| active.take());
    let Some(tracker) = tracker else {
        return;
    };
    let turn_id = tracker.turn_id().clone();
    match result {
        Ok(Ok(response)) => emit_success(&context, turn_id, tracker, response.stop_reason),
        Ok(Err(error)) => emit_protocol_error(&context, turn_id, error),
        Err(_) => emit_timeout(&context, turn_id).await,
    }
    transition_state(&context.state, AcpConnectionPhase::Ready);
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
        "ACP 请求失败",
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
        "ACP 请求超时",
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
