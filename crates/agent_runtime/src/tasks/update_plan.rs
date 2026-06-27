//! `update_plan`:codex 风格的任务清单(TODO checklist)工具。
//!
//! 与"每轮强制规划"不同,`update_plan` 是模型**按需调用**的可选工具:简单问题
//! 模型直接回答、不会调用它(因此不产生任何计划);多步任务模型主动调用它维护一份
//! checklist。它**不经 [`ToolRouter`](crate::tools::ToolRouter)**——因为需要写回
//! [`Session`](crate::runtime::Session) 的当前计划并发出
//! [`RuntimeEvent::PlanUpdated`](crate::runtime::RuntimeEvent),由
//! [`AgentTask`](super::agent::AgentTask) 在循环中拦截处理。本模块只提供工具规格
//! (交给模型)与参数解析(JSON → [`Plan`])。

use crate::planner::{Plan, PlanSource, PlanStatus, PlanStep, StepStatus};
use crate::tools::ToolSpec;
use serde::Deserialize;

/// `update_plan` 工具名(模型 function-calling 中的函数名)。
pub const UPDATE_PLAN_TOOL: &str = "update_plan";

/// `update_plan` 工具规格,供模型在对话循环中按需调用。
pub fn update_plan_spec() -> ToolSpec {
    ToolSpec::new(
        UPDATE_PLAN_TOOL,
        "维护一份任务清单(TODO checklist)。仅用于多步任务;简单问题请直接回答,无需调用。\
         每次传入**完整**的步骤列表及各自状态;最多一个步骤处于 in_progress。每完成一步就再次调用以更新状态。",
        serde_json::json!({
            "type": "object",
            "properties": {
                "explanation": {
                    "type": "string",
                    "description": "(可选)本次更新计划的简短说明"
                },
                "plan": {
                    "type": "array",
                    "description": "完整、有序的步骤列表",
                    "items": {
                        "type": "object",
                        "properties": {
                            "step": {"type": "string", "description": "步骤内容"},
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "completed"]
                            }
                        },
                        "required": ["step", "status"]
                    }
                }
            },
            "required": ["plan"]
        }),
    )
}

#[derive(Deserialize)]
struct UpdatePlanArgs {
    #[serde(default)]
    explanation: Option<String>,
    #[serde(default)]
    plan: Vec<PlanItemArg>,
}

#[derive(Deserialize)]
struct PlanItemArg {
    #[serde(default)]
    step: String,
    #[serde(default)]
    status: Option<String>,
}

/// 把 `update_plan` 的参数解析为 [`Plan`] 与可选说明。
///
/// `goal` 作为计划标题(渲染在 Tasks 面板)。参数非法时返回 `None`,调用方据此
/// 回一条失败观测,让模型纠正后重试。
pub fn parse_plan(goal: &str, arguments: &serde_json::Value) -> Option<(Plan, Option<String>)> {
    let args: UpdatePlanArgs = serde_json::from_value(arguments.clone()).ok()?;
    let steps: Vec<PlanStep> = args
        .plan
        .iter()
        .map(|item| {
            let mut step = PlanStep::new(item.step.clone(), "");
            step.status = parse_step_status(item.status.as_deref());
            step
        })
        .collect();

    let status = derive_plan_status(&steps);
    let mut plan = Plan::new(goal.to_string(), PlanSource::Llm).with_steps(steps);
    plan.set_status(status);
    Some((plan, args.explanation))
}

/// 把 codex 的三态映射为运行时的 [`StepStatus`]。
fn parse_step_status(status: Option<&str>) -> StepStatus {
    match status {
        Some("in_progress") => StepStatus::Running,
        Some("completed") => StepStatus::Completed,
        _ => StepStatus::Pending,
    }
}

/// 由步骤推导计划整体状态:全部完成→Completed,空→Draft,其余→Running。
fn derive_plan_status(steps: &[PlanStep]) -> PlanStatus {
    if steps.is_empty() {
        PlanStatus::Draft
    } else if steps
        .iter()
        .all(|s| matches!(s.status, StepStatus::Completed))
    {
        PlanStatus::Completed
    } else {
        PlanStatus::Running
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_steps_and_maps_status() {
        let args = serde_json::json!({
            "explanation": "开始排查",
            "plan": [
                {"step": "查看连接数", "status": "completed"},
                {"step": "分析慢查询", "status": "in_progress"},
                {"step": "给出结论", "status": "pending"}
            ]
        });
        let (plan, explanation) = parse_plan("排查慢查询", &args).expect("应解析成功");
        assert_eq!(plan.goal, "排查慢查询");
        assert_eq!(plan.steps.len(), 3);
        assert_eq!(plan.steps[0].status, StepStatus::Completed);
        assert_eq!(plan.steps[1].status, StepStatus::Running);
        assert_eq!(plan.steps[2].status, StepStatus::Pending);
        assert_eq!(plan.status, PlanStatus::Running);
        assert_eq!(explanation.as_deref(), Some("开始排查"));
    }

    #[test]
    fn all_completed_marks_plan_completed() {
        let args = serde_json::json!({
            "plan": [{"step": "唯一一步", "status": "completed"}]
        });
        let (plan, _) = parse_plan("目标", &args).expect("应解析成功");
        assert_eq!(plan.status, PlanStatus::Completed);
    }

    #[test]
    fn unknown_status_defaults_to_pending() {
        let args = serde_json::json!({"plan": [{"step": "x", "status": "weird"}]});
        let (plan, _) = parse_plan("g", &args).expect("应解析成功");
        assert_eq!(plan.steps[0].status, StepStatus::Pending);
    }
}
