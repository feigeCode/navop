use public_mcp::{
    permissions::PermissionMode,
    tools::{PublicMcpToolContext, PublicMcpToolRegistry, ToolRuntimeMcpProvider},
};
use serde_json::json;
use std::sync::{Arc, Mutex};
use tool_runtime::{
    ToolAdapter, ToolAnnotations, ToolContext, ToolDescriptor, ToolHandler, ToolMode, ToolRegistry,
    ToolResult,
};

#[test]
fn runtime_provider_mcp_schema_uses_target_field() {
    let registry = registry_with(RuntimeConnectionTool::new("db.query"));

    let tools = registry.tools();
    let schema = tools[0].input_schema.as_ref();
    let properties = schema["properties"].as_object().unwrap();

    assert_eq!(json!(["target", "sql"]), schema["required"]);
    assert!(properties.contains_key("target"));
    assert!(!properties.contains_key("connection"));
}

#[test]
fn runtime_provider_maps_target_to_provider_field() {
    let handler = Arc::new(RuntimeConnectionTool::new("db.query"));
    let registry = PublicMcpToolRegistry::new(vec![Arc::new(ToolRuntimeMcpProvider::new(
        ToolRegistry::new(vec![handler.clone()]),
    ))]);

    let result = futures::executor::block_on(registry.call_tool(
        "db.query",
        Some(rmcp::model::JsonObject::from_iter([
            ("target".to_string(), json!("42")),
            ("sql".to_string(), json!("select 1")),
        ])),
        context(),
    ))
    .expect("target-backed runtime MCP call should run");

    assert_eq!(
        Some(json!({ "connection": "42", "sql": "select 1" })),
        result.structured_content
    );
    assert_eq!(
        json!({ "connection": "42", "sql": "select 1" }),
        handler.last_input()
    );
}

#[test]
fn runtime_provider_rejects_provider_target_fields() {
    let registry = registry_with(RuntimeConnectionTool::new("db.query"));

    let error = futures::executor::block_on(registry.call_tool(
        "db.query",
        Some(rmcp::model::JsonObject::from_iter([
            ("connection".to_string(), json!("42")),
            ("sql".to_string(), json!("select 1")),
        ])),
        context(),
    ))
    .expect_err("provider target field should be rejected");

    assert!(error.to_string().contains("use `target`"));
}

fn registry_with(tool: RuntimeConnectionTool) -> PublicMcpToolRegistry {
    PublicMcpToolRegistry::new(vec![Arc::new(ToolRuntimeMcpProvider::new(
        ToolRegistry::new(vec![Arc::new(tool)]),
    ))])
}

fn context() -> PublicMcpToolContext {
    PublicMcpToolContext {
        permission_mode: PermissionMode::Deny,
        approver: Default::default(),
    }
}

#[derive(Clone)]
struct RuntimeConnectionTool {
    id: &'static str,
    last_input: Arc<Mutex<Option<serde_json::Value>>>,
}

impl RuntimeConnectionTool {
    fn new(id: &'static str) -> Self {
        Self {
            id,
            last_input: Arc::new(Mutex::new(None)),
        }
    }

    fn last_input(&self) -> serde_json::Value {
        self.last_input
            .lock()
            .expect("last input lock")
            .clone()
            .expect("runtime tool should receive input")
    }
}

impl ToolHandler for RuntimeConnectionTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            id: self.id.to_string(),
            title: "Query".to_string(),
            description: "Query input".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "connection": {
                        "type": "string",
                        "description": "Saved connection id or name."
                    },
                    "sql": { "type": "string" }
                },
                "required": ["connection", "sql"]
            }),
            output_schema: json!({ "type": "object" }),
            permissions: Vec::new(),
            mode: ToolMode::Deterministic,
            adapters: vec![ToolAdapter::Mcp],
            annotations: ToolAnnotations::read_only("Query"),
        }
    }

    fn call(&self, input: serde_json::Value, _context: ToolContext) -> tool_runtime::ToolFuture {
        *self.last_input.lock().expect("last input lock") = Some(input.clone());
        Box::pin(async move { Ok(ToolResult::structured(input)) })
    }
}
