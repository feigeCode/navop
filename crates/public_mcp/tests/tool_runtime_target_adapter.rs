use public_mcp::{
    permissions::PermissionMode,
    tools::{PublicMcpToolContext, PublicMcpToolRegistry, ToolRuntimeMcpProvider},
};
use serde_json::json;
use std::sync::{Arc, Mutex};
use tool_runtime::{
    ResourceKind, ResourcePool, ResourceRef, ToolAdapter, ToolAnnotations, ToolContext,
    ToolDescriptor, ToolHandler, ToolMode, ToolRegistry, ToolResult, ToolTargetSpec,
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
fn runtime_provider_resolves_resource_label_to_provider_target_id() {
    let handler = Arc::new(RuntimeConnectionTool::new("db.query"));
    let registry = registry_with_pool(
        handler.clone(),
        ResourcePool::new().with_resource(resource("db-prod", "prod database", "primary-db")),
    );

    futures::executor::block_on(registry.call_tool(
        "db.query",
        Some(rmcp::model::JsonObject::from_iter([
            ("target".to_string(), json!("prod database")),
            ("sql".to_string(), json!("select 1")),
        ])),
        context(),
    ))
    .expect("resource label target should resolve");

    assert_eq!(
        json!({ "connection": "db-prod", "sql": "select 1" }),
        handler.last_input()
    );
}

#[test]
fn runtime_provider_resolves_resource_alias_to_provider_target_id() {
    let handler = Arc::new(RuntimeConnectionTool::new("db.query"));
    let registry = registry_with_pool(
        handler.clone(),
        ResourcePool::new().with_resource(resource("db-prod", "prod database", "primary-db")),
    );

    futures::executor::block_on(registry.call_tool(
        "db.query",
        Some(rmcp::model::JsonObject::from_iter([
            ("target".to_string(), json!("primary-db")),
            ("sql".to_string(), json!("select 1")),
        ])),
        context(),
    ))
    .expect("resource alias target should resolve");

    assert_eq!(
        json!({ "connection": "db-prod", "sql": "select 1" }),
        handler.last_input()
    );
}

#[test]
fn runtime_provider_resolves_target_with_tool_resource_kind() {
    let handler = Arc::new(
        RuntimeConnectionTool::new("terminal.exec").with_target_kinds(vec![ResourceKind::Terminal]),
    );
    let registry = registry_with_pool(
        handler.clone(),
        ResourcePool::new()
            .with_resource(resource("db-prod", "prod", "primary-db"))
            .with_resource(terminal_resource("terminal-prod", "prod")),
    );

    futures::executor::block_on(registry.call_tool(
        "terminal.exec",
        Some(rmcp::model::JsonObject::from_iter([
            ("target".to_string(), json!("prod")),
            ("sql".to_string(), json!("select 1")),
        ])),
        context(),
    ))
    .expect("tool target kind should disambiguate resources");

    assert_eq!(
        json!({ "connection": "terminal-prod", "sql": "select 1" }),
        handler.last_input()
    );
}

#[test]
fn runtime_provider_reads_resource_pool_at_call_time() {
    let handler = Arc::new(RuntimeConnectionTool::new("db.query"));
    let pool = Arc::new(Mutex::new(ResourcePool::new()));
    let registry = registry_with_pool_provider(handler.clone(), {
        let pool = pool.clone();
        Arc::new(move || Some(pool.lock().expect("resource pool lock").clone()))
    });

    *pool.lock().expect("resource pool lock") =
        ResourcePool::new().with_resource(resource("db-prod", "prod database", "primary-db"));

    futures::executor::block_on(registry.call_tool(
        "db.query",
        Some(rmcp::model::JsonObject::from_iter([
            ("target".to_string(), json!("primary-db")),
            ("sql".to_string(), json!("select 1")),
        ])),
        context(),
    ))
    .expect("resource pool provider should be evaluated for each call");

    assert_eq!(
        json!({ "connection": "db-prod", "sql": "select 1" }),
        handler.last_input()
    );
}

#[test]
fn runtime_provider_rejects_ambiguous_resource_target() {
    let registry = registry_with_pool(
        Arc::new(RuntimeConnectionTool::new("db.query")),
        ResourcePool::new()
            .with_resource(resource("db-prod-a", "prod", "primary"))
            .with_resource(resource("db-prod-b", "prod", "replica")),
    );

    let error = futures::executor::block_on(registry.call_tool(
        "db.query",
        Some(rmcp::model::JsonObject::from_iter([
            ("target".to_string(), json!("prod")),
            ("sql".to_string(), json!("select 1")),
        ])),
        context(),
    ))
    .expect_err("ambiguous resource target should be rejected");

    assert!(error.to_string().contains("ambiguous"));
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

fn registry_with_pool(
    tool: Arc<RuntimeConnectionTool>,
    resource_pool: ResourcePool,
) -> PublicMcpToolRegistry {
    let provider = ToolRuntimeMcpProvider::new(ToolRegistry::new(vec![tool]))
        .with_resource_pool(resource_pool);
    PublicMcpToolRegistry::new(vec![Arc::new(provider)])
}

fn registry_with_pool_provider(
    tool: Arc<RuntimeConnectionTool>,
    provider: public_mcp::tools::ResourcePoolProvider,
) -> PublicMcpToolRegistry {
    let provider = ToolRuntimeMcpProvider::new(ToolRegistry::new(vec![tool]))
        .with_resource_pool_provider(provider);
    PublicMcpToolRegistry::new(vec![Arc::new(provider)])
}

fn resource(id: &str, label: &str, alias: &str) -> ResourceRef {
    ResourceRef::new(id, ResourceKind::Mysql, label).with_alias(alias)
}

fn terminal_resource(id: &str, label: &str) -> ResourceRef {
    ResourceRef::new(id, ResourceKind::Terminal, label)
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
    target_kinds: Vec<ResourceKind>,
}

impl RuntimeConnectionTool {
    fn new(id: &'static str) -> Self {
        Self {
            id,
            last_input: Arc::new(Mutex::new(None)),
            target_kinds: Vec::new(),
        }
    }

    fn with_target_kinds(mut self, target_kinds: Vec<ResourceKind>) -> Self {
        self.target_kinds = target_kinds;
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

    fn target_spec(&self) -> ToolTargetSpec {
        if self.target_kinds.is_empty() {
            return ToolTargetSpec::none();
        }
        ToolTargetSpec::required(self.target_kinds.clone())
    }
}
