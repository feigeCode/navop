use agent_client_protocol::schema::{
    ContentBlock, ContentChunk, SessionUpdate, StopReason, TextContent, ToolCall,
};
use agent_runtime::TurnId;

use super::turn::{AcpTurnTracker, TurnOutcome};

#[test]
fn successful_rpc_without_agent_output_is_empty_response() {
    let tracker = AcpTurnTracker::new(TurnId::from_string("turn"));

    assert_eq!(
        TurnOutcome::EmptyResponse,
        tracker.finish_success(StopReason::EndTurn)
    );
}

#[test]
fn tool_activity_makes_the_turn_successful() {
    let mut tracker = AcpTurnTracker::new(TurnId::from_string("turn"));
    tracker.observe(&SessionUpdate::ToolCall(ToolCall::new("call", "Read file")));

    assert_eq!(
        TurnOutcome::Completed,
        tracker.finish_success(StopReason::EndTurn)
    );
}

#[test]
fn empty_assistant_text_is_not_valid_output() {
    let mut tracker = AcpTurnTracker::new(TurnId::from_string("turn"));
    tracker.observe(&SessionUpdate::AgentMessageChunk(ContentChunk::new(
        ContentBlock::Text(TextContent::new("")),
    )));

    assert_eq!(
        TurnOutcome::EmptyResponse,
        tracker.finish_success(StopReason::EndTurn)
    );
}

#[test]
fn cancelled_stop_reason_is_cancelled() {
    let tracker = AcpTurnTracker::new(TurnId::from_string("turn"));

    assert_eq!(
        TurnOutcome::Cancelled,
        tracker.finish_success(StopReason::Cancelled)
    );
}
