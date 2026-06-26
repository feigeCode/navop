use public_mcp::approval::{
    PublicMcpApprovalFuture, PublicMcpApprovalManager, PublicMcpApprovalOutcome,
    PublicMcpApprovalRequest, PublicMcpApprover,
};
use public_mcp::permissions::{PermissionMode, PublicMcpOperationKind};
use public_mcp::tools::{
    InternalFunctionDefinition, PublicMcpToolContext, PublicMcpToolRegistry,
    ToolRuntimeMcpProvider, internal_function_tool_registry,
};
use rmcp::model::JsonObject;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

#[tokio::test]
async fn internal_function_provider_lists_registered_functions() {
    let registry = PublicMcpToolRegistry::new(vec![Arc::new(provider(vec![echo_function()]))]);

    let result = registry
        .call_tool(
            "internal_functions.list",
            None,
            context(PermissionMode::Deny, Default::default()),
        )
        .await
        .expect("list tool should be available");

    assert_eq!(
        Some(json!({
            "functions": [{
                "name": "internal.echo",
                "description": "Echo an input value.",
                "read_only": true,
                "input_schema": { "type": "object" }
            }]
        })),
        result.structured_content
    );
}

#[tokio::test]
async fn read_only_internal_function_calls_are_allowed_in_deny_mode() {
    let registry = PublicMcpToolRegistry::new(vec![Arc::new(provider(vec![echo_function()]))]);

    let result = registry
        .call_tool(
            "internal_functions.call",
            Some(call_args("internal.echo", json!({ "input": "hello" }))),
            context(PermissionMode::Deny, Default::default()),
        )
        .await
        .expect("read-only internal function should run");

    assert_eq!(
        Some(json!({ "result": { "echo": "hello" } })),
        result.structured_content
    );
}

#[tokio::test]
async fn writable_internal_function_calls_follow_permission_mode() {
    let registry = PublicMcpToolRegistry::new(vec![Arc::new(provider(vec![write_function()]))]);

    let denied = registry
        .call_tool(
            "internal_functions.call",
            Some(call_args("internal.write_note", json!({ "note": "ship" }))),
            context(PermissionMode::Deny, Default::default()),
        )
        .await
        .expect("denied call should return structured error");
    assert_eq!(
        Some(json!({
            "code": "permission_denied",
            "message": "tool runtime call denied by permission mode"
        })),
        denied.structured_content
    );

    let approver = Arc::new(RecordingApprover::approved());
    let approved = registry
        .call_tool(
            "internal_functions.call",
            Some(call_args("internal.write_note", json!({ "note": "ship" }))),
            context(
                PermissionMode::Ask,
                PublicMcpApprovalManager::new(approver.clone()),
            ),
        )
        .await
        .expect("approved call should run");

    assert_eq!(
        Some(json!({ "result": { "written": "ship" } })),
        approved.structured_content
    );
    let requests = approver.requests();
    assert_eq!(1, requests.len());
    assert_eq!(
        PublicMcpOperationKind::CallToolRuntimeTool,
        requests[0].operation
    );
    assert_eq!("internal_functions.call", requests[0].tool_name);
}

fn provider(functions: Vec<InternalFunctionDefinition>) -> ToolRuntimeMcpProvider {
    ToolRuntimeMcpProvider::new(internal_function_tool_registry(functions))
}

fn echo_function() -> InternalFunctionDefinition {
    InternalFunctionDefinition::new(
        "internal.echo",
        "Echo an input value.",
        empty_object_schema(),
        true,
        |args| async move {
            Ok(json!({
                "echo": args.get("input").cloned().unwrap_or(Value::Null)
            }))
        },
    )
}

fn write_function() -> InternalFunctionDefinition {
    InternalFunctionDefinition::new(
        "internal.write_note",
        "Write a note into the app.",
        empty_object_schema(),
        false,
        |args| async move {
            Ok(json!({
                "written": args.get("note").cloned().unwrap_or(Value::Null)
            }))
        },
    )
}

fn context(
    permission_mode: PermissionMode,
    approver: PublicMcpApprovalManager,
) -> PublicMcpToolContext {
    PublicMcpToolContext {
        permission_mode,
        approver,
    }
}

fn call_args(name: &str, arguments: Value) -> JsonObject {
    let mut args = JsonObject::new();
    args.insert("name".to_string(), json!(name));
    args.insert("arguments".to_string(), arguments);
    args
}

fn empty_object_schema() -> Arc<JsonObject> {
    let mut schema = JsonObject::new();
    schema.insert("type".to_string(), Value::String("object".to_string()));
    Arc::new(schema)
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
