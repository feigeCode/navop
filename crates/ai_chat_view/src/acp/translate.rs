//! ACP `SessionUpdate` → `agent_runtime::RuntimeEvent` 翻译。
//!
//! 复用点:翻译成现有 `RuntimeEvent` 后,直接喂 `AgentTranscript` 的归约逻辑,
//! ACP 后端无需任何新增渲染代码。纯函数,便于单测。

use agent_client_protocol::schema::{
    ContentBlock, Plan as AcpPlan, PlanEntryStatus, SessionUpdate, ToolCall as AcpToolCall,
    ToolCallStatus, ToolCallUpdate,
};
use agent_runtime::tools::{ObservationData, ToolName};
use agent_runtime::{
    Plan, PlanSource, PlanStatus, PlanStep, RuntimeEvent, SessionId, StepStatus, ToolCallId,
    ToolObservation, TurnId,
};

const MAX_DELTA_CHARS: usize = 8;

#[derive(Default)]
pub(crate) struct AcpEventTranslator;

impl AcpEventTranslator {
    pub(crate) fn session_update_to_events(
        &mut self,
        update: &SessionUpdate,
        session_id: &SessionId,
        turn_id: &TurnId,
    ) -> Vec<RuntimeEvent> {
        session_update_to_events(update, session_id, turn_id)
    }
}

/// 把一条 ACP `SessionUpdate` 翻译为 0..N 条 `RuntimeEvent`。
///
/// `session_id` / `turn_id` 为本次 ACP 会话的合成 id(view 侧事件泵据此过滤)。
pub(crate) fn session_update_to_events(
    update: &SessionUpdate,
    session_id: &SessionId,
    turn_id: &TurnId,
) -> Vec<RuntimeEvent> {
    match update {
        SessionUpdate::UserMessageChunk(chunk) => {
            let text = content_block_text(&chunk.content);
            user_message_events(text, session_id, turn_id)
        }
        SessionUpdate::AgentMessageChunk(chunk) => {
            let delta = content_block_text(&chunk.content);
            assistant_delta_events(delta, session_id, turn_id)
        }
        SessionUpdate::AgentThoughtChunk(chunk) => {
            let delta = content_block_text(&chunk.content);
            reasoning_delta_events(delta, session_id, turn_id)
        }
        SessionUpdate::ToolCall(call) => tool_call_events(call, session_id, turn_id),
        SessionUpdate::ToolCallUpdate(update) => {
            tool_call_update_events(update, session_id, turn_id)
        }
        SessionUpdate::Plan(plan) => vec![RuntimeEvent::PlanUpdated {
            session_id: session_id.clone(),
            turn_id: turn_id.clone(),
            plan: acp_plan_to_runtime(plan),
        }],
        SessionUpdate::AvailableCommandsUpdate(_)
        | SessionUpdate::CurrentModeUpdate(_)
        | SessionUpdate::ConfigOptionUpdate(_)
        | SessionUpdate::SessionInfoUpdate(_)
        | SessionUpdate::UsageUpdate(_) => Vec::new(),
        _ => {
            tracing::debug!(
                update = acp_update_kind(update),
                "ignoring acp session update"
            );
            Vec::new()
        }
    }
}

fn user_message_events(
    text: String,
    session_id: &SessionId,
    turn_id: &TurnId,
) -> Vec<RuntimeEvent> {
    if text.is_empty() {
        return Vec::new();
    }
    vec![RuntimeEvent::UserMessage {
        session_id: session_id.clone(),
        turn_id: turn_id.clone(),
        text,
    }]
}

fn assistant_delta_events(
    delta: String,
    session_id: &SessionId,
    turn_id: &TurnId,
) -> Vec<RuntimeEvent> {
    chunk_text(&delta, MAX_DELTA_CHARS)
        .into_iter()
        .map(|delta| RuntimeEvent::AssistantMessageDelta {
            session_id: session_id.clone(),
            turn_id: turn_id.clone(),
            delta,
        })
        .collect()
}

fn reasoning_delta_events(
    delta: String,
    session_id: &SessionId,
    turn_id: &TurnId,
) -> Vec<RuntimeEvent> {
    if delta.is_empty() {
        return Vec::new();
    }
    vec![RuntimeEvent::ReasoningDelta {
        session_id: session_id.clone(),
        turn_id: turn_id.clone(),
        delta,
    }]
}

fn status_done(title: String, session_id: &SessionId, turn_id: &TurnId) -> RuntimeEvent {
    RuntimeEvent::Status {
        session_id: session_id.clone(),
        turn_id: turn_id.clone(),
        title,
        is_done: true,
    }
}

fn chunk_text(text: &str, max_chars: usize) -> Vec<String> {
    if text.is_empty() || max_chars == 0 {
        return Vec::new();
    }
    let mut chunks = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        current.push(ch);
        if current.chars().count() >= max_chars {
            chunks.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn acp_update_kind(update: &SessionUpdate) -> &'static str {
    match update {
        SessionUpdate::UserMessageChunk(_) => "user_message_chunk",
        SessionUpdate::AgentMessageChunk(_) => "agent_message_chunk",
        SessionUpdate::AgentThoughtChunk(_) => "agent_thought_chunk",
        SessionUpdate::ToolCall(_) => "tool_call",
        SessionUpdate::ToolCallUpdate(_) => "tool_call_update",
        SessionUpdate::Plan(_) => "plan",
        SessionUpdate::AvailableCommandsUpdate(_) => "available_commands_update",
        SessionUpdate::CurrentModeUpdate(_) => "current_mode_update",
        SessionUpdate::ConfigOptionUpdate(_) => "config_option_update",
        SessionUpdate::SessionInfoUpdate(_) => "session_info_update",
        SessionUpdate::UsageUpdate(_) => "usage_update",
        _ => "unknown",
    }
}

fn tool_call_events(call: &AcpToolCall, sid: &SessionId, tid: &TurnId) -> Vec<RuntimeEvent> {
    let call_id = ToolCallId::from_string(call.tool_call_id.0.to_string());
    let tool_name = ToolName::new(call.title.clone());
    let mut events = vec![RuntimeEvent::ToolCallStarted {
        session_id: sid.clone(),
        turn_id: tid.clone(),
        call_id: call_id.clone(),
        tool_name: tool_name.clone(),
        arguments: call.raw_input.clone().unwrap_or(serde_json::Value::Null),
    }];
    // ToolCall 携带终态(部分 agent 一步到位),补观测 + 完成事件。
    if let Some(success) = terminal_success(call.status) {
        let text = tool_text(
            call.raw_output.as_ref(),
            serde_json::to_string(&call.content).ok(),
        );
        events.push(RuntimeEvent::ObservationAdded {
            session_id: sid.clone(),
            turn_id: tid.clone(),
            observation: build_observation(call_id.clone(), tool_name, &call.title, text, success),
        });
        events.push(RuntimeEvent::ToolCallFinished {
            session_id: sid.clone(),
            turn_id: tid.clone(),
            call_id,
            success,
        });
    }
    events
}

fn tool_call_update_events(u: &ToolCallUpdate, sid: &SessionId, tid: &TurnId) -> Vec<RuntimeEvent> {
    let Some(status) = u.fields.status else {
        return Vec::new();
    };
    let Some(success) = terminal_success(status) else {
        return Vec::new();
    };
    let call_id = ToolCallId::from_string(u.tool_call_id.0.to_string());
    let title = u.fields.title.clone().unwrap_or_else(|| "tool".to_string());
    let tool_name = ToolName::new(title.clone());
    let content_json = u
        .fields
        .content
        .as_ref()
        .and_then(|c| serde_json::to_string(c).ok());
    let text = tool_text(u.fields.raw_output.as_ref(), content_json);
    vec![
        RuntimeEvent::ObservationAdded {
            session_id: sid.clone(),
            turn_id: tid.clone(),
            observation: build_observation(call_id.clone(), tool_name, &title, text, success),
        },
        RuntimeEvent::ToolCallFinished {
            session_id: sid.clone(),
            turn_id: tid.clone(),
            call_id,
            success,
        },
    ]
}

fn build_observation(
    call_id: ToolCallId,
    tool_name: ToolName,
    summary: &str,
    text: String,
    success: bool,
) -> ToolObservation {
    if success {
        ToolObservation::success(call_id, tool_name, summary, ObservationData::Text(text))
    } else {
        let message = if text.is_empty() {
            summary.to_string()
        } else {
            text
        };
        ToolObservation::failure(call_id, tool_name, message)
    }
}

/// ACP 工具状态 → 是否终态(`Some(success)`)/进行中(`None`)。
fn terminal_success(status: ToolCallStatus) -> Option<bool> {
    match status {
        ToolCallStatus::Completed => Some(true),
        ToolCallStatus::Failed => Some(false),
        _ => None,
    }
}

/// 优先用 `raw_output`,否则退回 content JSON。
fn tool_text(raw_output: Option<&serde_json::Value>, content_json: Option<String>) -> String {
    if let Some(out) = raw_output {
        return serde_json::to_string_pretty(out).unwrap_or_default();
    }
    content_json.unwrap_or_default()
}

fn content_block_text(block: &ContentBlock) -> String {
    match block {
        ContentBlock::Text(t) => t.text.clone(),
        _ => serde_json::to_string(block).unwrap_or_else(|_| "[非文本 ACP 内容]".to_string()),
    }
}

fn acp_plan_to_runtime(plan: &AcpPlan) -> Plan {
    let goal = plan
        .entries
        .first()
        .map(|e| e.content.clone())
        .unwrap_or_else(|| "执行计划".to_string());
    let steps: Vec<PlanStep> = plan
        .entries
        .iter()
        .map(|entry| {
            let mut step = PlanStep::new(entry.content.clone(), "");
            step.status = map_step_status(&entry.status);
            step
        })
        .collect();
    let mut runtime_plan = Plan::new(goal, PlanSource::Llm).with_steps(steps);
    let status = if !plan.entries.is_empty()
        && plan
            .entries
            .iter()
            .all(|e| matches!(e.status, PlanEntryStatus::Completed))
    {
        PlanStatus::Completed
    } else {
        PlanStatus::Running
    };
    runtime_plan.set_status(status);
    runtime_plan
}

fn map_step_status(status: &PlanEntryStatus) -> StepStatus {
    match status {
        PlanEntryStatus::Pending => StepStatus::Pending,
        PlanEntryStatus::InProgress => StepStatus::Running,
        PlanEntryStatus::Completed => StepStatus::Completed,
        _ => StepStatus::Pending,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::{
        AvailableCommand, AvailableCommandsUpdate, ConfigOptionUpdate, ContentChunk,
        CurrentModeUpdate, PlanEntry, PlanEntryPriority, SessionInfoUpdate, TextContent,
        ToolCall as AcpToolCall, ToolCallUpdate, ToolCallUpdateFields, UsageUpdate,
    };

    fn ids() -> (SessionId, TurnId) {
        (
            SessionId::from_string("acp_s"),
            TurnId::from_string("acp_t"),
        )
    }

    #[test]
    fn agent_message_chunk_becomes_delta() {
        let (sid, tid) = ids();
        let update = SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
            TextContent::new("你好"),
        )));
        let events = session_update_to_events(&update, &sid, &tid);
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            RuntimeEvent::AssistantMessageDelta { delta, .. } if delta == "你好"
        ));
    }

    #[test]
    fn user_message_chunk_becomes_user_message() {
        let (sid, tid) = ids();
        let update = SessionUpdate::UserMessageChunk(ContentChunk::new(ContentBlock::Text(
            TextContent::new("用户补充"),
        )));
        let events = session_update_to_events(&update, &sid, &tid);

        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            RuntimeEvent::UserMessage { text, .. } if text == "用户补充"
        ));
    }

    #[test]
    fn large_agent_message_chunk_is_split_for_streaming_updates() {
        let (sid, tid) = ids();
        let update = SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
            TextContent::new("abcdefghijklmnopq"),
        )));
        let events = session_update_to_events(&update, &sid, &tid);

        assert_eq!(events.len(), 3);
        assert!(matches!(
            &events[0],
            RuntimeEvent::AssistantMessageDelta { delta, .. } if delta == "abcdefgh"
        ));
        assert!(matches!(
            &events[1],
            RuntimeEvent::AssistantMessageDelta { delta, .. } if delta == "ijklmnop"
        ));
        assert!(matches!(
            &events[2],
            RuntimeEvent::AssistantMessageDelta { delta, .. } if delta == "q"
        ));
    }

    #[test]
    fn agent_thought_chunk_becomes_reasoning_delta() {
        let (sid, tid) = ids();
        let update = SessionUpdate::AgentThoughtChunk(ContentChunk::new(ContentBlock::Text(
            TextContent::new("internal reasoning"),
        )));
        let events = session_update_to_events(&update, &sid, &tid);

        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            RuntimeEvent::ReasoningDelta { delta, .. } if delta == "internal reasoning"
        ));
    }

    #[test]
    fn acp_metadata_updates_do_not_emit_chat_events() {
        let (sid, tid) = ids();
        let updates = vec![
            SessionUpdate::AvailableCommandsUpdate(AvailableCommandsUpdate::new(vec![
                AvailableCommand::new("plan", "Create plan"),
                AvailableCommand::new("review", "Review changes"),
            ])),
            SessionUpdate::CurrentModeUpdate(CurrentModeUpdate::new("plan")),
            SessionUpdate::ConfigOptionUpdate(ConfigOptionUpdate::new(Vec::new())),
            SessionUpdate::SessionInfoUpdate(SessionInfoUpdate::new().title("会话标题")),
            SessionUpdate::UsageUpdate(UsageUpdate::new(1200, 8000)),
        ];

        for update in updates {
            let events = session_update_to_events(&update, &sid, &tid);
            assert!(events.is_empty(), "metadata update should stay out of chat");
        }
    }

    #[test]
    fn tool_call_emits_started() {
        let (sid, tid) = ids();
        let update = SessionUpdate::ToolCall(
            AcpToolCall::new("call_1", "执行 SQL")
                .raw_input(serde_json::json!({"sql": "select 1"})),
        );
        let events = session_update_to_events(&update, &sid, &tid);
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            RuntimeEvent::ToolCallStarted { tool_name, arguments, .. }
                if tool_name.as_str() == "SQL" && arguments["sql"] == "select 1"
        ));
    }

    #[test]
    fn task_titled_tool_call_stays_tool_call() {
        let (sid, tid) = ids();
        let update = SessionUpdate::ToolCall(
            AcpToolCall::new("sub_1", "Task: review runtime").raw_input(serde_json::json!({
                "subagent_type": "reviewer",
                "description": "检查 agent runtime 的事件流"
            })),
        );

        let events = session_update_to_events(&update, &sid, &tid);

        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            RuntimeEvent::ToolCallStarted { tool_name, arguments, .. }
                if tool_name.as_str() == "Task_review_runtime"
                    && arguments["description"] == "检查 agent runtime 的事件流"
        ));
    }

    #[test]
    fn task_titled_tool_update_stays_tool_finished() {
        let (sid, tid) = ids();
        let mut fields = ToolCallUpdateFields::default();
        fields.status = Some(ToolCallStatus::Completed);
        fields.title = Some("Subagent: review runtime".to_string());
        fields.raw_output = Some(serde_json::json!("review complete"));
        let update = SessionUpdate::ToolCallUpdate(ToolCallUpdate::new("sub_1", fields));

        let events = session_update_to_events(&update, &sid, &tid);

        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], RuntimeEvent::ObservationAdded { .. }));
        assert!(matches!(
            events[1],
            RuntimeEvent::ToolCallFinished { success: true, .. }
        ));
    }

    #[test]
    fn translator_does_not_track_task_tool_calls_as_subagents() {
        let (sid, tid) = ids();
        let mut translator = AcpEventTranslator::default();
        let start = SessionUpdate::ToolCall(AcpToolCall::new("sub_1", "Task: review runtime"));

        let started = translator.session_update_to_events(&start, &sid, &tid);

        assert_eq!(started.len(), 1);
        assert!(matches!(
            &started[0],
            RuntimeEvent::ToolCallStarted { tool_name, .. }
                if tool_name.as_str() == "Task_review_runtime"
        ));

        let mut fields = ToolCallUpdateFields::default();
        fields.status = Some(ToolCallStatus::Completed);
        fields.raw_output = Some(serde_json::json!("review complete"));
        let update = SessionUpdate::ToolCallUpdate(ToolCallUpdate::new("sub_1", fields));

        let finished = translator.session_update_to_events(&update, &sid, &tid);

        assert_eq!(finished.len(), 2);
        assert!(matches!(finished[0], RuntimeEvent::ObservationAdded { .. }));
        assert!(matches!(
            finished[1],
            RuntimeEvent::ToolCallFinished { success: true, .. }
        ));
    }

    #[test]
    fn completed_tool_update_emits_observation_and_finished() {
        let (sid, tid) = ids();
        let mut fields = ToolCallUpdateFields::default();
        fields.status = Some(ToolCallStatus::Completed);
        fields.title = Some("执行 SQL".to_string());
        let update = SessionUpdate::ToolCallUpdate(ToolCallUpdate::new("call_1", fields));
        let events = session_update_to_events(&update, &sid, &tid);
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], RuntimeEvent::ObservationAdded { .. }));
        assert!(matches!(
            events[1],
            RuntimeEvent::ToolCallFinished { success: true, .. }
        ));
    }

    #[test]
    fn in_progress_tool_update_is_ignored() {
        let (sid, tid) = ids();
        let mut fields = ToolCallUpdateFields::default();
        fields.status = Some(ToolCallStatus::InProgress);
        let update = SessionUpdate::ToolCallUpdate(ToolCallUpdate::new("call_1", fields));
        assert!(session_update_to_events(&update, &sid, &tid).is_empty());
    }

    #[test]
    fn plan_maps_entries_to_steps() {
        let (sid, tid) = ids();
        let plan = AcpPlan::new(vec![
            PlanEntry::new(
                "查连接数",
                PlanEntryPriority::Medium,
                PlanEntryStatus::InProgress,
            ),
            PlanEntry::new(
                "看慢查询",
                PlanEntryPriority::Medium,
                PlanEntryStatus::Pending,
            ),
        ]);
        let events = session_update_to_events(&SessionUpdate::Plan(plan), &sid, &tid);
        assert_eq!(events.len(), 1);
        let RuntimeEvent::PlanUpdated { plan, .. } = &events[0] else {
            panic!("expected PlanUpdated");
        };
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.goal, "查连接数");
    }
}
