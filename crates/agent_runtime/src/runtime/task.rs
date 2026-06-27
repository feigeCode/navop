//! 运行时任务抽象。
//!
//! 对应 Codex 的 `SessionTask`。一个任务封装一种工作流(普通对话、诊断等),
//! 在后台 Tokio 任务上运行,期间通过 [`Session`] 发事件、写历史。

use crate::runtime::RuntimeServices;
use crate::runtime::input_queue::{InputImage, TurnInput};
use crate::runtime::session::Session;
use crate::runtime::turn_context::TurnContext;
use async_trait::async_trait;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// 任务类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TaskKind {
    /// codex 风格统一 Agent:模型驱动,按需调用业务工具与 `update_plan` checklist。
    /// 当前唯一的任务类型(涵盖简单问答与多步运维)。
    #[default]
    Agent,
}

/// 任务运行结果。
#[derive(Clone, Debug)]
pub enum TaskOutcome {
    /// 完成,带可选最终回答。
    Completed { answer: Option<String> },
    /// 需要用户补充输入。
    NeedUserInput { question: String },
    /// 失败。
    Failed { reason: String },
    /// 被取消。
    Cancelled,
}

/// 任务执行所需的上下文。
pub struct TaskContext {
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
