use agent_runtime::{
    ResourceContext, ResourceKind, ResourceRef, SessionId, ToolCallId, ToolName, TurnId,
};
use serde_json::json;
use std::sync::{Arc, Mutex};
use tool_runtime::{
    ToolAdapter, ToolAnnotations, ToolContext, ToolDescriptor, ToolHandler, ToolMode, ToolRegistry,
    ToolResult,
};

#[tokio::test]
async fn runtime_registry_agent_spec_uses_target_field() {
    let registry = ToolRegistry::new(vec![Arc::new(
        RuntimeEchoTool::new("db.query").with_input_schema(connection_sql_schema()),
    )]);
    let agent_registry = agent_runtime::tools::tool_runtime_agent_tool_registry(
        registry,
        tool_runtime::ToolAdapter::FunctionCalling,
    );

    let specs = agent_registry.specs(&ResourceContext::new());
    let properties = specs[0].parameters["properties"].as_object().unwrap();

    assert_eq!(json!(["target", "sql"]), specs[0].parameters["required"]);
    assert!(properties.contains_key("target"));
    assert!(!properties.contains_key("connection"));
}

#[tokio::test]
async fn runtime_registry_agent_tool_maps_target_to_runtime_connection() {
    let handler =
        Arc::new(RuntimeEchoTool::new("db.query").with_input_schema(connection_sql_schema()));
    let registry = ToolRegistry::new(vec![handler.clone()]);
    let agent_registry = agent_runtime::tools::tool_runtime_agent_tool_registry(
        registry,
        tool_runtime::ToolAdapter::FunctionCalling,
    );
    let tool = agent_registry.get(&ToolName::new("db.query")).unwrap();
    let resources = ResourceContext::new().with_resource(ResourceRef::new(
        "42",
        ResourceKind::Mysql,
        "prod db",
    ));

    let observation = tool
        .execute(agent_invocation(
            "db.query",
            json!({ "target": "prod db", "sql": "select 1" }),
            resources,
        ))
        .await
        .expect("runtime target should map to connection");

    assert!(observation.success);
    assert_eq!(
        json!({ "connection": "42", "sql": "select 1" }),
        handler.last_input()
    );
    assert_eq!(
        Some(agent_runtime::ResourceId::new("42")),
        observation.resource_id
    );
}

#[tokio::test]
async fn runtime_registry_agent_tool_maps_default_target_to_runtime_connection() {
    let handler =
        Arc::new(RuntimeEchoTool::new("db.query").with_input_schema(connection_sql_schema()));
    let registry = ToolRegistry::new(vec![handler.clone()]);
    let agent_registry = agent_runtime::tools::tool_runtime_agent_tool_registry(
        registry,
        tool_runtime::ToolAdapter::FunctionCalling,
    );
    let tool = agent_registry.get(&ToolName::new("db.query")).unwrap();
    let resources = ResourceContext::new().with_resource(ResourceRef::new(
        "42",
        ResourceKind::Mysql,
        "prod db",
    ));

    tool.execute(agent_invocation(
        "db.query",
        json!({ "sql": "select 1" }),
        resources,
    ))
    .await
    .expect("default target should map to connection");

    assert_eq!(
        json!({ "connection": "42", "sql": "select 1" }),
        handler.last_input()
    );
}

#[tokio::test]
async fn runtime_registry_agent_tool_rejects_provider_target_fields() {
    let handler =
        Arc::new(RuntimeEchoTool::new("db.query").with_input_schema(connection_sql_schema()));
    let registry = ToolRegistry::new(vec![handler]);
    let agent_registry = agent_runtime::tools::tool_runtime_agent_tool_registry(
        registry,
        tool_runtime::ToolAdapter::FunctionCalling,
    );
    let tool = agent_registry.get(&ToolName::new("db.query")).unwrap();

    let error = tool
        .execute(agent_invocation(
            "db.query",
            json!({ "connection": "42", "sql": "select 1" }),
            ResourceContext::new(),
        ))
        .await
        .expect_err("agent-facing provider field should be rejected");

    assert!(error.to_string().contains("use `target`"));
}

#[derive(Clone)]
struct RuntimeEchoTool {
    descriptor: ToolDescriptor,
    last_input: Arc<Mutex<Option<serde_json::Value>>>,
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
                adapters: vec![ToolAdapter::FunctionCalling],
                annotations: ToolAnnotations::read_only("Echo"),
            },
            last_input: Arc::new(Mutex::new(None)),
        }
    }

    fn with_input_schema(mut self, input_schema: serde_json::Value) -> Self {
        self.descriptor.input_schema = input_schema;
        self
    }

    fn last_input(&self) -> serde_json::Value {
        self.last_input
            .lock()
            .expect("last input lock")
            .clone()
            .expect("runtime tool should receive input")
    }
}

impl ToolHandler for RuntimeEchoTool {
    fn descriptor(&self) -> ToolDescriptor {
        self.descriptor.clone()
    }

    fn call(&self, input: serde_json::Value, _context: ToolContext) -> tool_runtime::ToolFuture {
        *self.last_input.lock().expect("last input lock") = Some(input.clone());
        Box::pin(async move { Ok(ToolResult::structured(input)) })
    }
}

fn agent_invocation(
    tool_name: &str,
    arguments: serde_json::Value,
    resources: ResourceContext,
) -> agent_runtime::tools::ToolInvocation {
    agent_runtime::tools::ToolInvocation {
        session_id: SessionId::from_string("session-1"),
        turn_id: TurnId::from_string("turn-1"),
        call_id: ToolCallId::from_string("call-1"),
        tool_name: ToolName::new(tool_name),
        arguments,
        resource_id: None,
        resources,
        cancellation: tokio_util::sync::CancellationToken::new(),
    }
}

fn connection_sql_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "connection": {
                "type": "string",
                "description": "Saved database connection id or name."
            },
            "sql": { "type": "string" }
        },
        "required": ["connection", "sql"]
    })
}
