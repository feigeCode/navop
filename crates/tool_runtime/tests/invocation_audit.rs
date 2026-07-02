use serde_json::json;
use tool_runtime::{
    ApprovalRequest, ApprovalStatus, AuditEvent, PermissionPolicy, PermissionProfile, ResourceId,
    ResourceKind, ResourcePool, ResourceRef, ResourceTarget, RiskLevel, ToolCaller, ToolId,
    ToolInvocation, ToolOrigin,
};

#[test]
fn invocation_carries_resource_pool_permission_and_target() {
    let pool =
        ResourcePool::new().with_resource(ResourceRef::new("ssh-a", ResourceKind::Ssh, "prod-a"));
    let invocation = ToolInvocation::new(
        ToolId::new("ssh.exec"),
        json!({ "command": "df -h" }),
        pool.clone(),
        PermissionPolicy::for_profile(PermissionProfile::Confirm),
        ToolCaller::Agent,
    )
    .with_target(ResourceTarget::Id(ResourceId::new("ssh-a")));

    assert_eq!(ToolId::new("ssh.exec"), invocation.tool_id);
    assert_eq!(
        Some(ResourceTarget::Id(ResourceId::new("ssh-a"))),
        invocation.target
    );
    assert_eq!(
        Some(&ResourceId::new("ssh-a")),
        invocation.resources.default_target.as_ref()
    );
}

#[test]
fn audit_event_records_target_risk_and_approval_status() {
    let event = AuditEvent {
        session_id: Some("session-1".to_string()),
        turn_id: Some("turn-1".to_string()),
        tool_id: ToolId::new("db.exec"),
        origin: ToolOrigin::Database,
        target_resource: Some(ResourceId::new("prod-db")),
        caller: ToolCaller::Agent,
        risk: RiskLevel::High,
        approval_status: ApprovalStatus::Approved,
        arguments_redacted: json!({ "sql": "update users set name = ?" }),
        result_summary: Some("1 row affected".to_string()),
        started_at: "2026-07-02T00:00:00Z".to_string(),
        finished_at: Some("2026-07-02T00:00:01Z".to_string()),
    };

    assert_eq!(ApprovalStatus::Approved, event.approval_status);
    assert_eq!(Some(ResourceId::new("prod-db")), event.target_resource);
}

#[test]
fn approval_request_uses_same_core_tool_identity() {
    let request = ApprovalRequest {
        id: "approval-1".to_string(),
        tool_id: ToolId::new("sftp.write"),
        target_resource: Some(ResourceId::new("prod-a")),
        caller: ToolCaller::Mcp,
        risk: RiskLevel::High,
        summary: "Write SFTP file".to_string(),
        arguments_redacted: json!({ "path": "/tmp/out" }),
    };

    assert_eq!(ToolId::new("sftp.write"), request.tool_id);
    assert_eq!(ToolCaller::Mcp, request.caller);
}
