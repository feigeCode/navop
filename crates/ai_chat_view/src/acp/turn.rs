use agent_client_protocol::schema::{ContentBlock, SessionUpdate, StopReason};
use agent_runtime::TurnId;

#[derive(Clone, Debug)]
pub(crate) struct AcpTurnTracker {
    turn_id: TurnId,
    received_assistant_content: bool,
    received_reasoning: bool,
    received_tool_activity: bool,
    received_plan: bool,
}

impl AcpTurnTracker {
    pub(crate) fn new(turn_id: TurnId) -> Self {
        Self {
            turn_id,
            received_assistant_content: false,
            received_reasoning: false,
            received_tool_activity: false,
            received_plan: false,
        }
    }

    pub(crate) fn turn_id(&self) -> &TurnId {
        &self.turn_id
    }

    pub(crate) fn observe(&mut self, update: &SessionUpdate) {
        match update {
            SessionUpdate::AgentMessageChunk(chunk) => {
                self.received_assistant_content |= content_is_non_empty(&chunk.content);
            }
            SessionUpdate::AgentThoughtChunk(chunk) => {
                self.received_reasoning |= content_is_non_empty(&chunk.content);
            }
            SessionUpdate::ToolCall(_) | SessionUpdate::ToolCallUpdate(_) => {
                self.received_tool_activity = true;
            }
            SessionUpdate::Plan(_) => self.received_plan = true,
            _ => {}
        }
    }

    pub(crate) fn finish_success(self, stop_reason: StopReason) -> TurnOutcome {
        if stop_reason == StopReason::Cancelled {
            return TurnOutcome::Cancelled;
        }
        if self.has_output() {
            TurnOutcome::Completed
        } else {
            TurnOutcome::EmptyResponse
        }
    }

    fn has_output(&self) -> bool {
        self.received_assistant_content
            || self.received_reasoning
            || self.received_tool_activity
            || self.received_plan
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TurnOutcome {
    Completed,
    Cancelled,
    EmptyResponse,
}

fn content_is_non_empty(content: &ContentBlock) -> bool {
    match content {
        ContentBlock::Text(text) => !text.text.is_empty(),
        _ => true,
    }
}
