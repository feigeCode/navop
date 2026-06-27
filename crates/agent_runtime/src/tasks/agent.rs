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
use crate::runtime::{RuntimeServices, RuntimeTask, Session, TaskContext, TaskKind, TaskOutcome};
use crate::tasks::update_plan::{UPDATE_PLAN_TOOL, parse_plan, update_plan_spec};
use crate::tools::{ObservationData, ToolCall, ToolDispatchContext, ToolName, ToolObservation};
use async_trait::async_trait;
use futures::StreamExt;
use llm_connector::types::{Message, ToolChoice};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

const AGENT_SYSTEM: &str = "你是 onetcli 的 AI 运维助手。请根据用户目标自主决定如何行动:\
简单问题直接、简洁地用简体中文回答;需要查询或操作资源时调用相应工具;\
面对多步任务时,先调用 `update_plan` 列出步骤并随进展更新状态(每完成一步就更新)。\
不要为简单问题强行制定计划。完成后直接给出最终回答。";

/// 单轮内模型 / 工具往返的最大迭代次数,防止失控。
const MAX_ITERATIONS: usize = 16;

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
        let session = ctx.session.clone();
        let services = ctx.services.clone();
        let turn_id = ctx.turn.turn_id.clone();
        let resources = ctx.turn.resources.clone();

        session.record_user_input_with_images(goal.clone(), ctx.input_images());

        let dispatch_ctx = ToolDispatchContext {
            session_id: session.id().clone(),
            turn_id: turn_id.clone(),
            resources: resources.clone(),
        };

        for _ in 0..MAX_ITERATIONS {
            if cancellation.is_cancelled() {
                return TaskOutcome::Cancelled;
            }

            // 构造请求:system + 历史 + (业务工具 + update_plan)。
            let mut messages = vec![Message::system(AGENT_SYSTEM)];
            messages.extend(history_to_messages(&session.history_snapshot()));

            let mut tools: Vec<_> = services
                .tools
                .specs(&resources)
                .iter()
                .map(|s| s.to_llm_tool())
                .collect();
            tools.push(update_plan_spec().to_llm_tool());

            let request = ModelRequest::new(messages)
                .with_tools(tools)
                .with_tool_choice(ToolChoice::auto());

            // 流式采样:边收边推文本增量,聚合出完整响应(文本 + 工具调用)。
            let response = match sample(&services, request, &session, &turn_id, &cancellation).await
            {
                Ok(Some(resp)) => resp,
                Ok(None) => return TaskOutcome::Cancelled,
                Err(reason) => return TaskOutcome::Failed { reason },
            };

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

            // 无工具调用:本次文本即最终回答(增量已流式发出,这里落历史并发完整消息)。
            if response.tool_calls.is_empty() {
                let answer = response.text.unwrap_or_default();
                session.record_assistant_message(&turn_id, answer.clone());
                return TaskOutcome::Completed {
                    answer: Some(answer),
                };
            }

            // 模型在调用工具前可能附带一段说明:落历史并 finalize 当前流式消息。
            if let Some(text) = response.text.as_ref().filter(|t| !t.is_empty()) {
                session.record_assistant_message(&turn_id, text.clone());
            }

            // 逐个执行工具调用,把调用与观测写回历史供下一轮 follow-up。
            for llm_call in &response.tool_calls {
                let call = match ToolCall::from_llm(llm_call) {
                    Ok(call) => call,
                    Err(err) => {
                        // 参数非法:回一条失败观测,让模型据此纠正后重试。
                        let call_id = llm_tool_call_id(llm_call);
                        let tool_name = ToolName::new(llm_call.function.name.clone());
                        session.record_observation(
                            &turn_id,
                            ToolObservation::from_error(call_id, tool_name, &err),
                        );
                        continue;
                    }
                };

                tracing::info!(
                    tool = %call.tool_name,
                    args = %call.arguments,
                    "Agent 执行工具调用"
                );
                session.record_tool_call(&turn_id, &call);

                let observation = if call.tool_name.as_str() == UPDATE_PLAN_TOOL {
                    handle_update_plan(&session, &turn_id, &goal, &call)
                } else {
                    services
                        .tools
                        .dispatch(&dispatch_ctx, call, cancellation.clone())
                        .await
                };
                tracing::debug!(
                    tool = %observation.tool_name,
                    success = observation.success,
                    summary = %observation.summary,
                    "Agent 工具观测"
                );
                session.record_observation(&turn_id, observation);
            }
        }

        TaskOutcome::Failed {
            reason: format!("超过单轮最大迭代次数({MAX_ITERATIONS})仍未完成"),
        }
    }
}

/// 执行一次流式采样:把文本增量作为事件推送,聚合出完整 [`ModelResponse`]。
///
/// 返回 `Ok(None)` 表示中途被取消;`Err` 表示模型调用失败。
async fn sample(
    services: &RuntimeServices,
    request: ModelRequest,
    session: &Session,
    turn_id: &TurnId,
    cancellation: &CancellationToken,
) -> Result<Option<ModelResponse>, String> {
    let mut stream = services
        .model
        .complete_stream(request)
        .await
        .map_err(|e| format!("模型调用失败: {e}"))?;

    let mut text = String::new();
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
            ModelStreamEvent::ReasoningDelta(_) => {}
            // 工具调用以 Completed 聚合结果为准,避免重复计数。
            ModelStreamEvent::ToolCall(_) => {}
            ModelStreamEvent::Completed(resp) => completed = Some(resp),
        }
    }

    let tool_calls = completed.map(|r| r.tool_calls).unwrap_or_default();
    Ok(Some(ModelResponse {
        text: (!text.is_empty()).then_some(text),
        tool_calls,
    }))
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

/// 从 `llm-connector` 的工具调用取 call id(空则新生成)。
fn llm_tool_call_id(call: &llm_connector::types::ToolCall) -> ToolCallId {
    if call.id.is_empty() {
        ToolCallId::new()
    } else {
        ToolCallId::from_string(call.id.clone())
    }
}
