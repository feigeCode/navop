//! `agent_runtime` 端到端集成测试。
//!
//! 用 [`MockModelClient`] 脚本化模型响应 + 内置 [`EchoTool`],在无真实模型与连接的
//! 前提下,验证 codex 风格 `AgentTask` 的统一循环:
//! - 简单问答(模型直接回答,不产生计划)
//! - 模型按需调用 `update_plan` 维护 checklist(产生 `PlanUpdated`)
//! - 模型调用业务工具(echo)后 follow-up 给出最终回答

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use agent_runtime::error::ToolError;
use agent_runtime::model::{
    MockModelClient, ModelRequest, ModelResponse, ModelStream, ModelStreamEvent, function_tool_call,
};
use agent_runtime::tools::builtin::EchoTool;
use agent_runtime::tools::{
    ObservationData, Tool, ToolInvocation, ToolName, ToolObservation, ToolSpec,
};
use agent_runtime::{
    ModelClient, ResourceContext, ResourceKind, ResourceRef, ResourceScope, RiskLevel, Runtime,
    RuntimeError, RuntimeEvent, RuntimeServices, StepStatus, TaskKind, TaskOutcome,
    ToolExecutionMode, ToolRegistry, ToolRouter,
};
use async_trait::async_trait;
use serde_json::json;
use tokio::sync::{Barrier, Notify};

/// 用脚本化模型 + 给定工具注册表构造 Runtime。
fn build_runtime(responses: Vec<ModelResponse>, registry: ToolRegistry) -> Runtime {
    let model: Arc<dyn ModelClient> = Arc::new(MockModelClient::new(responses));
    let tools = Arc::new(ToolRouter::new(registry));
    Runtime::new(RuntimeServices::new(model, tools))
}

struct WriteTool;

struct ParallelProbeTool {
    name: &'static str,
    barrier: Arc<Barrier>,
    started: Arc<AtomicUsize>,
    delay_after_barrier: Duration,
}

impl ParallelProbeTool {
    fn new(name: &'static str, barrier: Arc<Barrier>, started: Arc<AtomicUsize>) -> Self {
        Self::new_with_delay(name, barrier, started, Duration::ZERO)
    }

    fn new_with_delay(
        name: &'static str,
        barrier: Arc<Barrier>,
        started: Arc<AtomicUsize>,
        delay_after_barrier: Duration,
    ) -> Self {
        Self {
            name,
            barrier,
            started,
            delay_after_barrier,
        }
    }
}

#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> ToolName {
        ToolName::new("write_data")
    }

    fn spec(&self, _resources: &ResourceContext) -> ToolSpec {
        ToolSpec::new(
            "write_data",
            "写入测试数据。",
            json!({
                "type": "object",
                "properties": {
                    "value": {"type": "string"}
                },
                "required": ["value"]
            }),
        )
        .with_risk(RiskLevel::Low)
    }

    async fn execute(&self, invocation: ToolInvocation) -> Result<ToolObservation, ToolError> {
        Ok(ToolObservation::success(
            invocation.call_id,
            invocation.tool_name,
            "write executed",
            ObservationData::Text("executed".into()),
        ))
    }
}

#[async_trait]
impl Tool for ParallelProbeTool {
    fn name(&self) -> ToolName {
        ToolName::new(self.name)
    }

    fn spec(&self, _resources: &ResourceContext) -> ToolSpec {
        ToolSpec::new(
            self.name,
            "并发探测工具。",
            json!({
                "type": "object",
                "properties": {}
            }),
        )
        .with_risk(RiskLevel::Low)
    }

    fn supports_parallel(&self) -> bool {
        true
    }

    async fn execute(&self, invocation: ToolInvocation) -> Result<ToolObservation, ToolError> {
        self.started.fetch_add(1, Ordering::SeqCst);
        self.barrier.wait().await;
        if !self.delay_after_barrier.is_zero() {
            tokio::time::sleep(self.delay_after_barrier).await;
        }
        Ok(ToolObservation::success(
            invocation.call_id,
            invocation.tool_name,
            format!("{} executed", self.name),
            ObservationData::Text("ok".into()),
        ))
    }
}

struct PromptOnlyTool {
    name: &'static str,
    description: &'static str,
    risk: RiskLevel,
}

impl PromptOnlyTool {
    fn new(name: &'static str, description: &'static str, risk: RiskLevel) -> Self {
        Self {
            name,
            description,
            risk,
        }
    }
}

#[async_trait]
impl Tool for PromptOnlyTool {
    fn name(&self) -> ToolName {
        ToolName::new(self.name)
    }

    fn spec(&self, _resources: &ResourceContext) -> ToolSpec {
        ToolSpec::new(
            self.name,
            self.description,
            json!({
                "type": "object",
                "properties": {
                    "target": {"type": "string"},
                    "command": {"type": "string"}
                },
                "required": ["target", "command"]
            }),
        )
        .with_risk(self.risk)
    }

    async fn execute(&self, invocation: ToolInvocation) -> Result<ToolObservation, ToolError> {
        Ok(ToolObservation::success(
            invocation.call_id,
            invocation.tool_name,
            "prompt only tool executed",
            ObservationData::Text("ok".into()),
        ))
    }
}

fn drain_events(rx: &mut agent_runtime::RuntimeEventReceiver) -> Vec<RuntimeEvent> {
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }
    events
}

struct ReasoningStreamModel;

#[async_trait]
impl ModelClient for ReasoningStreamModel {
    async fn complete(&self, _request: ModelRequest) -> Result<ModelResponse, RuntimeError> {
        Ok(ModelResponse::text("这是最终回答。"))
    }

    async fn complete_stream(&self, _request: ModelRequest) -> Result<ModelStream, RuntimeError> {
        let response = ModelResponse::text("这是最终回答。");
        Ok(Box::pin(futures::stream::iter([
            Ok(ModelStreamEvent::ReasoningDelta("先判断问题边界。".into())),
            Ok(ModelStreamEvent::TextDelta("这是最终回答。".into())),
            Ok(ModelStreamEvent::Completed(response)),
        ])))
    }

    fn model_name(&self) -> &str {
        "reasoning-stream"
    }
}

struct ReasoningToolFollowupModel {
    count: AtomicUsize,
    requests: Mutex<Vec<ModelRequest>>,
}

struct MultiToolReasoningFollowupModel {
    count: AtomicUsize,
    requests: Mutex<Vec<ModelRequest>>,
}

impl ReasoningToolFollowupModel {
    fn new() -> Self {
        Self {
            count: AtomicUsize::new(0),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn received_requests(&self) -> Vec<ModelRequest> {
        self.requests.lock().expect("requests lock").clone()
    }
}

impl MultiToolReasoningFollowupModel {
    fn new() -> Self {
        Self {
            count: AtomicUsize::new(0),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn received_requests(&self) -> Vec<ModelRequest> {
        self.requests.lock().expect("requests lock").clone()
    }
}

#[async_trait]
impl ModelClient for ReasoningToolFollowupModel {
    async fn complete(&self, _request: ModelRequest) -> Result<ModelResponse, RuntimeError> {
        Ok(ModelResponse::text("unused"))
    }

    async fn complete_stream(&self, request: ModelRequest) -> Result<ModelStream, RuntimeError> {
        self.requests.lock().expect("requests lock").push(request);
        match self.count.fetch_add(1, Ordering::SeqCst) {
            0 => {
                let call =
                    function_tool_call("call_echo", "echo", json!({"text": "hello"}).to_string());
                Ok(Box::pin(futures::stream::iter([
                    Ok(ModelStreamEvent::ReasoningDelta("需要调用工具。".into())),
                    Ok(ModelStreamEvent::ToolCall(call.clone())),
                    Ok(ModelStreamEvent::Completed(ModelResponse::tool_call(call))),
                ])))
            }
            _ => Ok(agent_runtime::model::model_response_into_stream(
                ModelResponse::text("工具调用完成。"),
            )),
        }
    }

    fn model_name(&self) -> &str {
        "reasoning-tool-followup"
    }
}

#[async_trait]
impl ModelClient for MultiToolReasoningFollowupModel {
    async fn complete(&self, _request: ModelRequest) -> Result<ModelResponse, RuntimeError> {
        Ok(ModelResponse::text("unused"))
    }

    async fn complete_stream(&self, request: ModelRequest) -> Result<ModelStream, RuntimeError> {
        self.requests.lock().expect("requests lock").push(request);
        match self.count.fetch_add(1, Ordering::SeqCst) {
            0 => {
                let first =
                    function_tool_call("call_echo_1", "echo", json!({"text": "hello"}).to_string());
                let second =
                    function_tool_call("call_echo_2", "echo", json!({"text": "world"}).to_string());
                Ok(Box::pin(futures::stream::iter([
                    Ok(ModelStreamEvent::ReasoningDelta(
                        "需要调用两个工具。".into(),
                    )),
                    Ok(ModelStreamEvent::ToolCall(first.clone())),
                    Ok(ModelStreamEvent::ToolCall(second.clone())),
                    Ok(ModelStreamEvent::Completed(ModelResponse::tool_calls(
                        vec![first, second],
                    ))),
                ])))
            }
            _ => Ok(agent_runtime::model::model_response_into_stream(
                ModelResponse::text("工具调用完成。"),
            )),
        }
    }

    fn model_name(&self) -> &str {
        "multi-tool-reasoning-followup"
    }
}

struct PendingStreamModel {
    called: Arc<Notify>,
}

#[async_trait]
impl ModelClient for PendingStreamModel {
    async fn complete(&self, _request: ModelRequest) -> Result<ModelResponse, RuntimeError> {
        Ok(ModelResponse::text("unused"))
    }

    async fn complete_stream(&self, _request: ModelRequest) -> Result<ModelStream, RuntimeError> {
        self.called.notify_waiters();
        futures::future::pending().await
    }

    fn model_name(&self) -> &str {
        "pending-stream"
    }
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
async fn reasoning_stream_events_are_forwarded_to_runtime_events() {
    let model: Arc<dyn ModelClient> = Arc::new(ReasoningStreamModel);
    let runtime = Runtime::new(RuntimeServices::new(
        model,
        Arc::new(ToolRouter::new(ToolRegistry::new())),
    ));
    let session = runtime.create_session(ResourceContext::new());
    let mut rx = runtime.subscribe();

    runtime
        .run_turn_blocking(session.id(), "解释执行计划".into(), TaskKind::Ask)
        .await
        .expect("run ask turn");

    let events = drain_events(&mut rx);
    assert!(events.iter().any(|event| {
        matches!(
            event,
            RuntimeEvent::ReasoningDelta { delta, .. } if delta == "先判断问题边界。"
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            RuntimeEvent::AssistantMessage { text, .. } if text == "这是最终回答。"
        )
    }));
    assert!(session.history_snapshot().items().iter().any(|item| {
        matches!(
            item,
            agent_runtime::HistoryItem::AssistantWithReasoning { text, reasoning }
                if text == "这是最终回答。" && reasoning == "先判断问题边界。"
        )
    }));
}

#[tokio::test]
async fn reasoning_before_tool_call_is_passed_back_in_followup_request() {
    let model = Arc::new(ReasoningToolFollowupModel::new());
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool));
    let runtime = Runtime::new(RuntimeServices::new(
        model.clone(),
        Arc::new(ToolRouter::new(registry)),
    ));
    let session = runtime.create_session(ResourceContext::new());

    runtime
        .run_turn_blocking(session.id(), "调用 echo".into(), TaskKind::Agent)
        .await
        .expect("run agent turn");

    let requests = model.received_requests();
    assert_eq!(2, requests.len());
    assert!(
        requests[1].messages.iter().any(|message| {
            message.tool_calls.is_some()
                && message.reasoning_content.as_deref() == Some("需要调用工具。")
        }),
        "follow-up request must pass reasoning_content back with assistant tool call"
    );
}

#[tokio::test]
async fn reasoning_before_multiple_tool_calls_is_grouped_in_followup_request() {
    let model = Arc::new(MultiToolReasoningFollowupModel::new());
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool));
    let runtime = Runtime::new(RuntimeServices::new(
        model.clone(),
        Arc::new(ToolRouter::new(registry)),
    ));
    let session = runtime.create_session(ResourceContext::new());

    runtime
        .run_turn_blocking(session.id(), "调用两次 echo".into(), TaskKind::Agent)
        .await
        .expect("run agent turn");

    let requests = model.received_requests();
    assert_eq!(2, requests.len());
    let assistant_tool_message = requests[1]
        .messages
        .iter()
        .find(|message| {
            message
                .tool_calls
                .as_ref()
                .is_some_and(|calls| calls.len() == 2)
        })
        .expect("follow-up request should group sibling tool calls");
    assert_eq!(
        assistant_tool_message.reasoning_content.as_deref(),
        Some("需要调用两个工具。")
    );
}

#[tokio::test]
async fn custom_system_instruction_is_included_in_model_prompt() {
    let model = Arc::new(MockModelClient::new([ModelResponse::text("直接回答。")]));
    let runtime = Runtime::new(RuntimeServices::new(
        model.clone(),
        Arc::new(ToolRouter::new(ToolRegistry::new())),
    ));
    let session = runtime.create_session(ResourceContext::new());
    session.set_system_instruction(Some("始终用 DBA 视角回答。".into()));

    runtime
        .run_turn_blocking(session.id(), "解释索引".into(), TaskKind::Ask)
        .await
        .expect("run ask turn");

    let requests = model.received_requests();
    assert_eq!(1, requests.len());
    let system = requests[0].messages[0].content_as_text();
    assert!(system.contains("始终用 DBA 视角回答。"));
}

#[tokio::test]
async fn interrupt_cancels_turn_while_model_stream_is_starting() {
    let called = Arc::new(Notify::new());
    let model: Arc<dyn ModelClient> = Arc::new(PendingStreamModel {
        called: called.clone(),
    });
    let runtime = Runtime::new(RuntimeServices::new(
        model,
        Arc::new(ToolRouter::new(ToolRegistry::new())),
    ));
    let session = runtime.create_session(ResourceContext::new());
    let mut rx = runtime.subscribe();

    runtime
        .start_turn(session.id(), "hello".into(), TaskKind::Ask)
        .expect("start turn");
    tokio::time::timeout(Duration::from_secs(1), called.notified())
        .await
        .expect("model should be called");
    runtime.interrupt(session.id()).expect("interrupt turn");

    let event = tokio::time::timeout(Duration::from_millis(200), async {
        loop {
            if let RuntimeEvent::TurnFailed { reason, .. } = rx.recv().await.unwrap() {
                break reason;
            }
        }
    })
    .await
    .expect("interrupt should emit TurnFailed promptly");
    assert_eq!("任务已取消", event);
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
async fn agent_delegate_task_runs_isolated_subagent_and_emits_events() {
    let model = Arc::new(MockModelClient::new([
        ModelResponse::tool_call(function_tool_call(
            "c_sub",
            "delegate_task",
            json!({
                "name": "reviewer",
                "task": "检查 agent runtime 的事件流"
            })
            .to_string(),
        )),
        ModelResponse::text("子代理结论: reasoning 没有转发。"),
        ModelResponse::text("已根据子代理结论完成修复建议。"),
    ]));
    let runtime = Runtime::new(RuntimeServices::new(
        model.clone(),
        Arc::new(ToolRouter::new(ToolRegistry::new())),
    ));
    let session = runtime.create_session(ResourceContext::new());
    let mut rx = runtime.subscribe();

    let outcome = runtime
        .run_turn_blocking(session.id(), "排查 agent runtime".into(), TaskKind::Agent)
        .await
        .expect("run agent turn");

    assert!(matches!(outcome, TaskOutcome::Completed { .. }));
    assert_eq!(3, model.request_count());
    let events = drain_events(&mut rx);
    assert!(events.iter().any(|event| {
        matches!(
            event,
            RuntimeEvent::SubAgentStarted { name, task, .. }
                if name == "reviewer" && task == "检查 agent runtime 的事件流"
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            RuntimeEvent::SubAgentFinished { success: true, summary, .. }
                if summary.contains("reasoning 没有转发")
        )
    }));
    assert!(session.history_snapshot().items().iter().any(|item| {
        matches!(
            item,
            agent_runtime::HistoryItem::Observation(observation)
                if observation.tool_name.as_str() == "delegate_task"
                    && observation.summary.contains("子代理 reviewer 完成")
        )
    }));
}

#[tokio::test]
async fn agent_delegate_task_requires_subagent_name() {
    let model = Arc::new(MockModelClient::new([
        ModelResponse::tool_call(function_tool_call(
            "c_sub",
            "delegate_task",
            json!({
                "task": "检查 agent runtime 的事件流"
            })
            .to_string(),
        )),
        ModelResponse::text("已要求补充子代理名称。"),
    ]));
    let runtime = Runtime::new(RuntimeServices::new(
        model.clone(),
        Arc::new(ToolRouter::new(ToolRegistry::new())),
    ));
    let session = runtime.create_session(ResourceContext::new());
    let mut rx = runtime.subscribe();

    let outcome = runtime
        .run_turn_blocking(session.id(), "排查 agent runtime".into(), TaskKind::Agent)
        .await
        .expect("run agent turn");

    assert!(matches!(outcome, TaskOutcome::Completed { .. }));
    assert_eq!(2, model.request_count());
    let events = drain_events(&mut rx);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, RuntimeEvent::SubAgentStarted { .. }))
    );
    assert!(session.history_snapshot().items().iter().any(|item| {
        matches!(
            item,
            agent_runtime::HistoryItem::Observation(observation)
                if observation.tool_name.as_str() == "delegate_task"
                    && !observation.success
                    && observation.summary.contains("name")
        )
    }));
}

#[tokio::test]
async fn agent_delegate_task_allows_read_only_subagent_tools() {
    let model = Arc::new(MockModelClient::new([
        ModelResponse::tool_call(function_tool_call(
            "c_sub",
            "delegate_task",
            json!({
                "name": "researcher",
                "task": "查询连接状态"
            })
            .to_string(),
        )),
        ModelResponse::tool_call(function_tool_call(
            "sub_echo",
            "echo",
            json!({"message": "连接数正常"}).to_string(),
        )),
        ModelResponse::text("子代理结论: echo 返回连接数正常。"),
        ModelResponse::text("主代理收到子代理结论。"),
    ]));
    let runtime = Runtime::new(RuntimeServices::new(
        model.clone(),
        Arc::new(ToolRouter::new(
            ToolRegistry::new()
                .with_tool(Arc::new(EchoTool))
                .with_tool(Arc::new(WriteTool)),
        )),
    ));
    let session = runtime.create_session(ResourceContext::new());
    let mut rx = runtime.subscribe();

    let outcome = runtime
        .run_turn_blocking(session.id(), "排查 agent runtime".into(), TaskKind::Agent)
        .await
        .expect("run agent turn");

    assert!(matches!(outcome, TaskOutcome::Completed { .. }));
    assert_eq!(4, model.request_count());
    let requests = model.received_requests();
    let subagent_request = &requests[1];
    let subagent_tool_names: Vec<&str> = subagent_request
        .tools
        .iter()
        .map(|tool| tool.function.name.as_str())
        .collect();
    assert_eq!(subagent_tool_names, vec!["echo"]);

    let events = drain_events(&mut rx);
    assert!(events.iter().any(|event| {
        matches!(
            event,
            RuntimeEvent::SubAgentFinished { success: true, summary, .. }
                if summary.contains("连接数正常")
        )
    }));
    assert!(session.history_snapshot().items().iter().any(|item| {
        matches!(
            item,
            agent_runtime::HistoryItem::Observation(observation)
                if observation.tool_name.as_str() == "delegate_task"
                    && observation.success
                    && observation.data.to_text().contains("连接数正常")
        )
    }));
}

#[tokio::test]
async fn current_plan_is_included_in_next_turn_prompt() {
    let model = Arc::new(MockModelClient::new([
        ModelResponse::tool_call(function_tool_call(
            "c_plan",
            "update_plan",
            json!({
                "plan": [
                    {"step": "写作业", "status": "completed"},
                    {"step": "做晚饭", "status": "in_progress"},
                    {"step": "打游戏", "status": "pending"}
                ]
            })
            .to_string(),
        )),
        ModelResponse::text("计划已记录。"),
        ModelResponse::text("继续做晚饭。"),
    ]));
    let runtime = Runtime::new(RuntimeServices::new(
        model.clone(),
        Arc::new(ToolRouter::new(ToolRegistry::new())),
    ));
    let session = runtime.create_session(ResourceContext::new());

    runtime
        .run_turn_blocking(session.id(), "安排今天晚上".into(), TaskKind::Agent)
        .await
        .expect("run first turn");
    runtime
        .run_turn_blocking(session.id(), "继续".into(), TaskKind::Agent)
        .await
        .expect("run follow-up turn");

    let requests = model.received_requests();
    assert_eq!(3, requests.len());
    let system = requests[2].messages[0].content_as_text();
    assert!(system.contains("当前计划(Todo)状态"));
    assert!(system.contains("目标: 安排今天晚上"));
    assert!(system.contains("写作业"));
    assert!(system.contains("做晚饭"));
    assert!(system.contains("Pending"));
    assert!(system.contains("不要把工具调用写成普通文本"));
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
async fn parallel_tool_calls_start_before_first_finishes() {
    let barrier = Arc::new(Barrier::new(2));
    let started = Arc::new(AtomicUsize::new(0));
    let runtime = build_runtime(
        vec![
            ModelResponse::tool_calls(vec![
                function_tool_call("call_parallel_a", "parallel_a", json!({}).to_string()),
                function_tool_call("call_parallel_b", "parallel_b", json!({}).to_string()),
            ]),
            ModelResponse::text("并发检查完成。"),
        ],
        ToolRegistry::new()
            .with_tool(Arc::new(ParallelProbeTool::new(
                "parallel_a",
                barrier.clone(),
                started.clone(),
            )))
            .with_tool(Arc::new(ParallelProbeTool::new(
                "parallel_b",
                barrier,
                started.clone(),
            ))),
    );
    let session = runtime.create_session(ResourceContext::new());

    let outcome = tokio::time::timeout(
        Duration::from_secs(1),
        runtime.run_turn_blocking(session.id(), "并发检查".into(), TaskKind::Agent),
    )
    .await
    .expect("parallel-safe tool calls should both start before either finishes")
    .expect("run agent turn");

    assert!(matches!(outcome, TaskOutcome::Completed { .. }));
    assert_eq!(2, started.load(Ordering::SeqCst));
}

#[tokio::test]
async fn parallel_tool_observations_preserve_original_call_order() {
    let barrier = Arc::new(Barrier::new(2));
    let started = Arc::new(AtomicUsize::new(0));
    let runtime = build_runtime(
        vec![
            ModelResponse::tool_calls(vec![
                function_tool_call("call_parallel_slow", "parallel_slow", json!({}).to_string()),
                function_tool_call("call_parallel_fast", "parallel_fast", json!({}).to_string()),
            ]),
            ModelResponse::text("并发检查完成。"),
        ],
        ToolRegistry::new()
            .with_tool(Arc::new(ParallelProbeTool::new_with_delay(
                "parallel_slow",
                barrier.clone(),
                started.clone(),
                Duration::from_millis(40),
            )))
            .with_tool(Arc::new(ParallelProbeTool::new(
                "parallel_fast",
                barrier,
                started.clone(),
            ))),
    );
    let session = runtime.create_session(ResourceContext::new());
    let mut rx = runtime.subscribe();

    runtime
        .run_turn_blocking(session.id(), "并发顺序检查".into(), TaskKind::Agent)
        .await
        .expect("run agent turn");

    let observed_tools = drain_events(&mut rx)
        .into_iter()
        .filter_map(|event| match event {
            RuntimeEvent::ObservationAdded { observation, .. }
                if observation.tool_name.as_str().starts_with("parallel_") =>
            {
                Some(observation.tool_name.to_string())
            }
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(vec!["parallel_slow", "parallel_fast"], observed_tools);
    assert_eq!(2, started.load(Ordering::SeqCst));
}

#[tokio::test]
async fn manual_mode_pauses_parallel_safe_tool_before_dispatch() {
    let barrier = Arc::new(Barrier::new(1));
    let started = Arc::new(AtomicUsize::new(0));
    let runtime = build_runtime(
        vec![
            ModelResponse::tool_call(function_tool_call(
                "call_parallel_manual",
                "parallel_manual",
                json!({}).to_string(),
            )),
            ModelResponse::text("并发手动工具完成。"),
        ],
        ToolRegistry::new().with_tool(Arc::new(ParallelProbeTool::new(
            "parallel_manual",
            barrier,
            started.clone(),
        ))),
    );
    let session = runtime.create_session(ResourceContext::new());

    let outcome = runtime
        .run_turn_blocking_with_tool_mode(
            session.id(),
            "手动并发工具".into(),
            TaskKind::Agent,
            ToolExecutionMode::Manual,
        )
        .await
        .expect("run manual turn");

    assert!(matches!(
        outcome,
        TaskOutcome::NeedUserInput {
            tool_name: Some(name),
            ..
        } if name.as_str() == "parallel_manual"
    ));
    assert_eq!(0, started.load(Ordering::SeqCst));
}

#[tokio::test]
async fn ask_mode_does_not_send_tools_or_tool_choice() {
    let model = Arc::new(MockModelClient::new([ModelResponse::text("直接回答。")]));
    let runtime = Runtime::new(RuntimeServices::new(
        model.clone(),
        Arc::new(ToolRouter::new(
            ToolRegistry::new().with_tool(Arc::new(EchoTool)),
        )),
    ));
    let session = runtime.create_session(ResourceContext::new());

    let outcome = runtime
        .run_turn_blocking(session.id(), "解释一下索引".into(), TaskKind::Ask)
        .await
        .expect("run ask turn");

    assert!(matches!(outcome, TaskOutcome::Completed { .. }));
    let requests = model.received_requests();
    assert_eq!(1, requests.len());
    assert!(
        requests[0].tools.is_empty(),
        "Ask 模式不能向模型传递任何工具"
    );
    assert!(
        requests[0].tool_choice.is_none(),
        "Ask 模式不能向模型传递 tool_choice"
    );
    let system = requests[0].messages[0].content_as_text();
    assert!(!system.contains("可用 function calling 工具名"));
    assert!(!system.contains("update_plan"));
    assert!(!system.contains("delegate_task"));
}

#[tokio::test]
async fn read_only_tool_mode_exposes_only_read_tools() {
    let model = Arc::new(MockModelClient::new([ModelResponse::text("直接回答。")]));
    let registry = ToolRegistry::new()
        .with_tool(Arc::new(EchoTool))
        .with_tool(Arc::new(WriteTool));
    let runtime = Runtime::new(RuntimeServices::new(
        model.clone(),
        Arc::new(ToolRouter::new(registry)),
    ));
    let session = runtime.create_session(ResourceContext::new());

    let outcome = runtime
        .run_turn_blocking_with_tool_mode(
            session.id(),
            "只读分析".into(),
            TaskKind::Agent,
            ToolExecutionMode::ReadOnly,
        )
        .await
        .expect("run readonly turn");

    assert!(matches!(outcome, TaskOutcome::Completed { .. }));
    let requests = model.received_requests();
    let tool_names = requests[0]
        .tools
        .iter()
        .map(|tool| tool.function.name.as_str())
        .collect::<Vec<_>>();
    assert!(tool_names.contains(&"echo"));
    assert!(!tool_names.contains(&"write_data"));
}

#[tokio::test]
async fn manual_tool_mode_requires_confirmation_before_business_tool_dispatch() {
    let runtime = build_runtime(
        vec![
            ModelResponse::tool_call(function_tool_call(
                "c_write",
                "write_data",
                json!({"value": "x"}).to_string(),
            )),
            ModelResponse::text("写入已完成。"),
        ],
        ToolRegistry::new().with_tool(Arc::new(WriteTool)),
    );
    let session = runtime.create_session(ResourceContext::new());
    let mut rx = runtime.subscribe();

    let outcome = runtime
        .run_turn_blocking_with_tool_mode(
            session.id(),
            "写入 x".into(),
            TaskKind::Agent,
            ToolExecutionMode::Manual,
        )
        .await
        .expect("run manual turn");

    let call_id = match outcome {
        TaskOutcome::NeedUserInput {
            pending_tool_call_id: Some(call_id),
            ..
        } => call_id,
        other => panic!("manual mode should pause for tool approval, got {other:?}"),
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
                && tool_name.as_str() == "write_data"
                && arguments["value"] == "x"
        )
    }));
    assert!(
        !events.iter().any(|event| {
            matches!(
                event,
                RuntimeEvent::ObservationAdded { observation, .. }
                    if observation.summary.contains("write executed")
            )
        }),
        "manual mode must not dispatch the business tool before confirmation"
    );

    let outcome = runtime
        .approve_pending_tool(session.id(), &call_id)
        .await
        .expect("approve pending tool");

    assert!(
        matches!(outcome, TaskOutcome::Completed { answer: Some(answer) } if answer == "写入已完成。")
    );
    let events = drain_events(&mut rx);
    assert!(events.iter().any(|event| {
        matches!(
            event,
            RuntimeEvent::ToolApprovalResolved {
                call_id: event_call_id,
                approved: true,
                ..
            } if event_call_id == &call_id
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            RuntimeEvent::ObservationAdded { observation, .. }
                if observation.success && observation.summary.contains("write executed")
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            RuntimeEvent::TurnCompleted { answer: Some(answer), .. }
                if answer == "写入已完成。"
        )
    }));
}

#[tokio::test]
async fn manual_tool_mode_rejects_pending_tool_and_continues_followup() {
    let model = Arc::new(MockModelClient::new([
        ModelResponse::tool_call(function_tool_call(
            "c_write",
            "write_data",
            json!({"value": "x"}).to_string(),
        )),
        ModelResponse::text("已取消写入。"),
    ]));
    let runtime = Runtime::new(RuntimeServices::new(
        model.clone(),
        Arc::new(ToolRouter::new(
            ToolRegistry::new().with_tool(Arc::new(WriteTool)),
        )),
    ));
    let session = runtime.create_session(ResourceContext::new());
    let mut rx = runtime.subscribe();

    let outcome = runtime
        .run_turn_blocking_with_tool_mode(
            session.id(),
            "写入 x".into(),
            TaskKind::Agent,
            ToolExecutionMode::Manual,
        )
        .await
        .expect("run manual turn");
    let call_id = match outcome {
        TaskOutcome::NeedUserInput {
            pending_tool_call_id: Some(call_id),
            ..
        } => call_id,
        other => panic!("manual mode should pause for tool approval, got {other:?}"),
    };
    let _ = drain_events(&mut rx);

    let outcome = runtime
        .reject_pending_tool(session.id(), &call_id)
        .await
        .expect("reject pending tool");

    assert!(
        matches!(outcome, TaskOutcome::Completed { answer: Some(answer) } if answer == "已取消写入。")
    );
    assert_eq!(2, model.request_count());
    let events = drain_events(&mut rx);
    assert!(events.iter().any(|event| {
        matches!(
            event,
            RuntimeEvent::ToolApprovalResolved {
                call_id: event_call_id,
                approved: false,
                ..
            } if event_call_id == &call_id
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            RuntimeEvent::ObservationAdded { observation, .. }
                if !observation.success && observation.summary.contains("用户拒绝执行工具")
        )
    }));
    assert!(
        !events.iter().any(|event| {
            matches!(
                event,
                RuntimeEvent::ObservationAdded { observation, .. }
                    if observation.summary.contains("write executed")
            )
        }),
        "rejecting a pending tool must not dispatch the business tool"
    );
}

#[tokio::test]
async fn agent_retries_after_unknown_pseudo_tool_call() {
    let model = Arc::new(MockModelClient::new([
        ModelResponse::tool_call(function_tool_call("c_bad", "tool", "db.schema")),
        ModelResponse::tool_call(function_tool_call(
            "c_plan",
            "update_plan",
            json!({
                "plan": [
                    {"step": "改用 update_plan 记录计划", "status": "completed"},
                    {"step": "给出结论", "status": "in_progress"}
                ]
            })
            .to_string(),
        )),
        ModelResponse::text("已纠正工具调用。"),
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

    assert!(matches!(outcome, TaskOutcome::Completed { .. }));
    assert_eq!(3, model.request_count(), "伪工具调用应反馈给模型并重试");
    let events = drain_events(&mut rx);
    assert!(events.iter().any(|event| {
        matches!(
            event,
            RuntimeEvent::ObservationAdded { observation, .. }
                if !observation.success && observation.tool_name.as_str() == "tool"
        )
    }));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, RuntimeEvent::PlanUpdated { .. })),
        "模型重试后应能使用真实 update_plan"
    );
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
    assert!(system.contains("delegate_task"));
    assert!(system.contains("arguments 必须是合法 JSON object"));
    assert!(system.contains("不要调用名为 `tool`"));
}

#[tokio::test]
async fn system_prompt_guides_visible_terminal_requests_to_terminal_exec() {
    let model = Arc::new(MockModelClient::new([ModelResponse::text("ok")]));
    let runtime = Runtime::new(RuntimeServices::new(
        model.clone(),
        Arc::new(ToolRouter::new(
            ToolRegistry::new()
                .with_tool(Arc::new(PromptOnlyTool::new(
                    "terminal.exec",
                    "Execute in a visible terminal.",
                    RiskLevel::High,
                )))
                .with_tool(Arc::new(PromptOnlyTool::new(
                    "ssh.exec",
                    "Execute a structured SSH command.",
                    RiskLevel::Low,
                ))),
        )),
    ));
    let session = runtime.create_session(ResourceContext::new().with_resource(ResourceRef::new(
        "terminal-1",
        ResourceKind::Terminal,
        "prod terminal",
    )));

    runtime
        .run_turn_blocking(
            session.id(),
            "就在这个终端里执行 df -h".into(),
            TaskKind::Agent,
        )
        .await
        .expect("run agent turn");

    let requests = model.received_requests();
    let system = requests[0].messages[0].content_as_text();
    assert!(system.contains("terminal_exec"));
    assert!(system.contains("ssh_exec"));
    assert!(system.contains("可见终端"));
    assert!(system.contains("submit=true"));
    assert!(system.contains("不要声称有 exit code"));
    assert!(system.contains("不要用 `ssh_exec` 替代"));
}

#[tokio::test]
async fn system_prompt_prefers_canonical_runtime_tool_names() {
    let model = Arc::new(MockModelClient::new([ModelResponse::text("ok")]));
    let runtime = Runtime::new(RuntimeServices::new(
        model.clone(),
        Arc::new(ToolRouter::new(
            ToolRegistry::new()
                .with_tool(Arc::new(PromptOnlyTool::new(
                    "db.exec",
                    "Execute database script.",
                    RiskLevel::High,
                )))
                .with_tool(Arc::new(PromptOnlyTool::new(
                    "sftp.read",
                    "Read SFTP file.",
                    RiskLevel::Read,
                )))
                .with_tool(Arc::new(PromptOnlyTool::new(
                    "redis.get",
                    "Get Redis key.",
                    RiskLevel::Low,
                ))),
        )),
    ));
    let session = runtime.create_session(ResourceContext::new());

    runtime
        .run_turn_blocking(session.id(), "检查资源".into(), TaskKind::Agent)
        .await
        .expect("run agent turn");

    let requests = model.received_requests();
    let system = requests[0].messages[0].content_as_text();
    assert!(system.contains("统一工具命名规则"));
    assert!(system.contains("数据库写入使用 `db_exec`"));
    assert!(system.contains("SFTP 文件操作使用 `sftp_read`"));
    assert!(system.contains("Redis 操作使用 `redis_get`"));
    assert!(!system.contains("兼容"));
    assert!(!system.contains("db_execute_sql"));
    assert!(!system.contains("ssh_read_file"));
    assert!(!system.contains("redis_execute_command"));
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
