//! 运行时任务抽象。
//!
//! 对应 Codex 的 `SessionTask`。一个任务封装一种工作流(普通对话、诊断等),
//! 在后台 Tokio 任务上运行,期间通过 [`Session`] 发事件、写历史。

use crate::ids::{ToolCallId, TurnId};
use crate::resource::ResourceContext;
use crate::runtime::RuntimeServices;
use crate::runtime::input_queue::{InputImage, TurnInput};
use crate::runtime::session::Session;
use crate::runtime::turn_context::TurnContext;
use crate::tools::{ToolCall, ToolName};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// 任务类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TaskKind {
    /// codex 风格统一 Agent:模型驱动,按需调用业务工具与 `update_plan` checklist。
    /// 默认模式:简单问题直接回答,复杂任务自主规划。
    #[default]
    Agent,
    /// Ask 模式:优先直接回答,不主动规划或调用工具,除非用户明确要求。
    Ask,
    /// Plan 模式:先规划再执行,适合多步骤任务。
    Plan,
}

/// 本轮工具执行策略。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ToolExecutionMode {
    /// 自动:执行模型请求的工具,包括 High/Critical,不进行人工确认。
    Auto,
    /// 只读:只暴露 `RiskLevel::Read` 工具。
    ReadOnly,
    /// 手动确认:允许模型请求工具,但业务工具执行前需要用户确认。
    #[default]
    Manual,
}

/// 任务运行结果。
#[derive(Clone, Debug)]
pub enum TaskOutcome {
    /// 完成,带可选最终回答。
    Completed { answer: Option<String> },
    /// 需要用户补充输入。
    NeedUserInput {
        question: String,
        pending_tool_call_id: Option<ToolCallId>,
        tool_name: Option<ToolName>,
        arguments: Option<Value>,
        pending_tool_calls: Vec<PendingToolCallSummary>,
    },
    /// 失败。
    Failed { reason: String },
    /// 被取消。
    Cancelled,
}

/// 等待审批的工具调用摘要,用于 UI / adapter 展示批量确认。
#[derive(Clone, Debug, PartialEq)]
pub struct PendingToolCallSummary {
    pub call_id: ToolCallId,
    pub tool_name: ToolName,
    pub arguments: Value,
}

impl PendingToolCallSummary {
    pub fn from_call(call: &ToolCall) -> Self {
        Self {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            arguments: call.arguments.clone(),
        }
    }
}

/// 暂停等待用户审批的一次工具调用。
#[derive(Clone, Debug)]
pub struct PendingToolApproval {
    pub turn_id: TurnId,
    pub task_kind: TaskKind,
    pub tool_mode: ToolExecutionMode,
    pub goal: String,
    pub call: ToolCall,
    pub additional_calls: Vec<ToolCall>,
    pub resources: ResourceContext,
}

impl PendingToolApproval {
    pub fn call_count(&self) -> usize {
        1 + self.additional_calls.len()
    }

    pub fn calls(&self) -> Vec<ToolCall> {
        let mut calls = Vec::with_capacity(self.call_count());
        calls.push(self.call.clone());
        calls.extend(self.additional_calls.iter().cloned());
        calls
    }
}

/// 任务执行所需的上下文。
pub struct TaskContext {
    pub kind: TaskKind,
    pub tool_mode: ToolExecutionMode,
    pub session: Arc<Session>,
    pub services: Arc<RuntimeServices>,
    pub turn: Arc<TurnContext>,
    /// 本轮输入。
    pub input: Vec<TurnInput>,
}

impl TaskContext {
    /// 把全部输入文本拼成本轮目标。
    pub fn goal(&self) -> String {
        self.input
            .iter()
            .map(|i| i.as_text())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 收集本轮全部输入附带的图片。
    pub fn input_images(&self) -> Vec<InputImage> {
        self.input
            .iter()
            .flat_map(|i| i.images().iter().cloned())
            .collect()
    }
}

/// 运行时任务接口。
#[async_trait]
pub trait RuntimeTask: Send + Sync + 'static {
    fn kind(&self) -> TaskKind;

    /// 执行任务直至完成或被取消。
    async fn run(self: Arc<Self>, ctx: TaskContext, cancellation: CancellationToken)
    -> TaskOutcome;
}
