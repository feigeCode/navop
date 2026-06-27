//! 计划数据模型与历史 → 模型消息转换。
//!
//! 架构转向 codex 风格后,显式 Planner(`create_plan` / `next` / `replan`)已移除;
//! 本模块只保留:
//! - 计划数据结构([`Plan`] / [`PlanStep`] 及状态枚举),供 `update_plan` checklist
//!   工具([`crate::tasks`])复用;
//! - [`history_to_messages`]:把会话历史转换为模型输入消息,供任务构造 prompt。

mod plan;
mod prompt;
mod step;

pub use plan::{Plan, PlanSource, PlanStatus};
pub use step::{PlanStep, StepStatus};

// 供 tasks 模块复用:把历史转换为模型消息。
pub(crate) use prompt::history_to_messages;
