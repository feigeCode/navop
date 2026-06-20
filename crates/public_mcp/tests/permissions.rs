use public_mcp::permissions::{
    ApprovalDecision, PermissionMode, PublicMcpOperationKind, decide_permission,
};

#[test]
fn read_operations_are_allowed_in_every_mode() {
    let decision = decide_permission(PermissionMode::Deny, PublicMcpOperationKind::ReadTerminal);

    assert_eq!(ApprovalDecision::Allow, decision);
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
