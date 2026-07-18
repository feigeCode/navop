//! `agent_runtime` —— 无渲染层的 Agent 运行时与 Planner 内核。
//!
//! 该 crate 参考 codex 的 Runtime 骨架实现,但完全解耦于 GPUI 与具体的
//! 数据库 / SSH 等资源 crate。它只依赖 `llm-connector` 的消息与工具类型,
//! 并通过 [`model::ModelClient`] trait 抽象模型调用,因此可在无 UI、无真实
//! 连接的前提下进行单元测试。
//!
//! 顶层数据流(codex 风格,模型驱动):
//!
//! ```text
//! 用户输入 -> AgentTask 循环:模型流式采样
//!   -> 无工具调用  => 该文本即最终回答(结束)
//!   -> 有工具调用  => 执行(业务工具经 ToolRouter;update_plan 更新 checklist 并发 PlanUpdated)
//!                     -> ToolObservation 写回 Session.history -> 再次采样(follow-up)
//! ```

rust_i18n::i18n!("locales", fallback = "en");

pub mod error;
pub mod history;
pub mod ids;
pub mod model;
pub mod planner;
pub mod resource;
pub mod resource_scope;
pub mod risk;
pub mod runtime;
pub mod skill;
pub mod tasks;
pub mod tools;

pub use error::{RuntimeError, ToolError};
pub use history::{HistoryItem, RuntimeHistory};
pub use ids::{PlanId, PlanStepId, SessionId, SubAgentId, ToolCallId, TurnId};
pub use model::{ModelClient, ModelRequest, ModelResponse, ModelStream, ModelStreamEvent};
pub use planner::{Plan, PlanSource, PlanStatus, PlanStep, StepStatus};
pub use resource::{
    ResourceCapability, ResourceContext, ResourceId, ResourceKind, ResourceRef, ResourceScope,
};
pub use resource_scope::{AgentResourceScope, DefaultTarget, DefaultTargetReason, ResourceCatalog};
pub use risk::RiskLevel;
pub use runtime::{InputImage, TurnInput};
pub use runtime::{
    PendingToolCallSummary, Runtime, RuntimeCommand, RuntimeEvent, RuntimeEventReceiver,
    RuntimeServices, Session, SessionSnapshot, TaskKind, TaskOutcome, ToolExecutionMode,
    TurnContext, UserInput,
};
pub use skill::{
    SkillCatalog, SkillContext, SkillImportError, SkillLoadError, SkillMetadata, SkillRef,
    SkillSummary, import_skill_dir,
};
pub use tasks::AgentTask;
pub use tools::{
    ObservationData, Tool, ToolCall, ToolDispatchContext, ToolName, ToolObservation, ToolRegistry,
    ToolRouter, ToolSpec,
};
