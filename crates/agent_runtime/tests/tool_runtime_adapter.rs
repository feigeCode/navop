use agent_runtime::tools::{
    ObservationData, permission_policy_for_tool_mode, runtime_tool_invocation_from_call,
};
use agent_runtime::{
    ResourceContext, ResourceKind, ResourceRef, ResourceScope, RiskLevel, SessionId, SkillContext,
    ToolCall, ToolCallId, ToolExecutionMode, ToolName, ToolSpec, TurnId,
};
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tool_runtime::{
    RuntimeToolDescriptor, ToolAdapter, ToolAnnotations, ToolContext, ToolDescriptor, ToolHandler,
    ToolId, ToolMode, ToolOrigin, ToolRegistry, ToolResult, ToolTargetSpec,
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

#[tokio::test]
async fn runtime_registry_exposes_agent_specs_with_canonical_runtime_id() {
    let registry = ToolRegistry::new(vec![Arc::new(RuntimeEchoTool::new("db.query"))]);
    let agent_registry = agent_runtime::tools::tool_runtime_agent_tool_registry(
        registry,
        tool_runtime::ToolAdapter::FunctionCalling,
    );

    let specs = agent_registry.specs(&ResourceContext::new());

    assert_eq!(1, specs.len());
    assert_eq!("db_query", specs[0].name.as_str());
    assert_eq!("Echo input", specs[0].description);
    assert_eq!(json!({ "type": "object" }), specs[0].parameters);
}

#[tokio::test]
async fn runtime_registry_agent_tool_executes_canonical_runtime_tool() {
    let handler = Arc::new(RuntimeEchoTool::new("db.query"));
    let registry = ToolRegistry::new(vec![handler.clone()]);
    let agent_registry = agent_runtime::tools::tool_runtime_agent_tool_registry(
        registry,
        tool_runtime::ToolAdapter::FunctionCalling,
    );
    let tool = agent_registry
        .get(&ToolName::new("db.query"))
        .expect("runtime tool should be exposed to agent");

    let observation = tool
        .execute(agent_invocation("db.query", json!({ "message": "hello" })))
        .await
        .expect("runtime tool call should execute");

    assert!(observation.success);
    assert_eq!("db_query", observation.tool_name.as_str());
    assert_eq!(json!({ "message": "hello" }), handler.last_input());
    assert_eq!(
        json!({ "message": "hello" }),
        observation_data_json(&observation)
    );
}

#[tokio::test]
async fn runtime_registry_agent_tool_forwards_invocation_cancellation() {
    let handler = Arc::new(RuntimeEchoTool::new("terminal.exec"));
    let registry = ToolRegistry::new(vec![handler.clone()]);
    let agent_registry = agent_runtime::tools::tool_runtime_agent_tool_registry(
        registry,
        tool_runtime::ToolAdapter::FunctionCalling,
    );
    let tool = agent_registry
        .get(&ToolName::new("terminal.exec"))
        .expect("runtime tool should be exposed to agent");
    let invocation = agent_invocation("terminal.exec", json!({ "command": "pwd" }));
    invocation.cancellation.cancel();

    tool.execute(invocation)
        .await
        .expect("runtime tool call should execute");

    assert!(handler.last_cancellation_state());
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

#[derive(Clone)]
struct RuntimeEchoTool {
    descriptor: ToolDescriptor,
    last_input: Arc<Mutex<Option<serde_json::Value>>>,
    last_cancellation_state: Arc<AtomicBool>,
}

impl RuntimeEchoTool {
    fn new(id: &str) -> Self {
        Self {
            descriptor: ToolDescriptor {
                id: id.to_string(),
                title: "Echo".to_string(),
                description: "Echo input".to_string(),
                input_schema: json!({ "type": "object" }),
                output_schema: json!({ "type": "object" }),
                permissions: Vec::new(),
                mode: ToolMode::Deterministic,
                adapters: vec![tool_runtime::ToolAdapter::FunctionCalling],
                annotations: ToolAnnotations::read_only("Echo"),
            },
            last_input: Arc::new(Mutex::new(None)),
            last_cancellation_state: Arc::new(AtomicBool::new(false)),
        }
    }

    fn last_input(&self) -> serde_json::Value {
        self.last_input
            .lock()
            .expect("last input lock")
            .clone()
            .expect("runtime tool should receive input")
    }

    fn last_cancellation_state(&self) -> bool {
        self.last_cancellation_state.load(Ordering::Acquire)
    }
}

impl ToolHandler for RuntimeEchoTool {
    fn descriptor(&self) -> ToolDescriptor {
        self.descriptor.clone()
    }

    fn call(&self, input: serde_json::Value, context: ToolContext) -> tool_runtime::ToolFuture {
        *self.last_input.lock().expect("last input lock") = Some(input.clone());
        self.last_cancellation_state
            .store(context.cancellation.is_cancelled(), Ordering::Release);
        Box::pin(async move { Ok(ToolResult::structured(input)) })
    }
}

fn agent_invocation(
    tool_name: &str,
    arguments: serde_json::Value,
) -> agent_runtime::tools::ToolInvocation {
    agent_runtime::tools::ToolInvocation {
        session_id: SessionId::from_string("session-1"),
        turn_id: TurnId::from_string("turn-1"),
        call_id: ToolCallId::from_string("call-1"),
        tool_name: ToolName::new(tool_name),
        arguments,
        resource_id: None,
        resources: ResourceContext::new(),
        skills: SkillContext::new(),
        cancellation: tokio_util::sync::CancellationToken::new(),
    }
}

fn observation_data_json(observation: &agent_runtime::tools::ToolObservation) -> serde_json::Value {
    match &observation.data {
        ObservationData::Json(value) => value.clone(),
        other => panic!("expected json observation data, got {other:?}"),
    }
}
