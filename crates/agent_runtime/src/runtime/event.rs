//! 运行时对外事件。
//!
//! Runtime 通过 `tokio::sync::broadcast` 广播事件,UI / IPC 层订阅后渲染。
//! 每个事件都带 `session_id`(以及多数带 `turn_id`)以便订阅方过滤。

use crate::ids::{SessionId, SubAgentId, ToolCallId, TurnId};
use crate::planner::Plan;
use crate::runtime::PendingToolCallSummary;
use crate::tools::{ToolName, ToolObservation};
use serde_json::Value;

/// 事件发送端(克隆给各 Session)。
pub type RuntimeEventSender = tokio::sync::broadcast::Sender<RuntimeEvent>;
/// 事件接收端(订阅者持有)。
pub type RuntimeEventReceiver = tokio::sync::broadcast::Receiver<RuntimeEvent>;

/// 运行时事件。
#[derive(Clone, Debug)]
pub enum RuntimeEvent {
    /// 一轮开始。
    TurnStarted {
        session_id: SessionId,
        turn_id: TurnId,
    },
    /// 计划被创建或更新。
    PlanUpdated {
        session_id: SessionId,
        turn_id: TurnId,
        plan: Plan,
    },
    /// 工具调用开始。
    ToolCallStarted {
        session_id: SessionId,
        turn_id: TurnId,
        call_id: ToolCallId,
        tool_name: ToolName,
        arguments: Value,
    },
    /// 工具调用结束。
    ToolCallFinished {
        session_id: SessionId,
        turn_id: TurnId,
        call_id: ToolCallId,
        success: bool,
    },
    /// 运行时或外部 ACP agent 派发了一个子代理任务。
    SubAgentStarted {
        session_id: SessionId,
        turn_id: TurnId,
        subagent_id: SubAgentId,
        name: String,
        task: String,
    },
    /// 子代理任务有新的进展摘要。
    SubAgentUpdated {
        session_id: SessionId,
        turn_id: TurnId,
        subagent_id: SubAgentId,
        summary: String,
    },
    /// 子代理任务结束。
    SubAgentFinished {
        session_id: SessionId,
        turn_id: TurnId,
        subagent_id: SubAgentId,
        success: bool,
        summary: String,
    },
    /// 新增一条观测。
    ObservationAdded {
        session_id: SessionId,
        turn_id: TurnId,
        observation: ToolObservation,
    },
    /// 助手文本增量(流式逐段输出)。最终会有一条完整的 [`RuntimeEvent::AssistantMessage`]。
    AssistantMessageDelta {
        session_id: SessionId,
        turn_id: TurnId,
        delta: String,
    },
    /// 助手思考过程增量。UI 应与最终输出区分展示。
    ReasoningDelta {
        session_id: SessionId,
        turn_id: TurnId,
        delta: String,
    },
    /// 助手文本消息(完整 / 最终)。
    AssistantMessage {
        session_id: SessionId,
        turn_id: TurnId,
        text: String,
    },
    /// 用户文本消息(用于外部协议恢复/回放用户消息)。
    UserMessage {
        session_id: SessionId,
        turn_id: TurnId,
        text: String,
    },
    /// 轻量状态提示(例如外部 agent 正在思考)。
    Status {
        session_id: SessionId,
        turn_id: TurnId,
        title: String,
        is_done: bool,
    },
    /// 需要用户补充输入。
    NeedUserInput {
        session_id: SessionId,
        turn_id: TurnId,
        question: String,
        pending_tool_call_id: Option<ToolCallId>,
        tool_name: Option<ToolName>,
        arguments: Option<Value>,
        pending_tool_calls: Vec<PendingToolCallSummary>,
    },
    /// 用户已处理工具执行审批。
    ToolApprovalResolved {
        session_id: SessionId,
        turn_id: TurnId,
        call_id: ToolCallId,
        approved: bool,
    },
    /// 一轮成功完成。
    TurnCompleted {
        session_id: SessionId,
        turn_id: TurnId,
        answer: Option<String>,
    },
    /// 一轮已被用户取消。取消是独立终态,不等同于失败。
    TurnCancelled {
        session_id: SessionId,
        turn_id: TurnId,
    },
    /// 一轮失败。
    TurnFailed {
        session_id: SessionId,
        turn_id: TurnId,
        reason: String,
    },
}

impl RuntimeEvent {
    /// 事件所属会话。
    pub fn session_id(&self) -> &SessionId {
        match self {
            RuntimeEvent::TurnStarted { session_id, .. }
            | RuntimeEvent::PlanUpdated { session_id, .. }
            | RuntimeEvent::ToolCallStarted { session_id, .. }
            | RuntimeEvent::ToolCallFinished { session_id, .. }
            | RuntimeEvent::SubAgentStarted { session_id, .. }
            | RuntimeEvent::SubAgentUpdated { session_id, .. }
            | RuntimeEvent::SubAgentFinished { session_id, .. }
            | RuntimeEvent::ObservationAdded { session_id, .. }
            | RuntimeEvent::AssistantMessageDelta { session_id, .. }
            | RuntimeEvent::ReasoningDelta { session_id, .. }
            | RuntimeEvent::AssistantMessage { session_id, .. }
            | RuntimeEvent::UserMessage { session_id, .. }
            | RuntimeEvent::Status { session_id, .. }
            | RuntimeEvent::NeedUserInput { session_id, .. }
            | RuntimeEvent::ToolApprovalResolved { session_id, .. }
            | RuntimeEvent::TurnCompleted { session_id, .. }
            | RuntimeEvent::TurnCancelled { session_id, .. }
            | RuntimeEvent::TurnFailed { session_id, .. } => session_id,
        }
    }
}
