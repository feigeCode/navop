//! `agent_runtime` 端到端集成测试。
//!
//! 用 [`MockModelClient`] 脚本化模型响应 + 内置 [`EchoTool`],在无真实模型与连接的
//! 前提下,验证 codex 风格 `AgentTask` 的统一循环:
//! - 简单问答(模型直接回答,不产生计划)
//! - 模型按需调用 `update_plan` 维护 checklist(产生 `PlanUpdated`)
//! - 模型调用业务工具(echo)后 follow-up 给出最终回答

use std::sync::Arc;

use agent_runtime::model::{MockModelClient, ModelResponse, function_tool_call};
use agent_runtime::tools::builtin::EchoTool;
use agent_runtime::{
    ModelClient, ResourceContext, ResourceKind, ResourceRef, ResourceScope, Runtime,
    RuntimeEvent, RuntimeServices, StepStatus, TaskKind, TaskOutcome, ToolRegistry, ToolRouter,
};
use serde_json::json;

/// 用脚本化模型 + 给定工具注册表构造 Runtime。
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

#[tokio::test]
async fn unknown_session_is_rejected() {
    let runtime = build_runtime(vec![ModelResponse::text("hi")], ToolRegistry::new());
    let missing = agent_runtime::SessionId::new();
    let result = runtime
        .run_turn_blocking(&missing, "hi".into(), TaskKind::Agent)
        .await;
    assert!(result.is_err(), "未知会话应返回错误");
}

#[tokio::test]
async fn agent_simple_question_answers_without_plan() {
    // codex 风格:模型直接给文本回答、不调任何工具 —— 不应产生计划。
    let runtime = build_runtime(
        vec![ModelResponse::text("你好,有什么可以帮你?")],
        ToolRegistry::new(),
    );
    let session = runtime.create_session(ResourceContext::new());
    let mut rx = runtime.subscribe();

    let outcome = runtime
        .run_turn_blocking(session.id(), "你好".into(), TaskKind::Agent)
        .await
        .expect("run agent turn");

    assert!(
        matches!(outcome, TaskOutcome::Completed { answer: Some(a) } if a == "你好,有什么可以帮你?")
    );
    let events = drain_events(&mut rx);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, RuntimeEvent::PlanUpdated { .. })),
        "简单问答不应产生任何计划"
    );
}

#[tokio::test]
async fn agent_uses_update_plan_checklist() {
    // 模型先调用 update_plan 维护清单,再给出最终回答。
    let runtime = build_runtime(
        vec![
            ModelResponse::tool_call(function_tool_call(
                "c_plan",
                "update_plan",
                json!({
                    "plan": [
                        {"step": "查看连接数", "status": "completed"},
                        {"step": "分析慢查询", "status": "in_progress"}
                    ]
                })
                .to_string(),
            )),
            ModelResponse::text("已开始排查。"),
        ],
        ToolRegistry::new(),
    );
    let session = runtime.create_session(ResourceContext::new());
    let mut rx = runtime.subscribe();

    let outcome = runtime
        .run_turn_blocking(session.id(), "排查慢查询".into(), TaskKind::Agent)
        .await
        .expect("run agent turn");

    assert!(matches!(outcome, TaskOutcome::Completed { .. }));
    let events = drain_events(&mut rx);
    let plan = events
        .iter()
        .rev()
        .find_map(|e| match e {
            RuntimeEvent::PlanUpdated { plan, .. } => Some(plan.clone()),
            _ => None,
        })
        .expect("update_plan 应产生 PlanUpdated");
    assert_eq!(plan.steps.len(), 2);
    assert_eq!(plan.steps[0].status, StepStatus::Completed);
    assert_eq!(plan.steps[1].status, StepStatus::Running);
}

#[tokio::test]
async fn agent_calls_business_tool_then_finishes() {
    // 模型调用业务工具(echo),拿到观测后给出最终回答;未调 update_plan 故无计划。
    let runtime = build_runtime(
        vec![
            ModelResponse::tool_call(function_tool_call(
                "c_echo",
                "echo",
                json!({"message": "hello world"}).to_string(),
            )),
            ModelResponse::text("已回显完成。"),
        ],
        ToolRegistry::new().with_tool(Arc::new(EchoTool)),
    );
    let session = runtime.create_session(ResourceContext::new());
    let mut rx = runtime.subscribe();

    let outcome = runtime
        .run_turn_blocking(session.id(), "回显 hello world".into(), TaskKind::Agent)
        .await
        .expect("run agent turn");

    assert!(matches!(outcome, TaskOutcome::Completed { .. }));
    let events = drain_events(&mut rx);
    let observed_ok = events.iter().any(|e| {
        matches!(
            e,
            RuntimeEvent::ObservationAdded { observation, .. }
                if observation.success && observation.summary.contains("hello world")
        )
    });
    assert!(observed_ok, "echo 工具应产生成功观测");
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, RuntimeEvent::PlanUpdated { .. })),
        "未调用 update_plan 不应产生计划"
    );
}

#[tokio::test]
async fn malformed_tool_call_arguments_fail_without_retry_loop() {
    let model = Arc::new(MockModelClient::new([
        ModelResponse::tool_call(function_tool_call("c_bad", "tool", "db.schema")),
        ModelResponse::text("不应进入第二轮"),
    ]));
    let runtime = Runtime::new(RuntimeServices::new(
        model.clone(),
        Arc::new(ToolRouter::new(
            ToolRegistry::new().with_tool(Arc::new(EchoTool)),
        )),
    ));
    let session = runtime.create_session(ResourceContext::new());
    let mut rx = runtime.subscribe();

    let outcome = runtime
        .run_turn_blocking(session.id(), "列出数据库".into(), TaskKind::Agent)
        .await
        .expect("run agent turn");

    assert!(matches!(outcome, TaskOutcome::Failed { reason } if reason.contains("无效工具调用")));
    assert_eq!(1, model.request_count(), "无效 tool call 不应继续循环重试");
    let events = drain_events(&mut rx);
    assert!(events.iter().any(|event| {
        matches!(
            event,
            RuntimeEvent::ObservationAdded { observation, .. }
                if !observation.success && observation.tool_name.as_str() == "tool"
        )
    }));
}

#[tokio::test]
async fn system_prompt_lists_available_tools_and_json_rule() {
    let model = Arc::new(MockModelClient::new([ModelResponse::text("ok")]));
    let runtime = Runtime::new(RuntimeServices::new(
        model.clone(),
        Arc::new(ToolRouter::new(
            ToolRegistry::new().with_tool(Arc::new(EchoTool)),
        )),
    ));
    let session = runtime.create_session(ResourceContext::new());

    runtime
        .run_turn_blocking(session.id(), "hi".into(), TaskKind::Agent)
        .await
        .expect("run agent turn");

    let requests = model.received_requests();
    let system = requests[0].messages[0].content_as_text();
    assert!(system.contains("echo"));
    assert!(system.contains("update_plan"));
    assert!(system.contains("arguments 必须是合法 JSON object"));
    assert!(system.contains("不要调用名为 `tool`"));
}

#[tokio::test]
async fn system_prompt_includes_current_resource_context() {
    let model = Arc::new(MockModelClient::new([ModelResponse::text("ok")]));
    let runtime = Runtime::new(RuntimeServices::new(
        model.clone(),
        Arc::new(ToolRouter::new(
            ToolRegistry::new().with_tool(Arc::new(EchoTool)),
        )),
    ));
    let resources = ResourceContext::new().with_resource(
        ResourceRef::new("db-1", ResourceKind::Postgres, "prod analytics")
            .with_scope(ResourceScope::new("database", "Database", "ai_app"))
            .with_scope(ResourceScope::new("schema", "Schema", "public")),
    );
    let session = runtime.create_session(resources);

    runtime
        .run_turn_blocking(session.id(), "分析当前数据库".into(), TaskKind::Agent)
        .await
        .expect("run agent turn");

    let requests = model.received_requests();
    let system = requests[0].messages[0].content_as_text();
    assert!(system.contains("当前可操作资源"));
    assert!(system.contains("prod analytics"));
    assert!(system.contains("类型=postgres"));
    assert!(system.contains("id=db-1"));
    assert!(system.contains("[当前]"));
    assert!(system.contains("database=ai_app"));
    assert!(system.contains("schema=public"));
}
