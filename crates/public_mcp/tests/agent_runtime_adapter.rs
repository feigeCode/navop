use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use agent_runtime::model::{MockModelClient, ModelResponse, function_tool_call};
use agent_runtime::tools::ToolInvocation;
use agent_runtime::{
    ModelClient, ResourceContext, Runtime, RuntimeServices, SessionId, SkillContext, TaskKind,
    TaskOutcome, ToolCallId, ToolExecutionMode, ToolName, ToolRouter, TurnId,
};
use public_mcp::permissions::PermissionMode;
use public_mcp::tools::{
    PublicMcpToolRegistry, ToolRuntimeMcpProvider, agent_runtime_tool_registry,
};
use serde_json::{Value, json};
use tool_runtime::{
    RiskLevel, ToolAdapter, ToolAnnotations, ToolContext, ToolDescriptor, ToolHandler, ToolMode,
    ToolRegistry, ToolResult,
};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn agent_auto_mode_executes_public_mcp_open_world_tool_without_approval() {
    let handler = Arc::new(RuntimeOpenWorldTool::default());
    let agent_registry = public_mcp_agent_registry(handler.clone());
    let runtime = agent_runtime(agent_registry);
    let session = runtime.create_session(ResourceContext::new());

    let outcome = runtime
        .run_turn_blocking_with_tool_mode(
            session.id(),
            "在终端执行 df -h".into(),
            TaskKind::Agent,
            ToolExecutionMode::Auto,
        )
        .await
        .expect("agent turn should run");

    assert!(matches!(
        outcome,
        TaskOutcome::Completed { answer: Some(answer) } if answer == "磁盘信息已查看。"
    ));
    assert_eq!(1, handler.call_count());
    assert_eq!(json!({ "command": "df -h" }), handler.last_input());
}

#[tokio::test]
async fn agent_runtime_fails_closed_on_malformed_public_mcp_schema() {
    let handler = Arc::new(MalformedSchemaTool::default());
    let provider = ToolRuntimeMcpProvider::new(ToolRegistry::new(vec![handler.clone()]));
    let public_registry = PublicMcpToolRegistry::new(vec![Arc::new(provider)]);
    let agent_registry =
        agent_runtime_tool_registry(public_registry, PermissionMode::Deny, Default::default());
    let model = Arc::new(MockModelClient::new([ModelResponse::text(
        "the malformed schema must prevent this request",
    )]));
    let runtime = Runtime::new(RuntimeServices::new(
        model.clone(),
        Arc::new(ToolRouter::new(agent_registry)),
    ));
    let session = runtime.create_session(ResourceContext::new());

    let outcome = runtime
        .run_turn_blocking_with_tool_mode(
            session.id(),
            "try the malformed tool".into(),
            TaskKind::Agent,
            ToolExecutionMode::Auto,
        )
        .await
        .expect("schema incompatibility should be reported as a task outcome");

    let TaskOutcome::Failed { reason } = outcome else {
        panic!("malformed schema must fail closed before contacting the model");
    };
    assert!(reason.contains("incompatible function-calling schema"));
    assert!(reason.contains("/type"));
    assert!(reason.contains("root schema must declare type \"object\""));
    assert_eq!(0, model.request_count());
    assert_eq!(0, handler.call_count());
}

#[tokio::test]
async fn agent_adapter_forwards_turn_cancellation_to_public_mcp_runtime_tool() {
    let handler = Arc::new(RuntimeOpenWorldTool::default());
    let agent_registry = public_mcp_agent_registry(handler.clone());
    let tool = agent_registry
        .get(&ToolName::new("terminal.exec"))
        .expect("public MCP runtime tool should be exposed");
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    tool.execute(ToolInvocation {
        session_id: SessionId::from_string("session-1"),
        turn_id: TurnId::from_string("turn-1"),
        call_id: ToolCallId::from_string("call-1"),
        tool_name: ToolName::new("terminal.exec"),
        arguments: json!({ "command": "pwd" }),
        resource_id: None,
        resources: ResourceContext::new(),
        skills: SkillContext::new(),
        cancellation,
    })
    .await
    .expect("direct adapter call should reach the runtime tool");

    assert!(handler.last_cancellation_state());
}

fn public_mcp_agent_registry(handler: Arc<RuntimeOpenWorldTool>) -> agent_runtime::ToolRegistry {
    let provider = ToolRuntimeMcpProvider::new(ToolRegistry::new(vec![handler]));
    let public_registry = PublicMcpToolRegistry::new(vec![Arc::new(provider)]);
    agent_runtime_tool_registry(public_registry, PermissionMode::Deny, Default::default())
}

fn agent_runtime(agent_registry: agent_runtime::ToolRegistry) -> Runtime {
    let model: Arc<dyn ModelClient> = Arc::new(MockModelClient::new(vec![
        ModelResponse::tool_call(function_tool_call(
            "call_terminal",
            "terminal_exec",
            json!({ "command": "df -h" }).to_string(),
        )),
        ModelResponse::text("磁盘信息已查看。"),
    ]));
    Runtime::new(RuntimeServices::new(
        model,
        Arc::new(ToolRouter::new(agent_registry)),
    ))
}

#[derive(Clone, Default)]
struct RuntimeOpenWorldTool {
    inputs: Arc<Mutex<Vec<Value>>>,
    last_cancellation_state: Arc<AtomicBool>,
}

impl RuntimeOpenWorldTool {
    fn call_count(&self) -> usize {
        self.inputs.lock().expect("inputs lock").len()
    }

    fn last_input(&self) -> Value {
        self.inputs
            .lock()
            .expect("inputs lock")
            .last()
            .cloned()
            .expect("tool should have been called")
    }

    fn last_cancellation_state(&self) -> bool {
        self.last_cancellation_state.load(Ordering::Acquire)
    }
}

#[derive(Clone, Default)]
struct MalformedSchemaTool {
    calls: Arc<Mutex<usize>>,
}

impl MalformedSchemaTool {
    fn call_count(&self) -> usize {
        *self.calls.lock().expect("calls lock")
    }
}

impl ToolHandler for MalformedSchemaTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            id: "malformed.schema".to_string(),
            title: "Malformed schema".to_string(),
            description: "A test tool whose root input schema is not an object.".to_string(),
            input_schema: json!({ "type": "string" }),
            output_schema: json!({ "type": "object" }),
            permissions: Vec::new(),
            mode: ToolMode::Deterministic,
            adapters: vec![ToolAdapter::Mcp],
            annotations: ToolAnnotations {
                title: "Malformed schema".to_string(),
                read_only: true,
                destructive: false,
                idempotent: true,
                open_world: false,
                supports_parallel: false,
                risk: RiskLevel::Low,
            },
        }
    }

    fn call(&self, _input: Value, _context: ToolContext) -> tool_runtime::ToolFuture {
        *self.calls.lock().expect("calls lock") += 1;
        Box::pin(async { Ok(ToolResult::structured(json!({ "unexpected": true }))) })
    }
}

impl ToolHandler for RuntimeOpenWorldTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            id: "terminal.exec".to_string(),
            title: "Execute in terminal".to_string(),
            description: "Execute shell-like input in a visible terminal.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" }
                },
                "required": ["command"]
            }),
            output_schema: json!({ "type": "object" }),
            permissions: Vec::new(),
            mode: ToolMode::Deterministic,
            adapters: vec![ToolAdapter::Mcp],
            annotations: ToolAnnotations {
                title: "Execute in terminal".to_string(),
                read_only: false,
                destructive: false,
                idempotent: false,
                open_world: true,
                supports_parallel: false,
                risk: RiskLevel::High,
            },
        }
    }

    fn call(&self, input: Value, context: ToolContext) -> tool_runtime::ToolFuture {
        self.inputs.lock().expect("inputs lock").push(input.clone());
        self.last_cancellation_state
            .store(context.cancellation.is_cancelled(), Ordering::Release);
        Box::pin(async move {
            Ok(ToolResult::structured(json!({
                "ok": true,
                "input": input
            })))
        })
    }
}
