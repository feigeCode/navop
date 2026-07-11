use std::sync::{Arc, Mutex};

use agent_runtime::model::{MockModelClient, ModelResponse, function_tool_call};
use agent_runtime::{
    ModelClient, ResourceContext, Runtime, RuntimeServices, TaskKind, TaskOutcome,
    ToolExecutionMode, ToolRouter,
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

    fn call(&self, input: Value, _context: ToolContext) -> tool_runtime::ToolFuture {
        self.inputs.lock().expect("inputs lock").push(input.clone());
        Box::pin(async move {
            Ok(ToolResult::structured(json!({
                "ok": true,
                "input": input
            })))
        })
    }
}
