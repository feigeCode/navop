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

pub fn session_update_to_events(
    update: &SessionUpdate,
    session_id: &SessionId,
    turn_id: &TurnId,
) -> Vec<RuntimeEvent> {
    match update {
        SessionUpdate::AgentMessageChunk(chunk) => {
            assistant_delta_events(content_block_text(&chunk.content), session_id, turn_id)
        }
        SessionUpdate::AgentThoughtChunk(chunk) => {
            if content_block_text(&chunk.content).is_empty() {
                Vec::new()
            } else {
                vec![RuntimeEvent::Status {
                    session_id: session_id.clone(),
                    turn_id: turn_id.clone(),
                    title: "思考中...".to_string(),
                    is_done: false,
                }]
            }
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
        _ => Vec::new(),
    }
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

fn tool_call_events(call: &AcpToolCall, sid: &SessionId, tid: &TurnId) -> Vec<RuntimeEvent> {
    let call_id = ToolCallId::from_string(call.tool_call_id.0.to_string());
    let tool_name = ToolName::new(call.title.clone());
    let mut events = vec![RuntimeEvent::ToolCallStarted {
        session_id: sid.clone(),
        turn_id: tid.clone(),
        call_id: call_id.clone(),
        tool_name: tool_name.clone(),
    }];
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
        .and_then(|content| serde_json::to_string(content).ok());
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

fn terminal_success(status: ToolCallStatus) -> Option<bool> {
    match status {
        ToolCallStatus::Completed => Some(true),
        ToolCallStatus::Failed => Some(false),
        _ => None,
    }
}

fn tool_text(raw_output: Option<&serde_json::Value>, content_json: Option<String>) -> String {
    if let Some(output) = raw_output {
        return serde_json::to_string_pretty(output).unwrap_or_default();
    }
    content_json.unwrap_or_default()
}

fn content_block_text(block: &ContentBlock) -> String {
    match block {
        ContentBlock::Text(text) => text.text.clone(),
        _ => String::new(),
    }
}

fn acp_plan_to_runtime(plan: &AcpPlan) -> Plan {
    let goal = plan
        .entries
        .first()
        .map(|entry| entry.content.clone())
        .unwrap_or_else(|| "执行计划".to_string());
    let steps = plan
        .entries
        .iter()
        .map(|entry| {
            let mut step = PlanStep::new(entry.content.clone(), "");
            step.status = map_step_status(&entry.status);
            step
        })
        .collect();
    let mut runtime_plan = Plan::new(goal, PlanSource::Llm).with_steps(steps);
    let complete = !plan.entries.is_empty()
        && plan
            .entries
            .iter()
            .all(|entry| matches!(entry.status, PlanEntryStatus::Completed));
    runtime_plan.set_status(if complete {
        PlanStatus::Completed
    } else {
        PlanStatus::Running
    });
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
        ContentChunk, PlanEntry, PlanEntryPriority, TextContent, ToolCallUpdateFields,
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
            TextContent::new("abcdefghijklmnopq"),
        )));
        let events = session_update_to_events(&update, &sid, &tid);

        assert_eq!(3, events.len());
        assert!(matches!(
            &events[0],
            RuntimeEvent::AssistantMessageDelta { delta, .. } if delta == "abcdefgh"
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

        assert_eq!(2, events.len());
        assert!(matches!(events[0], RuntimeEvent::ObservationAdded { .. }));
        assert!(matches!(
            events[1],
            RuntimeEvent::ToolCallFinished { success: true, .. }
        ));
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

        let RuntimeEvent::PlanUpdated { plan, .. } = &events[0] else {
            panic!("expected PlanUpdated");
        };
        assert_eq!(2, plan.steps.len());
        assert_eq!("查连接数", plan.goal);
    }
}
