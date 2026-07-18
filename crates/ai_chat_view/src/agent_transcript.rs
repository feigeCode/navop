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
use std::collections::{HashMap, HashSet};

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
const DELEGATE_TASK_TOOL: &str = "delegate_task";

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum TerminalEventKey {
    AwaitingInput {
        turn_id: TurnId,
        call_id: Option<ToolCallId>,
        question: String,
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
    /// 已归约的审批/终态事件,防止重复事件写入转录或触发持久化。
    terminal_events: HashSet<TerminalEventKey>,
}

impl AgentTranscript {
    pub fn new() -> Self {
        Self::default()
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
        self.terminal_events.clear();
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
            format!(
                "{}\n\n当前已开启“安全确认（手动确认）”模式。允许本次 ACP 权限请求后，实际工具执行还需要在弹出的安全确认窗口中进行二次审批；如不需要二次审批，可将 MCP 权限模式切换为“自动执行”。",
                request.summary
            )
        } else {
            request.summary.clone()
        };
        let data = AcpPermissionCardData {
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
        self.messages
            .push(ChatMessageUI::card(ACP_PERMISSION_CARD, data.to_json()));
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
    }

    /// 将未决 ACP 权限卡更新为取消态。
    pub(crate) fn cancel_acp_permission(&mut self, request_id: &str) {
        let Some(mut data) = self.find_acp_permission_card(request_id) else {
            return;
        };
        data.status = "cancelled".into();
        data.selected_option_name.clear();
        self.replace_acp_permission_card(request_id, data);
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
                } => self.push_system(format!(
                    "上下文摘要（压缩 {original_items} 条历史）:\n{text}"
                )),
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
    pub fn apply(&mut self, event: &RuntimeEvent) -> bool {
        if let Some(key) = terminal_event_key(event)
            && !self.terminal_events.insert(key)
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
        let data = ToolConfirmCardData {
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
        self.messages
            .push(ChatMessageUI::card(TOOL_CONFIRM_CARD, data.to_json()));
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
        let data = ToolConfirmCardData {
            call_id: request.request_id.clone(),
            tool_name: request.tool_name.clone(),
            items: Vec::new(),
            input_summary: input.summary,
            input_json: input.json,
            question: format!(
                "设置中已开启“安全确认”，因此 ACP 授权后，实际工具执行仍需再次审批。\n\n如不需要二次审批，可将 MCP 权限模式修改为“自动执行”。\n\n{}",
                request.summary
            ),
            status: "pending".into(),
        };
        self.messages
            .push(ChatMessageUI::card(TOOL_CONFIRM_CARD, data.to_json()));
    }

    fn apply_terminal_event(&mut self, event: &RuntimeEvent) {
        self.finish_active_status();
        match event {
            RuntimeEvent::TurnFailed { reason, .. } => {
                self.streaming_id = None;
                self.messages
                    .push(ChatMessageUI::system(format!("⚠️ 任务失败:{reason}")));
            }
            RuntimeEvent::TurnCancelled { .. } => {
                self.close_streaming_segment();
                self.messages.push(ChatMessageUI::system("任务已取消"));
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
        self.tool_inputs
            .insert(call_id.to_string(), arguments.clone());
        let input = build_tool_input_display(tool_name, arguments);
        let data = ToolCardData {
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
        self.tool_inputs.remove(call_id);
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
        if self.find_subagent_card(subagent_id).is_none() {
            return false;
        }
        let summary = history_subagent_summary(obs);
        self.finish_subagent(subagent_id, obs.success, &summary);
        true
    }

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
        self.upsert_active_subagent(data.clone());
        self.messages
            .push(ChatMessageUI::card(SUBAGENT_CARD, data.to_json()));
    }

    fn update_subagent(&mut self, subagent_id: &str, summary: &str) {
        if let Some(mut data) = self.find_subagent_card(subagent_id) {
            data.summary = summary.to_string();
            self.upsert_active_subagent(data.clone());
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
            self.upsert_active_subagent(data.clone());
            self.replace_subagent_card(subagent_id, data);
        }
    }

    fn upsert_active_subagent(&mut self, data: SubAgentCardData) {
        if let Some(existing) = self
            .active_subagents
            .iter_mut()
            .find(|item| item.subagent_id == data.subagent_id)
        {
            *existing = data;
        } else {
            self.active_subagents.push(data);
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
            question: question.clone(),
        }),
        RuntimeEvent::TurnCompleted { turn_id, .. }
        | RuntimeEvent::TurnCancelled { turn_id, .. }
        | RuntimeEvent::TurnFailed { turn_id, .. } => {
            Some(TerminalEventKey::Final(turn_id.clone()))
        }
        _ => None,
    }
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
        assert!(
            transcript
                .last_message_content()
                .is_some_and(|content| content.contains("没有返回任何内容"))
        );
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
        assert_eq!("任务已取消", tr.messages[1].content);
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
        assert!(data.question.contains("安全确认"));
        assert!(data.question.contains("二次审批"));
        assert!(data.question.contains("自动执行"));
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
        assert!(data.summary.contains("安全确认"));
        assert!(data.summary.contains("二次审批"));
        assert!(data.summary.contains("自动执行"));
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
        assert!(!data.summary.contains("二次审批"));
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
