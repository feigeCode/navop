//! Agent 对话转录:把 [`RuntimeEvent`] 流归约为可渲染的消息列表(纯逻辑,无 GPUI)。
//!
//! 把这一层与视图解耦,既便于单元测试(无需 GPUI),又让事件处理逻辑集中、可读。
//! 视图([`AgentChatView`](crate::agent_view::AgentChatView))只需在收到事件时调用
//! [`AgentTranscript::apply`],再渲染 `messages`。

use agent_runtime::{
    HistoryItem, PendingToolCallSummary, Plan, PlanStatus, ResourceContext, RuntimeEvent,
    StepStatus, ToolObservation,
    ids::{ToolCallId, TurnId},
};
use rust_i18n::t;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};

use crate::acp::{AcpPermissionOption, AcpPermissionRequest, AcpPublicMcpApprovalRequest};
use crate::agent_cards::{
    ACP_PERMISSION_CARD, AcpPermissionCardData, AcpPermissionOptionData, PlanCardData,
    PlanStepData, SUBAGENT_CARD, SubAgentCardData, TOOL_CARD, TOOL_CONFIRM_CARD, ToolCardData,
    ToolConfirmCardData, ToolConfirmItemData,
};
use crate::agent_tool_input::build_tool_input_display;
use crate::code_block::extract_fenced_code_blocks;
use crate::{ChatMessageUI, MessageVariant, parse_chart_json_block};

mod acp;

/// 观测数据文本入卡片时的最大字符数(渲染时还会再截断展示)。
const MAX_DATA_CHARS: usize = 2000;
const MAX_TERMINAL_EXEC_DATA_CHARS: usize = 64_000;
const MAX_TRANSCRIPT_MESSAGES: usize = 500;
const MAX_TRANSCRIPT_TEXT_BYTES: usize = 8 * 1024 * 1024;
const MAX_TRANSCRIPT_FIELD_BYTES: usize = 1024 * 1024;
const MAX_ACTIVE_SUBAGENTS: usize = 64;
const MAX_TERMINAL_EVENTS: usize = 1024;
const MAX_CACHED_TOOL_INPUTS: usize = 128;
const MAX_CACHED_TOOL_INPUT_BYTES: usize = 1024 * 1024;
const MAX_CARD_FIELD_BYTES: usize = 64 * 1024;
const MAX_CARD_ITEM_FIELD_BYTES: usize = 8 * 1024;
const MAX_CARD_COLLECTION_ITEMS: usize = 64;
const TRANSCRIPT_TRUNCATION_MARKER: &str = "[...earlier content truncated...]\n";
const DELEGATE_TASK_TOOL: &str = "delegate_task";

#[derive(Clone, Copy, Debug)]
struct TranscriptBudget {
    max_messages: usize,
    max_text_bytes: usize,
    max_field_bytes: usize,
}

impl Default for TranscriptBudget {
    fn default() -> Self {
        Self {
            max_messages: MAX_TRANSCRIPT_MESSAGES,
            max_text_bytes: MAX_TRANSCRIPT_TEXT_BYTES,
            max_field_bytes: MAX_TRANSCRIPT_FIELD_BYTES,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum TerminalEventKey {
    AwaitingInput {
        turn_id: TurnId,
        call_id: Option<ToolCallId>,
        question_hash: u64,
    },
    Final(TurnId),
}

/// Agent 对话转录状态。
#[derive(Default)]
pub struct AgentTranscript {
    /// 渲染用的消息列表。
    pub messages: Vec<ChatMessageUI>,
    /// 当前流式助手消息 id(若正在流式)。
    streaming_id: Option<String>,
    /// 当前轻量状态消息 id(若存在未完成状态)。
    active_status_id: Option<String>,
    /// ACP 连接生命周期状态消息 id，与单轮执行状态分开维护。
    acp_status_id: Option<String>,
    /// 本轮最新计划(渲染到输入框上方的 Tasks 面板,不进消息流)。
    latest_plan: Option<PlanCardData>,
    /// 当前会话最近的子代理(渲染到输入框上方的子代理面板,不进消息流)。
    active_subagents: Vec<SubAgentCardData>,
    /// 当前资源池 id -> label 快照,用于工具结果卡片展示目标资源。
    resource_labels: HashMap<String, String>,
    /// 当前会话工具调用的原始入参；用于把精简 ACP permission 与实际 MCP 调用精确关联。
    tool_inputs: HashMap<String, serde_json::Value>,
    /// 原始工具入参插入顺序,用于淘汰长期未完成的陈旧调用。
    tool_input_order: VecDeque<String>,
    /// 已归约的审批/终态事件,防止重复事件写入转录或触发持久化。
    terminal_events: HashSet<TerminalEventKey>,
    /// 已归约事件插入顺序,只保留有限的近期去重窗口。
    terminal_event_order: VecDeque<TerminalEventKey>,
    /// UI transcript 的逻辑内存预算。
    budget: TranscriptBudget,
}

impl AgentTranscript {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    fn with_limits(max_messages: usize, max_text_bytes: usize, max_field_bytes: usize) -> Self {
        Self {
            budget: TranscriptBudget {
                max_messages,
                max_text_bytes,
                max_field_bytes,
            },
            ..Self::default()
        }
    }

    /// 更新当前会话资源池快照,供后续工具 observation 展示目标资源。
    pub fn set_resource_context(&mut self, resources: &ResourceContext) {
        self.resource_labels = resources
            .resources
            .iter()
            .map(|resource| (resource.id.as_str().to_string(), resource.label.clone()))
            .collect();
    }

    /// 清空(切换 / 新建会话)。
    pub fn clear(&mut self) {
        self.messages.clear();
        self.streaming_id = None;
        self.active_status_id = None;
        self.acp_status_id = None;
        self.latest_plan = None;
        self.active_subagents.clear();
        self.tool_inputs.clear();
        self.tool_input_order.clear();
        self.terminal_events.clear();
        self.terminal_event_order.clear();
    }

    /// 当前轮的最新计划(供输入框上方的 Tasks 面板渲染;不进消息流)。
    pub fn latest_plan(&self) -> Option<&PlanCardData> {
        self.latest_plan.as_ref()
    }

    /// 当前会话最近的子代理(供输入框上方的子代理面板渲染;不进消息流)。
    pub fn active_subagents(&self) -> &[SubAgentCardData] {
        &self.active_subagents
    }

    /// 是否存在等待用户处理的工具确认卡。
    pub fn has_pending_tool_confirm(&self, call_id: &str) -> bool {
        self.find_confirm_card(call_id)
            .is_some_and(|data| data.status == "pending")
    }

    /// 是否存在等待用户处理的 ACP 权限请求。
    pub fn has_pending_acp_permission(&self, request_id: &str) -> bool {
        self.find_acp_permission_card(request_id)
            .is_some_and(|data| data.status == "pending")
    }

    /// 把当前 ACP 连接收到的权限请求追加到消息流。
    pub(crate) fn push_acp_permission(
        &mut self,
        request: &AcpPermissionRequest,
        requires_safety_confirmation: bool,
    ) {
        if self.has_pending_acp_permission(&request.request_id) {
            return;
        }
        self.finish_active_status();
        self.close_streaming_segment();
        let summary = if requires_safety_confirmation {
            t!(
                "AgentUi.acp_safety_confirmation_notice",
                summary = request.summary
            )
            .to_string()
        } else {
            request.summary.clone()
        };
        let mut data = AcpPermissionCardData {
            request_id: request.request_id.clone(),
            session_id: request.session_id.clone(),
            tool_call_id: request.tool_call_id.clone(),
            tool_name: request.tool_name.clone(),
            summary,
            details_json: serde_json::to_string_pretty(&request.details)
                .unwrap_or_else(|_| request.details.to_string()),
            options: request
                .options
                .iter()
                .map(|option| AcpPermissionOptionData {
                    option_id: option.option_id.clone(),
                    name: option.name.clone(),
                    kind: option.kind.clone(),
                })
                .collect(),
            status: "pending".into(),
            selected_option_name: String::new(),
        };
        bound_acp_permission_data(&mut data, self.card_field_limit());
        self.messages
            .push(ChatMessageUI::card(ACP_PERMISSION_CARD, data.to_json()));
        self.enforce_budget();
    }

    /// 将 ACP 权限卡更新为用户已经选择的终态。
    pub(crate) fn resolve_acp_permission(
        &mut self,
        request_id: &str,
        option: &AcpPermissionOption,
    ) {
        let Some(mut data) = self.find_acp_permission_card(request_id) else {
            return;
        };
        data.status = if option.kind.starts_with("reject") {
            "rejected"
        } else {
            "approved"
        }
        .into();
        data.selected_option_name = option.name.clone();
        self.replace_acp_permission_card(request_id, data);
        self.enforce_budget();
    }

    /// 将未决 ACP 权限卡更新为取消态。
    pub(crate) fn cancel_acp_permission(&mut self, request_id: &str) {
        let Some(mut data) = self.find_acp_permission_card(request_id) else {
            return;
        };
        data.status = "cancelled".into();
        data.selected_option_name.clear();
        self.replace_acp_permission_card(request_id, data);
        self.enforce_budget();
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
                HistoryItem::ContextSummary {
                    text,
                    original_items,
                } => self.push_system(
                    t!(
                        "AgentUi.context_summary",
                        count = original_items,
                        text = text
                    )
                    .to_string(),
                ),
                HistoryItem::ToolCall(call) => {
                    if !self.push_delegate_task_from_history(call) {
                        self.push_tool_call(
                            call.call_id.as_str(),
                            call.tool_name.as_str(),
                            &call.arguments,
                        );
                    }
                }
                HistoryItem::Observation(obs) => {
                    if !self.finish_delegate_task_from_history(obs) {
                        self.apply_observation(obs);
                        self.finish_tool_call(obs.call_id.as_str(), obs.success);
                    }
                }
            }
            // 历史恢复也逐项收敛,避免先完整装载大历史再统一淘汰的峰值。
            self.enforce_budget();
        }
        if let Some(plan) = plan {
            self.upsert_plan(plan);
        }
        self.enforce_budget();
    }

    /// 追加一条系统提示。
    pub fn push_system(&mut self, text: impl Into<String>) {
        self.messages.push(ChatMessageUI::system(text));
        self.enforce_budget();
    }

    /// 追加用户消息(提交时由视图调用;`image_count` 用于提示附带图片)。
    pub fn push_user(&mut self, text: &str, image_count: usize) {
        let content = if image_count > 0 {
            t!(
                "AgentUi.message_with_images",
                text = text,
                count = image_count
            )
            .to_string()
        } else {
            text.to_string()
        };
        self.messages.push(ChatMessageUI::user(content));
        self.enforce_budget();
    }

    /// 应用一个运行时事件,更新消息列表。
    pub fn apply(&mut self, event: &RuntimeEvent) -> bool {
        if let Some(key) = terminal_event_key(event)
            && !self.record_terminal_event(key)
        {
            return false;
        }
        match event {
            RuntimeEvent::TurnStarted { .. }
            | RuntimeEvent::AssistantMessageDelta { .. }
            | RuntimeEvent::ReasoningDelta { .. }
            | RuntimeEvent::AssistantMessage { .. }
            | RuntimeEvent::UserMessage { .. }
            | RuntimeEvent::Status { .. } => self.apply_message_event(event),
            RuntimeEvent::PlanUpdated { .. }
            | RuntimeEvent::SubAgentStarted { .. }
            | RuntimeEvent::SubAgentUpdated { .. }
            | RuntimeEvent::SubAgentFinished { .. } => self.apply_progress_event(event),
            RuntimeEvent::ToolCallStarted { .. }
            | RuntimeEvent::ObservationAdded { .. }
            | RuntimeEvent::ToolCallFinished { .. }
            | RuntimeEvent::NeedUserInput { .. }
            | RuntimeEvent::ToolApprovalResolved { .. } => self.apply_tool_event(event),
            RuntimeEvent::TurnFailed { .. }
            | RuntimeEvent::TurnCancelled { .. }
            | RuntimeEvent::TurnCompleted { .. } => self.apply_terminal_event(event),
        }
        self.enforce_budget();
        true
    }

    fn apply_message_event(&mut self, event: &RuntimeEvent) {
        match event {
            RuntimeEvent::TurnStarted { .. } => {
                self.streaming_id = None;
                self.active_status_id = None;
            }
            RuntimeEvent::AssistantMessageDelta { delta, .. } => self.append_delta(delta),
            RuntimeEvent::ReasoningDelta { delta, .. } => self.append_reasoning_delta(delta),
            RuntimeEvent::AssistantMessage { text, .. } => self.finalize_assistant(text),
            RuntimeEvent::UserMessage { text, .. } => self.push_user(text, 0),
            RuntimeEvent::Status { title, is_done, .. } => self.upsert_status(title, *is_done),
            _ => unreachable!("non-message event routed to apply_message_event"),
        }
    }

    fn apply_progress_event(&mut self, event: &RuntimeEvent) {
        match event {
            RuntimeEvent::PlanUpdated { plan, .. } => self.upsert_plan(plan),
            RuntimeEvent::SubAgentStarted {
                subagent_id,
                name,
                task,
                ..
            } => self.push_subagent(subagent_id.as_str(), name, task),
            RuntimeEvent::SubAgentUpdated {
                subagent_id,
                summary,
                ..
            } => self.update_subagent(subagent_id.as_str(), summary),
            RuntimeEvent::SubAgentFinished {
                subagent_id,
                success,
                summary,
                ..
            } => self.finish_subagent(subagent_id.as_str(), *success, summary),
            _ => unreachable!("non-progress event routed to apply_progress_event"),
        }
    }

    fn apply_tool_event(&mut self, event: &RuntimeEvent) {
        match event {
            RuntimeEvent::ToolCallStarted {
                call_id,
                tool_name,
                arguments,
                ..
            } => self.push_tool_call(call_id.as_str(), tool_name.as_str(), arguments),
            RuntimeEvent::ObservationAdded { observation, .. } => {
                self.apply_observation(observation)
            }
            RuntimeEvent::ToolCallFinished {
                call_id, success, ..
            } => self.finish_tool_call(call_id.as_str(), *success),
            RuntimeEvent::NeedUserInput { .. } => self.push_tool_confirmation(event),
            RuntimeEvent::ToolApprovalResolved {
                call_id, approved, ..
            } => self.resolve_tool_confirm(call_id.as_str(), *approved),
            _ => unreachable!("non-tool event routed to apply_tool_event"),
        }
    }

    fn push_tool_confirmation(&mut self, event: &RuntimeEvent) {
        let RuntimeEvent::NeedUserInput {
            question,
            pending_tool_call_id,
            tool_name,
            arguments,
            pending_tool_calls,
            ..
        } = event
        else {
            unreachable!("non-input event routed to push_tool_confirmation");
        };
        self.finish_active_status();
        let tool_name = tool_name
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "unknown".into());
        let input = self.confirm_input_display(
            pending_tool_call_id.as_ref(),
            &tool_name,
            arguments.as_ref(),
        );
        let mut data = ToolConfirmCardData {
            call_id: pending_tool_call_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            tool_name,
            items: self.confirm_items_display(pending_tool_calls),
            input_summary: input.summary,
            input_json: input.json,
            question: question.to_string(),
            status: "pending".into(),
        };
        bound_tool_confirm_data(&mut data, self.card_field_limit());
        self.messages
            .push(ChatMessageUI::card(TOOL_CONFIRM_CARD, data.to_json()));
        self.enforce_budget();
    }

    pub(crate) fn push_public_mcp_approval(&mut self, request: &AcpPublicMcpApprovalRequest) {
        if self.has_pending_tool_confirm(&request.request_id) {
            return;
        }
        self.finish_active_status();
        self.close_streaming_segment();
        let arguments = request
            .arguments()
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let input = build_tool_input_display(&request.tool_name, &arguments);
        let mut data = ToolConfirmCardData {
            call_id: request.request_id.clone(),
            tool_name: request.tool_name.clone(),
            items: Vec::new(),
            input_summary: input.summary,
            input_json: input.json,
            question: t!(
                "AgentUi.public_mcp_safety_confirmation",
                summary = request.summary
            )
            .to_string(),
            status: "pending".into(),
        };
        bound_tool_confirm_data(&mut data, self.card_field_limit());
        self.messages
            .push(ChatMessageUI::card(TOOL_CONFIRM_CARD, data.to_json()));
        self.enforce_budget();
    }

    fn apply_terminal_event(&mut self, event: &RuntimeEvent) {
        self.finish_active_status();
        match event {
            RuntimeEvent::TurnFailed { reason, .. } => {
                self.streaming_id = None;
                self.messages.push(ChatMessageUI::system(
                    t!("AgentUi.task_failed_warning", error = reason).to_string(),
                ));
            }
            RuntimeEvent::TurnCancelled { .. } => {
                self.close_streaming_segment();
                self.messages.push(ChatMessageUI::system(
                    t!("AgentUi.task_cancelled").to_string(),
                ));
            }
            RuntimeEvent::TurnCompleted { .. } => self.close_streaming_segment(),
            _ => unreachable!("non-terminal event routed to apply_terminal_event"),
        }
    }

    // ===== 助手文本 =====

    fn append_delta(&mut self, delta: &str) {
        self.finish_active_status();
        if let Some(id) = &self.streaming_id {
            if let Some(msg) = self.messages.iter_mut().find(|m| &m.id == id) {
                append_bounded(&mut msg.content, delta, self.budget.max_field_bytes);
                return;
            }
        }
        let mut content = String::new();
        append_bounded(&mut content, delta, self.budget.max_field_bytes);
        let msg = ChatMessageUI::streaming_assistant().with_content(content);
        self.streaming_id = Some(msg.id.clone());
        self.messages.push(msg);
    }

    fn append_reasoning_delta(&mut self, delta: &str) {
        self.finish_active_status();
        if let Some(id) = &self.streaming_id {
            if let Some(msg) = self.messages.iter_mut().find(|m| &m.id == id) {
                append_bounded(
                    &mut msg.reasoning_content,
                    delta,
                    self.budget.max_field_bytes,
                );
                return;
            }
        }
        let mut msg = ChatMessageUI::streaming_assistant();
        append_bounded(
            &mut msg.reasoning_content,
            delta,
            self.budget.max_field_bytes,
        );
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
        if let Some(id) = self.active_status_id.take() {
            self.messages.retain(|msg| msg.id != id);
        }
    }

    // ===== 计划卡片 =====

    fn upsert_plan(&mut self, plan: &Plan) {
        self.latest_plan = Some(plan_to_card(plan));
    }

    // ===== 工具卡片 =====

    fn push_tool_call(&mut self, call_id: &str, tool_name: &str, arguments: &serde_json::Value) {
        self.finish_active_status();
        self.close_streaming_segment();
        self.cache_tool_input(call_id, arguments);
        let input = build_tool_input_display(tool_name, arguments);
        let mut data = ToolCardData {
            call_id: call_id.to_string(),
            tool_name: tool_name.to_string(),
            target_id: None,
            target_label: None,
            input_summary: input.summary,
            input_json: input.json,
            running: true,
            success: None,
            summary: String::new(),
            data_text: String::new(),
        };
        bound_tool_card_data(&mut data, self.card_field_limit());
        self.messages
            .push(ChatMessageUI::card(TOOL_CARD, data.to_json()));
    }

    fn apply_observation(&mut self, obs: &ToolObservation) {
        let call_id = obs.call_id.to_string();
        let summary = obs.summary.clone();
        let data_text = truncate_chars(&obs.data.to_text(), max_data_chars_for_tool(obs));
        let success = obs.success;
        let target_id = obs.resource_id.as_ref().map(|id| id.as_str().to_string());
        let target_label = target_id.as_ref().map(|id| {
            self.resource_labels
                .get(id)
                .cloned()
                .unwrap_or_else(|| id.clone())
        });

        if let Some(mut data) = self.find_tool_card(&call_id) {
            data.target_id = target_id;
            data.target_label = target_label;
            data.summary = summary;
            data.data_text = data_text;
            data.success = Some(success);
            self.replace_tool_card(&call_id, data);
        } else {
            // 防御:没有对应的开始事件,直接建一张完成态卡片。
            let data = ToolCardData {
                call_id,
                tool_name: obs.tool_name.to_string(),
                target_id,
                target_label,
                input_summary: String::new(),
                input_json: String::new(),
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
        self.remove_tool_input(call_id);
        if let Some(mut data) = self.find_tool_card(call_id) {
            data.running = false;
            data.success = Some(success);
            self.replace_tool_card(call_id, data);
        }
    }

    pub(crate) fn tool_call_arguments(&self, call_id: &str) -> Option<&serde_json::Value> {
        self.tool_inputs.get(call_id)
    }

    pub(crate) fn resolve_tool_confirm(&mut self, call_id: &str, approved: bool) {
        let Some(mut data) = self.find_confirm_card(call_id) else {
            return;
        };
        data.status = if approved { "approved" } else { "rejected" }.into();
        self.replace_confirm_card(call_id, data);
        self.enforce_budget();
    }

    fn confirm_input_display(
        &self,
        call_id: Option<&ToolCallId>,
        tool_name: &str,
        arguments: Option<&serde_json::Value>,
    ) -> crate::agent_tool_input::ToolInputDisplay {
        if let Some(args) = arguments {
            let input = build_tool_input_display(tool_name, args);
            if !input.json.is_empty() {
                return input;
            }
        }
        call_id
            .and_then(|id| self.find_tool_card(id.as_str()))
            .map(|data| crate::agent_tool_input::ToolInputDisplay {
                summary: data.input_summary,
                json: data.input_json,
            })
            .unwrap_or_default()
    }

    fn confirm_items_display(
        &self,
        pending_tool_calls: &[PendingToolCallSummary],
    ) -> Vec<ToolConfirmItemData> {
        pending_tool_calls
            .iter()
            .map(|call| {
                let input = build_tool_input_display(call.tool_name.as_str(), &call.arguments);
                ToolConfirmItemData {
                    call_id: call.call_id.to_string(),
                    tool_name: call.tool_name.to_string(),
                    input_summary: input.summary,
                    input_json: input.json,
                }
            })
            .collect()
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

    fn find_confirm_card(&self, call_id: &str) -> Option<ToolConfirmCardData> {
        self.messages.iter().rev().find_map(|m| {
            if m.variant.card_kind() == Some(TOOL_CONFIRM_CARD) {
                ToolConfirmCardData::from_json(&m.content).filter(|d| d.call_id == call_id)
            } else {
                None
            }
        })
    }

    fn replace_confirm_card(&mut self, call_id: &str, data: ToolConfirmCardData) {
        let json = data.to_json();
        if let Some(msg) = self.messages.iter_mut().rev().find(|m| {
            m.variant.card_kind() == Some(TOOL_CONFIRM_CARD)
                && ToolConfirmCardData::from_json(&m.content).is_some_and(|d| d.call_id == call_id)
        }) {
            msg.content = json;
        }
    }

    fn find_acp_permission_card(&self, request_id: &str) -> Option<AcpPermissionCardData> {
        self.messages.iter().find_map(|message| {
            if !matches!(message.variant, MessageVariant::Card { ref kind } if kind == ACP_PERMISSION_CARD)
            {
                return None;
            }
            AcpPermissionCardData::from_json(&message.content)
                .filter(|data| data.request_id == request_id)
        })
    }

    fn replace_acp_permission_card(&mut self, request_id: &str, data: AcpPermissionCardData) {
        if let Some(message) = self.messages.iter_mut().find(|message| {
            matches!(message.variant, MessageVariant::Card { ref kind } if kind == ACP_PERMISSION_CARD)
                && AcpPermissionCardData::from_json(&message.content)
                    .is_some_and(|current| current.request_id == request_id)
        }) {
            message.content = data.to_json();
        }
    }

    // ===== 子代理卡片 =====

    fn push_delegate_task_from_history(&mut self, call: &agent_runtime::ToolCall) -> bool {
        if call.tool_name.as_str() != DELEGATE_TASK_TOOL {
            return false;
        }
        let Some((name, task)) = delegate_task_args(&call.arguments) else {
            return false;
        };
        self.push_subagent(call.call_id.as_str(), &name, &task);
        true
    }

    fn finish_delegate_task_from_history(&mut self, obs: &ToolObservation) -> bool {
        if obs.tool_name.as_str() != DELEGATE_TASK_TOOL {
            return false;
        }
        let subagent_id = obs.call_id.as_str();
        if self.find_subagent(subagent_id).is_none() {
            return false;
        }
        let summary = history_subagent_summary(obs);
        self.finish_subagent(subagent_id, obs.success, &summary);
        true
    }

    fn push_subagent(&mut self, subagent_id: &str, name: &str, task: &str) {
        self.finish_active_status();
        self.close_streaming_segment();
        let mut data = SubAgentCardData {
            subagent_id: subagent_id.to_string(),
            name: name.to_string(),
            task: task.to_string(),
            running: true,
            success: None,
            summary: String::new(),
        };
        bound_subagent_data(&mut data, self.card_field_limit());
        self.upsert_active_subagent(data.clone());
        self.messages
            .push(ChatMessageUI::card(SUBAGENT_CARD, data.to_json()));
    }

    fn update_subagent(&mut self, subagent_id: &str, summary: &str) {
        if let Some(mut data) = self.find_subagent(subagent_id) {
            data.summary = summary.to_string();
            self.upsert_active_subagent(data.clone());
            self.replace_subagent_card(subagent_id, data);
        }
    }

    fn finish_subagent(&mut self, subagent_id: &str, success: bool, summary: &str) {
        if let Some(mut data) = self.find_subagent(subagent_id) {
            data.running = false;
            data.success = Some(success);
            if !summary.is_empty() {
                data.summary = summary.to_string();
            }
            self.upsert_active_subagent(data.clone());
            self.replace_subagent_card(subagent_id, data);
        }
    }

    fn upsert_active_subagent(&mut self, data: SubAgentCardData) {
        let mut data = data;
        bound_subagent_data(&mut data, self.card_field_limit());
        if let Some(existing) = self
            .active_subagents
            .iter_mut()
            .find(|item| item.subagent_id == data.subagent_id)
        {
            *existing = data;
        } else {
            self.active_subagents.push(data);
        }
        while self.active_subagents.len() > MAX_ACTIVE_SUBAGENTS {
            let index = self
                .active_subagents
                .iter()
                .position(|item| !item.running)
                .unwrap_or(0);
            self.active_subagents.remove(index);
        }
    }

    fn find_subagent(&self, subagent_id: &str) -> Option<SubAgentCardData> {
        self.find_subagent_card(subagent_id).or_else(|| {
            self.active_subagents
                .iter()
                .find(|item| item.subagent_id == subagent_id)
                .cloned()
        })
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

    fn enforce_budget(&mut self) {
        for message in &mut self.messages {
            truncate_message_fields(message, self.budget.max_field_bytes);
        }

        loop {
            let over_message_budget = self.messages.len() > self.budget.max_messages;
            let over_text_budget =
                transcript_text_bytes(&self.messages) > self.budget.max_text_bytes;
            if !over_message_budget && !over_text_budget {
                break;
            }

            let streaming_id = self.streaming_id.as_deref();
            let active_status_id = self.active_status_id.as_deref();
            let acp_status_id = self.acp_status_id.as_deref();
            let index = self
                .messages
                .iter()
                .position(|message| {
                    !message_is_protected(message, streaming_id, active_status_id, acp_status_id)
                })
                // 正常优先淘汰已完成消息；若事务态本身填满预算，则退化为淘汰
                // 最旧保护项，保证异常事件洪峰下仍有真正的硬上限。后续 observation、
                // streaming delta 和子代理事件会通过各自的缺失状态防御逻辑恢复可渲染状态。
                .unwrap_or(0);
            self.remove_message(index);
        }
    }

    fn card_field_limit(&self) -> usize {
        self.budget.max_field_bytes.min(MAX_CARD_FIELD_BYTES)
    }

    fn cache_tool_input(&mut self, call_id: &str, arguments: &serde_json::Value) {
        let input_bytes = serde_json::to_vec(arguments)
            .map(|value| value.len())
            .unwrap_or(usize::MAX);
        self.remove_tool_input(call_id);
        if input_bytes > MAX_CACHED_TOOL_INPUT_BYTES {
            return;
        }

        let call_id = call_id.to_string();
        self.tool_inputs.insert(call_id.clone(), arguments.clone());
        self.tool_input_order.push_back(call_id);
        while self.tool_input_order.len() > MAX_CACHED_TOOL_INPUTS {
            if let Some(oldest) = self.tool_input_order.pop_front() {
                self.tool_inputs.remove(&oldest);
            }
        }
    }

    fn remove_tool_input(&mut self, call_id: &str) {
        self.tool_inputs.remove(call_id);
        self.tool_input_order.retain(|item| item != call_id);
    }

    fn record_terminal_event(&mut self, key: TerminalEventKey) -> bool {
        if !self.terminal_events.insert(key.clone()) {
            return false;
        }
        self.terminal_event_order.push_back(key);
        while self.terminal_event_order.len() > MAX_TERMINAL_EVENTS {
            if let Some(oldest) = self.terminal_event_order.pop_front() {
                self.terminal_events.remove(&oldest);
            }
        }
        true
    }

    fn remove_message(&mut self, index: usize) {
        let removed = self.messages.remove(index);
        if self.streaming_id.as_deref() == Some(removed.id.as_str()) {
            self.streaming_id = None;
        }
        if self.active_status_id.as_deref() == Some(removed.id.as_str()) {
            self.active_status_id = None;
        }
        if self.acp_status_id.as_deref() == Some(removed.id.as_str()) {
            self.acp_status_id = None;
        }
        if removed.variant.card_kind() == Some(TOOL_CARD)
            && let Some(data) = ToolCardData::from_json(&removed.content)
        {
            self.remove_tool_input(&data.call_id);
        }
    }
}

fn transcript_text_bytes(messages: &[ChatMessageUI]) -> usize {
    messages.iter().fold(0, |total, message| {
        total.saturating_add(message_text_bytes(message))
    })
}

fn message_text_bytes(message: &ChatMessageUI) -> usize {
    let status_bytes = match &message.variant {
        MessageVariant::Status { title, .. } => title.len(),
        _ => 0,
    };
    message
        .content
        .len()
        .saturating_add(message.reasoning_content.len())
        .saturating_add(status_bytes)
}

fn truncate_message_fields(message: &mut ChatMessageUI, max_bytes: usize) {
    if matches!(message.variant, MessageVariant::Text) {
        truncate_to_recent(&mut message.content, max_bytes);
    }
    truncate_to_recent(&mut message.reasoning_content, max_bytes);
    if let MessageVariant::Status { title, .. } = &mut message.variant {
        truncate_to_recent(title, max_bytes);
    }
    truncate_card_fields(message, max_bytes.min(MAX_CARD_FIELD_BYTES));
}

fn truncate_card_fields(message: &mut ChatMessageUI, max_bytes: usize) {
    let Some(kind) = message.variant.card_kind() else {
        return;
    };
    match kind {
        TOOL_CARD => {
            if let Some(mut data) = ToolCardData::from_json(&message.content) {
                bound_tool_card_data(&mut data, max_bytes);
                message.content = data.to_json();
            }
        }
        TOOL_CONFIRM_CARD => {
            if let Some(mut data) = ToolConfirmCardData::from_json(&message.content) {
                bound_tool_confirm_data(&mut data, max_bytes);
                message.content = data.to_json();
            }
        }
        ACP_PERMISSION_CARD => {
            if let Some(mut data) = AcpPermissionCardData::from_json(&message.content) {
                bound_acp_permission_data(&mut data, max_bytes);
                message.content = data.to_json();
            }
        }
        SUBAGENT_CARD => {
            if let Some(mut data) = SubAgentCardData::from_json(&message.content) {
                bound_subagent_data(&mut data, max_bytes);
                message.content = data.to_json();
            }
        }
        _ => {}
    }
}

fn bound_tool_card_data(data: &mut ToolCardData, max_bytes: usize) {
    truncate_to_recent(&mut data.tool_name, max_bytes);
    if let Some(target_id) = &mut data.target_id {
        truncate_to_recent(target_id, max_bytes);
    }
    if let Some(target_label) = &mut data.target_label {
        truncate_to_recent(target_label, max_bytes);
    }
    truncate_to_recent(&mut data.input_summary, max_bytes);
    truncate_to_recent(&mut data.input_json, max_bytes);
    truncate_to_recent(&mut data.summary, max_bytes);
    truncate_to_recent(&mut data.data_text, max_bytes);
}

fn bound_tool_confirm_data(data: &mut ToolConfirmCardData, max_bytes: usize) {
    truncate_to_recent(&mut data.tool_name, max_bytes);
    truncate_to_recent(&mut data.input_summary, max_bytes);
    truncate_to_recent(&mut data.input_json, max_bytes);
    truncate_to_recent(&mut data.question, max_bytes);
    data.items.truncate(MAX_CARD_COLLECTION_ITEMS);
    let item_max_bytes = max_bytes.min(MAX_CARD_ITEM_FIELD_BYTES);
    for item in &mut data.items {
        truncate_to_recent(&mut item.tool_name, item_max_bytes);
        truncate_to_recent(&mut item.input_summary, item_max_bytes);
        truncate_to_recent(&mut item.input_json, item_max_bytes);
    }
}

fn bound_acp_permission_data(data: &mut AcpPermissionCardData, max_bytes: usize) {
    truncate_to_recent(&mut data.tool_name, max_bytes);
    truncate_to_recent(&mut data.summary, max_bytes);
    truncate_to_recent(&mut data.details_json, max_bytes);
    truncate_to_recent(&mut data.selected_option_name, max_bytes);
    data.options.truncate(MAX_CARD_COLLECTION_ITEMS);
    let item_max_bytes = max_bytes.min(MAX_CARD_ITEM_FIELD_BYTES);
    for option in &mut data.options {
        truncate_to_recent(&mut option.name, item_max_bytes);
    }
}

fn bound_subagent_data(data: &mut SubAgentCardData, max_bytes: usize) {
    truncate_to_recent(&mut data.name, max_bytes);
    truncate_to_recent(&mut data.task, max_bytes);
    truncate_to_recent(&mut data.summary, max_bytes);
}

fn message_is_protected(
    message: &ChatMessageUI,
    streaming_id: Option<&str>,
    active_status_id: Option<&str>,
    acp_status_id: Option<&str>,
) -> bool {
    if streaming_id == Some(message.id.as_str())
        || active_status_id == Some(message.id.as_str())
        || acp_status_id == Some(message.id.as_str())
        || message.is_streaming
    {
        return true;
    }

    match message.variant.card_kind() {
        Some(TOOL_CARD) => {
            ToolCardData::from_json(&message.content).is_some_and(|data| data.running)
        }
        Some(TOOL_CONFIRM_CARD) => ToolConfirmCardData::from_json(&message.content)
            .is_some_and(|data| data.status == "pending"),
        Some(ACP_PERMISSION_CARD) => AcpPermissionCardData::from_json(&message.content)
            .is_some_and(|data| data.status == "pending"),
        Some(SUBAGENT_CARD) => {
            SubAgentCardData::from_json(&message.content).is_some_and(|data| data.running)
        }
        _ => matches!(
            message.variant,
            MessageVariant::Status { is_done: false, .. }
        ),
    }
}

fn append_bounded(target: &mut String, delta: &str, max_bytes: usize) {
    if target.len().saturating_add(delta.len()) <= max_bytes {
        target.push_str(delta);
        return;
    }
    if max_bytes <= TRANSCRIPT_TRUNCATION_MARKER.len() {
        *target = utf8_prefix(TRANSCRIPT_TRUNCATION_MARKER, max_bytes).to_string();
        return;
    }

    let tail_budget = max_bytes - TRANSCRIPT_TRUNCATION_MARKER.len();
    let existing = target
        .strip_prefix(TRANSCRIPT_TRUNCATION_MARKER)
        .unwrap_or(target);
    let mut tail = String::with_capacity(tail_budget);
    if delta.len() >= tail_budget {
        tail.push_str(utf8_suffix(delta, tail_budget));
    } else {
        tail.push_str(utf8_suffix(existing, tail_budget - delta.len()));
        tail.push_str(delta);
    }
    *target = format!("{TRANSCRIPT_TRUNCATION_MARKER}{tail}");
}

fn truncate_to_recent(target: &mut String, max_bytes: usize) {
    if target.len() <= max_bytes {
        return;
    }
    if max_bytes <= TRANSCRIPT_TRUNCATION_MARKER.len() {
        *target = utf8_prefix(TRANSCRIPT_TRUNCATION_MARKER, max_bytes).to_string();
        return;
    }
    let tail_budget = max_bytes - TRANSCRIPT_TRUNCATION_MARKER.len();
    let source = target
        .strip_prefix(TRANSCRIPT_TRUNCATION_MARKER)
        .unwrap_or(target);
    *target = format!(
        "{TRANSCRIPT_TRUNCATION_MARKER}{}",
        utf8_suffix(source, tail_budget)
    );
}

fn utf8_prefix(text: &str, max_bytes: usize) -> &str {
    let mut end = max_bytes.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn utf8_suffix(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut start = text.len() - max_bytes;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

fn terminal_event_key(event: &RuntimeEvent) -> Option<TerminalEventKey> {
    match event {
        RuntimeEvent::NeedUserInput {
            turn_id,
            pending_tool_call_id,
            question,
            ..
        } => Some(TerminalEventKey::AwaitingInput {
            turn_id: turn_id.clone(),
            call_id: pending_tool_call_id.clone(),
            question_hash: stable_hash(question),
        }),
        RuntimeEvent::TurnCompleted { turn_id, .. }
        | RuntimeEvent::TurnCancelled { turn_id, .. }
        | RuntimeEvent::TurnFailed { turn_id, .. } => {
            Some(TerminalEventKey::Final(turn_id.clone()))
        }
        _ => None,
    }
}

fn stable_hash(value: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

// ===== 枚举 → 卡片字符串 =====

fn delegate_task_args(arguments: &serde_json::Value) -> Option<(String, String)> {
    let name = arguments.get("name")?.as_str()?.trim();
    let task = arguments.get("task")?.as_str()?.trim();
    if name.is_empty() || task.is_empty() {
        return None;
    }
    Some((name.to_string(), task.to_string()))
}

fn history_subagent_summary(obs: &ToolObservation) -> String {
    let data_text = obs.data.to_text();
    if data_text.trim().is_empty() {
        obs.summary.clone()
    } else {
        data_text
    }
}

fn max_data_chars_for_tool(obs: &ToolObservation) -> usize {
    if is_terminal_exec_tool(obs.tool_name.as_str()) {
        MAX_TERMINAL_EXEC_DATA_CHARS
    } else {
        MAX_DATA_CHARS
    }
}

fn is_terminal_exec_tool(tool_name: &str) -> bool {
    matches!(tool_name, "terminal_exec" | "terminal.exec")
}

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
    use agent_runtime::tools::{ObservationData, ToolCall, ToolName};
    use agent_runtime::{
        PendingToolCallSummary, PlanSource, PlanStep, ResourceContext, ResourceId, ResourceKind,
        ResourceRef, SessionId,
    };

    fn sid() -> SessionId {
        SessionId::from_string("s1")
    }
    fn tid() -> TurnId {
        TurnId::from_string("t1")
    }

    #[test]
    fn transcript_evicts_oldest_completed_messages_over_count_budget() {
        let mut tr = AgentTranscript::with_limits(3, usize::MAX, usize::MAX);

        tr.push_system("first");
        tr.push_system("second");
        tr.push_system("third");
        tr.push_system("fourth");

        assert_eq!(3, tr.messages.len());
        assert_eq!(
            vec!["second", "third", "fourth"],
            tr.messages
                .iter()
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn transcript_preserves_pending_cards_and_status_when_evicting() {
        let mut tr = AgentTranscript::with_limits(3, usize::MAX, usize::MAX);
        let call_id = ToolCallId::from_string("call_pending");

        tr.push_system("completed");
        tr.apply(&RuntimeEvent::ToolCallStarted {
            session_id: sid(),
            turn_id: tid(),
            call_id: call_id.clone(),
            tool_name: ToolName::new("terminal_exec"),
            arguments: serde_json::json!({"command": "sleep 10"}),
        });
        tr.apply(&RuntimeEvent::NeedUserInput {
            session_id: sid(),
            turn_id: tid(),
            question: "确认继续吗？".into(),
            pending_tool_call_id: Some(call_id),
            tool_name: Some(ToolName::new("terminal_exec")),
            arguments: Some(serde_json::json!({"command": "sleep 10"})),
            pending_tool_calls: Vec::new(),
        });
        tr.set_acp_status("ACP 正在响应…");
        tr.push_system("also completed");

        assert_eq!(3, tr.messages.len());
        assert!(tr.messages.iter().any(|message| {
            message.variant.card_kind() == Some(TOOL_CARD)
                && ToolCardData::from_json(&message.content).is_some_and(|data| data.running)
        }));
        assert!(tr.messages.iter().any(|message| {
            message.variant.card_kind() == Some(TOOL_CONFIRM_CARD)
                && ToolConfirmCardData::from_json(&message.content)
                    .is_some_and(|data| data.status == "pending")
        }));
        assert!(tr.messages.iter().any(|message| {
            matches!(
                message.variant,
                MessageVariant::Status { is_done: false, .. }
            )
        }));
        assert!(
            !tr.messages
                .iter()
                .any(|message| message.content == "completed")
        );
        assert!(
            !tr.messages
                .iter()
                .any(|message| message.content == "also completed")
        );
    }

    #[test]
    fn load_history_applies_budget_without_breaking_tool_pairing() {
        let mut tr = AgentTranscript::with_limits(2, usize::MAX, usize::MAX);
        let call_id = ToolCallId::from_string("history_call");
        let call = ToolCall::new(
            ToolName::new("echo"),
            serde_json::json!({"text": "history"}),
        )
        .with_call_id(call_id.clone());
        let observation = ToolObservation::success(
            call_id,
            ToolName::new("echo"),
            "done",
            ObservationData::Text("history".into()),
        );

        tr.load_history(
            &[
                HistoryItem::User {
                    text: "first".into(),
                    images: Vec::new(),
                },
                HistoryItem::User {
                    text: "second".into(),
                    images: Vec::new(),
                },
                HistoryItem::ToolCall(call),
                HistoryItem::Observation(observation),
            ],
            None,
        );

        assert_eq!(2, tr.messages.len());
        let card = tr
            .messages
            .iter()
            .find(|message| message.variant.card_kind() == Some(TOOL_CARD))
            .expect("completed tool card should remain paired");
        let data = ToolCardData::from_json(&card.content).unwrap();
        assert!(!data.running);
        assert_eq!(Some(true), data.success);
        assert_eq!("done", data.summary);
        assert_eq!("history", data.data_text);
    }

    #[test]
    fn streaming_content_is_bounded_at_utf8_boundaries() {
        let mut tr = AgentTranscript::with_limits(10, 256, 64);

        tr.apply(&RuntimeEvent::AssistantMessageDelta {
            session_id: sid(),
            turn_id: tid(),
            delta: "开头".repeat(20),
        });
        tr.apply(&RuntimeEvent::AssistantMessageDelta {
            session_id: sid(),
            turn_id: tid(),
            delta: "最新🙂内容".repeat(20),
        });

        assert_eq!(1, tr.messages.len());
        assert!(tr.messages[0].is_streaming);
        assert!(tr.messages[0].content.len() <= 64);
        assert!(
            tr.messages[0]
                .content
                .starts_with(TRANSCRIPT_TRUNCATION_MARKER)
        );
        assert!(tr.messages[0].content.ends_with("内容"));
    }

    #[test]
    fn reasoning_content_is_bounded_separately_from_answer() {
        let mut tr = AgentTranscript::with_limits(10, 256, 64);

        tr.apply(&RuntimeEvent::ReasoningDelta {
            session_id: sid(),
            turn_id: tid(),
            delta: "推理🙂".repeat(40),
        });
        tr.apply(&RuntimeEvent::AssistantMessageDelta {
            session_id: sid(),
            turn_id: tid(),
            delta: "最终回答".into(),
        });

        assert_eq!(1, tr.messages.len());
        assert!(tr.messages[0].reasoning_content.len() <= 64);
        assert!(
            tr.messages[0]
                .reasoning_content
                .starts_with(TRANSCRIPT_TRUNCATION_MARKER)
        );
        assert_eq!("最终回答", tr.messages[0].content);
    }

    #[test]
    fn transcript_text_budget_evicts_oldest_completed_messages() {
        let mut tr = AgentTranscript::with_limits(10, 16, 64);

        tr.push_system("12345678");
        tr.push_system("abcdefgh");
        tr.push_system("ABCDEFGH");

        assert_eq!(2, tr.messages.len());
        assert_eq!("abcdefgh", tr.messages[0].content);
        assert_eq!("ABCDEFGH", tr.messages[1].content);
    }

    #[test]
    fn protected_messages_use_hard_count_fallback() {
        let mut tr = AgentTranscript::with_limits(1, usize::MAX, usize::MAX);

        for index in 0..2 {
            tr.apply(&RuntimeEvent::ToolCallStarted {
                session_id: sid(),
                turn_id: tid(),
                call_id: ToolCallId::from_string(format!("call_{index}")),
                tool_name: ToolName::new("terminal_exec"),
                arguments: serde_json::json!({"command": format!("sleep {index}")}),
            });
        }

        assert_eq!(1, tr.messages.len());
        assert!(tr.tool_call_arguments("call_0").is_none());
        assert!(tr.tool_call_arguments("call_1").is_some());
    }

    #[test]
    fn hard_evicted_tool_call_is_recreated_by_later_observation() {
        let mut tr = AgentTranscript::with_limits(1, usize::MAX, usize::MAX);
        let call_0 = ToolCallId::from_string("call_0");
        let call_1 = ToolCallId::from_string("call_1");

        for call_id in [&call_0, &call_1] {
            tr.apply(&RuntimeEvent::ToolCallStarted {
                session_id: sid(),
                turn_id: tid(),
                call_id: call_id.clone(),
                tool_name: ToolName::new("echo"),
                arguments: serde_json::json!({"text": call_id.as_str()}),
            });
        }
        assert!(tr.find_tool_card(call_0.as_str()).is_none());

        tr.apply(&RuntimeEvent::ToolCallFinished {
            session_id: sid(),
            turn_id: tid(),
            call_id: call_1,
            success: true,
        });
        tr.apply(&RuntimeEvent::ObservationAdded {
            session_id: sid(),
            turn_id: tid(),
            observation: ToolObservation::success(
                call_0.clone(),
                ToolName::new("echo"),
                "recovered",
                ObservationData::Text("late output".into()),
            ),
        });
        tr.apply(&RuntimeEvent::ToolCallFinished {
            session_id: sid(),
            turn_id: tid(),
            call_id: call_0.clone(),
            success: true,
        });

        let data = tr
            .find_tool_card(call_0.as_str())
            .expect("late observation should recreate a completed card");
        assert!(!data.running);
        assert_eq!(Some(true), data.success);
        assert_eq!("recovered", data.summary);
        assert_eq!("late output", data.data_text);
    }

    #[test]
    fn terminal_event_dedup_history_is_bounded() {
        let mut tr = AgentTranscript::new();

        for index in 0..=MAX_TERMINAL_EVENTS {
            assert!(
                tr.record_terminal_event(TerminalEventKey::Final(TurnId::from_string(format!(
                    "turn_{index}"
                ))))
            );
        }

        assert_eq!(MAX_TERMINAL_EVENTS, tr.terminal_events.len());
        assert!(tr.record_terminal_event(TerminalEventKey::Final(TurnId::from_string("turn_0"))));
    }

    #[test]
    fn acp_failure_dedup_history_is_bounded() {
        let mut tr = AgentTranscript::new();
        let error = crate::AcpError::empty_response("agent", "Agent");

        for index in 0..=MAX_TERMINAL_EVENTS {
            assert!(tr.apply_acp_failure(
                &RuntimeEvent::TurnFailed {
                    session_id: sid(),
                    turn_id: TurnId::from_string(format!("acp_turn_{index}")),
                    reason: "failed".into(),
                },
                &error,
            ));
        }

        assert_eq!(MAX_TERMINAL_EVENTS, tr.terminal_events.len());
        assert!(tr.apply_acp_failure(
            &RuntimeEvent::TurnFailed {
                session_id: sid(),
                turn_id: TurnId::from_string("acp_turn_0"),
                reason: "failed again".into(),
            },
            &error,
        ));
    }

    #[test]
    fn completed_subagent_history_and_fields_are_bounded() {
        let mut tr = AgentTranscript::new();

        for index in 0..=MAX_ACTIVE_SUBAGENTS {
            let id = format!("sub_{index}");
            tr.push_subagent(&id, &"n".repeat(MAX_CARD_FIELD_BYTES + 1), "task");
            tr.finish_subagent(&id, true, "done");
        }

        assert_eq!(MAX_ACTIVE_SUBAGENTS, tr.active_subagents().len());
        assert_eq!("sub_1", tr.active_subagents()[0].subagent_id);
        assert!(tr.active_subagents()[0].name.len() <= MAX_CARD_FIELD_BYTES);
    }

    #[test]
    fn hard_evicted_subagent_card_still_updates_active_panel() {
        let mut tr = AgentTranscript::with_limits(1, usize::MAX, usize::MAX);
        let subagent_0 = SubAgentId::from_string("sub_0");
        let subagent_1 = SubAgentId::from_string("sub_1");

        for subagent_id in [&subagent_0, &subagent_1] {
            tr.apply(&RuntimeEvent::SubAgentStarted {
                session_id: sid(),
                turn_id: tid(),
                subagent_id: subagent_id.clone(),
                name: subagent_id.as_str().to_string(),
                task: "inspect".into(),
            });
        }
        assert!(tr.find_subagent_card(subagent_0.as_str()).is_none());

        tr.apply(&RuntimeEvent::SubAgentFinished {
            session_id: sid(),
            turn_id: tid(),
            subagent_id: subagent_0.clone(),
            success: true,
            summary: "finished after eviction".into(),
        });

        let data = tr
            .active_subagents()
            .iter()
            .find(|data| data.subagent_id == subagent_0.as_str())
            .expect("active panel should retain the evicted subagent state");
        assert!(!data.running);
        assert_eq!(Some(true), data.success);
        assert_eq!("finished after eviction", data.summary);
    }

    #[test]
    fn oversized_raw_tool_input_is_not_retained() {
        let mut tr = AgentTranscript::new();
        let arguments = serde_json::json!({
            "command": "x".repeat(MAX_CACHED_TOOL_INPUT_BYTES + 1)
        });

        tr.push_tool_call("oversized", "terminal_exec", &arguments);

        assert!(tr.tool_call_arguments("oversized").is_none());
        let card = tr.find_tool_card("oversized").expect("tool card");
        assert!(card.input_json.len() <= MAX_CARD_FIELD_BYTES);
    }

    #[test]
    fn acp_permission_card_bounds_structured_fields_and_options() {
        let mut tr = AgentTranscript::new();
        let request = AcpPermissionRequest {
            request_id: "request".into(),
            session_id: "session".into(),
            tool_call_id: "call".into(),
            tool_name: "tool".into(),
            summary: "s".repeat(MAX_CARD_FIELD_BYTES + 1),
            details: serde_json::json!({
                "rawInput": "d".repeat(MAX_CARD_FIELD_BYTES + 1)
            }),
            options: (0..=MAX_CARD_COLLECTION_ITEMS)
                .map(|index| AcpPermissionOption {
                    option_id: format!("option_{index}"),
                    name: "n".repeat(MAX_CARD_ITEM_FIELD_BYTES + 1),
                    kind: "allow_once".into(),
                })
                .collect(),
        };

        tr.push_acp_permission(&request, false);

        let card = tr
            .find_acp_permission_card("request")
            .expect("permission card");
        assert!(card.summary.len() <= MAX_CARD_FIELD_BYTES);
        assert!(card.details_json.len() <= MAX_CARD_FIELD_BYTES);
        assert_eq!(MAX_CARD_COLLECTION_ITEMS, card.options.len());
        assert!(
            card.options
                .iter()
                .all(|option| option.name.len() <= MAX_CARD_ITEM_FIELD_BYTES)
        );
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
    fn pending_status_is_visible_until_assistant_output_arrives() {
        let mut tr = AgentTranscript::new();
        tr.apply(&RuntimeEvent::Status {
            session_id: sid(),
            turn_id: tid(),
            title: "ACP 正在响应…".to_string(),
            is_done: false,
        });

        assert_eq!(1, tr.messages.len());
        assert!(matches!(
            &tr.messages[0].variant,
            MessageVariant::Status { title, is_done: false } if title == "ACP 正在响应…"
        ));

        tr.apply(&RuntimeEvent::AssistantMessageDelta {
            session_id: sid(),
            turn_id: tid(),
            delta: "回答".to_string(),
        });

        assert_eq!(1, tr.messages.len());
        assert_eq!("回答", tr.messages[0].content);
        assert!(matches!(tr.messages[0].variant, MessageVariant::Text));
    }

    #[test]
    fn acp_phase_status_replaces_previous_phase() {
        let mut transcript = AgentTranscript::new();

        transcript.set_acp_status("正在启动 Codex");
        transcript.set_acp_status("正在协商 ACP 协议");

        assert_eq!(1, transcript.acp_status_count());
        assert_eq!(Some("正在协商 ACP 协议"), transcript.acp_status_text());
    }

    #[test]
    fn empty_response_replaces_running_status_with_recovery_error() {
        let mut transcript = AgentTranscript::new();
        transcript.set_acp_status("ACP 正在响应…");

        transcript.set_acp_error(&crate::AcpError::empty_response("opencode", "OpenCode"));

        assert_eq!(0, transcript.pending_status_count());
        assert!(transcript.last_message_content().is_some_and(|content| {
            content.contains(t!("AgentUi.acp_empty_response_summary").as_ref())
        }));
    }

    #[test]
    fn pending_status_is_removed_when_turn_completes_without_output() {
        let mut tr = AgentTranscript::new();
        tr.apply(&RuntimeEvent::Status {
            session_id: sid(),
            turn_id: tid(),
            title: "ACP 正在响应…".to_string(),
            is_done: false,
        });
        tr.apply(&RuntimeEvent::TurnCompleted {
            session_id: sid(),
            turn_id: tid(),
            answer: None,
        });

        assert!(tr.messages.is_empty());
    }

    #[test]
    fn cancelled_event_closes_stream_and_is_idempotent() {
        let mut tr = AgentTranscript::new();
        tr.apply(&RuntimeEvent::AssistantMessageDelta {
            session_id: sid(),
            turn_id: tid(),
            delta: "部分回答".to_string(),
        });
        let event = RuntimeEvent::TurnCancelled {
            session_id: sid(),
            turn_id: tid(),
        };

        assert!(tr.apply(&event));
        assert!(!tr.apply(&event));

        assert_eq!(2, tr.messages.len());
        assert_eq!("部分回答", tr.messages[0].content);
        assert!(!tr.messages[0].is_streaming);
        assert_eq!(t!("AgentUi.task_cancelled"), tr.messages[1].content);
        assert!(!tr.messages[1].content.contains("失败"));
    }

    #[test]
    fn load_history_preserves_tool_input_display() {
        let mut tr = AgentTranscript::new();
        let call = ToolCall::new(
            ToolName::new("exec_command"),
            serde_json::json!({
                "command": "rtk cargo check -p ai_chat_view",
                "token": "secret"
            }),
        );

        tr.load_history(&[HistoryItem::ToolCall(call)], None);

        assert_eq!(1, tr.messages.len());
        let data = ToolCardData::from_json(&tr.messages[0].content).unwrap();
        assert_eq!("rtk cargo check -p ai_chat_view", data.input_summary);
        assert!(data.input_json.contains("rtk cargo check -p ai_chat_view"));
        assert!(data.input_json.contains("\"token\": \"***\""));
        assert!(!data.input_json.contains("secret"));
    }

    #[test]
    fn load_history_replays_delegate_task_as_subagent_card() {
        let mut tr = AgentTranscript::new();
        let call_id = ToolCallId::from_string("call_delegate");
        let call = ToolCall::new(
            ToolName::new("delegate_task"),
            serde_json::json!({
                "name": "reviewer",
                "task": "检查历史回放是否保留子代理卡片"
            }),
        )
        .with_call_id(call_id.clone());
        let observation = ToolObservation::success(
            call_id,
            ToolName::new("delegate_task"),
            "子代理 reviewer 完成",
            ObservationData::Text("历史回放应显示完成态子代理".into()),
        );

        tr.load_history(
            &[
                HistoryItem::ToolCall(call),
                HistoryItem::Observation(observation),
            ],
            None,
        );

        assert_eq!(1, tr.messages.len());
        assert_eq!(Some(SUBAGENT_CARD), tr.messages[0].variant.card_kind());
        let data = SubAgentCardData::from_json(&tr.messages[0].content).unwrap();
        assert_eq!("call_delegate", data.subagent_id);
        assert_eq!("reviewer", data.name);
        assert_eq!("检查历史回放是否保留子代理卡片", data.task);
        assert!(!data.running);
        assert_eq!(Some(true), data.success);
        assert_eq!("历史回放应显示完成态子代理", data.summary);
        assert_eq!(1, tr.active_subagents().len());
        assert_eq!("reviewer", tr.active_subagents()[0].name);
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
            arguments: serde_json::json!({"text": "hi"}),
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
            arguments: serde_json::json!({"text": "hi"}),
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
        assert_eq!(data.input_summary, "hi");
        assert!(data.input_json.contains("\"text\": \"hi\""));
    }

    #[test]
    fn terminal_exec_observation_keeps_long_output_for_card_display() {
        let mut tr = AgentTranscript::new();
        let call = ToolCallId::from_string("call_terminal");
        tr.apply(&RuntimeEvent::ToolCallStarted {
            session_id: sid(),
            turn_id: tid(),
            call_id: call.clone(),
            tool_name: ToolName::new("terminal_exec"),
            arguments: serde_json::json!({"command": "systemctl list-units"}),
        });
        let output = "x".repeat(MAX_DATA_CHARS + 100);
        let obs = ToolObservation::success(
            call,
            ToolName::new("terminal_exec"),
            "ok",
            ObservationData::Json(serde_json::json!({ "output": output })),
        );

        tr.apply(&RuntimeEvent::ObservationAdded {
            session_id: sid(),
            turn_id: tid(),
            observation: obs,
        });

        let data = ToolCardData::from_json(&tr.messages[0].content).unwrap();
        assert!(
            data.data_text.len() > MAX_DATA_CHARS,
            "terminal output should not be truncated at the generic card limit"
        );
    }

    #[test]
    fn non_terminal_observation_still_uses_generic_card_limit() {
        let mut tr = AgentTranscript::new();
        let call = ToolCallId::from_string("call_echo");
        tr.apply(&RuntimeEvent::ToolCallStarted {
            session_id: sid(),
            turn_id: tid(),
            call_id: call.clone(),
            tool_name: ToolName::new("echo"),
            arguments: serde_json::json!({"text": "hi"}),
        });
        let obs = ToolObservation::success(
            call,
            ToolName::new("echo"),
            "ok",
            ObservationData::Text("x".repeat(MAX_DATA_CHARS + 100)),
        );

        tr.apply(&RuntimeEvent::ObservationAdded {
            session_id: sid(),
            turn_id: tid(),
            observation: obs,
        });

        let data = ToolCardData::from_json(&tr.messages[0].content).unwrap();
        assert_eq!(MAX_DATA_CHARS, data.data_text.len());
    }

    #[test]
    fn tool_observation_records_target_resource_label_on_card() {
        let mut tr = AgentTranscript::new();
        tr.set_resource_context(
            &ResourceContext::new()
                .with_resource(ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"))
                .with_resource(ResourceRef::new("ssh-b", ResourceKind::Ssh, "prod-b")),
        );
        let call = ToolCallId::from_string("call_target");
        tr.apply(&RuntimeEvent::ToolCallStarted {
            session_id: sid(),
            turn_id: tid(),
            call_id: call.clone(),
            tool_name: ToolName::new("ssh.exec"),
            arguments: serde_json::json!({"command": "df -h"}),
        });
        let obs = ToolObservation::success(
            call,
            ToolName::new("ssh.exec"),
            "ok",
            ObservationData::Text("disk ok".into()),
        )
        .with_resource(Some(ResourceId::new("ssh-b")));

        tr.apply(&RuntimeEvent::ObservationAdded {
            session_id: sid(),
            turn_id: tid(),
            observation: obs,
        });

        let data = ToolCardData::from_json(&tr.messages[0].content).unwrap();
        assert_eq!(data.target_id.as_deref(), Some("ssh-b"));
        assert_eq!(data.target_label.as_deref(), Some("prod-b"));
    }

    #[test]
    fn need_user_input_renders_as_tool_confirm_card() {
        let mut tr = AgentTranscript::new();

        tr.apply(&RuntimeEvent::NeedUserInput {
            session_id: sid(),
            turn_id: tid(),
            question: "确认执行工具 `db_schema` 吗?".into(),
            pending_tool_call_id: Some(ToolCallId::from_string("call_confirm")),
            tool_name: Some(ToolName::new("db_schema")),
            arguments: Some(serde_json::json!({"sql": "show tables"})),
            pending_tool_calls: Vec::new(),
        });

        assert_eq!(1, tr.messages.len());
        assert_eq!(Some(TOOL_CONFIRM_CARD), tr.messages[0].variant.card_kind());
        let data = ToolConfirmCardData::from_json(&tr.messages[0].content).unwrap();
        assert_eq!(data.call_id, "call_confirm");
        assert_eq!(data.tool_name, "db_schema");
        assert_eq!(data.input_summary, "show tables");
        assert!(data.input_json.contains("\"sql\": \"show tables\""));
        assert_eq!(data.question, "确认执行工具 `db_schema` 吗?");
        assert_eq!(data.status, "pending");
    }

    #[test]
    fn public_mcp_safety_confirmation_renders_full_arguments_and_mode_hint() {
        let mut tr = AgentTranscript::new();
        tr.push_public_mcp_approval(&AcpPublicMcpApprovalRequest {
            request_id: "public-mcp-approval-1".into(),
            tool_name: "terminal.exec".into(),
            summary: "Call Execute in terminal".into(),
            details: serde_json::json!({
                "requestArguments": {
                    "target": "haiwai comi",
                    "command": "du -xhd1 / 2>/dev/null | sort -h"
                }
            }),
        });

        assert_eq!(1, tr.messages.len());
        assert_eq!(Some(TOOL_CONFIRM_CARD), tr.messages[0].variant.card_kind());
        let data = ToolConfirmCardData::from_json(&tr.messages[0].content).unwrap();
        assert_eq!(data.call_id, "public-mcp-approval-1");
        assert_eq!(data.tool_name, "terminal.exec");
        assert_eq!(data.input_summary, "du -xhd1 / 2>/dev/null | sort -h");
        assert!(data.input_json.contains("haiwai comi"));
        assert_eq!(
            t!(
                "AgentUi.public_mcp_safety_confirmation",
                summary = "Call Execute in terminal"
            ),
            data.question
        );
    }

    #[test]
    fn acp_permission_request_renders_and_resolves_inside_message_flow() {
        use crate::acp::{AcpPermissionOption, AcpPermissionRequest};
        use crate::agent_cards::{ACP_PERMISSION_CARD, AcpPermissionCardData};

        let request = AcpPermissionRequest {
            request_id: "session:call".into(),
            session_id: "session".into(),
            tool_call_id: "call".into(),
            tool_name: "Write file".into(),
            summary: "ACP Agent 请求执行工具：Write file".into(),
            details: serde_json::json!({"path": "/tmp/a"}),
            options: vec![
                AcpPermissionOption {
                    option_id: "reject".into(),
                    name: "Reject".into(),
                    kind: "reject_once".into(),
                },
                AcpPermissionOption {
                    option_id: "allow".into(),
                    name: "Allow once".into(),
                    kind: "allow_once".into(),
                },
            ],
        };
        let mut transcript = AgentTranscript::new();

        transcript.push_acp_permission(&request, true);

        assert_eq!(1, transcript.messages.len());
        assert_eq!(
            Some(ACP_PERMISSION_CARD),
            transcript.messages[0].variant.card_kind()
        );
        assert!(transcript.has_pending_acp_permission(&request.request_id));

        transcript.resolve_acp_permission(&request.request_id, &request.options[1]);
        let data = AcpPermissionCardData::from_json(&transcript.messages[0].content).unwrap();
        assert_eq!(
            t!(
                "AgentUi.acp_safety_confirmation_notice",
                summary = request.summary
            ),
            data.summary
        );
        assert_eq!("approved", data.status);
        assert_eq!("Allow once", data.selected_option_name);
        assert!(!transcript.has_pending_acp_permission(&request.request_id));
    }

    #[test]
    fn acp_permission_card_does_not_claim_second_approval_in_auto_mode() {
        use crate::acp::{AcpPermissionOption, AcpPermissionRequest};
        use crate::agent_cards::AcpPermissionCardData;

        let request = AcpPermissionRequest {
            request_id: "session:auto-call".into(),
            session_id: "session".into(),
            tool_call_id: "auto-call".into(),
            tool_name: "Execute command".into(),
            summary: "ACP Agent 请求执行工具：Execute command".into(),
            details: serde_json::json!({"command": "pwd"}),
            options: vec![AcpPermissionOption {
                option_id: "allow".into(),
                name: "Allow once".into(),
                kind: "allow_once".into(),
            }],
        };
        let mut transcript = AgentTranscript::new();

        transcript.push_acp_permission(&request, false);

        let data = AcpPermissionCardData::from_json(&transcript.messages[0].content).unwrap();
        assert_eq!(request.summary, data.summary);
    }

    #[test]
    fn need_user_input_renders_batch_tool_confirm_items() {
        let mut tr = AgentTranscript::new();

        tr.apply(&RuntimeEvent::NeedUserInput {
            session_id: sid(),
            turn_id: tid(),
            question: "确认执行 2 个工具吗?".into(),
            pending_tool_call_id: Some(ToolCallId::from_string("call_a")),
            tool_name: Some(ToolName::new("ssh.exec")),
            arguments: Some(serde_json::json!({"command": "rm -rf /tmp/a"})),
            pending_tool_calls: vec![
                PendingToolCallSummary {
                    call_id: ToolCallId::from_string("call_a"),
                    tool_name: ToolName::new("ssh.exec"),
                    arguments: serde_json::json!({"command": "rm -rf /tmp/a"}),
                },
                PendingToolCallSummary {
                    call_id: ToolCallId::from_string("call_b"),
                    tool_name: ToolName::new("ssh.exec"),
                    arguments: serde_json::json!({"command": "rm -rf /tmp/b"}),
                },
            ],
        });

        let data = ToolConfirmCardData::from_json(&tr.messages[0].content).unwrap();
        assert_eq!("call_a", data.call_id);
        assert_eq!(2, data.items.len());
        assert_eq!("call_a", data.items[0].call_id);
        assert_eq!("ssh_exec", data.items[0].tool_name);
        assert_eq!("rm -rf /tmp/a", data.items[0].input_summary);
        assert_eq!("call_b", data.items[1].call_id);
        assert_eq!("rm -rf /tmp/b", data.items[1].input_summary);
    }

    #[test]
    fn need_user_input_falls_back_to_existing_tool_call_input() {
        let mut tr = AgentTranscript::new();
        let call_id = ToolCallId::from_string("call_confirm");

        tr.apply(&RuntimeEvent::ToolCallStarted {
            session_id: sid(),
            turn_id: tid(),
            call_id: call_id.clone(),
            tool_name: ToolName::new("db_query"),
            arguments: serde_json::json!({"sql": "select count(*) from users"}),
        });
        tr.apply(&RuntimeEvent::NeedUserInput {
            session_id: sid(),
            turn_id: tid(),
            question: "确认执行工具 `db_query` 吗?".into(),
            pending_tool_call_id: Some(call_id),
            tool_name: Some(ToolName::new("db_query")),
            arguments: None,
            pending_tool_calls: Vec::new(),
        });

        assert_eq!(2, tr.messages.len());
        assert_eq!(Some(TOOL_CONFIRM_CARD), tr.messages[1].variant.card_kind());
        let data = ToolConfirmCardData::from_json(&tr.messages[1].content).unwrap();
        assert_eq!("select count(*) from users", data.input_summary);
        assert!(
            data.input_json
                .contains("\"sql\": \"select count(*) from users\"")
        );
    }

    #[test]
    fn tool_approval_resolved_updates_confirm_card_status() {
        let mut tr = AgentTranscript::new();
        let call_id = ToolCallId::from_string("call_confirm");

        tr.apply(&RuntimeEvent::NeedUserInput {
            session_id: sid(),
            turn_id: tid(),
            question: "确认执行工具 `db_schema` 吗?".into(),
            pending_tool_call_id: Some(call_id.clone()),
            tool_name: Some(ToolName::new("db_schema")),
            arguments: Some(serde_json::json!({"sql": "show tables"})),
            pending_tool_calls: Vec::new(),
        });
        tr.apply(&RuntimeEvent::ToolApprovalResolved {
            session_id: sid(),
            turn_id: tid(),
            call_id,
            approved: true,
        });

        let data = ToolConfirmCardData::from_json(&tr.messages[0].content).unwrap();
        assert_eq!(data.status, "approved");
    }

    #[test]
    fn same_turn_accepts_distinct_tool_approval_requests() {
        let mut tr = AgentTranscript::new();
        let first_call = ToolCallId::from_string("call_first");
        let second_call = ToolCallId::from_string("call_second");

        tr.apply(&RuntimeEvent::NeedUserInput {
            session_id: sid(),
            turn_id: tid(),
            question: "确认第一条命令吗？".into(),
            pending_tool_call_id: Some(first_call.clone()),
            tool_name: Some(ToolName::new("terminal_exec")),
            arguments: Some(serde_json::json!({"command": "kill 334"})),
            pending_tool_calls: Vec::new(),
        });
        tr.apply(&RuntimeEvent::ToolApprovalResolved {
            session_id: sid(),
            turn_id: tid(),
            call_id: first_call,
            approved: true,
        });
        let second = RuntimeEvent::NeedUserInput {
            session_id: sid(),
            turn_id: tid(),
            question: "确认第二条命令吗？".into(),
            pending_tool_call_id: Some(second_call),
            tool_name: Some(ToolName::new("terminal_exec")),
            arguments: Some(serde_json::json!({
                "command": "nohup ping 127.0.0.1 > /tmp/ping.log 2>&1 &"
            })),
            pending_tool_calls: Vec::new(),
        };

        assert!(tr.apply(&second));
        assert!(!tr.apply(&second));
        assert_eq!(2, tr.messages.len());

        let first = ToolConfirmCardData::from_json(&tr.messages[0].content).unwrap();
        let second = ToolConfirmCardData::from_json(&tr.messages[1].content).unwrap();
        assert_eq!("approved", first.status);
        assert_eq!("call_second", second.call_id);
        assert_eq!("pending", second.status);
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
            arguments: serde_json::json!({"path": "/tmp/a.txt"}),
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
    fn status_is_removed_when_assistant_text_starts() {
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

        assert_eq!(tr.messages.len(), 1);
        assert_eq!(tr.messages[0].content, "开始回答");
        assert!(tr.messages[0].is_streaming);
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
        assert_eq!(tr.active_subagents().len(), 1);
        assert_eq!(tr.active_subagents()[0].summary, "正在读取事件流");

        tr.apply(&RuntimeEvent::SubAgentFinished {
            session_id: sid(),
            turn_id: tid(),
            subagent_id,
            success: true,
            summary: "发现 reasoning 未转发".into(),
        });
        assert_eq!(tr.active_subagents().len(), 1);
        assert!(!tr.active_subagents()[0].running);
        assert_eq!(tr.active_subagents()[0].success, Some(true));

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
