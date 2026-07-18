use public_mcp::permissions::{
    ApprovalDecision, PermissionMode, PublicMcpOperationKind, decide_permission,
    permission_policy_for_mode,
};
use tool_runtime::{OperationPolicy, PermissionProfile};

#[test]
fn read_operations_are_allowed_in_every_mode() {
    let decision = decide_permission(PermissionMode::Deny, PublicMcpOperationKind::ReadTerminal);

    assert_eq!(ApprovalDecision::Allow, decision);

    assert_eq!(
        ApprovalDecision::Allow,
        decide_permission(
            PermissionMode::Deny,
            PublicMcpOperationKind::ReadSessionDiagnostics
        )
    );
    assert_eq!(
        ApprovalDecision::Allow,
        decide_permission(
            PermissionMode::Deny,
            PublicMcpOperationKind::ReadRemoteCommandOutput
        )
    );
}

#[test]
fn write_operations_follow_permission_mode() {
    assert_eq!(
        ApprovalDecision::Deny,
        decide_permission(PermissionMode::Deny, PublicMcpOperationKind::WriteTerminal)
    );
    assert_eq!(
        ApprovalDecision::Ask,
        decide_permission(PermissionMode::Ask, PublicMcpOperationKind::WriteTerminal)
    );
    assert_eq!(
        ApprovalDecision::Allow,
        decide_permission(PermissionMode::Allow, PublicMcpOperationKind::WriteTerminal)
    );
}

#[test]
fn internal_function_calls_follow_permission_mode() {
    assert_eq!(
        ApprovalDecision::Deny,
        decide_permission(
            PermissionMode::Deny,
            PublicMcpOperationKind::CallInternalFunction
        )
    );
    assert_eq!(
        ApprovalDecision::Ask,
        decide_permission(
            PermissionMode::Ask,
            PublicMcpOperationKind::CallInternalFunction
        )
    );
    assert_eq!(
        ApprovalDecision::Allow,
        decide_permission(
            PermissionMode::Allow,
            PublicMcpOperationKind::CallInternalFunction
        )
    );
}

#[test]
fn structured_remote_write_operations_follow_permission_mode() {
    for operation in [
        PublicMcpOperationKind::ExecuteRemoteCommand,
        PublicMcpOperationKind::WriteRemoteFile,
        PublicMcpOperationKind::CancelRemoteCommand,
    ] {
        assert_eq!(
            ApprovalDecision::Deny,
            decide_permission(PermissionMode::Deny, operation)
        );
        assert_eq!(
            ApprovalDecision::Ask,
            decide_permission(PermissionMode::Ask, operation)
        );
        assert_eq!(
            ApprovalDecision::Allow,
            decide_permission(PermissionMode::Allow, operation)
        );
    }
}

#[test]
fn permission_modes_map_to_unified_runtime_profiles() {
    let safe = permission_policy_for_mode(PermissionMode::Deny);
    assert_eq!(PermissionProfile::Safe, safe.mode);
    assert_eq!(OperationPolicy::Allow, safe.read_policy);
    assert_eq!(OperationPolicy::Deny, safe.write_policy);
    assert_eq!(OperationPolicy::Deny, safe.high_risk_policy);

    let confirm = permission_policy_for_mode(PermissionMode::Ask);
    assert_eq!(PermissionProfile::Confirm, confirm.mode);
    assert_eq!(OperationPolicy::Allow, confirm.read_policy);
    assert_eq!(OperationPolicy::Ask, confirm.write_policy);
    assert_eq!(OperationPolicy::Ask, confirm.high_risk_policy);

    let auto = permission_policy_for_mode(PermissionMode::Allow);
    assert_eq!(PermissionProfile::Auto, auto.mode);
    assert_eq!(OperationPolicy::Allow, auto.read_policy);
    assert_eq!(OperationPolicy::Allow, auto.write_policy);
    assert_eq!(OperationPolicy::Allow, auto.high_risk_policy);
}
