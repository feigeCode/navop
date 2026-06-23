use public_mcp::approval::{
    PublicMcpApprovalFuture, PublicMcpApprovalManager, PublicMcpApprovalOutcome,
    PublicMcpApprovalRequest, PublicMcpApprover,
};
use public_mcp::permissions::{PermissionMode, PublicMcpOperationKind};
use public_mcp::tools::{
    PublicMcpToolContext, PublicMcpToolFuture, PublicMcpToolProvider, PublicMcpToolRegistry,
};
use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, JsonObject, Tool},
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct EchoToolProvider;

impl PublicMcpToolProvider for EchoToolProvider {
    fn tools(&self) -> Vec<Tool> {
        vec![Tool::new(
            "test.echo",
            "Echo input for registry dispatch tests.",
            empty_object_schema(),
        )]
    }

    fn call_tool(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
        _context: PublicMcpToolContext,
    ) -> Option<PublicMcpToolFuture> {
        if name != "test.echo" {
            return None;
        }
        Some(Box::pin(async move {
            Ok(CallToolResult::structured(json!({
                "echo": arguments
                    .and_then(|args| args.get("input").cloned())
                    .unwrap_or(Value::Null)
            })))
        }))
    }
}

#[derive(Clone)]
struct RecordingApprover {
    outcome: PublicMcpApprovalOutcome,
    requests: Arc<Mutex<Vec<PublicMcpApprovalRequest>>>,
}

impl RecordingApprover {
    fn approved() -> Self {
        Self {
            outcome: PublicMcpApprovalOutcome::Approved,
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn denied(reason: impl Into<String>) -> Self {
        Self {
            outcome: PublicMcpApprovalOutcome::Denied {
                reason: Some(reason.into()),
            },
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
        let outcome = self.outcome.clone();
        Box::pin(async move { outcome })
    }
}

#[test]
fn tool_registry_lists_tools_from_registered_providers() {
    let registry = PublicMcpToolRegistry::new(vec![Arc::new(EchoToolProvider)]);

    let tools = registry.tools();

    assert!(tools.iter().any(|tool| tool.name == "test.echo"));
}

#[test]
fn tool_registry_reports_duplicate_tool_names() {
    let error = match PublicMcpToolRegistry::try_new(vec![
        Arc::new(EchoToolProvider),
        Arc::new(EchoToolProvider),
    ]) {
        Ok(_) => panic!("duplicate tool names should reject registry construction"),
        Err(error) => error,
    };

    assert_eq!(vec!["test.echo".to_string()], error.duplicate_tool_names());
    assert!(error.to_string().contains("test.echo"));
}

#[tokio::test]
async fn tool_registry_dispatches_calls_to_matching_provider() {
    let registry = PublicMcpToolRegistry::new(vec![Arc::new(EchoToolProvider)]);
    let mut args = JsonObject::new();
    args.insert("input".to_string(), json!("hello"));

    let result = registry
        .call_tool(
            "test.echo",
            Some(args),
            PublicMcpToolContext {
                permission_mode: PermissionMode::Deny,
                approver: Default::default(),
            },
        )
        .await
        .expect("registry should dispatch test.echo");

    assert_eq!(Some(json!({ "echo": "hello" })), result.structured_content);
}

#[tokio::test]
async fn tool_registry_rejects_unknown_tools() {
    let registry = PublicMcpToolRegistry::new(vec![Arc::new(EchoToolProvider)]);

    let error = registry
        .call_tool(
            "test.missing",
            None,
            PublicMcpToolContext {
                permission_mode: PermissionMode::Deny,
                approver: Default::default(),
            },
        )
        .await
        .expect_err("unknown tool should be rejected");

    assert!(matches!(error, McpError { .. }));
}

#[tokio::test]
async fn approval_context_allows_requested_operations_when_approved() {
    let approver = Arc::new(RecordingApprover::approved());
    let outcome = PublicMcpToolContext {
        permission_mode: PermissionMode::Ask,
        approver: PublicMcpApprovalManager::new(approver.clone()),
    }
    .request_approval(
        PublicMcpOperationKind::ExecuteRemoteCommand,
        "public_mcp.remote_exec",
        "Execute remote command",
        json!({ "session_id": "terminal-1" }),
    )
    .await;

    assert_eq!(PublicMcpApprovalOutcome::Approved, outcome);
    let requests = approver.requests();
    assert_eq!(1, requests.len());
    assert_eq!(
        PublicMcpOperationKind::ExecuteRemoteCommand,
        requests[0].operation
    );
    assert_eq!("public_mcp.remote_exec", requests[0].tool_name);
}

#[tokio::test]
async fn approval_context_denies_requested_operations_when_denied() {
    let approver = Arc::new(RecordingApprover::denied("operator rejected"));
    let outcome = PublicMcpToolContext {
        permission_mode: PermissionMode::Ask,
        approver: PublicMcpApprovalManager::new(approver),
    }
    .request_approval(
        PublicMcpOperationKind::ExecuteRemoteCommand,
        "public_mcp.remote_exec",
        "Execute remote command",
        json!({ "session_id": "terminal-1" }),
    )
    .await;

    assert_eq!(
        PublicMcpApprovalOutcome::Denied {
            reason: Some("operator rejected".to_string())
        },
        outcome
    );
}

fn empty_object_schema() -> Arc<JsonObject> {
    let mut schema = JsonObject::new();
    schema.insert("type".to_string(), Value::String("object".to_string()));
    Arc::new(schema)
}
