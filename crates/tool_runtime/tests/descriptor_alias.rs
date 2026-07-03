use std::sync::Arc;

use serde_json::json;
use tool_runtime::{
    ToolAdapter, ToolAlias, ToolAnnotations, ToolContext, ToolDescriptor, ToolHandler, ToolMode,
    ToolRegistry, ToolResult,
};

#[test]
fn registry_get_resolves_alias_to_canonical_descriptor() {
    let registry = ToolRegistry::new(vec![Arc::new(
        EchoHandler::new("ssh.exec").with_alias("ssh.remote_exec"),
    )]);

    let descriptor = registry.get("ssh.remote_exec", ToolAdapter::Mcp).unwrap();

    assert_eq!("ssh.exec", descriptor.id);
}

#[test]
fn registry_call_resolves_alias_to_canonical_handler() {
    let registry = ToolRegistry::new(vec![Arc::new(
        EchoHandler::new("db.query").with_alias("db_query"),
    )]);

    let result = futures::executor::block_on(registry.call(
        "db_query",
        json!({ "sql": "select 1" }),
        ToolContext::for_adapter(ToolAdapter::Mcp),
    ))
    .unwrap();

    assert_eq!(json!({ "sql": "select 1" }), result.structured_content);
}

#[test]
fn registry_rejects_duplicate_aliases() {
    let error = ToolRegistry::try_new(vec![
        Arc::new(EchoHandler::new("db.query").with_alias("db_read")),
        Arc::new(EchoHandler::new("db.schema").with_alias("db_read")),
    ])
    .expect_err("duplicate aliases should fail");

    assert_eq!(vec!["db_read"], error.duplicate_tool_ids());
}

#[derive(Clone)]
struct EchoHandler {
    descriptor: ToolDescriptor,
    aliases: Vec<ToolAlias>,
}

impl EchoHandler {
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
                adapters: vec![ToolAdapter::Mcp],
                annotations: ToolAnnotations::read_only("Echo"),
            },
            aliases: Vec::new(),
        }
    }

    fn with_alias(mut self, alias: &str) -> Self {
        self.aliases.push(ToolAlias::new(alias));
        self
    }
}

impl ToolHandler for EchoHandler {
    fn descriptor(&self) -> ToolDescriptor {
        self.descriptor.clone()
    }

    fn aliases(&self) -> Vec<ToolAlias> {
        self.aliases.clone()
    }

    fn call(&self, input: serde_json::Value, _context: ToolContext) -> tool_runtime::ToolFuture {
        Box::pin(async move { Ok(ToolResult::structured(input)) })
    }
}
