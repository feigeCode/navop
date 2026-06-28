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
    Plan, PlanSource, PlanStatus, PlanStep, RuntimeEvent, SessionId, StepStatus, SubAgentId,
    ToolCallId, ToolObservation, TurnId,
};

const MAX_DELTA_CHARS: usize = 8;

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
        SessionUpdate::AvailableCommandsUpdate(update) => {
            let names = update
                .available_commands
                .iter()
                .map(|command| command.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            vec![status_done(
                format!("ACP 可用命令已更新: {names}"),
                session_id,
                turn_id,
            )]
        }
        SessionUpdate::CurrentModeUpdate(update) => vec![status_done(
            format!("ACP 当前模式: {}", update.current_mode_id.0.as_ref()),
            session_id,
            turn_id,
        )],
        SessionUpdate::ConfigOptionUpdate(update) => vec![status_done(
            format!("ACP 配置选项已更新({} 项)", update.config_options.len()),
            session_id,
            turn_id,
        )],
        SessionUpdate::SessionInfoUpdate(update) => {
            vec![status_done(session_info_title(update), session_id, turn_id)]
        }
        SessionUpdate::UsageUpdate(update) => vec![status_done(
            usage_title(update.used, update.size, update.cost.as_ref()),
            session_id,
            turn_id,
        )],
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
    if let Some(subagent) = subagent_from_tool_call(call) {
        return subagent_call_events(call, subagent, sid, tid);
    }
    let call_id = ToolCallId::from_string(call.tool_call_id.0.to_string());
    let tool_name = ToolName::new(call.title.clone());
    let mut events = vec![RuntimeEvent::ToolCallStarted {
        session_id: sid.clone(),
        turn_id: tid.clone(),
        call_id: call_id.clone(),
        tool_name: tool_name.clone(),
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
    if let Some(subagent) = subagent_from_tool_update(u) {
        return subagent_update_events(u, subagent, sid, tid);
    }
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

struct SubAgentEventData {
    id: SubAgentId,
    name: String,
    task: String,
}

fn subagent_call_events(
    call: &AcpToolCall,
    data: SubAgentEventData,
    sid: &SessionId,
    tid: &TurnId,
) -> Vec<RuntimeEvent> {
    let mut events = vec![RuntimeEvent::SubAgentStarted {
        session_id: sid.clone(),
        turn_id: tid.clone(),
        subagent_id: data.id.clone(),
        name: data.name,
        task: data.task,
    }];
    if let Some(success) = terminal_success(call.status) {
        events.push(RuntimeEvent::SubAgentFinished {
            session_id: sid.clone(),
            turn_id: tid.clone(),
            subagent_id: data.id,
            success,
            summary: tool_text(
                call.raw_output.as_ref(),
                serde_json::to_string(&call.content).ok(),
            ),
        });
    }
    events
}

fn subagent_update_events(
    update: &ToolCallUpdate,
    data: SubAgentEventData,
    sid: &SessionId,
    tid: &TurnId,
) -> Vec<RuntimeEvent> {
    let summary = tool_text(
        update.fields.raw_output.as_ref(),
        update
            .fields
            .content
            .as_ref()
            .and_then(|content| serde_json::to_string(content).ok()),
    );
    match update.fields.status.and_then(terminal_success) {
        Some(success) => vec![RuntimeEvent::SubAgentFinished {
            session_id: sid.clone(),
            turn_id: tid.clone(),
            subagent_id: data.id,
            success,
            summary,
        }],
        None if !summary.is_empty() => vec![RuntimeEvent::SubAgentUpdated {
            session_id: sid.clone(),
            turn_id: tid.clone(),
            subagent_id: data.id,
            summary,
        }],
        None => Vec::new(),
    }
}

fn subagent_from_tool_call(call: &AcpToolCall) -> Option<SubAgentEventData> {
    let (name, title_task) = parse_subagent_title(&call.title)?;
    let raw_input = call.raw_input.as_ref();
    Some(SubAgentEventData {
        id: SubAgentId::from_string(call.tool_call_id.0.to_string()),
        name: name_from_raw_input(raw_input).unwrap_or(name),
        task: task_from_raw_input(raw_input).unwrap_or(title_task),
    })
}

fn subagent_from_tool_update(update: &ToolCallUpdate) -> Option<SubAgentEventData> {
    let title = update.fields.title.as_ref()?;
    let (name, title_task) = parse_subagent_title(title)?;
    let raw_input = update.fields.raw_input.as_ref();
    Some(SubAgentEventData {
        id: SubAgentId::from_string(update.tool_call_id.0.to_string()),
        name: name_from_raw_input(raw_input).unwrap_or(name),
        task: task_from_raw_input(raw_input).unwrap_or(title_task),
    })
}

fn parse_subagent_title(title: &str) -> Option<(String, String)> {
    let trimmed = title.trim();
    for prefix in ["Task:", "Subagent:", "Sub-agent:", "子代理:"] {
        if let Some(task) = trimmed.strip_prefix(prefix) {
            return Some((
                prefix.trim_end_matches(':').to_string(),
                task.trim().to_string(),
            ));
        }
    }
    if trimmed.eq_ignore_ascii_case("task") || trimmed.eq_ignore_ascii_case("subagent") {
        return Some((trimmed.to_string(), String::new()));
    }
    None
}

fn task_from_raw_input(input: Option<&serde_json::Value>) -> Option<String> {
    string_field_from_raw_input(input, &["description", "task", "prompt"])
}

fn name_from_raw_input(input: Option<&serde_json::Value>) -> Option<String> {
    string_field_from_raw_input(input, &["subagent_type", "subagent", "agent", "name"])
}

fn string_field_from_raw_input(input: Option<&serde_json::Value>, keys: &[&str]) -> Option<String> {
    let object = input?.as_object()?;
    keys.iter()
        .filter_map(|key| object.get(*key)?.as_str())
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(ToString::to_string)
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

fn session_info_title(update: &agent_client_protocol::schema::SessionInfoUpdate) -> String {
    let title = update
        .title
        .as_opt_ref()
        .flatten()
        .filter(|title| !title.is_empty());
    let updated_at = update
        .updated_at
        .as_opt_ref()
        .flatten()
        .filter(|updated_at| !updated_at.is_empty());
    match (title, updated_at) {
        (Some(title), Some(updated_at)) => format!("ACP 会话信息已更新: {title} · {updated_at}"),
        (Some(title), None) => format!("ACP 会话信息已更新: {title}"),
        (None, Some(updated_at)) => format!("ACP 会话信息已更新: {updated_at}"),
        (None, None) => "ACP 会话信息已更新".to_string(),
    }
}

fn usage_title(used: u64, size: u64, cost: Option<&agent_client_protocol::schema::Cost>) -> String {
    let usage = format!("ACP 用量: {used}/{size} tokens");
    if let Some(cost) = cost {
        format!("{usage} · {:.4} {}", cost.amount, cost.currency)
    } else {
        usage
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
    fn acp_metadata_updates_become_status_events() {
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
            assert_eq!(events.len(), 1, "update should produce one status event");
            assert!(matches!(
                &events[0],
                RuntimeEvent::Status { title, is_done: true, .. } if title.starts_with("ACP ")
            ));
        }
    }

    #[test]
    fn tool_call_emits_started() {
        let (sid, tid) = ids();
        let update = SessionUpdate::ToolCall(AcpToolCall::new("call_1", "执行 SQL"));
        let events = session_update_to_events(&update, &sid, &tid);
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            RuntimeEvent::ToolCallStarted { tool_name, .. } if tool_name.as_str() == "SQL"
        ));
    }

    #[test]
    fn subagent_tool_call_emits_subagent_started() {
        let (sid, tid) = ids();
        let update = SessionUpdate::ToolCall(AcpToolCall::new("sub_1", "Task: review runtime"));

        let events = session_update_to_events(&update, &sid, &tid);

        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            RuntimeEvent::SubAgentStarted { name, task, .. }
                if name == "Task" && task == "review runtime"
        ));
    }

    #[test]
    fn subagent_tool_call_uses_raw_input_name_and_task() {
        let (sid, tid) = ids();
        let update = SessionUpdate::ToolCall(AcpToolCall::new("sub_1", "Task").raw_input(
            serde_json::json!({
                "subagent_type": "reviewer",
                "description": "检查 agent runtime 的事件流"
            }),
        ));

        let events = session_update_to_events(&update, &sid, &tid);

        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            RuntimeEvent::SubAgentStarted { name, task, .. }
                if name == "reviewer" && task == "检查 agent runtime 的事件流"
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
