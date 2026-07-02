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
    CallInternalFunction,
    ReadSessionDiagnostics,
    ExecuteRemoteCommand,
    CancelRemoteCommand,
    WriteRemoteFile,
    ReadRemoteCommandOutput,
    CallToolRuntimeTool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalDecision {
    Allow,
    Ask,
    Deny,
}

pub fn permission_policy_for_mode(mode: PermissionMode) -> tool_runtime::PermissionPolicy {
    let profile = match mode {
        PermissionMode::Deny => tool_runtime::PermissionProfile::Safe,
        PermissionMode::Ask => tool_runtime::PermissionProfile::Confirm,
        PermissionMode::Allow => tool_runtime::PermissionProfile::Auto,
    };
    tool_runtime::PermissionPolicy::for_profile(profile)
}

pub fn decide_permission(
    mode: PermissionMode,
    operation: PublicMcpOperationKind,
) -> ApprovalDecision {
    if matches!(
        operation,
        PublicMcpOperationKind::ReadTerminal
            | PublicMcpOperationKind::ReadSessionDiagnostics
            | PublicMcpOperationKind::ReadRemoteCommandOutput
    ) {
        return ApprovalDecision::Allow;
    }

    match mode {
        PermissionMode::Deny => ApprovalDecision::Deny,
        PermissionMode::Ask => ApprovalDecision::Ask,
        PermissionMode::Allow => ApprovalDecision::Allow,
    }
}
