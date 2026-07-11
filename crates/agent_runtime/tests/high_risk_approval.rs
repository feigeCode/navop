use std::sync::Arc;

use agent_runtime::error::ToolError;
use agent_runtime::model::{MockModelClient, ModelResponse, function_tool_call};
use agent_runtime::tools::{
    ObservationData, Tool, ToolInvocation, ToolName, ToolObservation, ToolSpec,
};
use agent_runtime::{
    ModelClient, ResourceContext, RiskLevel, Runtime, RuntimeEvent, RuntimeServices, TaskKind,
    TaskOutcome, ToolExecutionMode, ToolRegistry, ToolRouter,
};
use async_trait::async_trait;
use serde_json::json;

struct HighRiskTool;
struct CriticalRiskTool;

#[async_trait]
impl Tool for HighRiskTool {
    fn name(&self) -> ToolName {
        ToolName::new("dangerous_write")
    }

    fn spec(&self, _resources: &ResourceContext) -> ToolSpec {
        ToolSpec::new(
            "dangerous_write",
            "执行高风险写入。",
            json!({
                "type": "object",
                "properties": {"sql": {"type": "string"}},
                "required": ["sql"]
            }),
        )
        .with_risk(RiskLevel::High)
    }

    async fn execute(&self, invocation: ToolInvocation) -> Result<ToolObservation, ToolError> {
        Ok(ToolObservation::success(
            invocation.call_id,
            invocation.tool_name,
            "dangerous write executed",
            ObservationData::Text("executed".into()),
        ))
    }
}

#[async_trait]
impl Tool for CriticalRiskTool {
    fn name(&self) -> ToolName {
        ToolName::new("critical_write")
    }

    fn spec(&self, _resources: &ResourceContext) -> ToolSpec {
        ToolSpec::new(
            "critical_write",
            "执行关键风险写入。",
            json!({ "type": "object" }),
        )
        .with_risk(RiskLevel::Critical)
    }

    async fn execute(&self, invocation: ToolInvocation) -> Result<ToolObservation, ToolError> {
        Ok(ToolObservation::success(
            invocation.call_id,
            invocation.tool_name,
            "critical write executed",
            ObservationData::Text("executed".into()),
        ))
    }
}

#[tokio::test]
async fn auto_tool_mode_executes_high_risk_tools_without_confirmation() {
    let runtime = build_runtime(
        vec![
            ModelResponse::tool_call(function_tool_call(
                "c_danger",
                "dangerous_write",
                json!({"sql": "drop table users"}).to_string(),
            )),
            ModelResponse::text("危险 SQL 已执行。"),
        ],
        ToolRegistry::new().with_tool(Arc::new(HighRiskTool)),
    );
    let session = runtime.create_session(ResourceContext::new());
    let mut rx = runtime.subscribe();

    let outcome = runtime
        .run_turn_blocking_with_tool_mode(
            session.id(),
            "删除 users 表".into(),
            TaskKind::Agent,
            ToolExecutionMode::Auto,
        )
        .await
        .expect("run auto turn");

    assert!(
        matches!(outcome, TaskOutcome::Completed { answer: Some(answer) } if answer == "危险 SQL 已执行。")
    );
    let events = drain_events(&mut rx);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, RuntimeEvent::NeedUserInput { .. })),
        "auto mode must not pause high-risk tools for confirmation"
    );
    assert!(events.iter().any(|event| {
        matches!(
            event,
            RuntimeEvent::ObservationAdded { observation, .. }
                if observation.summary == "dangerous write executed"
        )
    }));
}

#[tokio::test]
async fn auto_tool_mode_executes_sibling_high_risk_tools_without_confirmation() {
    let runtime = build_runtime(
        vec![
            ModelResponse::tool_calls(vec![
                function_tool_call(
                    "c_drop_users",
                    "dangerous_write",
                    json!({"sql": "drop table users"}).to_string(),
                ),
                function_tool_call(
                    "c_drop_orders",
                    "dangerous_write",
                    json!({"sql": "drop table orders"}).to_string(),
                ),
            ]),
            ModelResponse::text("危险 SQL 已执行。"),
        ],
        ToolRegistry::new().with_tool(Arc::new(HighRiskTool)),
    );
    let session = runtime.create_session(ResourceContext::new());
    let mut rx = runtime.subscribe();

    let outcome = runtime
        .run_turn_blocking_with_tool_mode(
            session.id(),
            "删除 users 和 orders 表".into(),
            TaskKind::Agent,
            ToolExecutionMode::Auto,
        )
        .await
        .expect("run auto turn");

    assert!(
        matches!(outcome, TaskOutcome::Completed { answer: Some(answer) } if answer == "危险 SQL 已执行。")
    );
    let events = drain_events(&mut rx);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, RuntimeEvent::NeedUserInput { .. })),
        "auto mode must not batch high-risk tools for approval"
    );
    let observations = session
        .history_snapshot()
        .items()
        .iter()
        .filter_map(|item| match item {
            agent_runtime::HistoryItem::Observation(obs) => Some(obs.summary.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        vec!["dangerous write executed", "dangerous write executed"],
        observations
    );
}

#[tokio::test]
async fn auto_tool_mode_executes_critical_tools_without_confirmation() {
    let runtime = build_runtime(
        vec![
            ModelResponse::tool_call(function_tool_call("c_critical", "critical_write", "{}")),
            ModelResponse::text("关键操作已执行。"),
        ],
        ToolRegistry::new().with_tool(Arc::new(CriticalRiskTool)),
    );
    let session = runtime.create_session(ResourceContext::new());
    let mut rx = runtime.subscribe();

    let outcome = runtime
        .run_turn_blocking_with_tool_mode(
            session.id(),
            "执行关键操作".into(),
            TaskKind::Agent,
            ToolExecutionMode::Auto,
        )
        .await
        .expect("run auto critical turn");

    assert!(
        matches!(outcome, TaskOutcome::Completed { answer: Some(answer) } if answer == "关键操作已执行。")
    );
    let events = drain_events(&mut rx);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, RuntimeEvent::NeedUserInput { .. }))
    );
    assert!(events.iter().any(|event| {
        matches!(
            event,
            RuntimeEvent::ObservationAdded { observation, .. }
                if observation.summary == "critical write executed"
        )
    }));
}

fn build_runtime(responses: Vec<ModelResponse>, registry: ToolRegistry) -> Runtime {
    let model: Arc<dyn ModelClient> = Arc::new(MockModelClient::new(responses));
    let tools = Arc::new(ToolRouter::new(registry));
    Runtime::new(RuntimeServices::new(model, tools))
}

fn drain_events(rx: &mut agent_runtime::RuntimeEventReceiver) -> Vec<RuntimeEvent> {
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }
    events
}
