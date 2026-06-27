//! 运行时统一错误类型。
//!
//! 区分两类错误:
//! - `ToolError`:单个工具执行层面的错误,**可恢复**——会被包装成失败的
//!   [`ToolObservation`](crate::tools::ToolObservation) 写回历史,交由 Planner
//!   决定是否 replan。
//! - `RuntimeError`:运行时 / Planner / 模型层面的错误,通常意味着整轮失败。

use crate::ids::SessionId;

/// 工具执行错误。这类错误不会直接中断整轮,而是被转换为失败观测写回历史。
#[derive(Debug, Clone, thiserror::Error)]
pub enum ToolError {
    /// 注册表中找不到对应名称的工具。
    #[error("未找到工具: {0}")]
    NotFound(String),

    /// 模型给出的参数无法解析或不满足 schema。
    #[error("工具参数非法: {0}")]
    InvalidArguments(String),

    /// 工具执行过程中失败。
    #[error("工具执行失败: {0}")]
    Execution(String),

    /// 资源上下文缺少该工具所需的资源。
    #[error("缺少所需资源: {0}")]
    MissingResource(String),

    /// 执行被取消。
    #[error("工具执行被取消")]
    Cancelled,
}

/// 运行时顶层错误。
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    /// 模型调用失败。
    #[error("模型调用失败: {0}")]
    Model(String),

    /// Planner 规划 / 决策失败。
    #[error("规划失败: {0}")]
    Planner(String),

    /// 工具层错误(一般已被转换为观测,此变体用于无法恢复的场景)。
    #[error(transparent)]
    Tool(#[from] ToolError),

    /// 指定的会话不存在。
    #[error("会话不存在: {0}")]
    SessionNotFound(SessionId),

    /// 当前会话已有正在运行的任务。
    #[error("会话已有正在运行的任务: {0}")]
    SessionBusy(SessionId),

    /// 整轮被取消。
    #[error("任务被取消")]
    Cancelled,

    /// 其它错误。
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl RuntimeError {
    /// 便捷构造模型错误。
    pub fn model(msg: impl Into<String>) -> Self {
        Self::Model(msg.into())
    }

    /// 便捷构造规划错误。
    pub fn planner(msg: impl Into<String>) -> Self {
        Self::Planner(msg.into())
    }
}
