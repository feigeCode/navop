//! Agent 对话转录:把 [`RuntimeEvent`] 流归约为可渲染的消息列表(纯逻辑,无 GPUI)。
//!
//! 把这一层与视图解耦,既便于单元测试(无需 GPUI),又让事件处理逻辑集中、可读。
//! 视图([`AgentChatView`](crate::agent_view::AgentChatView))只需在收到事件时调用
//! [`AgentTranscript::apply`],再渲染 `messages`。

use agent_runtime::{HistoryItem, Plan, PlanStatus, RuntimeEvent, StepStatus, ToolObservation};

use crate::agent_cards::{
    PlanCardData, PlanStepData, SUBAGENT_CARD, SubAgentCardData, TOOL_CARD, ToolCardData,
};
use crate::code_block::extract_fenced_code_blocks;
use crate::{ChatMessageUI, MessageVariant, parse_chart_json_block};

/// 观测数据文本入卡片时的最大字符数(渲染时还会再截断展示)。
const MAX_DATA_CHARS: usize = 2000;

/// Agent 对话转录状态。
#[derive(Default)]
pub struct AgentTranscript {
    /// 渲染用的消息列表。
    pub messages: Vec<ChatMessageUI>,
    /// 当前流式助手消息 id(若正在流式)。
    streaming_id: Option<String>,
    /// 当前轻量状态消息 id(若存在未完成状态)。
    active_status_id: Option<String>,
    /// 本轮最新计划(渲染到输入框上方的 Tasks 面板,不进消息流)。
    latest_plan: Option<PlanCardData>,
}

impl AgentTranscript {
    pub fn new() -> Self {
        Self::default()
    }

    /// 清空(切换 / 新建会话)。
    pub fn clear(&mut self) {
        self.messages.clear();
        self.streaming_id = None;
        self.active_status_id = None;
        self.latest_plan = None;
    }

    /// 当前轮的最新计划(供输入框上方的 Tasks 面板渲染;不进消息流)。
    pub fn latest_plan(&self) -> Option<&PlanCardData> {
        self.latest_plan.as_ref()
    }

    /// 用持久化的历史条目重建转录(切换 / 恢复会话时调用)。
    ///
    /// 复用与实时事件相同的归约逻辑:工具调用 + 观测合并为一张完成态卡片,
    /// 助手 / 用户 / 系统消息逐条还原,最后把 `plan` 填入 Tasks 面板。
    pub fn load_history(&mut self, items: &[HistoryItem], plan: Option<&Plan>) {
        self.clear();
        for item in items {
            match item {
                HistoryItem::User { text, images } => self.push_user(text, images.len()),
                HistoryItem::Assistant(text) => {
                    self.messages.push(ChatMessageUI::assistant(text.clone()));
                }
                HistoryItem::AssistantWithReasoning { text, reasoning } => {
                    self.messages.push(
                        ChatMessageUI::assistant(text.clone())
                            .with_reasoning_content(reasoning.clone()),
                    );
                }
                HistoryItem::System(text) => self.push_system(text.clone()),
                HistoryItem::ToolCall(call) => {
                    self.push_tool_call(call.call_id.as_str(), call.tool_name.as_str());
                }
                HistoryItem::Observation(obs) => {
                    self.apply_observation(obs);
                    self.finish_tool_call(obs.call_id.as_str(), obs.success);
                }
            }
        }
        if let Some(plan) = plan {
            self.upsert_plan(plan);
        }
    }

    /// 追加一条系统提示。
    pub fn push_system(&mut self, text: impl Into<String>) {
        self.messages.push(ChatMessageUI::system(text));
    }

    /// 追加用户消息(提交时由视图调用;`image_count` 用于提示附带图片)。
    pub fn push_user(&mut self, text: &str, image_count: usize) {
        let content = if image_count > 0 {
            format!("{text}\n\n[附带 {image_count} 张图片]")
        } else {
            text.to_string()
        };
        self.messages.push(ChatMessageUI::user(content));
    }

    /// 应用一个运行时事件,更新消息列表。
    pub fn apply(&mut self, event: &RuntimeEvent) {
        match event {
            RuntimeEvent::TurnStarted { .. } => {
                // 新一轮只重置本轮临时输出;已有计划需保留到下一次 PlanUpdated。
                self.streaming_id = None;
                self.active_status_id = None;
            }
            RuntimeEvent::AssistantMessageDelta { delta, .. } => {
                self.append_delta(delta);
            }
            RuntimeEvent::ReasoningDelta { delta, .. } => {
                self.append_reasoning_delta(delta);
            }
            RuntimeEvent::AssistantMessage { text, .. } => {
                self.finalize_assistant(text);
            }
            RuntimeEvent::UserMessage { text, .. } => {
                self.push_user(text, 0);
            }
            RuntimeEvent::Status { title, is_done, .. } => {
                self.upsert_status(title, *is_done);
            }
            RuntimeEvent::PlanUpdated { plan, .. } => {
                self.upsert_plan(plan);
            }
            RuntimeEvent::ToolCallStarted {
                call_id, tool_name, ..
            } => {
                self.push_tool_call(call_id.as_str(), tool_name.as_str());
            }
            RuntimeEvent::ObservationAdded { observation, .. } => {
                self.apply_observation(observation);
            }
            RuntimeEvent::ToolCallFinished {
                call_id, success, ..
            } => {
                self.finish_tool_call(call_id.as_str(), *success);
            }
            RuntimeEvent::SubAgentStarted {
                subagent_id,
                name,
                task,
                ..
            } => {
                self.push_subagent(subagent_id.as_str(), name, task);
            }
            RuntimeEvent::SubAgentUpdated {
                subagent_id,
                summary,
                ..
            } => {
                self.update_subagent(subagent_id.as_str(), summary);
            }
            RuntimeEvent::SubAgentFinished {
                subagent_id,
                success,
                summary,
                ..
            } => {
                self.finish_subagent(subagent_id.as_str(), *success, summary);
            }
            RuntimeEvent::NeedUserInput { question, .. } => {
                self.messages
                    .push(ChatMessageUI::assistant(format!("❓ {question}")));
            }
            RuntimeEvent::TurnFailed { reason, .. } => {
                self.streaming_id = None;
                self.messages
                    .push(ChatMessageUI::system(format!("⚠️ 任务失败:{reason}")));
            }
            RuntimeEvent::TurnCompleted { .. } => {
                self.finish_active_status();
                self.close_streaming_segment();
            }
        }
    }

    // ===== 助手文本 =====

    fn append_delta(&mut self, delta: &str) {
        self.finish_active_status();
        if let Some(id) = &self.streaming_id {
            if let Some(msg) = self.messages.iter_mut().find(|m| &m.id == id) {
                msg.content.push_str(delta);
                return;
            }
        }
        let msg = ChatMessageUI::streaming_assistant().with_content(delta.to_string());
        self.streaming_id = Some(msg.id.clone());
        self.messages.push(msg);
    }

    fn append_reasoning_delta(&mut self, delta: &str) {
        self.finish_active_status();
        if let Some(id) = &self.streaming_id {
            if let Some(msg) = self.messages.iter_mut().find(|m| &m.id == id) {
                msg.reasoning_content.push_str(delta);
                return;
            }
        }
        let mut msg = ChatMessageUI::streaming_assistant();
        msg.reasoning_content.push_str(delta);
        self.streaming_id = Some(msg.id.clone());
        self.messages.push(msg);
    }

    fn finalize_assistant(&mut self, text: &str) {
        self.finish_active_status();
        if let Some(id) = self.streaming_id.take() {
            if let Some(index) = self.messages.iter().position(|m| m.id == id) {
                let reasoning = self.messages[index].reasoning_content.clone();
                let mut messages = assistant_messages_from_content(text);
                if let Some(first) = messages.first_mut() {
                    first.reasoning_content = reasoning;
                }
                self.messages.splice(index..=index, messages);
                return;
            }
        }
        self.messages.extend(assistant_messages_from_content(text));
    }

    fn close_streaming_segment(&mut self) {
        if let Some(id) = self.streaming_id.take()
            && let Some(index) = self.messages.iter().position(|m| m.id == id)
        {
            let content = self.messages[index].content.clone();
            let reasoning = self.messages[index].reasoning_content.clone();
            let mut messages = assistant_messages_from_content(&content);
            if let Some(first) = messages.first_mut() {
                first.reasoning_content = reasoning;
            }
            self.messages.splice(index..=index, messages);
        }
    }

    fn upsert_status(&mut self, title: &str, is_done: bool) {
        if let Some(id) = &self.active_status_id
            && let Some(msg) = self.messages.iter_mut().find(|m| &m.id == id)
        {
            msg.variant = MessageVariant::Status {
                title: title.to_string(),
                is_done,
            };
            msg.is_streaming = !is_done;
            if is_done {
                self.active_status_id = None;
            }
            return;
        }
        let msg = ChatMessageUI::status(title.to_string(), is_done);
        if !is_done {
            self.active_status_id = Some(msg.id.clone());
        }
        self.messages.push(msg);
    }

    fn finish_active_status(&mut self) {
        if let Some(id) = self.active_status_id.take()
            && let Some(msg) = self.messages.iter_mut().find(|m| m.id == id)
        {
            msg.variant = MessageVariant::Status {
                title: "思考完成".to_string(),
                is_done: true,
            };
            msg.is_streaming = false;
        }
    }

    // ===== 计划卡片 =====

    fn upsert_plan(&mut self, plan: &Plan) {
        self.latest_plan = Some(plan_to_card(plan));
    }

    // ===== 工具卡片 =====

    fn push_tool_call(&mut self, call_id: &str, tool_name: &str) {
        self.finish_active_status();
        self.close_streaming_segment();
        let data = ToolCardData {
            call_id: call_id.to_string(),
            tool_name: tool_name.to_string(),
            running: true,
            success: None,
            summary: String::new(),
            data_text: String::new(),
        };
        self.messages
            .push(ChatMessageUI::card(TOOL_CARD, data.to_json()));
    }

    fn apply_observation(&mut self, obs: &ToolObservation) {
        let call_id = obs.call_id.to_string();
        let summary = obs.summary.clone();
        let data_text = truncate_chars(&obs.data.to_text(), MAX_DATA_CHARS);
        let success = obs.success;

        if let Some(mut data) = self.find_tool_card(&call_id) {
            data.summary = summary;
            data.data_text = data_text;
            data.success = Some(success);
            self.replace_tool_card(&call_id, data);
        } else {
            // 防御:没有对应的开始事件,直接建一张完成态卡片。
            let data = ToolCardData {
                call_id,
                tool_name: obs.tool_name.to_string(),
                running: false,
                success: Some(success),
                summary,
                data_text,
            };
            self.messages
                .push(ChatMessageUI::card(TOOL_CARD, data.to_json()));
        }
    }

    fn finish_tool_call(&mut self, call_id: &str, success: bool) {
        if let Some(mut data) = self.find_tool_card(call_id) {
            data.running = false;
            data.success = Some(success);
            self.replace_tool_card(call_id, data);
        }
    }

    /// 按 call_id 查找工具卡片数据。
    fn find_tool_card(&self, call_id: &str) -> Option<ToolCardData> {
        self.messages.iter().rev().find_map(|m| {
            if m.variant.card_kind() == Some(TOOL_CARD) {
                ToolCardData::from_json(&m.content).filter(|d| d.call_id == call_id)
            } else {
                None
            }
        })
    }

    /// 按 call_id 替换工具卡片内容。
    fn replace_tool_card(&mut self, call_id: &str, data: ToolCardData) {
        let json = data.to_json();
        if let Some(msg) = self.messages.iter_mut().rev().find(|m| {
            m.variant.card_kind() == Some(TOOL_CARD)
                && ToolCardData::from_json(&m.content).is_some_and(|d| d.call_id == call_id)
        }) {
            msg.content = json;
        }
    }

    // ===== 子代理卡片 =====

    fn push_subagent(&mut self, subagent_id: &str, name: &str, task: &str) {
        self.finish_active_status();
        self.close_streaming_segment();
        let data = SubAgentCardData {
            subagent_id: subagent_id.to_string(),
            name: name.to_string(),
            task: task.to_string(),
            running: true,
            success: None,
            summary: String::new(),
        };
        self.messages
            .push(ChatMessageUI::card(SUBAGENT_CARD, data.to_json()));
    }

    fn update_subagent(&mut self, subagent_id: &str, summary: &str) {
        if let Some(mut data) = self.find_subagent_card(subagent_id) {
            data.summary = summary.to_string();
            self.replace_subagent_card(subagent_id, data);
        }
    }

    fn finish_subagent(&mut self, subagent_id: &str, success: bool, summary: &str) {
        if let Some(mut data) = self.find_subagent_card(subagent_id) {
            data.running = false;
            data.success = Some(success);
            if !summary.is_empty() {
                data.summary = summary.to_string();
            }
            self.replace_subagent_card(subagent_id, data);
        }
    }

    fn find_subagent_card(&self, subagent_id: &str) -> Option<SubAgentCardData> {
        self.messages.iter().rev().find_map(|m| {
            if m.variant.card_kind() == Some(SUBAGENT_CARD) {
                SubAgentCardData::from_json(&m.content).filter(|d| d.subagent_id == subagent_id)
            } else {
                None
            }
        })
    }

    fn replace_subagent_card(&mut self, subagent_id: &str, data: SubAgentCardData) {
        let json = data.to_json();
        if let Some(msg) = self.messages.iter_mut().rev().find(|m| {
            m.variant.card_kind() == Some(SUBAGENT_CARD)
                && SubAgentCardData::from_json(&m.content)
                    .is_some_and(|d| d.subagent_id == subagent_id)
        }) {
            msg.content = json;
        }
    }
}

// ===== 枚举 → 卡片字符串 =====

fn plan_to_card(plan: &Plan) -> PlanCardData {
    PlanCardData {
        goal: plan.goal.clone(),
        status: plan_status_str(plan.status).to_string(),
        steps: plan
            .steps
            .iter()
            .map(|s| PlanStepData {
                title: s.title.clone(),
                description: s.description.clone(),
                status: step_status_str(s.status).to_string(),
                risk: s.risk.as_str().to_string(),
                tool: s.tool_hint.as_ref().map(|t| t.to_string()),
            })
            .collect(),
    }
}

fn plan_status_str(status: PlanStatus) -> &'static str {
    match status {
        PlanStatus::Draft => "draft",
        PlanStatus::Running => "running",
        PlanStatus::WaitingUser => "waiting_user",
        PlanStatus::Completed => "completed",
        PlanStatus::Failed => "failed",
    }
}

fn step_status_str(status: StepStatus) -> &'static str {
    match status {
        StepStatus::Pending => "pending",
        StepStatus::Running => "running",
        StepStatus::Observed => "observed",
        StepStatus::Skipped => "skipped",
        StepStatus::Failed => "failed",
        StepStatus::Completed => "completed",
    }
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    text.chars().take(max_chars).collect()
}

fn assistant_messages_from_content(text: &str) -> Vec<ChatMessageUI> {
    let blocks = extract_fenced_code_blocks(text);
    let mut out = Vec::new();
    let mut cursor = 0;
    for block in blocks {
        if parse_chart_json_block(&block.code, block.language.as_deref()).is_none() {
            continue;
        }
        push_assistant_text_segment(&mut out, &text[cursor..block.start]);
        out.push(ChatMessageUI::card("chart-json", block.code));
        cursor = block.end;
    }
    if out.is_empty() {
        return vec![ChatMessageUI::assistant(text.to_string())];
    }
    push_assistant_text_segment(&mut out, &text[cursor..]);
    out
}

fn push_assistant_text_segment(messages: &mut Vec<ChatMessageUI>, text: &str) {
    let trimmed = text.trim();
    if !trimmed.is_empty() {
        messages.push(ChatMessageUI::assistant(trimmed.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime::ids::{SubAgentId, ToolCallId, TurnId};
    use agent_runtime::tools::{ObservationData, ToolName};
    use agent_runtime::{PlanSource, PlanStep, SessionId};

    fn sid() -> SessionId {
        SessionId::from_string("s1")
    }
    fn tid() -> TurnId {
        TurnId::from_string("t1")
    }

    #[test]
    fn streams_then_finalizes_single_assistant_message() {
        let mut tr = AgentTranscript::new();
        tr.apply(&RuntimeEvent::AssistantMessageDelta {
            session_id: sid(),
            turn_id: tid(),
            delta: "你".into(),
        });
        tr.apply(&RuntimeEvent::AssistantMessageDelta {
            session_id: sid(),
            turn_id: tid(),
            delta: "好".into(),
        });
        tr.apply(&RuntimeEvent::AssistantMessage {
            session_id: sid(),
            turn_id: tid(),
            text: "你好".into(),
        });
        // 增量与最终合并为一条消息。
        assert_eq!(tr.messages.len(), 1);
        assert_eq!(tr.messages[0].content, "你好");
        assert!(!tr.messages[0].is_streaming);
    }

    #[test]
    fn reasoning_delta_is_preserved_when_streaming_segment_closes() {
        let mut tr = AgentTranscript::new();
        tr.apply(&RuntimeEvent::ReasoningDelta {
            session_id: sid(),
            turn_id: tid(),
            delta: "先分析".to_string(),
        });
        tr.apply(&RuntimeEvent::AssistantMessageDelta {
            session_id: sid(),
            turn_id: tid(),
            delta: "答案".to_string(),
        });
        tr.apply(&RuntimeEvent::TurnCompleted {
            session_id: sid(),
            turn_id: tid(),
            answer: None,
        });

        assert_eq!(1, tr.messages.len());
        assert_eq!("答案", tr.messages[0].content);
        assert_eq!("先分析", tr.messages[0].reasoning_content);
    }

    #[test]
    fn reasoning_delta_is_preserved_when_final_message_replaces_stream() {
        let mut tr = AgentTranscript::new();
        tr.apply(&RuntimeEvent::ReasoningDelta {
            session_id: sid(),
            turn_id: tid(),
            delta: "先分析".to_string(),
        });
        tr.apply(&RuntimeEvent::AssistantMessageDelta {
            session_id: sid(),
            turn_id: tid(),
            delta: "临时答案".to_string(),
        });
        tr.apply(&RuntimeEvent::AssistantMessage {
            session_id: sid(),
            turn_id: tid(),
            text: "最终答案".to_string(),
        });

        assert_eq!(1, tr.messages.len());
        assert_eq!("最终答案", tr.messages[0].content);
        assert_eq!("先分析", tr.messages[0].reasoning_content);
    }

    #[test]
    fn load_history_preserves_assistant_reasoning() {
        let mut tr = AgentTranscript::new();
        tr.load_history(
            &[HistoryItem::AssistantWithReasoning {
                text: "最终回答".to_string(),
                reasoning: "内部推理".to_string(),
            }],
            None,
        );

        assert_eq!(1, tr.messages.len());
        assert_eq!("最终回答", tr.messages[0].content);
        assert_eq!("内部推理", tr.messages[0].reasoning_content);
    }

    #[test]
    fn tool_call_closes_reasoning_segment_before_followup_answer() {
        let mut tr = AgentTranscript::new();
        let call = ToolCallId::from_string("call_1");
        tr.apply(&RuntimeEvent::ReasoningDelta {
            session_id: sid(),
            turn_id: tid(),
            delta: "先分析工具选择".to_string(),
        });
        tr.apply(&RuntimeEvent::ToolCallStarted {
            session_id: sid(),
            turn_id: tid(),
            call_id: call.clone(),
            tool_name: ToolName::new("echo"),
        });
        tr.apply(&RuntimeEvent::ToolCallFinished {
            session_id: sid(),
            turn_id: tid(),
            call_id: call,
            success: true,
        });
        tr.apply(&RuntimeEvent::AssistantMessageDelta {
            session_id: sid(),
            turn_id: tid(),
            delta: "最终回答".to_string(),
        });
        tr.apply(&RuntimeEvent::TurnCompleted {
            session_id: sid(),
            turn_id: tid(),
            answer: None,
        });

        assert_eq!(3, tr.messages.len());
        assert_eq!("先分析工具选择", tr.messages[0].reasoning_content);
        assert_eq!("", tr.messages[0].content);
        assert_eq!(Some(TOOL_CARD), tr.messages[1].variant.card_kind());
        assert_eq!("最终回答", tr.messages[2].content);
    }

    #[test]
    fn tool_call_then_observation_updates_same_card() {
        let mut tr = AgentTranscript::new();
        let call = ToolCallId::from_string("call_9");
        tr.apply(&RuntimeEvent::ToolCallStarted {
            session_id: sid(),
            turn_id: tid(),
            call_id: call.clone(),
            tool_name: ToolName::new("echo"),
        });
        assert_eq!(tr.messages.len(), 1);

        let obs = ToolObservation::success(
            call.clone(),
            ToolName::new("echo"),
            "echo: hi",
            ObservationData::Text("hi".into()),
        );
        tr.apply(&RuntimeEvent::ObservationAdded {
            session_id: sid(),
            turn_id: tid(),
            observation: obs,
        });
        tr.apply(&RuntimeEvent::ToolCallFinished {
            session_id: sid(),
            turn_id: tid(),
            call_id: call,
            success: true,
        });

        // 仍是同一张卡片,被更新为完成态。
        assert_eq!(tr.messages.len(), 1);
        let data = ToolCardData::from_json(&tr.messages[0].content).unwrap();
        assert!(!data.running);
        assert_eq!(data.success, Some(true));
        assert_eq!(data.summary, "echo: hi");
    }

    #[test]
    fn streaming_text_is_split_around_tool_card_when_delta_continues() {
        let mut tr = AgentTranscript::new();
        let call = ToolCallId::from_string("call_9");
        tr.apply(&RuntimeEvent::AssistantMessageDelta {
            session_id: sid(),
            turn_id: tid(),
            delta: "读取文件前".into(),
        });
        tr.apply(&RuntimeEvent::ToolCallStarted {
            session_id: sid(),
            turn_id: tid(),
            call_id: call,
            tool_name: ToolName::new("Read"),
        });
        tr.apply(&RuntimeEvent::AssistantMessageDelta {
            session_id: sid(),
            turn_id: tid(),
            delta: "继续输出".into(),
        });

        assert_eq!(tr.messages.len(), 3);
        assert_eq!(tr.messages[0].content, "读取文件前");
        assert!(!tr.messages[0].is_streaming);
        assert_eq!(tr.messages[1].variant.card_kind(), Some(TOOL_CARD));
        assert_eq!(tr.messages[2].content, "继续输出");
        assert!(tr.messages[2].is_streaming);

        tr.apply(&RuntimeEvent::TurnCompleted {
            session_id: sid(),
            turn_id: tid(),
            answer: None,
        });
        assert_eq!(tr.messages[2].content, "继续输出");
        assert!(!tr.messages[2].is_streaming);
    }

    #[test]
    fn status_is_closed_when_assistant_text_starts() {
        let mut tr = AgentTranscript::new();
        tr.apply(&RuntimeEvent::Status {
            session_id: sid(),
            turn_id: tid(),
            title: "思考中…".into(),
            is_done: false,
        });
        tr.apply(&RuntimeEvent::AssistantMessageDelta {
            session_id: sid(),
            turn_id: tid(),
            delta: "开始回答".into(),
        });

        assert_eq!(tr.messages.len(), 2);
        assert!(matches!(
            &tr.messages[0].variant,
            MessageVariant::Status { title, is_done: true } if title == "思考完成"
        ));
        assert_eq!(tr.messages[1].content, "开始回答");
        assert!(tr.messages[1].is_streaming);
    }

    #[test]
    fn plan_goes_to_latest_plan_not_messages() {
        let mut tr = AgentTranscript::new();
        let mut plan = Plan::new("排查慢查询", PlanSource::Llm)
            .with_steps(vec![PlanStep::new("查看连接数", "SHOW PROCESSLIST")]);
        plan.set_status(PlanStatus::Running);
        tr.apply(&RuntimeEvent::PlanUpdated {
            session_id: sid(),
            turn_id: tid(),
            plan,
        });
        // 计划不进消息流。
        assert!(tr.messages.is_empty());
        // 计划存入 latest_plan,可读出目标与步骤。
        let data = tr.latest_plan().expect("latest_plan 应已填充");
        assert_eq!(data.goal, "排查慢查询");
        assert_eq!(data.steps.len(), 1);
    }

    #[test]
    fn subagent_events_update_same_card() {
        let mut tr = AgentTranscript::new();
        let subagent_id = SubAgentId::from_string("sub_1");
        tr.apply(&RuntimeEvent::SubAgentStarted {
            session_id: sid(),
            turn_id: tid(),
            subagent_id: subagent_id.clone(),
            name: "reviewer".into(),
            task: "检查 agent runtime".into(),
        });
        tr.apply(&RuntimeEvent::SubAgentUpdated {
            session_id: sid(),
            turn_id: tid(),
            subagent_id: subagent_id.clone(),
            summary: "正在读取事件流".into(),
        });
        tr.apply(&RuntimeEvent::SubAgentFinished {
            session_id: sid(),
            turn_id: tid(),
            subagent_id,
            success: true,
            summary: "发现 reasoning 未转发".into(),
        });

        assert_eq!(tr.messages.len(), 1);
        assert_eq!(tr.messages[0].variant.card_kind(), Some(SUBAGENT_CARD));
        let data = SubAgentCardData::from_json(&tr.messages[0].content).unwrap();
        assert_eq!(data.name, "reviewer");
        assert_eq!(data.task, "检查 agent runtime");
        assert!(!data.running);
        assert_eq!(data.success, Some(true));
        assert_eq!(data.summary, "发现 reasoning 未转发");
    }

    #[test]
    fn turn_started_preserves_latest_plan() {
        let mut tr = AgentTranscript::new();
        let plan = Plan::new("目标", PlanSource::Llm).with_steps(vec![PlanStep::new("step", "")]);
        tr.apply(&RuntimeEvent::PlanUpdated {
            session_id: sid(),
            turn_id: tid(),
            plan,
        });
        assert!(tr.latest_plan().is_some());
        tr.apply(&RuntimeEvent::TurnStarted {
            session_id: sid(),
            turn_id: tid(),
        });
        assert!(tr.latest_plan().is_some());
    }

    #[test]
    fn final_assistant_message_expands_chart_json_code_block_to_card() {
        let mut tr = AgentTranscript::new();
        let text = r#"下面是图表:

```chart-json
{"chart_type":"bar","data":[{"x":"Jan","y":1}]}
```

结论如上。"#;

        tr.apply(&RuntimeEvent::AssistantMessage {
            session_id: sid(),
            turn_id: tid(),
            text: text.to_string(),
        });

        assert_eq!(3, tr.messages.len());
        assert_eq!("下面是图表:", tr.messages[0].content.trim());
        assert_eq!(Some("chart-json"), tr.messages[1].variant.card_kind());
        assert!(tr.messages[1].content.contains("\"chart_type\":\"bar\""));
        assert_eq!("结论如上。", tr.messages[2].content.trim());
    }

    #[test]
    fn non_chart_json_code_block_remains_assistant_markdown() {
        let mut tr = AgentTranscript::new();
        let text = "```json\n{\"ok\":true}\n```";

        tr.apply(&RuntimeEvent::AssistantMessage {
            session_id: sid(),
            turn_id: tid(),
            text: text.to_string(),
        });

        assert_eq!(1, tr.messages.len());
        assert_eq!(text, tr.messages[0].content);
        assert_eq!(None, tr.messages[0].variant.card_kind());
    }
}
