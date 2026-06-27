//! 计划(Plan)。

use crate::ids::{PlanId, PlanStepId};
use crate::planner::step::{PlanStep, StepStatus};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 计划来源。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanSource {
    /// 由大模型生成。
    Llm,
    /// 由运维 Runbook 生成(预留)。
    Runbook,
    /// 人工录入。
    Manual,
}

/// 计划状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    /// 草稿(刚创建)。
    Draft,
    /// 执行中。
    Running,
    /// 等待用户输入。
    WaitingUser,
    /// 已完成。
    Completed,
    /// 已失败。
    Failed,
}

/// 一个可执行计划。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub id: PlanId,
    pub goal: String,
    pub source: PlanSource,
    pub status: PlanStatus,
    pub steps: Vec<PlanStep>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Plan {
    pub fn new(goal: impl Into<String>, source: PlanSource) -> Self {
        let now = Utc::now();
        Self {
            id: PlanId::new(),
            goal: goal.into(),
            source,
            status: PlanStatus::Draft,
            steps: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    pub fn with_steps(mut self, steps: Vec<PlanStep>) -> Self {
        self.steps = steps;
        self
    }

    /// 第一个待执行步骤。
    pub fn next_pending_step(&self) -> Option<&PlanStep> {
        self.steps.iter().find(|s| s.is_pending())
    }

    pub fn step(&self, id: &PlanStepId) -> Option<&PlanStep> {
        self.steps.iter().find(|s| &s.id == id)
    }

    pub fn step_mut(&mut self, id: &PlanStepId) -> Option<&mut PlanStep> {
        self.steps.iter_mut().find(|s| &s.id == id)
    }

    /// 更新某步状态;成功返回 `true`。会刷新 `updated_at`。
    pub fn mark_step(&mut self, id: &PlanStepId, status: StepStatus) -> bool {
        let found = if let Some(step) = self.step_mut(id) {
            step.status = status;
            true
        } else {
            false
        };
        if found {
            self.updated_at = Utc::now();
        }
        found
    }

    /// 所有步骤均处于终态。
    pub fn all_done(&self) -> bool {
        !self.steps.is_empty() && self.steps.iter().all(|s| s.status.is_terminal())
    }

    pub fn set_status(&mut self, status: PlanStatus) {
        self.status = status;
        self.updated_at = Utc::now();
    }

    /// 生成供模型阅读的计划清单。
    pub fn describe(&self) -> String {
        if self.steps.is_empty() {
            return "（计划暂无步骤）".to_string();
        }
        let mut out = String::new();
        for (i, step) in self.steps.iter().enumerate() {
            out.push_str(&format!(
                "{}. [{:?}] {} —— {}\n",
                i + 1,
                step.status,
                step.title,
                step.description
            ));
        }
        out
    }
}
