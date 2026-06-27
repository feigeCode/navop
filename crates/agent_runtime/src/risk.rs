//! 风险等级。被工具规格([`crate::tools::ToolSpec`])与计划步骤
//! ([`crate::planner::PlanStep`])共用,用于审批与展示。

use serde::{Deserialize, Serialize};
use std::fmt;

/// 操作的风险等级,由低到高。
///
/// 第一版不接入审批流程,仅用于在计划 / 工具规格中标注语义,为后续接入
/// 人工审批(高危操作需确认)预留信息。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    /// 只读操作(查询、查看状态),默认。
    #[default]
    Read,
    /// 低风险写操作。
    Low,
    /// 中等风险(可逆的变更)。
    Medium,
    /// 高风险(难以回滚的变更)。
    High,
    /// 危险操作(删除、重启等)。
    Critical,
}

impl RiskLevel {
    /// 是否属于需要人工确认的高危操作(>= High)。
    pub fn requires_confirmation(self) -> bool {
        self >= RiskLevel::High
    }

    /// 稳定的字符串表示,用于 prompt / 日志。
    pub fn as_str(self) -> &'static str {
        match self {
            RiskLevel::Read => "read",
            RiskLevel::Low => "low",
            RiskLevel::Medium => "medium",
            RiskLevel::High => "high",
            RiskLevel::Critical => "critical",
        }
    }
}

impl fmt::Display for RiskLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordering_is_low_to_high() {
        assert!(RiskLevel::Read < RiskLevel::Critical);
        assert!(RiskLevel::High.requires_confirmation());
        assert!(!RiskLevel::Medium.requires_confirmation());
    }
}
