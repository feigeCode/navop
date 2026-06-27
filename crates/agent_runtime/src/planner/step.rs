//! 计划步骤。

use crate::ids::PlanStepId;
use crate::resource::ResourceId;
use crate::risk::RiskLevel;
use crate::tools::ToolName;
use serde::{Deserialize, Serialize};

/// 单个步骤的状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    /// 待执行。
    Pending,
    /// 执行中。
    Running,
    /// 已执行并取得观测(尚未判定是否成功完成目标)。
    Observed,
    /// 被跳过。
    Skipped,
    /// 执行失败。
    Failed,
    /// 已完成。
    Completed,
}

impl StepStatus {
    /// 是否为终态(不会再变化)。
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            StepStatus::Skipped | StepStatus::Failed | StepStatus::Completed
        )
    }
}

/// 计划中的一个步骤。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlanStep {
    pub id: PlanStepId,
    pub title: String,
    pub description: String,
    pub status: StepStatus,
    /// 建议使用的工具。
    pub tool_hint: Option<ToolName>,
    /// 建议操作的资源。
    pub resource_hint: Option<ResourceId>,
    /// 期望观测到的结果(供模型自检)。
    pub expected_observation: Option<String>,
    pub risk: RiskLevel,
}

impl PlanStep {
    pub fn new(title: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            id: PlanStepId::new(),
            title: title.into(),
            description: description.into(),
            status: StepStatus::Pending,
            tool_hint: None,
            resource_hint: None,
            expected_observation: None,
            risk: RiskLevel::Read,
        }
    }

    pub fn with_tool_hint(mut self, tool: Option<ToolName>) -> Self {
        self.tool_hint = tool;
        self
    }

    pub fn with_resource_hint(mut self, resource: Option<ResourceId>) -> Self {
        self.resource_hint = resource;
        self
    }

    pub fn with_expected_observation(mut self, expected: Option<String>) -> Self {
        self.expected_observation = expected;
        self
    }

    pub fn with_risk(mut self, risk: RiskLevel) -> Self {
        self.risk = risk;
        self
    }

    pub fn is_pending(&self) -> bool {
        self.status == StepStatus::Pending
    }
}
