use agent_runtime::tools::{permission_policy_for_tool_mode, runtime_tool_invocation_from_call};
use agent_runtime::{
    ResourceContext, ResourceKind, ResourceRef, ResourceScope, RiskLevel, SessionId, ToolCall,
    ToolCallId, ToolExecutionMode, ToolName, ToolSpec, TurnId,
};
use serde_json::json;
use tool_runtime::{
    RuntimeToolDescriptor, ToolAdapter, ToolAnnotations, ToolId, ToolMode, ToolOrigin,
    ToolTargetSpec,
};

#[test]
fn runtime_descriptor_converts_to_agent_tool_spec() {
    let descriptor = runtime_descriptor("db.query", ToolAnnotations::read_only("Query"));

    let spec = ToolSpec::from_runtime_descriptor(&descriptor);

    assert_eq!(ToolName::new("db.query"), spec.name);
    assert_eq!("Run query", spec.description);
    assert_eq!(json!({ "type": "object" }), spec.parameters);
    assert_eq!(RiskLevel::Read, spec.risk);
}

#[test]
fn runtime_descriptor_maps_high_risk_annotations() {
    let descriptor = runtime_descriptor(
        "db.exec",
        ToolAnnotations::mutating("Exec").with_risk(tool_runtime::RiskLevel::High),
    );

    let spec = ToolSpec::from_runtime_descriptor(&descriptor);

    assert_eq!(RiskLevel::High, spec.risk);
}

#[test]
fn resource_context_converts_to_runtime_resource_pool() {
    let context = ResourceContext::new()
        .with_resource(
            ResourceRef::new("db-prod", ResourceKind::Mysql, "prod db")
                .with_scope(ResourceScope::new("database", "Database", "ai_app")),
        )
        .with_resource(ResourceRef::new("ssh-prod", ResourceKind::Ssh, "prod ssh"));

    let pool = context.to_runtime_resource_pool();

    assert_eq!(
        Some(&tool_runtime::ResourceId::new("db-prod")),
        pool.default_target.as_ref()
    );
    assert_eq!("prod db", pool.resolve_target("prod db").unwrap().label);
    assert_eq!("ai_app", pool.resources[0].scopes[0].value);
}

#[test]
fn tool_execution_mode_maps_to_runtime_permission_profile() {
    assert_eq!(
        tool_runtime::PermissionProfile::Safe,
        permission_policy_for_tool_mode(ToolExecutionMode::ReadOnly).mode,
    );
    assert_eq!(
        tool_runtime::PermissionProfile::Confirm,
        permission_policy_for_tool_mode(ToolExecutionMode::Manual).mode,
    );
    assert_eq!(
        tool_runtime::PermissionProfile::Auto,
        permission_policy_for_tool_mode(ToolExecutionMode::Auto).mode,
    );
}

#[test]
fn tool_call_converts_to_runtime_invocation_with_resource_pool() {
    let context = ResourceContext::new().with_resource(ResourceRef::new(
        "ssh-prod",
        ResourceKind::Ssh,
        "prod ssh",
    ));
    let call = ToolCall::new("ssh.exec", json!({ "command": "df -h" }))
        .with_call_id(ToolCallId::from_string("call-1".to_string()));

    let invocation = runtime_tool_invocation_from_call(
        &call,
        &context,
        ToolExecutionMode::Manual,
        SessionId::from_string("session-1".to_string()),
        TurnId::from_string("turn-1".to_string()),
    );

    assert_eq!(tool_runtime::ToolId::new("ssh_exec"), invocation.tool_id);
    assert_eq!(json!({ "command": "df -h" }), invocation.arguments);
    assert_eq!(tool_runtime::ToolCaller::Agent, invocation.caller);
    assert_eq!(
        tool_runtime::PermissionProfile::Confirm,
        invocation.permission.mode
    );
    assert_eq!(Some("session-1".to_string()), invocation.audit.session_id);
    assert_eq!(Some("turn-1".to_string()), invocation.audit.turn_id);
    assert_eq!(Some("call-1".to_string()), invocation.audit.request_id);
}

fn runtime_descriptor(id: &str, annotations: ToolAnnotations) -> RuntimeToolDescriptor {
    RuntimeToolDescriptor {
        id: ToolId::new(id),
        title: "Query".to_string(),
        description: "Run query".to_string(),
        input_schema: json!({ "type": "object" }),
        output_schema: json!({ "type": "object" }),
        permissions: Vec::new(),
        mode: ToolMode::Deterministic,
        adapters: vec![ToolAdapter::FunctionCalling],
        annotations,
        target: ToolTargetSpec::default(),
        origin: ToolOrigin::Database,
        aliases: Vec::new(),
    }
}
