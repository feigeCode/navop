//! 子代理采样循环:只允许业务注册表中的只读工具。

use crate::ids::{SubAgentId, ToolCallId, TurnId};
use crate::model::{ModelRequest, ModelResponse, ModelStreamEvent};
use crate::runtime::{RuntimeServices, Session};
use crate::tools::{ToolCall, ToolDispatchContext, ToolName, ToolObservation, ToolSpec};
use futures::StreamExt;
use llm_connector::types::{Message, MessageBlock, Role, ToolCall as LlmToolCall, ToolChoice};
use rust_i18n::t;
use tokio_util::sync::CancellationToken;

const MAX_SUBAGENT_ITERATIONS: usize = 6;
const MAX_SUBAGENT_OBSERVATION_BYTES: usize = 4096;

pub(super) async fn run_subagent_model(
    session: &Session,
    services: &RuntimeServices,
    turn_id: &TurnId,
    subagent_id: &SubAgentId,
    name: &str,
    task: &str,
    cancellation: &CancellationToken,
) -> Result<String, String> {
    let resources = session.resources();
    let specs = subagent_tool_specs(services, &resources);
    let dispatch_ctx = ToolDispatchContext {
        session_id: session.id().clone(),
        turn_id: turn_id.clone(),
        skills: session.skills(),
        resources,
    };
    let mut messages = vec![
        Message::system(subagent_system_prompt(name)),
        Message::user(task.to_string()),
    ];
    for _ in 0..MAX_SUBAGENT_ITERATIONS {
        let request = subagent_request(messages.clone(), &specs);
        let response = sample_subagent_response(services, request, cancellation).await?;
        if response.tool_calls.is_empty() {
            return Ok(response.text.unwrap_or_default().trim().to_string());
        }
        messages.push(assistant_tool_call_message(&response));
        append_tool_results(
            &mut messages,
            session,
            turn_id,
            subagent_id,
            SubagentToolContext {
                services,
                specs: &specs,
                dispatch: &dispatch_ctx,
                cancellation,
            },
            response.tool_calls,
        )
        .await;
    }
    Err(t!(
        "AgentRuntime.subagent_max_iterations_exceeded",
        count = MAX_SUBAGENT_ITERATIONS
    )
    .to_string())
}

struct SubagentToolContext<'a> {
    services: &'a RuntimeServices,
    specs: &'a [ToolSpec],
    dispatch: &'a ToolDispatchContext,
    cancellation: &'a CancellationToken,
}

async fn append_tool_results(
    messages: &mut Vec<Message>,
    session: &Session,
    turn_id: &TurnId,
    subagent_id: &SubAgentId,
    ctx: SubagentToolContext<'_>,
    calls: Vec<LlmToolCall>,
) {
    for llm_call in calls {
        let observation = dispatch_subagent_tool(
            ctx.dispatch,
            ctx.services,
            ctx.specs,
            &llm_call,
            ctx.cancellation,
        )
        .await;
        session.update_subagent(turn_id, subagent_id.clone(), observation.summary.clone());
        messages.push(tool_result_message(&observation));
    }
}

fn subagent_tool_specs(
    services: &RuntimeServices,
    resources: &crate::resource::ResourceContext,
) -> Vec<ToolSpec> {
    services
        .tools
        .specs(resources)
        .into_iter()
        .filter(|spec| spec.risk == crate::risk::RiskLevel::Read)
        .collect()
}

fn subagent_request(messages: Vec<Message>, specs: &[ToolSpec]) -> ModelRequest {
    let tools: Vec<_> = specs.iter().map(|spec| spec.to_llm_tool()).collect();
    let request = ModelRequest::new(messages);
    if tools.is_empty() {
        request
    } else {
        request
            .with_tools(tools)
            .with_tool_choice(ToolChoice::auto())
    }
}

async fn sample_subagent_response(
    services: &RuntimeServices,
    request: ModelRequest,
    cancellation: &CancellationToken,
) -> Result<ModelResponse, String> {
    let mut stream = tokio::select! {
        biased;
        _ = cancellation.cancelled() => {
            return Err(t!("AgentRuntime.subagent_cancelled").to_string())
        },
        result = services.model.complete_stream(request) => {
            result.map_err(|error| {
                t!("AgentRuntime.subagent_model_failed", error = error).to_string()
            })?
        }
    };
    collect_subagent_response(&mut stream, cancellation).await
}

async fn collect_subagent_response(
    stream: &mut crate::model::ModelStream,
    cancellation: &CancellationToken,
) -> Result<ModelResponse, String> {
    let mut text = String::new();
    let mut completed: Option<ModelResponse> = None;
    while let Some(event) = stream.next().await {
        if cancellation.is_cancelled() {
            return Err(t!("AgentRuntime.subagent_cancelled").to_string());
        }
        match event
            .map_err(|error| t!("AgentRuntime.subagent_stream_failed", error = error).to_string())?
        {
            ModelStreamEvent::TextDelta(delta) => text.push_str(&delta),
            ModelStreamEvent::ReasoningDelta(_) | ModelStreamEvent::ToolCall(_) => {}
            ModelStreamEvent::Completed(response) => completed = Some(response),
        }
    }
    let mut response = completed.unwrap_or_default();
    if !text.is_empty() {
        response.text = Some(text);
    }
    Ok(response)
}

async fn dispatch_subagent_tool(
    ctx: &ToolDispatchContext,
    services: &RuntimeServices,
    specs: &[ToolSpec],
    llm_call: &LlmToolCall,
    cancellation: &CancellationToken,
) -> ToolObservation {
    let tool_name = ToolName::new(llm_call.function.name.clone());
    let call_id = llm_tool_call_id(llm_call);
    if !specs.iter().any(|spec| spec.name == tool_name) {
        return unavailable_subagent_tool(call_id, tool_name, specs);
    }
    let call = match ToolCall::from_llm(llm_call) {
        Ok(call) => call,
        Err(err) => {
            return ToolObservation::failure(
                call_id,
                tool_name,
                t!("AgentRuntime.subagent_tool_invalid_arguments", error = err).to_string(),
            );
        }
    };
    services
        .tools
        .dispatch(ctx, call, cancellation.clone())
        .await
}

fn assistant_tool_call_message(response: &ModelResponse) -> Message {
    Message {
        role: Role::Assistant,
        content: response
            .text
            .as_ref()
            .filter(|text| !text.is_empty())
            .map(|text| vec![MessageBlock::text(text.clone())])
            .unwrap_or_default(),
        tool_calls: Some(response.tool_calls.clone()),
        ..Default::default()
    }
}

fn tool_result_message(observation: &ToolObservation) -> Message {
    Message {
        role: Role::Tool,
        content: vec![MessageBlock::text(
            observation.model_text(MAX_SUBAGENT_OBSERVATION_BYTES),
        )],
        tool_call_id: Some(observation.call_id.to_string()),
        ..Default::default()
    }
}

fn unavailable_subagent_tool(
    call_id: ToolCallId,
    tool_name: ToolName,
    specs: &[ToolSpec],
) -> ToolObservation {
    let available = if specs.is_empty() {
        t!("AgentRuntime.no_available_tools").to_string()
    } else {
        specs
            .iter()
            .map(|spec| spec.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    ToolObservation::failure(
        call_id,
        tool_name,
        t!("AgentRuntime.subagent_tool_unavailable", tools = available).to_string(),
    )
}

fn llm_tool_call_id(call: &LlmToolCall) -> ToolCallId {
    if call.id.is_empty() {
        ToolCallId::new()
    } else {
        ToolCallId::from_string(call.id.clone())
    }
}

fn subagent_system_prompt(name: &str) -> String {
    format!(
        "你是 Navop agent runtime 派发的隔离子代理 `{name}`。\
只完成用户给你的子任务。你可以使用提供的只读工具查询上下文,但不要修改外部状态。\
不要调用 `delegate_task` 或 `update_plan`;它们不会提供给你。\
输出简体中文结论摘要,包含关键发现、证据和下一步建议。"
    )
}
