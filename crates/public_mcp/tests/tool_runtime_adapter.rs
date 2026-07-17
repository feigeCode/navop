use std::sync::Arc;

use public_mcp::approval::{
    PublicMcpApprovalFuture, PublicMcpApprovalManager, PublicMcpApprovalOutcome,
    PublicMcpApprovalRequest, PublicMcpApprover,
};
use public_mcp::{
    permissions::PermissionMode,
    tools::{PublicMcpToolContext, PublicMcpToolRegistry, ToolRuntimeMcpProvider},
};
use serde_json::json;
use std::sync::Mutex;
use tool_runtime::{
    ToolAdapter, ToolAnnotations, ToolContext, ToolDescriptor, ToolHandler, ToolMode, ToolRegistry,
    ToolResult,
};

#[test]
fn tool_runtime_provider_exposes_mcp_tools_and_dispatches_calls() {
    let provider = ToolRuntimeMcpProvider::new(ToolRegistry::new(vec![Arc::new(RuntimeEchoTool)]));
    let registry = PublicMcpToolRegistry::new(vec![Arc::new(provider)]);

    let tools = registry.tools();
    assert_eq!(1, tools.len());
    assert_eq!("example.echo", tools[0].name);
    assert_eq!(Some("Echo input"), tools[0].description.as_deref());

    let result = futures::executor::block_on(registry.call_tool(
        "example.echo",
        Some(rmcp::model::JsonObject::from_iter([(
            "message".to_string(),
            json!("hello"),
        )])),
        PublicMcpToolContext {
            permission_mode: PermissionMode::Deny,
            approver: Default::default(),
        },
    ))
    .expect("tool runtime MCP provider should dispatch call");

    assert_eq!(
        Some(json!({ "message": "hello" })),
        result.structured_content
    );
}

#[test]
fn tool_runtime_provider_denies_mutating_tools_in_deny_mode() {
    let provider = ToolRuntimeMcpProvider::new(ToolRegistry::new(vec![Arc::new(RuntimeWriteTool)]));
    let registry = PublicMcpToolRegistry::new(vec![Arc::new(provider)]);

    let result = futures::executor::block_on(registry.call_tool(
        "example.write",
        Some(rmcp::model::JsonObject::from_iter([(
            "password".to_string(),
            json!("secret"),
        )])),
        PublicMcpToolContext {
            permission_mode: PermissionMode::Deny,
            approver: Default::default(),
        },
    ))
    .expect("denied mutating tool should return structured error");

    assert_eq!(
        Some(json!({
            "code": "permission_denied",
            "message": "tool runtime call denied by permission mode; set MCP Permission Profile to Confirm or Auto"
        })),
        result.structured_content
    );
}

#[test]
fn tool_runtime_provider_requests_approval_for_mutating_tools_and_redacts_secrets() {
    let provider = ToolRuntimeMcpProvider::new(ToolRegistry::new(vec![Arc::new(RuntimeWriteTool)]));
    let registry = PublicMcpToolRegistry::new(vec![Arc::new(provider)]);
    let approver = Arc::new(RecordingApprover::approved());

    let result = futures::executor::block_on(registry.call_tool(
        "example.write",
        Some(rmcp::model::JsonObject::from_iter([
            ("message".to_string(), json!("ship")),
            ("password".to_string(), json!("secret")),
        ])),
        PublicMcpToolContext {
            permission_mode: PermissionMode::Ask,
            approver: PublicMcpApprovalManager::new(approver.clone()),
        },
    ))
    .expect("approved mutating tool should run");

    assert_eq!(
        Some(json!({ "written": "ship" })),
        result.structured_content
    );
    let requests = approver.requests();
    assert_eq!(1, requests.len());
    assert_eq!("example.write", requests[0].tool_name);
    assert_eq!(
        json!({
            "tool": "example.write",
            "requestArguments": {
                "message": "ship",
                "password": "<redacted>"
            },
            "arguments": {
                "message": "ship",
                "password": "<redacted>"
            }
        }),
        requests[0].details
    );
}

#[test]
fn tool_runtime_provider_asks_for_high_risk_tools_in_allow_mode() {
    let provider = ToolRuntimeMcpProvider::new(ToolRegistry::new(vec![Arc::new(RuntimeWriteTool)]));
    let registry = PublicMcpToolRegistry::new(vec![Arc::new(provider)]);
    let approver = Arc::new(RecordingApprover::approved());

    let result = futures::executor::block_on(registry.call_tool(
        "example.write",
        Some(rmcp::model::JsonObject::from_iter([(
            "message".to_string(),
            json!("ship"),
        )])),
        PublicMcpToolContext {
            permission_mode: PermissionMode::Allow,
            approver: PublicMcpApprovalManager::new(approver.clone()),
        },
    ))
    .expect("approved high-risk runtime tool should run");

    assert_eq!(
        Some(json!({ "written": "ship" })),
        result.structured_content
    );
    let requests = approver.requests();
    assert_eq!(1, requests.len());
    assert_eq!("example.write", requests[0].tool_name);
}

#[derive(Clone)]
struct RuntimeEchoTool;

impl ToolHandler for RuntimeEchoTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            id: "example.echo".to_string(),
            title: "Echo".to_string(),
            description: "Echo input".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string" }
                },
                "required": ["message"]
            }),
            output_schema: json!({ "type": "object" }),
            permissions: Vec::new(),
            mode: ToolMode::Deterministic,
            adapters: vec![ToolAdapter::Mcp],
            annotations: ToolAnnotations::read_only("Echo"),
        }
    }

    fn call(&self, input: serde_json::Value, _context: ToolContext) -> tool_runtime::ToolFuture {
        Box::pin(async move { Ok(ToolResult::structured(input)) })
    }
}

#[derive(Clone)]
struct RuntimeWriteTool;

impl ToolHandler for RuntimeWriteTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            id: "example.write".to_string(),
            title: "Write".to_string(),
            description: "Write input".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string" },
                    "password": { "type": "string" }
                }
            }),
            output_schema: json!({ "type": "object" }),
            permissions: Vec::new(),
            mode: ToolMode::Deterministic,
            adapters: vec![ToolAdapter::Mcp],
            annotations: ToolAnnotations::mutating("Write"),
        }
    }

    fn call(&self, input: serde_json::Value, _context: ToolContext) -> tool_runtime::ToolFuture {
        Box::pin(async move {
            Ok(ToolResult::structured(json!({
                "written": input.get("message").cloned().unwrap_or(serde_json::Value::Null)
            })))
        })
    }
}

#[derive(Clone)]
struct RecordingApprover {
    requests: Arc<Mutex<Vec<PublicMcpApprovalRequest>>>,
}

impl RecordingApprover {
    fn approved() -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn requests(&self) -> Vec<PublicMcpApprovalRequest> {
        self.requests
            .lock()
            .expect("requests lock poisoned")
            .clone()
    }
}

impl PublicMcpApprover for RecordingApprover {
    fn request_approval(&self, request: PublicMcpApprovalRequest) -> PublicMcpApprovalFuture {
        self.requests
            .lock()
            .expect("requests lock poisoned")
            .push(request);
        Box::pin(async { PublicMcpApprovalOutcome::Approved })
    }
}
