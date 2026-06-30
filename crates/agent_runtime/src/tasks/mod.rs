//! 运行时任务实现集合。
//!
//! - `AgentTask`:codex 风格的统一循环(模型驱动,按需调用业务工具与 `update_plan`
//!   checklist)。当前唯一的任务类型,涵盖简单问答与多步运维。

mod agent;
mod agent_prompt;
mod agent_tool_validation;
mod delegate_task;
mod subagent_task;
mod update_plan;

pub use agent::AgentTask;
pub(crate) use agent::continue_after_tool_decision;
