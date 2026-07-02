use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ResourceId, RiskLevel, ToolCaller, ToolId, ToolOrigin};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    NotRequired,
    Pending,
    Approved,
    Denied,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ApprovalRequest {
    pub id: String,
    pub tool_id: ToolId,
    pub target_resource: Option<ResourceId>,
    pub caller: ToolCaller,
    pub risk: RiskLevel,
    pub summary: String,
    pub arguments_redacted: Value,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AuditEvent {
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub tool_id: ToolId,
    pub origin: ToolOrigin,
    pub target_resource: Option<ResourceId>,
    pub caller: ToolCaller,
    pub risk: RiskLevel,
    pub approval_status: ApprovalStatus,
    pub arguments_redacted: Value,
    pub result_summary: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
}
