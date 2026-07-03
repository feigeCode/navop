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

#[tokio::test]
async fn auto_tool_mode_requires_confirmation_for_high_risk_tools() {
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

    let call_id = match outcome {
        TaskOutcome::NeedUserInput {
            pending_tool_call_id: Some(call_id),
            ..
        } => call_id,
        other => panic!("high risk tool should pause for approval, got {other:?}"),
    };
    let events = drain_events(&mut rx);
    assert!(events.iter().any(|event| {
        matches!(
            event,
            RuntimeEvent::NeedUserInput {
                pending_tool_call_id: Some(event_call_id),
                tool_name: Some(tool_name),
                arguments: Some(arguments),
                ..
            } if event_call_id == &call_id
                && tool_name.as_str() == "dangerous_write"
                && arguments["sql"] == "drop table users"
        )
    }));
    assert!(
        !events.iter().any(|event| {
            matches!(
                event,
                RuntimeEvent::ObservationAdded { observation, .. }
                    if observation.summary.contains("dangerous write executed")
            )
        }),
        "high risk tool must not dispatch before approval in auto mode"
    );

    let outcome = runtime
        .approve_pending_tool(session.id(), &call_id)
        .await
        .expect("approve pending high risk tool");

    assert!(
        matches!(outcome, TaskOutcome::Completed { answer: Some(answer) } if answer == "危险 SQL 已执行。")
    );
}

#[tokio::test]
async fn auto_tool_mode_batches_sibling_high_risk_approvals() {
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

    let call_id = match outcome {
        TaskOutcome::NeedUserInput {
            pending_tool_call_id: Some(call_id),
            question,
            ..
        } => {
            assert!(question.contains("2 个工具"));
            call_id
        }
        other => panic!("high risk batch should pause for approval, got {other:?}"),
    };
    let events = drain_events(&mut rx);
    assert!(events.iter().any(|event| {
        matches!(
            event,
            RuntimeEvent::NeedUserInput {
                pending_tool_calls,
                ..
            } if pending_tool_calls.len() == 2
                && pending_tool_calls[0].call_id.as_str() == "c_drop_users"
                && pending_tool_calls[1].call_id.as_str() == "c_drop_orders"
        )
    }));

    let outcome = runtime
        .approve_pending_tool(session.id(), &call_id)
        .await
        .expect("approve pending high risk tool batch");

    assert!(
        matches!(outcome, TaskOutcome::Completed { answer: Some(answer) } if answer == "危险 SQL 已执行。")
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
