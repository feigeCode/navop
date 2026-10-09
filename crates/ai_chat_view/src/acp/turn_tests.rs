use agent_client_protocol::schema::v1::{
    ContentBlock, ContentChunk, SessionUpdate, StopReason, TextContent, ToolCall,
    ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields,
};
use agent_runtime::TurnId;

use super::turn::{AcpTurnTracker, TurnOutcome, TurnProgress};

#[test]
fn successful_rpc_without_agent_output_is_empty_response() {
    let tracker = AcpTurnTracker::new(TurnId::from_string("turn"), "ses_main".to_string());

    assert_eq!(
        TurnOutcome::EmptyResponse,
        tracker.finish_success(StopReason::EndTurn)
    );
}

#[test]
fn tool_activity_makes_the_turn_successful() {
    let mut tracker = AcpTurnTracker::new(TurnId::from_string("turn"), "ses_main".to_string());
    tracker.observe(&SessionUpdate::ToolCall(ToolCall::new("call", "Read file")));

    assert_eq!(
        TurnOutcome::Completed,
        tracker.finish_success(StopReason::EndTurn)
    );
}

#[test]
fn empty_assistant_text_is_not_valid_output() {
    let mut tracker = AcpTurnTracker::new(TurnId::from_string("turn"), "ses_main".to_string());
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
    let tracker = AcpTurnTracker::new(TurnId::from_string("turn"), "ses_main".to_string());

    assert_eq!(
        TurnOutcome::Cancelled,
        tracker.finish_success(StopReason::Cancelled)
    );
}

fn tool_call(id: &str, status: ToolCallStatus) -> SessionUpdate {
    let mut call = ToolCall::new(id.to_string(), "subagent");
    call.status = status;
    SessionUpdate::ToolCall(call)
}

fn tool_status(id: &str, status: ToolCallStatus) -> SessionUpdate {
    SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
        id.to_string(),
        ToolCallUpdateFields::new().status(status),
    ))
}

fn progress_of(tracker: &AcpTurnTracker) -> TurnProgress {
    *tracker.progress().borrow()
}

#[test]
fn a_started_tool_is_inflight_and_stops_being_inflight_when_it_settles() {
    let mut tracker = AcpTurnTracker::new(TurnId::from_string("turn"), "ses_main".to_string());

    tracker.observe(&tool_call("call", ToolCallStatus::InProgress));
    assert_eq!(1, progress_of(&tracker).inflight_tools);

    tracker.observe(&tool_status("call", ToolCallStatus::Completed));
    assert_eq!(
        0,
        progress_of(&tracker).inflight_tools,
        "工具收尾后必须退出在飞集合，否则这一轮永远不会被判定超时"
    );
}

#[test]
fn a_pending_tool_counts_as_inflight_because_it_is_waiting_on_approval() {
    let mut tracker = AcpTurnTracker::new(TurnId::from_string("turn"), "ses_main".to_string());

    // `Pending` 的语义是「入参还在流，或者正在等审批」——agent 在等外部条件，不是卡死。
    tracker.observe(&tool_call("call", ToolCallStatus::Pending));

    assert_eq!(1, progress_of(&tracker).inflight_tools);
}

#[test]
fn a_failed_tool_is_no_longer_inflight() {
    let mut tracker = AcpTurnTracker::new(TurnId::from_string("turn"), "ses_main".to_string());

    tracker.observe(&tool_call("call", ToolCallStatus::InProgress));
    tracker.observe(&tool_status("call", ToolCallStatus::Failed));

    // 失败也是一次收尾：agent 会拿到错误继续往下走，不该再压着超时。
    assert_eq!(0, progress_of(&tracker).inflight_tools);
}

#[test]
fn a_status_free_update_does_not_settle_a_running_tool() {
    let mut tracker = AcpTurnTracker::new(TurnId::from_string("turn"), "ses_main".to_string());
    tracker.observe(&tool_call("call", ToolCallStatus::InProgress));

    // 工具的输出是增量推送的：这些更新不带 status，必须原样保留在飞状态。
    tracker.observe(&SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
        "call",
        ToolCallUpdateFields::new().title("still running"),
    )));

    assert_eq!(1, progress_of(&tracker).inflight_tools);
}

#[test]
fn concurrent_tools_are_counted_separately() {
    let mut tracker = AcpTurnTracker::new(TurnId::from_string("turn"), "ses_main".to_string());

    tracker.observe(&tool_call("first", ToolCallStatus::InProgress));
    tracker.observe(&tool_call("second", ToolCallStatus::InProgress));
    tracker.observe(&tool_status("first", ToolCallStatus::Completed));

    assert_eq!(
        1,
        progress_of(&tracker).inflight_tools,
        "一个工具收尾不能把另一个还在跑的工具一起放掉"
    );
}

#[test]
fn every_update_advances_the_progress_revision() {
    let mut tracker = AcpTurnTracker::new(TurnId::from_string("turn"), "ses_main".to_string());
    let before = progress_of(&tracker).revision;

    tracker.observe(&SessionUpdate::AgentMessageChunk(ContentChunk::new(
        ContentBlock::Text(TextContent::new("hi")),
    )));
    tracker.observe(&tool_call("call", ToolCallStatus::InProgress));

    assert_eq!(
        before + 2,
        progress_of(&tracker).revision,
        "每条通知都要推进刻度，否则等待方会把「正在流式输出」当成静默"
    );
}

#[test]
fn a_subscribed_receiver_sees_progress_published_after_it_subscribed() {
    let mut tracker = AcpTurnTracker::new(TurnId::from_string("turn"), "ses_main".to_string());
    let mut receiver = tracker.progress();

    tracker.observe(&tool_call("call", ToolCallStatus::InProgress));

    assert!(
        receiver.has_changed().expect("sender is alive"),
        "订阅之后发生的进展必须能被等待方看见"
    );
    assert_eq!(1, receiver.borrow_and_update().inflight_tools);
}
