//! ACP `SessionUpdate` → `agent_runtime::RuntimeEvent` 翻译。
//!
//! 复用点:翻译成现有 `RuntimeEvent` 后,直接喂 `AgentTranscript` 的归约逻辑,
//! ACP 后端无需任何新增渲染代码。纯函数,便于单测。

use agent_client_protocol::schema::v1::{
    ContentBlock, Plan as AcpPlan, PlanEntryStatus, SessionUpdate, ToolCall as AcpToolCall,
    ToolCallContent, ToolCallLocation, ToolCallStatus, ToolCallUpdate, ToolKind,
};
use agent_runtime::tools::{FileChange, ObservationData, ToolAction, ToolName};
use agent_runtime::{
    Plan, PlanSource, PlanStatus, PlanStep, RuntimeEvent, SessionId, StepStatus, ToolCallId,
    ToolObservation, TurnId,
};
use rust_i18n::t;
use serde_json::Value;

const MAX_DELTA_CHARS: usize = 8;

#[derive(Default)]
pub(crate) struct AcpEventTranslator;

impl AcpEventTranslator {
    pub(crate) fn session_update_to_events(
        &mut self,
        update: &SessionUpdate,
        session_id: &SessionId,
        turn_id: &TurnId,
        agent_name: &str,
        replay: bool,
    ) -> Vec<RuntimeEvent> {
        session_update_to_events_for_agent(update, session_id, turn_id, agent_name, replay)
    }
}

/// 把一条 ACP `SessionUpdate` 翻译为 0..N 条 `RuntimeEvent`。
///
/// `session_id` / `turn_id` 为本次 ACP 会话的合成 id(view 侧事件泵据此过滤)。
#[cfg(test)]
pub(crate) fn session_update_to_events(
    update: &SessionUpdate,
    session_id: &SessionId,
    turn_id: &TurnId,
) -> Vec<RuntimeEvent> {
    session_update_to_events_for_agent(update, session_id, turn_id, "Agent", false)
}

/// `replay` 为真表示这条 update 是 `session/load` 重放出来的历史，而不是刚跑出来的输出。
///
/// 两者的差别只在**助手文本**上：直播是流式增量（一条 update 只是一小段），回放是
/// agent 按 message 整段重放。按增量翻译回放，等于把「若干条独立消息」当成一段连续
/// 输出，历史里的段落会被粘成一大块；而且回放没有终态事件来收尾，最后那个气泡会一直
/// 停在「流式中」。详见 [`assistant_events`]。
pub(crate) fn session_update_to_events_for_agent(
    update: &SessionUpdate,
    session_id: &SessionId,
    turn_id: &TurnId,
    agent_name: &str,
    replay: bool,
) -> Vec<RuntimeEvent> {
    match update {
        SessionUpdate::UserMessageChunk(chunk) => {
            let text = content_block_text(&chunk.content);
            user_message_events(text, session_id, turn_id)
        }
        SessionUpdate::AgentMessageChunk(chunk) => {
            let delta = content_block_text(&chunk.content);
            if chunk.message_id.is_none()
                && let Some(model) = model_metadata_fallback_warning_model(&delta)
            {
                tracing::warn!(
                    agent = agent_name,
                    model,
                    warning = %delta.trim(),
                    "ACP agent model metadata fallback"
                );
                return Vec::new();
            }
            assistant_events(delta, session_id, turn_id, replay)
        }
        SessionUpdate::AgentThoughtChunk(chunk) => {
            let delta = content_block_text(&chunk.content);
            reasoning_delta_events(delta, session_id, turn_id)
        }
        SessionUpdate::ToolCall(call) => tool_call_events(call, session_id, turn_id, agent_name),
        SessionUpdate::ToolCallUpdate(update) => {
            tool_call_update_events(update, session_id, turn_id, agent_name)
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

/// 助手文本 → 事件。直播切增量，回放整段落定。
///
/// 回放为什么不能按增量走：`session/load` 的一条 chunk 就是**一整条历史消息**，而
/// 「这是新的一条」这个信息只存在于真实流式的开始/结束里——回放没有那些事件。照增量
/// 翻译的话，历史里相邻的几条助手消息会续进同一个气泡（中间隔着工具调用才会被切开），
/// 而且最后一条永远等不到收尾，界面上会一直转「流式中」。
fn assistant_events(
    text: String,
    session_id: &SessionId,
    turn_id: &TurnId,
    replay: bool,
) -> Vec<RuntimeEvent> {
    if replay {
        return assistant_message_events(text, session_id, turn_id);
    }
    assistant_delta_events(text, session_id, turn_id)
}

fn assistant_message_events(
    text: String,
    session_id: &SessionId,
    turn_id: &TurnId,
) -> Vec<RuntimeEvent> {
    if text.is_empty() {
        return Vec::new();
    }
    vec![RuntimeEvent::AssistantMessage {
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

fn model_metadata_fallback_warning_model(text: &str) -> Option<&str> {
    const PREFIX: &str = "Warning: Model metadata for ";
    const SUFFIX: &str = " not found. Defaulting to fallback metadata; this can degrade performance and cause issues.";

    let body = text.trim().strip_prefix(PREFIX)?;
    let model = body.strip_suffix(SUFFIX)?.trim();
    (!model.is_empty()).then_some(model)
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

/// 入参里哪些键是「这次调用指向什么」。
///
/// 与 [`crate::agent_tool_input`] 的键组同源:那里把入参渲染成一行摘要,这里挑出
/// 该当行标题的那一个值。两边都只认**协议声明过的字段名**,不按名字猜语义。
const TARGET_INPUT_KEYS: &[&str] = &[
    "filePath",
    "file_path",
    "path",
    "command",
    "cmd",
    "query",
    "pattern",
    "sql",
    "url",
];

/// 这次调用指向什么,以及「能解释这个目标的入参」。
///
/// ACP 把「调用在做什么」拆在两处声明:调用开始的 `tool_call` 用 `locations` 说
/// 「在动哪些文件」,真实入参(`rawInput`)则要等 `tool_call_update`。两处都可能
/// 单独出现,所以统一在这里算成一个值。
///
/// `arguments` **只在它真的解释了 `target` 时才带上** —— 观测携带的入参会被卡片
/// 拿来重算行标题。调用开始时那份占位(实测 OpenCode 只给 `{"cwd": …}`)不是声明,
/// 没有资格覆盖已经显示出来的东西。
#[derive(Debug, Default, PartialEq)]
struct ToolCallTarget {
    target: Option<String>,
    arguments: Option<Value>,
}

/// 从「声明」里算出这次调用指向什么。
fn tool_call_target(raw_input: Option<&Value>, locations: &[ToolCallLocation]) -> ToolCallTarget {
    if let Some(target) = target_from_input(raw_input) {
        return ToolCallTarget {
            target: Some(target),
            arguments: raw_input.cloned(),
        };
    }
    ToolCallTarget {
        // 入参说不出目标时,退回 agent 声明的「在哪个文件上」。**排在真实入参之后**:
        // 对命令类工具,`locations` 给的是工作目录(「在哪儿跑」),不是命令本身。
        target: locations
            .first()
            .map(|location| location.path.display().to_string())
            .filter(|path| !path.trim().is_empty()),
        arguments: None,
    }
}

/// 按 [`TARGET_INPUT_KEYS`] 取一个非空字符串值;取不到就是「没声明」。
fn target_from_input(raw_input: Option<&Value>) -> Option<String> {
    let object = raw_input?.as_object()?;
    TARGET_INPUT_KEYS.iter().find_map(|key| {
        object
            .get(*key)
            .or_else(|| {
                object
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case(key))
                    .map(|(_, value)| value)
            })
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn tool_call_events(
    call: &AcpToolCall,
    sid: &SessionId,
    tid: &TurnId,
    agent_name: &str,
) -> Vec<RuntimeEvent> {
    let call_id = ToolCallId::from_string(call.tool_call_id.0.to_string());
    let tool_name = ToolName::new(call.title.clone());
    let declared = tool_call_target(call.raw_input.as_ref(), &call.locations);
    let mut events = vec![RuntimeEvent::ToolCallStarted {
        session_id: sid.clone(),
        turn_id: tid.clone(),
        call_id: call_id.clone(),
        tool_name: tool_name.clone(),
        // 类别由 agent 通过协议字段声明,本地不猜。
        kind: tool_action(call.kind),
        // 原始入参照原样转交:卡片自己要判断「这份入参够不够当这一行的话」——
        // 实测调用开始时给的是占位(`{"cwd": …}`),展示层认不出已知键就不拿它当标题。
        arguments: call.raw_input.clone().unwrap_or(serde_json::Value::Null),
    }];
    // ToolCall 携带终态(部分 agent 一步到位),补观测 + 完成事件。
    if let Some(success) = terminal_success(call.status) {
        let (text, file_changes) = tool_payload(&call.content, call.raw_output.as_ref());
        events.push(RuntimeEvent::ObservationAdded {
            session_id: sid.clone(),
            turn_id: tid.clone(),
            observation: build_observation(call_id.clone(), tool_name, &call.title, text, success)
                .with_file_changes(file_changes)
                .with_target(declared.target.clone())
                .with_arguments(declared.arguments.clone()),
        });
        events.push(RuntimeEvent::ToolCallFinished {
            session_id: sid.clone(),
            turn_id: tid.clone(),
            call_id,
            success,
        });
        events.push(continuing_status_event(sid, tid, agent_name));
    }
    events
}

fn tool_call_update_events(
    u: &ToolCallUpdate,
    sid: &SessionId,
    tid: &TurnId,
    agent_name: &str,
) -> Vec<RuntimeEvent> {
    let Some(status) = u.fields.status else {
        return Vec::new();
    };
    let Some(success) = terminal_success(status) else {
        return Vec::new();
    };
    let call_id = ToolCallId::from_string(u.tool_call_id.0.to_string());
    let title = u.fields.title.clone().unwrap_or_else(|| "tool".to_string());
    let tool_name = ToolName::new(title.clone());
    let content: &[ToolCallContent] = u.fields.content.as_deref().unwrap_or_default();
    let (text, file_changes) = tool_payload(content, u.fields.raw_output.as_ref());
    let declared = tool_call_target(
        u.fields.raw_input.as_ref(),
        u.fields.locations.as_deref().unwrap_or_default(),
    );
    vec![
        RuntimeEvent::ObservationAdded {
            session_id: sid.clone(),
            turn_id: tid.clone(),
            observation: build_observation(call_id.clone(), tool_name, &title, text, success)
                .with_file_changes(file_changes)
                .with_target(declared.target)
                .with_arguments(declared.arguments),
        },
        RuntimeEvent::ToolCallFinished {
            session_id: sid.clone(),
            turn_id: tid.clone(),
            call_id,
            success,
        },
        continuing_status_event(sid, tid, agent_name),
    ]
}

fn continuing_status_event(
    session_id: &SessionId,
    turn_id: &TurnId,
    agent_name: &str,
) -> RuntimeEvent {
    RuntimeEvent::Status {
        session_id: session_id.clone(),
        turn_id: turn_id.clone(),
        title: t!("AgentUi.acp_continuing_response", name = agent_name).to_string(),
        is_done: false,
    }
}

fn build_observation(
    call_id: ToolCallId,
    tool_name: ToolName,
    summary: &str,
    text: String,
    success: bool,
) -> ToolObservation {
    let permission_denied = is_public_mcp_permission_denied(&text);
    if success && !permission_denied {
        ToolObservation::success(call_id, tool_name, summary, ObservationData::Text(text))
    } else {
        let message = if text.is_empty() {
            summary.to_string()
        } else if permission_denied {
            t!("AgentChat.public_mcp_permission_denied").to_string()
        } else {
            text
        };
        ToolObservation::failure(call_id, tool_name, message)
    }
}

fn is_public_mcp_permission_denied(text: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return false;
    };
    contains_structured_code(&value, "permission_denied")
}

fn contains_structured_code(value: &Value, expected: &str) -> bool {
    match value {
        Value::Object(object) => {
            object
                .get("code")
                .and_then(Value::as_str)
                .is_some_and(|code| code == expected)
                || object
                    .values()
                    .any(|value| contains_structured_code(value, expected))
        }
        Value::Array(values) => values
            .iter()
            .any(|value| contains_structured_code(value, expected)),
        Value::String(string) => serde_json::from_str::<Value>(string)
            .ok()
            .is_some_and(|value| contains_structured_code(&value, expected)),
        _ => false,
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

/// 协议声明的工具类别 → 本地动作类别。
///
/// 这是**协议字段的直接映射**,不是从工具名反推:ACP agent 自己声明了它在读、
/// 在改、还是在跑命令,UI 直接用。未知类别落到 `Other`,由 UI 显示工具名。
fn tool_action(kind: ToolKind) -> ToolAction {
    match kind {
        ToolKind::Read => ToolAction::Read,
        ToolKind::Edit => ToolAction::Edit,
        ToolKind::Delete => ToolAction::Delete,
        ToolKind::Move => ToolAction::Move,
        ToolKind::Search => ToolAction::Search,
        ToolKind::Execute => ToolAction::Execute,
        ToolKind::Think => ToolAction::Think,
        ToolKind::Fetch => ToolAction::Fetch,
        ToolKind::SwitchMode => ToolAction::SwitchMode,
        // `ToolKind::Other` 与协议将来新增的类别都落到这里:宁可显示工具名,
        // 也不要给一个猜来的动词。
        _ => ToolAction::Other,
    }
}

/// 把协议内容拆成「文件改动」与「其余文本载荷」。
///
/// 关键取舍:[`ToolCallContent::Diff`] **不进文本载荷**。它自带改动前后的完整
/// 文件内容,再序列化一遍 JSON 就是同一份内容存两处——旧实现正是这么做的,
/// 于是卡片里出现了「两个完整文件当 JSON 展示」的样子。拆出来之后,文本载荷
/// 只留真正的输出,文件改动走结构化通道给 UI 渲染成 diff。
fn tool_payload(
    content: &[ToolCallContent],
    raw_output: Option<&serde_json::Value>,
) -> (String, Vec<FileChange>) {
    let mut file_changes = Vec::new();
    let mut rest: Vec<&ToolCallContent> = Vec::new();
    for item in content {
        match item {
            ToolCallContent::Diff(diff) => {
                let path = diff.path.display().to_string();
                file_changes.push(match diff.old_text.clone() {
                    Some(old) => FileChange::modified(path, old, diff.new_text.clone()),
                    None => FileChange::created(path, diff.new_text.clone()),
                });
            }
            other => rest.push(other),
        }
    }
    let content_json = (!rest.is_empty())
        .then(|| serde_json::to_string(&rest).ok())
        .flatten();
    (tool_text(raw_output, content_json), file_changes)
}

fn content_block_text(block: &ContentBlock) -> String {
    match block {
        ContentBlock::Text(t) => t.text.clone(),
        _ => serde_json::to_string(block)
            .unwrap_or_else(|_| t!("AgentUi.acp_non_text_content").to_string()),
    }
}

fn acp_plan_to_runtime(plan: &AcpPlan) -> Plan {
    let goal = plan
        .entries
        .first()
        .map(|e| e.content.clone())
        .unwrap_or_else(|| t!("AgentUi.execution_plan").to_string());
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
    use agent_client_protocol::schema::v1::{
        AvailableCommand, AvailableCommandsUpdate, ConfigOptionUpdate, ContentChunk,
        CurrentModeUpdate, Diff, PlanEntry, PlanEntryPriority, SessionInfoUpdate, TextContent,
        ToolCall as AcpToolCall, ToolCallLocation, ToolCallUpdate, ToolCallUpdateFields,
        UsageUpdate,
    };

    fn ids() -> (SessionId, TurnId) {
        (
            SessionId::from_string("acp_s"),
            TurnId::from_string("acp_t"),
        )
    }

    /// 一批事件里的观测(测试只关心卡片最后拿到的那个)。
    fn observation_of(events: &[RuntimeEvent]) -> &ToolObservation {
        events
            .iter()
            .find_map(|event| match event {
                RuntimeEvent::ObservationAdded { observation, .. } => Some(observation),
                _ => None,
            })
            .expect("observation")
    }

    /// 按**历史回放**翻译一条 update（`session/load` 重放出来的那一批）。
    fn replayed(update: &SessionUpdate, sid: &SessionId, tid: &TurnId) -> Vec<RuntimeEvent> {
        session_update_to_events_for_agent(update, sid, tid, "OpenCode", true)
    }

    /// 把一条 update 按回放翻译后落进转录。
    fn apply_replayed(
        transcript: &mut crate::agent_transcript::AgentTranscript,
        update: &SessionUpdate,
        sid: &SessionId,
        tid: &TurnId,
    ) {
        for event in replayed(update, sid, tid) {
            transcript.apply(&event);
        }
    }

    fn text_message(text: &str) -> SessionUpdate {
        SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new(
            text,
        ))))
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
    fn replayed_history_message_arrives_whole_instead_of_as_deltas() {
        let (sid, tid) = ids();
        let text = "这是一条历史回答，直播时会按 8 个字符一条切成增量。";
        let update = text_message(text);

        let live = session_update_to_events(&update, &sid, &tid);
        let history = replayed(&update, &sid, &tid);

        assert!(live.len() > 1, "直播仍然是增量：{}", live.len());
        assert_eq!(history.len(), 1, "回放的一条 chunk 就是整条消息");
        assert!(matches!(
            &history[0],
            RuntimeEvent::AssistantMessage { text: got, .. } if got == text
        ));
    }

    #[test]
    fn replayed_messages_stay_separate_and_finished() {
        let (sid, tid) = ids();
        let mut transcript = crate::agent_transcript::AgentTranscript::new();

        apply_replayed(&mut transcript, &text_message("第一段回答"), &sid, &tid);
        apply_replayed(&mut transcript, &text_message("第二段回答"), &sid, &tid);

        assert_eq!(
            transcript
                .messages
                .iter()
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>(),
            vec!["第一段回答", "第二段回答"],
            "两条历史消息不能粘进同一个气泡"
        );
        assert!(
            transcript
                .messages
                .iter()
                .all(|message| !message.is_streaming),
            "回放没有终态事件收尾，落下的气泡不能停在「流式中」"
        );
    }

    #[test]
    fn replayed_history_keeps_tool_calls_between_messages() {
        let (sid, tid) = ids();
        let mut transcript = crate::agent_transcript::AgentTranscript::new();
        let call = AcpToolCall::new("call_1", "read").kind(ToolKind::Read);

        apply_replayed(&mut transcript, &text_message("先看一下文件"), &sid, &tid);
        apply_replayed(&mut transcript, &SessionUpdate::ToolCall(call), &sid, &tid);
        apply_replayed(&mut transcript, &text_message("然后改了它"), &sid, &tid);

        assert_eq!(transcript.messages.len(), 3);
        assert_eq!(transcript.messages[0].content, "先看一下文件");
        assert_eq!(
            transcript.messages[1].variant.card_kind(),
            Some(crate::agent_cards::TOOL_CARD)
        );
        assert_eq!(transcript.messages[2].content, "然后改了它");
    }

    #[test]
    fn model_metadata_fallback_warning_is_logged_instead_of_rendered() {
        let (sid, tid) = ids();
        let update = SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
            TextContent::new(
                "Warning: Model metadata for gpt-5.6-sol not found. Defaulting to fallback metadata; this can degrade performance and cause issues.",
            ),
        )));

        let events = session_update_to_events(&update, &sid, &tid);

        assert!(events.is_empty());
    }

    #[test]
    fn ordinary_warning_from_agent_remains_visible() {
        let (sid, tid) = ids();
        let update = SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
            TextContent::new("Warning: production database is read-only."),
        )));

        let events = session_update_to_events(&update, &sid, &tid);

        assert!(!events.is_empty());
    }

    #[test]
    fn model_metadata_words_inside_normal_answer_remain_visible() {
        let (sid, tid) = ids();
        let update = SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
            TextContent::new(
                "The log said: Warning: Model metadata for gpt-x not found. Defaulting to fallback metadata; this can degrade performance and cause issues. Please update the config.",
            ),
        )));

        let events = session_update_to_events(&update, &sid, &tid);

        assert!(!events.is_empty());
    }

    #[test]
    fn metadata_warning_with_normal_message_id_remains_visible() {
        let (sid, tid) = ids();
        let update = SessionUpdate::AgentMessageChunk(
            ContentChunk::new(ContentBlock::Text(TextContent::new(
                "Warning: Model metadata for gpt-x not found. Defaulting to fallback metadata; this can degrade performance and cause issues.",
            )))
            .message_id("assistant-message"),
        );

        let events = session_update_to_events(&update, &sid, &tid);

        assert!(!events.is_empty());
    }

    #[test]
    fn public_mcp_permission_error_is_recognized_from_nested_structured_output() {
        let text = serde_json::json!({
            "result": {
                "structuredContent": {
                    "code": "permission_denied",
                    "message": "tool runtime call denied by permission mode"
                }
            }
        })
        .to_string();

        assert!(is_public_mcp_permission_denied(&text));
    }

    #[test]
    fn unrelated_structured_tool_error_is_not_treated_as_permission_denied() {
        let text = serde_json::json!({
            "structuredContent": {
                "code": "connection_failed"
            }
        })
        .to_string();

        assert!(!is_public_mcp_permission_denied(&text));
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
    fn declared_tool_kind_becomes_the_local_action() {
        let (sid, tid) = ids();
        let mut call = AcpToolCall::new("call_1", "编辑文件");
        call.kind = ToolKind::Edit;
        let events = session_update_to_events(&SessionUpdate::ToolCall(call), &sid, &tid);

        assert!(
            matches!(
                &events[0],
                RuntimeEvent::ToolCallStarted { kind, .. } if *kind == ToolAction::Edit
            ),
            "动词只能来自协议声明"
        );

        // 协议没声明(或声明了将来新增的类别)时,不给动词。
        let mut unknown = AcpToolCall::new("call_2", "某个工具");
        unknown.kind = ToolKind::Other;
        let events = session_update_to_events(&SessionUpdate::ToolCall(unknown), &sid, &tid);
        assert!(matches!(
            &events[0],
            RuntimeEvent::ToolCallStarted { kind, .. } if *kind == ToolAction::Other
        ));
    }

    #[test]
    fn acp_diff_content_becomes_a_file_change_and_leaves_the_text_payload() {
        let (sid, tid) = ids();
        let mut fields = ToolCallUpdateFields::default();
        fields.status = Some(ToolCallStatus::Completed);
        fields.content = Some(vec![ToolCallContent::Diff(Diff::new("src/lib.rs", "a\nb\n"))]);
        let update = SessionUpdate::ToolCallUpdate(ToolCallUpdate::new("call_1", fields));

        let events = session_update_to_events(&update, &sid, &tid);

        let observation = events
            .iter()
            .find_map(|event| match event {
                RuntimeEvent::ObservationAdded { observation, .. } => Some(observation),
                _ => None,
            })
            .expect("observation");

        assert_eq!(1, observation.file_changes.len());
        assert_eq!("src/lib.rs", observation.file_changes[0].path);
        assert_eq!("a\nb\n", observation.file_changes[0].new_text);
        assert!(
            !observation.data.to_text().contains("new_text"),
            "改动内容不该再以 JSON 形式出现一遍: {:?}",
            observation.data.to_text()
        );
    }

    #[test]
    fn read_update_declares_the_file_it_read() {
        // OpenCode 的 read:调用开始时 title 只是工具名("read"),文件要到终态
        // update 的 rawInput.filePath 才出现 —— 卡片要显示的就是它。
        let (sid, tid) = ids();
        let mut fields = ToolCallUpdateFields::default();
        fields.status = Some(ToolCallStatus::Completed);
        fields.title = Some("crates/agent_runtime/src/tools/action.rs".to_string());
        fields.raw_input = Some(serde_json::json!({
            "filePath": "/repo/crates/agent_runtime/src/tools/action.rs"
        }));
        let update = SessionUpdate::ToolCallUpdate(ToolCallUpdate::new("call_1", fields));

        let events = session_update_to_events(&update, &sid, &tid);
        let observation = observation_of(&events);

        assert_eq!(
            Some("/repo/crates/agent_runtime/src/tools/action.rs"),
            observation.target.as_deref()
        );
        assert_eq!(
            Some(&serde_json::json!({
                "filePath": "/repo/crates/agent_runtime/src/tools/action.rs"
            })),
            observation.arguments.as_ref(),
            "入参要一并转交,卡片才能把行标题从占位换成真实入参"
        );
    }

    #[test]
    fn command_update_declares_the_command_it_ran() {
        let (sid, tid) = ids();
        let mut fields = ToolCallUpdateFields::default();
        fields.status = Some(ToolCallStatus::Completed);
        fields.title = Some("git status && git log --oneline -15".to_string());
        fields.raw_input = Some(serde_json::json!({
            "command": "git status && git log --oneline -15",
            "workdir": "/repo"
        }));
        let update = SessionUpdate::ToolCallUpdate(ToolCallUpdate::new("call_1", fields));

        let events = session_update_to_events(&update, &sid, &tid);
        let observation = observation_of(&events);

        assert_eq!(
            Some("git status && git log --oneline -15"),
            observation.target.as_deref(),
            "命令类工具该显示的是命令,不是工作目录"
        );
    }

    #[test]
    fn call_start_placeholder_input_is_not_a_declaration() {
        // 实测 OpenCode 在 tool_start 只发 `{"cwd": …}`:占位,不是这次调用的声明。
        // 它会进展开区的入参,但不能当行标题 —— 否则就是 `bash {"cwd":"/repo"}`。
        let title = row_title_from_acp(
            AcpToolCall::new("call_1", "bash")
                .raw_input(serde_json::json!({"cwd": "/Users/me/repo"})),
            ToolKind::Execute,
            None,
        );

        assert_eq!("bash", title);
    }

    #[test]
    fn declared_input_target_wins_over_the_location() {
        // 命令类工具的 locations 是「在哪儿跑」,入参里才有真正的命令。
        let (sid, tid) = ids();
        let mut call = AcpToolCall::new("call_1", "bash");
        call.locations = vec![ToolCallLocation::new("/Users/me/repo")];
        call.raw_input = Some(serde_json::json!({"command": "pwd", "workdir": "/Users/me/repo"}));
        call.status = ToolCallStatus::Completed;

        let events = session_update_to_events(&SessionUpdate::ToolCall(call), &sid, &tid);
        let observation = observation_of(&events);

        assert_eq!(Some("pwd"), observation.target.as_deref());
    }

    #[test]
    fn location_is_the_fallback_target_when_input_says_nothing() {
        let (sid, tid) = ids();
        let mut call = AcpToolCall::new("call_1", "read");
        call.locations = vec![ToolCallLocation::new("/Users/me/repo/a.rs")];
        call.status = ToolCallStatus::Completed;

        let events = session_update_to_events(&SessionUpdate::ToolCall(call), &sid, &tid);
        let observation = observation_of(&events);

        assert_eq!(
            Some("/Users/me/repo/a.rs"),
            observation.target.as_deref()
        );
        assert!(
            observation.arguments.is_none(),
            "没有入参解释目标时不要伪造一份"
        );
    }

    #[test]
    fn non_string_input_values_are_not_targets() {
        // `grep` 的 path 可能是数组(多个搜索根)。那不是「指向什么」,不能当行标题。
        let (sid, tid) = ids();
        let mut fields = ToolCallUpdateFields::default();
        fields.status = Some(ToolCallStatus::Completed);
        fields.raw_input = Some(serde_json::json!({"path": ["/a", "/b"]}));
        let update = SessionUpdate::ToolCallUpdate(ToolCallUpdate::new("call_1", fields));

        let events = session_update_to_events(&update, &sid, &tid);
        let observation = observation_of(&events);

        assert_eq!(None, observation.target);
        assert_eq!(None, observation.arguments);
    }

    #[test]
    fn a_read_call_ends_up_naming_the_file_it_read() {
        // 端到端:ACP 的两条通知 → 转录的卡片 → 折叠行标题。用户看到的那行
        // `读取 read` 就是这里出来的,修完要变成 `读取 …/tools/action.rs`。
        let title = row_title_from_acp(
            AcpToolCall::new("call_1", "read"),
            ToolKind::Read,
            Some(serde_json::json!({
                "filePath": "/repo/crates/agent_runtime/src/tools/action.rs"
            })),
        );

        assert_eq!(
            format!("{} …/tools/action.rs", t!("AgentUi.action_read")),
            title
        );
    }

    #[test]
    fn a_bash_call_ends_up_naming_the_command_it_ran() {
        // 同一端到端,另一条被截图点名的行:`bash {"cwd":"…"}` → `bash <命令>`。
        let title = row_title_from_acp(
            AcpToolCall::new("call_1", "bash")
                .raw_input(serde_json::json!({"cwd": "/repo"})),
            ToolKind::Execute,
            Some(serde_json::json!({
                "command": "git status && git log --oneline -15",
                "workdir": "/repo"
            })),
        );

        assert_eq!("bash git status && git log --oneline -15", title);
    }

    /// 跑一遍「调用开始 → 终态」两条通知,返回卡片折叠行的标题。
    ///
    /// 调用开始时 `title` 只有工具名、`rawInput` 是占位(实测的 OpenCode 行为);
    /// 真实入参随终态 update 到达,`None` 表示只跑开始那一步。
    fn row_title_from_acp(
        mut started: AcpToolCall,
        kind: ToolKind,
        completed_input: Option<Value>,
    ) -> String {
        let (sid, tid) = ids();
        started.kind = kind;
        let mut translator = AcpEventTranslator;
        let mut transcript = crate::agent_transcript::AgentTranscript::new();
        for event in translator.session_update_to_events(
            &SessionUpdate::ToolCall(started),
            &sid,
            &tid,
            "OpenCode",
            false,
        ) {
            transcript.apply(&event);
        }

        if let Some(raw_input) = completed_input {
            let mut fields = ToolCallUpdateFields::default();
            fields.status = Some(ToolCallStatus::Completed);
            fields.raw_input = Some(raw_input);
            for event in translator.session_update_to_events(
                &SessionUpdate::ToolCallUpdate(ToolCallUpdate::new("call_1", fields)),
                &sid,
                &tid,
                "OpenCode",
                false,
            ) {
                transcript.apply(&event);
            }
        }

        let card = crate::agent_cards::ToolCardData::from_json(&transcript.messages[0].content)
            .expect("tool card");
        crate::agent_cards::tool_row_title(&card)
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

        assert_eq!(events.len(), 3);
        assert!(matches!(events[0], RuntimeEvent::ObservationAdded { .. }));
        assert!(matches!(
            events[1],
            RuntimeEvent::ToolCallFinished { success: true, .. }
        ));
        assert!(matches!(events[2], RuntimeEvent::Status { .. }));
    }

    #[test]
    fn translator_does_not_track_task_tool_calls_as_subagents() {
        let (sid, tid) = ids();
        let mut translator = AcpEventTranslator::default();
        let start = SessionUpdate::ToolCall(AcpToolCall::new("sub_1", "Task: review runtime"));

        let started = translator.session_update_to_events(&start, &sid, &tid, "Agent", false);

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

        let finished = translator.session_update_to_events(&update, &sid, &tid, "Agent", false);

        assert_eq!(finished.len(), 3);
        assert!(matches!(finished[0], RuntimeEvent::ObservationAdded { .. }));
        assert!(matches!(
            finished[1],
            RuntimeEvent::ToolCallFinished { success: true, .. }
        ));
        assert!(matches!(finished[2], RuntimeEvent::Status { .. }));
    }

    #[test]
    fn completed_tool_update_restores_pending_response_status() {
        let (sid, tid) = ids();
        let mut translator = AcpEventTranslator;
        let mut fields = ToolCallUpdateFields::default();
        fields.status = Some(ToolCallStatus::Completed);
        fields.title = Some("执行 SQL".to_string());
        let update = SessionUpdate::ToolCallUpdate(ToolCallUpdate::new("call_1", fields));
        let events = translator.session_update_to_events(&update, &sid, &tid, "OpenCode", false);
        assert_eq!(events.len(), 3);
        assert!(matches!(events[0], RuntimeEvent::ObservationAdded { .. }));
        assert!(matches!(
            events[1],
            RuntimeEvent::ToolCallFinished { success: true, .. }
        ));
        assert!(matches!(
            &events[2],
            RuntimeEvent::Status { title, is_done: false, .. }
                if title == &t!("AgentUi.acp_continuing_response", name = "OpenCode")
        ));
    }

    #[test]
    fn assistant_delta_replaces_post_tool_pending_status() {
        let (sid, tid) = ids();
        let mut translator = AcpEventTranslator;
        let mut transcript = crate::agent_transcript::AgentTranscript::new();
        let mut fields = ToolCallUpdateFields::default();
        fields.status = Some(ToolCallStatus::Completed);
        let update = SessionUpdate::ToolCallUpdate(ToolCallUpdate::new("call_1", fields));

        for event in translator.session_update_to_events(&update, &sid, &tid, "Codex", false) {
            transcript.apply(&event);
        }
        assert!(matches!(
            transcript.messages.last().map(|message| &message.variant),
            Some(crate::MessageVariant::Status { title, is_done: false })
                if title == &t!("AgentUi.acp_continuing_response", name = "Codex")
        ));

        let delta = SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
            TextContent::new("继续回答"),
        )));
        for event in translator.session_update_to_events(&delta, &sid, &tid, "Codex", false) {
            transcript.apply(&event);
        }

        assert_eq!(2, transcript.messages.len());
        assert!(!transcript.messages.iter().any(|message| matches!(
            message.variant,
            crate::MessageVariant::Status { is_done: false, .. }
        )));
        let answer = transcript.messages.last().expect("answer should be last");
        assert_eq!("继续回答", answer.content);
        assert!(matches!(answer.variant, crate::MessageVariant::Text));
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
