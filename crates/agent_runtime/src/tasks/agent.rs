//! codex 风格的统一 Agent 任务:模型驱动的工具调用循环。
//!
//! 与 [`DiagnosisTask`](super::DiagnosisTask) 的"每轮强制规划"不同,本任务参考
//! codex 的做法:模型在一个循环里**自主决定**——直接回答 / 调用业务工具 /
//! 调用 [`update_plan`](super::update_plan) 维护任务清单。
//!
//! - 简单问题:模型直接流式回答,不产生任何计划(Tasks 面板保持空)。
//! - 多步任务:模型自行调用 `update_plan` 维护 checklist,接到输入框上方的 Tasks 面板。
//!
//! 循环:流式采样 → 若无工具调用则该文本即最终回答;若有工具调用则逐个执行
//! (`update_plan` 本地拦截更新计划;其余走 [`ToolRouter`](crate::tools::ToolRouter)),
//! 把调用与观测写回历史后再次采样(follow-up),直到模型给出最终回答或达上限。

use crate::ids::{ToolCallId, TurnId};
use crate::model::{ModelRequest, ModelResponse, ModelStreamEvent};
use crate::planner::history_to_messages;
use crate::resource::ResourceContext;
use crate::runtime::{
    PendingToolApproval, PendingToolCallSummary, RuntimeServices, RuntimeTask, Session,
    TaskContext, TaskKind, TaskOutcome, ToolExecutionMode,
};
use crate::tasks::agent_prompt::build_system_prompt;
use crate::tasks::agent_tool_validation::{
    available_tool_names, malformed_tool_call_reason, specs_for_task, tool_is_available,
};
use crate::tasks::delegate_task::{DELEGATE_TASK_TOOL, handle_delegate_task};
use crate::tasks::update_plan::{UPDATE_PLAN_TOOL, parse_plan};
use crate::tools::{ObservationData, ToolCall, ToolDispatchContext, ToolName, ToolObservation};
use async_trait::async_trait;
use futures::StreamExt;
use llm_connector::types::{Message, ToolCall as LlmToolCall, ToolChoice};
use serde_json::Value;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// 单轮内模型 / 工具往返的最大迭代次数,防止失控。
const MAX_ITERATIONS: usize = 16;
const DEBUG_PREVIEW_CHARS: usize = 1200;

/// codex 风格的统一 Agent 任务。
#[derive(Default)]
pub struct AgentTask;

impl AgentTask {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl RuntimeTask for AgentTask {
    fn kind(&self) -> TaskKind {
        TaskKind::Agent
    }

    async fn run(
        self: Arc<Self>,
        ctx: TaskContext,
        cancellation: CancellationToken,
    ) -> TaskOutcome {
        let goal = ctx.goal();
        let task_kind = ctx.kind;
        let tool_mode = ctx.tool_mode;
        let session = ctx.session.clone();
        let services = ctx.services.clone();
        let turn_id = ctx.turn.turn_id.clone();
        let resources = ctx.turn.resources.clone();

        session.record_user_input_with_images(goal.clone(), ctx.input_images());

        run_agent_loop(
            AgentLoopContext {
                goal,
                task_kind,
                tool_mode,
                session,
                services,
                turn_id,
                resources,
            },
            cancellation,
        )
        .await
    }
}

struct AgentLoopContext {
    goal: String,
    task_kind: TaskKind,
    tool_mode: ToolExecutionMode,
    session: Arc<Session>,
    services: Arc<RuntimeServices>,
    turn_id: TurnId,
    resources: ResourceContext,
}

pub(crate) async fn continue_after_tool_decision(
    services: Arc<RuntimeServices>,
    session: Arc<Session>,
    pending: PendingToolApproval,
    approved: bool,
    cancellation: CancellationToken,
) -> TaskOutcome {
    let turn_id = pending.turn_id.clone();
    let calls = pending.calls();
    for call in &calls {
        session.record_tool_call(&turn_id, call);
    }
    let dispatch_ctx = dispatch_context(&session, &turn_id, &pending.resources);
    for call in calls {
        let observation = if approved {
            execute_agent_tool(
                &session,
                &services,
                &dispatch_ctx,
                &turn_id,
                &pending.goal,
                call,
                &cancellation,
            )
            .await
        } else {
            rejected_tool_observation(&call)
        };
        session.record_observation(&turn_id, observation);
    }

    run_agent_loop(
        AgentLoopContext {
            goal: pending.goal,
            task_kind: pending.task_kind,
            tool_mode: pending.tool_mode,
            session,
            services,
            turn_id,
            resources: pending.resources,
        },
        cancellation,
    )
    .await
}

async fn run_agent_loop(ctx: AgentLoopContext, cancellation: CancellationToken) -> TaskOutcome {
    let dispatch_ctx = dispatch_context(&ctx.session, &ctx.turn_id, &ctx.resources);
    for iteration in 0..MAX_ITERATIONS {
        if cancellation.is_cancelled() {
            return TaskOutcome::Cancelled;
        }

        let tool_specs =
            specs_for_task(ctx.task_kind, ctx.tool_mode, &ctx.services, &ctx.resources);
        let tools: Vec<_> = tool_specs.iter().map(|s| s.to_llm_tool()).collect();

        // 构造请求:system + 历史 + (业务工具 + update_plan)。
        let mut messages = vec![Message::system(build_system_prompt(
            ctx.task_kind,
            &tool_specs,
            &ctx.resources,
            ctx.session.system_instruction().as_deref(),
            ctx.session.current_plan().as_ref(),
        ))];
        messages.extend(history_to_messages(&ctx.session.history_snapshot()));

        let mut request = ModelRequest::new(messages);
        if !tools.is_empty() {
            request = request
                .with_tools(tools)
                .with_tool_choice(ToolChoice::auto());
        }
        log_model_request(iteration, &request);

        // 流式采样:边收边推文本增量,聚合出完整响应(文本 + 工具调用)。
        let sample = match sample(
            &ctx.services,
            request,
            &ctx.session,
            &ctx.turn_id,
            &cancellation,
        )
        .await
        {
            Ok(Some(sample)) => sample,
            Ok(None) => return TaskOutcome::Cancelled,
            Err(reason) => return TaskOutcome::Failed { reason },
        };
        let response = sample.response;
        let reasoning = sample.reasoning;

        let tool_names: Vec<&str> = response
            .tool_calls
            .iter()
            .map(|tc| tc.function.name.as_str())
            .collect();
        tracing::info!(
            text_len = response.text.as_deref().map(str::len).unwrap_or(0),
            tool_calls = ?tool_names,
            "Agent 模型响应"
        );
        log_model_response(iteration, &response);

        // 无工具调用:本次文本即最终回答(增量已流式发出,这里落历史并发完整消息)。
        if response.tool_calls.is_empty() {
            let answer = response.text.unwrap_or_default();
            ctx.session.record_assistant_message_with_reasoning(
                &ctx.turn_id,
                answer.clone(),
                reasoning,
            );
            return TaskOutcome::Completed {
                answer: Some(answer),
            };
        }

        if ctx.task_kind == TaskKind::Ask {
            return TaskOutcome::Failed {
                reason: "Ask 模式不支持工具调用;请切换到 Agent 或 Plan 模式后再使用工具。".into(),
            };
        }

        // 模型在调用工具前可能附带一段说明:落历史并 finalize 当前流式消息。
        if let Some(text) = response.text.as_ref().filter(|t| !t.is_empty()) {
            ctx.session.record_assistant_message_with_reasoning(
                &ctx.turn_id,
                text.clone(),
                reasoning.clone(),
            );
        } else if !reasoning.is_empty() {
            ctx.session.record_assistant_message_with_reasoning(
                &ctx.turn_id,
                "",
                reasoning.clone(),
            );
        }

        // 先写入同一轮模型返回的所有合法工具调用,再写观测结果。OpenAI thinking
        // mode 要求 assistant tool_calls 消息原样带回对应 reasoning_content。
        let mut executable_calls = Vec::new();
        let mut approval_calls = Vec::new();
        for llm_call in &response.tool_calls {
            log_llm_tool_call("agent_dispatch", llm_call);
            let call_id = llm_tool_call_id(llm_call);
            let tool_name = ToolName::new(llm_call.function.name.clone());
            if !tool_is_available(&tool_specs, &tool_name) {
                ctx.session.record_observation(
                    &ctx.turn_id,
                    unavailable_tool_observation(call_id, tool_name, &tool_specs),
                );
                continue;
            }

            let call = match ToolCall::from_llm(llm_call) {
                Ok(call) => call,
                Err(err) => {
                    // 非 JSON 参数通常是模型/Provider 返回了非标准工具格式;
                    // 写回观测让模型用同一批可用工具和 JSON object 参数纠正。
                    let reason = malformed_tool_call_reason(llm_call, &err);
                    let observation = ToolObservation::failure(call_id, tool_name, reason);
                    ctx.session.record_observation(&ctx.turn_id, observation);
                    continue;
                }
            };

            if requires_tool_approval(ctx.tool_mode, &call, &tool_specs) {
                approval_calls.push(call);
                continue;
            }

            executable_calls.push(call);
        }

        if !approval_calls.is_empty() {
            let first = approval_calls.remove(0);
            let pending_tool_call_id = first.call_id.clone();
            let tool_name = first.tool_name.clone();
            let arguments = first.arguments.clone();
            let pending = PendingToolApproval {
                turn_id: ctx.turn_id.clone(),
                task_kind: ctx.task_kind,
                tool_mode: ctx.tool_mode,
                goal: ctx.goal.clone(),
                call: first,
                additional_calls: approval_calls,
                resources: ctx.resources.clone(),
            };
            let question = approval_question(&pending);
            let pending_tool_calls = pending
                .calls()
                .iter()
                .map(PendingToolCallSummary::from_call)
                .collect();
            ctx.session.set_pending_tool_approval(pending);
            return TaskOutcome::NeedUserInput {
                question,
                pending_tool_call_id: Some(pending_tool_call_id),
                tool_name: Some(tool_name),
                arguments: Some(arguments),
                pending_tool_calls,
            };
        }

        for call in &executable_calls {
            ctx.session.record_tool_call(&ctx.turn_id, call);
        }

        for batch in executable_call_batches(executable_calls, |call| {
            ctx.services.tools.supports_parallel(call)
        }) {
            let observations = if batch.parallel {
                futures::future::join_all(batch.calls.into_iter().map(|call| {
                    execute_logged_agent_tool(
                        &ctx.session,
                        &ctx.services,
                        &dispatch_ctx,
                        &ctx.turn_id,
                        &ctx.goal,
                        call,
                        &cancellation,
                    )
                }))
                .await
            } else {
                let mut observations = Vec::new();
                for call in batch.calls {
                    observations.push(
                        execute_logged_agent_tool(
                            &ctx.session,
                            &ctx.services,
                            &dispatch_ctx,
                            &ctx.turn_id,
                            &ctx.goal,
                            call,
                            &cancellation,
                        )
                        .await,
                    );
                }
                observations
            };
            for observation in observations {
                ctx.session.record_observation(&ctx.turn_id, observation);
            }
        }
    }

    TaskOutcome::Failed {
        reason: format!("超过单轮最大迭代次数({MAX_ITERATIONS})仍未完成"),
    }
}

fn dispatch_context(
    session: &Session,
    turn_id: &TurnId,
    resources: &ResourceContext,
) -> ToolDispatchContext {
    ToolDispatchContext {
        session_id: session.id().clone(),
        turn_id: turn_id.clone(),
        resources: resources.clone(),
    }
}

async fn execute_agent_tool(
    session: &Session,
    services: &RuntimeServices,
    dispatch_ctx: &ToolDispatchContext,
    turn_id: &TurnId,
    goal: &str,
    call: ToolCall,
    cancellation: &CancellationToken,
) -> ToolObservation {
    if call.tool_name.as_str() == UPDATE_PLAN_TOOL {
        handle_update_plan(session, turn_id, goal, &call)
    } else if call.tool_name.as_str() == DELEGATE_TASK_TOOL {
        handle_delegate_task(session, services, turn_id, &call, cancellation).await
    } else {
        services
            .tools
            .dispatch(dispatch_ctx, call, cancellation.clone())
            .await
    }
}

async fn execute_logged_agent_tool(
    session: &Session,
    services: &RuntimeServices,
    dispatch_ctx: &ToolDispatchContext,
    turn_id: &TurnId,
    goal: &str,
    call: ToolCall,
    cancellation: &CancellationToken,
) -> ToolObservation {
    tracing::info!(
        tool = %call.tool_name,
        args = %debug_json_redacted(&call.arguments),
        "Agent 执行工具调用"
    );
    let observation = execute_agent_tool(
        session,
        services,
        dispatch_ctx,
        turn_id,
        goal,
        call,
        cancellation,
    )
    .await;
    tracing::debug!(
        tool = %observation.tool_name,
        success = observation.success,
        summary = %observation.summary,
        "Agent 工具观测"
    );
    observation
}

fn rejected_tool_observation(call: &ToolCall) -> ToolObservation {
    ToolObservation::failure(
        call.call_id.clone(),
        call.tool_name.clone(),
        format!("用户拒绝执行工具 `{}`。", call.tool_name),
    )
}

fn requires_manual_confirmation(tool_name: &str) -> bool {
    !matches!(tool_name, UPDATE_PLAN_TOOL | DELEGATE_TASK_TOOL)
}

fn requires_tool_approval(
    mode: ToolExecutionMode,
    call: &ToolCall,
    specs: &[crate::tools::ToolSpec],
) -> bool {
    if !requires_manual_confirmation(call.tool_name.as_str()) {
        return false;
    }
    if mode == ToolExecutionMode::Manual {
        return true;
    }
    specs
        .iter()
        .find(|spec| spec.name == call.tool_name)
        .is_some_and(|spec| spec.risk.requires_confirmation())
}

fn approval_question(pending: &PendingToolApproval) -> String {
    if pending.call_count() == 1 {
        return format!("确认执行工具 `{}` 吗?", pending.call.tool_name);
    }
    format!("确认执行 {} 个工具吗?", pending.call_count())
}

/// 执行一次流式采样:把文本增量作为事件推送,聚合出完整 [`ModelResponse`]。
///
/// 返回 `Ok(None)` 表示中途被取消;`Err` 表示模型调用失败。
struct AgentSample {
    response: ModelResponse,
    reasoning: String,
}

async fn sample(
    services: &RuntimeServices,
    request: ModelRequest,
    session: &Session,
    turn_id: &TurnId,
    cancellation: &CancellationToken,
) -> Result<Option<AgentSample>, String> {
    let mut stream = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Ok(None),
        result = services.model.complete_stream(request) => {
            result.map_err(|e| format!("模型调用失败: {e}"))?
        }
    };

    let mut text = String::new();
    let mut reasoning = String::new();
    let mut completed: Option<ModelResponse> = None;
    while let Some(event) = stream.next().await {
        if cancellation.is_cancelled() {
            return Ok(None);
        }
        match event.map_err(|e| format!("模型流式输出失败: {e}"))? {
            ModelStreamEvent::TextDelta(delta) => {
                text.push_str(&delta);
                session.emit_assistant_delta(turn_id, delta);
            }
            ModelStreamEvent::ReasoningDelta(delta) => {
                reasoning.push_str(&delta);
                session.emit_reasoning_delta(turn_id, delta);
            }
            // 工具调用以 Completed 聚合结果为准,避免重复计数。
            ModelStreamEvent::ToolCall(_) => {}
            ModelStreamEvent::Completed(resp) => completed = Some(resp),
        }
    }

    let tool_calls = completed.map(|r| r.tool_calls).unwrap_or_default();
    Ok(Some(AgentSample {
        response: ModelResponse {
            text: (!text.is_empty()).then_some(text),
            tool_calls,
        },
        reasoning,
    }))
}

fn log_model_request(iteration: usize, request: &ModelRequest) {
    let tool_names = request
        .tools
        .iter()
        .map(|tool| tool.function.name.as_str())
        .collect::<Vec<_>>();
    tracing::debug!(
        iteration,
        messages = request.messages.len(),
        tools = ?tool_names,
        tool_choice = ?request.tool_choice,
        temperature = ?request.temperature,
        max_tokens = ?request.max_tokens,
        "Agent 模型输入摘要"
    );
    tracing::trace!(
        iteration,
        messages = %debug_model_messages(&request.messages),
        tools = %debug_json_redacted(&request.tools),
        tool_choice = %debug_json_redacted(&request.tool_choice),
        "Agent 模型输入详情"
    );
}

fn log_model_response(iteration: usize, response: &ModelResponse) {
    let tool_calls = response
        .tool_calls
        .iter()
        .map(debug_tool_call)
        .collect::<Vec<_>>();
    tracing::debug!(
        iteration,
        text_len = response.text.as_deref().map(str::len).unwrap_or(0),
        text_preview = %redacted_text_summary(response.text.as_deref().unwrap_or_default()),
        tool_calls = ?tool_calls,
        "Agent 模型输出详情"
    );
}

fn log_llm_tool_call(stage: &str, call: &LlmToolCall) {
    tracing::debug!(
        stage,
        id = %call.id,
        index = ?call.index,
        call_type = %call.call_type,
        function_name = %call.function.name,
        arguments_len = call.function.arguments.len(),
        arguments_preview = %redacted_arguments_preview(&call.function.arguments),
        "Agent 工具调用原始字段"
    );
}

fn debug_tool_call(call: &LlmToolCall) -> String {
    format!(
        "id={}, index={:?}, type={}, name={}, args_len={}, args={}",
        call.id,
        call.index,
        call.call_type,
        call.function.name,
        call.function.arguments.len(),
        redacted_arguments_preview(&call.function.arguments)
    )
}

fn debug_model_messages(messages: &[Message]) -> String {
    match serde_json::to_value(messages) {
        Ok(mut value) => {
            redact_model_payloads(&mut value);
            redact_sensitive_value(&mut value);
            serde_json::to_string(&value)
                .unwrap_or_else(|err| format!("<json encode failed: {err}>"))
        }
        Err(err) => format!("<json encode failed: {err}>"),
    }
}

fn debug_json_redacted<T: serde::Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(mut value) => {
            redact_sensitive_value(&mut value);
            serde_json::to_string(&value)
                .unwrap_or_else(|err| format!("<json encode failed: {err}>"))
        }
        Err(err) => format!("<json encode failed: {err}>"),
    }
}

fn redacted_arguments_preview(arguments: &str) -> String {
    match serde_json::from_str::<Value>(arguments) {
        Ok(mut value) => {
            redact_sensitive_value(&mut value);
            preview(&serde_json::to_string(&value).unwrap_or_else(|_| "<redacted>".into()))
        }
        Err(_) => format!(
            "<non-json arguments redacted chars={}>",
            arguments.chars().count()
        ),
    }
}

fn redacted_text_summary(value: &str) -> String {
    if value.is_empty() {
        String::new()
    } else {
        format!("<text redacted chars={}>", value.chars().count())
    }
}

fn redact_model_payloads(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for (key, item) in object.iter_mut() {
                if is_model_payload_key(key) {
                    *item = redacted_summary(item);
                } else {
                    redact_model_payloads(item);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_model_payloads(item);
            }
        }
        _ => {}
    }
}

fn redact_sensitive_value(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for (key, item) in object.iter_mut() {
                if is_sensitive_key(key) {
                    *item = Value::String("***".into());
                } else if key_is_json_arguments(key) {
                    redact_arguments_value(item);
                } else {
                    redact_sensitive_value(item);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_sensitive_value(item);
            }
        }
        _ => {}
    }
}

fn redact_arguments_value(value: &mut Value) {
    if let Value::String(text) = value
        && let Ok(mut parsed) = serde_json::from_str::<Value>(text)
    {
        redact_sensitive_value(&mut parsed);
        *value = parsed;
        return;
    }
    redact_sensitive_value(value);
}

fn redacted_summary(value: &Value) -> Value {
    Value::String(format!(
        "<redacted chars={}>",
        value.to_string().chars().count()
    ))
}

fn is_model_payload_key(key: &str) -> bool {
    matches!(
        normalize_key(key).as_str(),
        "content" | "reasoningcontent" | "reasoning" | "thought" | "thinking"
    )
}

fn key_is_json_arguments(key: &str) -> bool {
    normalize_key(key) == "arguments"
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = normalize_key(key);
    [
        "password",
        "passwd",
        "token",
        "apikey",
        "secret",
        "authorization",
        "cookie",
        "privatekey",
        "accesskey",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn normalize_key(key: &str) -> String {
    key.chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn preview(value: &str) -> String {
    let mut out: String = value.chars().take(DEBUG_PREVIEW_CHARS).collect();
    if value.chars().count() > DEBUG_PREVIEW_CHARS {
        out.push_str("…<truncated>");
    }
    out
}

/// 本地处理 `update_plan`:解析参数 → 更新会话计划(发 `PlanUpdated`)→ 回一条确认观测。
fn handle_update_plan(
    session: &Session,
    turn_id: &TurnId,
    goal: &str,
    call: &ToolCall,
) -> ToolObservation {
    match parse_plan(goal, &call.arguments) {
        Some((plan, explanation)) => {
            let step_count = plan.steps.len();
            session.update_plan(turn_id, plan);
            let summary =
                explanation.unwrap_or_else(|| format!("已更新任务清单(共 {step_count} 步)"));
            ToolObservation::success(
                call.call_id.clone(),
                call.tool_name.clone(),
                summary,
                ObservationData::Text("ok".into()),
            )
        }
        None => ToolObservation::from_error(
            call.call_id.clone(),
            call.tool_name.clone(),
            &crate::error::ToolError::InvalidArguments("update_plan 参数解析失败".into()),
        ),
    }
}

fn unavailable_tool_observation(
    call_id: ToolCallId,
    tool_name: ToolName,
    tool_specs: &[crate::tools::ToolSpec],
) -> ToolObservation {
    ToolObservation::failure(
        call_id,
        tool_name.clone(),
        format!(
            "模型请求了未注册工具 `{}`。可用工具: {}。不要调用名为 `tool` 的通用伪工具;请改用可用工具名,且 arguments 必须是合法 JSON object。",
            tool_name,
            available_tool_names(tool_specs)
        ),
    )
}

/// 从 `llm-connector` 的工具调用取 call id(空则新生成)。
fn llm_tool_call_id(call: &llm_connector::types::ToolCall) -> ToolCallId {
    if call.id.is_empty() {
        ToolCallId::new()
    } else {
        ToolCallId::from_string(call.id.clone())
    }
}

struct ExecutableCallBatch {
    parallel: bool,
    calls: Vec<ToolCall>,
}

fn executable_call_batches(
    calls: Vec<ToolCall>,
    supports_parallel: impl Fn(&ToolCall) -> bool,
) -> Vec<ExecutableCallBatch> {
    let mut batches = Vec::new();
    let mut current_parallel = Vec::new();

    for call in calls {
        if supports_parallel(&call) {
            current_parallel.push(call);
        } else {
            if !current_parallel.is_empty() {
                batches.push(ExecutableCallBatch {
                    parallel: true,
                    calls: std::mem::take(&mut current_parallel),
                });
            }
            batches.push(ExecutableCallBatch {
                parallel: false,
                calls: vec![call],
            });
        }
    }

    if !current_parallel.is_empty() {
        batches.push(ExecutableCallBatch {
            parallel: true,
            calls: current_parallel,
        });
    }

    batches
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm_connector::types::{FunctionCall, Role};
    use serde_json::json;

    #[test]
    fn debug_json_redacts_sensitive_keys_recursively() {
        let rendered = debug_json_redacted(&json!({
            "headers": {
                "Authorization": "Bearer secret-token",
                "cookie": "sid=abc"
            },
            "nested": [
                {"password": "p@ss"},
                {"api_key": "key-123"},
                {"query": "select 1"}
            ]
        }));

        assert!(rendered.contains("***"));
        assert!(rendered.contains("select 1"));
        assert!(!rendered.contains("secret-token"));
        assert!(!rendered.contains("sid=abc"));
        assert!(!rendered.contains("p@ss"));
        assert!(!rendered.contains("key-123"));
    }

    #[test]
    fn tool_call_arguments_preview_redacts_json_argument_strings() {
        let call = LlmToolCall {
            id: "call_1".into(),
            call_type: "function".into(),
            function: FunctionCall {
                name: "http".into(),
                arguments: json!({
                    "url": "https://example.test",
                    "headers": {"Authorization": "Bearer token"},
                    "password": "plain-secret"
                })
                .to_string(),
                thought_signature: None,
            },
            index: Some(0),
            thought_signature: None,
        };

        let rendered = debug_tool_call(&call);

        assert!(rendered.contains("https://example.test"));
        assert!(rendered.contains("***"));
        assert!(!rendered.contains("Bearer token"));
        assert!(!rendered.contains("plain-secret"));
    }

    #[test]
    fn non_json_tool_arguments_are_not_logged_verbatim() {
        let rendered = redacted_arguments_preview("password=plain-secret");

        assert!(rendered.contains("<non-json arguments redacted"));
        assert!(!rendered.contains("plain-secret"));
    }

    #[test]
    fn executable_call_batches_group_adjacent_parallel_safe_calls() {
        let calls = vec![
            ToolCall::new("read_a", json!({})),
            ToolCall::new("read_b", json!({})),
            ToolCall::new("write_a", json!({})),
            ToolCall::new("read_c", json!({})),
        ];
        let batches =
            executable_call_batches(calls, |call| call.tool_name.as_str().starts_with("read"));

        assert_eq!(batches.len(), 3);
        assert!(batches[0].parallel);
        assert_eq!(batches[0].calls.len(), 2);
        assert!(!batches[1].parallel);
        assert_eq!(batches[1].calls.len(), 1);
        assert!(batches[2].parallel);
        assert_eq!(batches[2].calls.len(), 1);
    }

    #[test]
    fn executable_call_batches_keep_serial_calls_separate() {
        let calls = vec![
            ToolCall::new("write_a", json!({})),
            ToolCall::new("write_b", json!({})),
        ];
        let batches = executable_call_batches(calls, |_| false);

        assert_eq!(batches.len(), 2);
        assert!(batches.iter().all(|batch| !batch.parallel));
    }

    #[test]
    fn model_message_payloads_are_summarized_not_logged_verbatim() {
        let mut message = Message::text(Role::User, "客户 token 是 secret-token");
        message.reasoning_content = Some("private chain".into());
        let rendered = debug_model_messages(&[message]);

        assert!(rendered.contains("<redacted chars="));
        assert!(!rendered.contains("secret-token"));
        assert!(!rendered.contains("private chain"));
    }
}
