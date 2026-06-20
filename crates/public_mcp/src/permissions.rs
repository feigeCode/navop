use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    Deny,
    Ask,
    Allow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicMcpOperationKind {
    ReadTerminal,
    WriteTerminal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalDecision {
    Allow,
    Ask,
    Deny,
}

pub fn decide_permission(
    mode: PermissionMode,
    operation: PublicMcpOperationKind,
) -> ApprovalDecision {
    if matches!(operation, PublicMcpOperationKind::ReadTerminal) {
        return ApprovalDecision::Allow;
    }

    match mode {
        PermissionMode::Deny => ApprovalDecision::Deny,
        PermissionMode::Ask => ApprovalDecision::Ask,
        PermissionMode::Allow => ApprovalDecision::Allow,
    }
}
