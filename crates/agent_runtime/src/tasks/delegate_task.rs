//! 内建子代理派发工具。
//!
//! `delegate_task` 不是业务工具,也不是某个具体 CLI 后端。它表示当前 agent
//! 把一个清晰子任务交给隔离的子代理采样执行,结果再作为 observation 回到主会话。

use crate::ids::{SubAgentId, TurnId};
use crate::model::{ModelRequest, ModelResponse, ModelStreamEvent};
use crate::runtime::{RuntimeServices, Session};
use crate::tools::{ObservationData, ToolCall, ToolName, ToolObservation, ToolSpec};
use futures::StreamExt;
use llm_connector::types::Message;
use serde::Deserialize;
use serde_json::json;
use tokio_util::sync::CancellationToken;

pub const DELEGATE_TASK_TOOL: &str = "delegate_task";

#[derive(Deserialize)]
struct DelegateTaskArgs {
    #[serde(default)]
    name: Option<String>,
    task: String,
}

pub fn delegate_task_spec() -> ToolSpec {
    ToolSpec::new(
        DELEGATE_TASK_TOOL,
        "将一个边界清晰的子任务派发给隔离子代理执行。子代理不能调用工具,只返回结论摘要。",
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "子代理显示名称,例如 reviewer、researcher、planner"
                },
                "task": {
                    "type": "string",
                    "description": "子代理需要完成的明确任务"
                }
            },
            "required": ["task"],
            "additionalProperties": false
        }),
    )
}

pub async fn handle_delegate_task(
    session: &Session,
    services: &RuntimeServices,
    turn_id: &TurnId,
    call: &ToolCall,
    cancellation: &CancellationToken,
) -> ToolObservation {
    let args = match parse_args(call) {
        Ok(args) => args,
        Err(observation) => return observation,
    };
    let name = args.name.unwrap_or_else(|| "worker".to_string());
    let subagent_id = SubAgentId::from_string(call.call_id.as_str().to_string());
    session.start_subagent(
        turn_id,
        subagent_id.clone(),
        name.clone(),
        args.task.clone(),
    );

    match run_subagent_model(services, &name, &args.task, cancellation).await {
        Ok(summary) => finish_success(session, turn_id, call, subagent_id, &name, summary),
        Err(message) => finish_failure(session, turn_id, call, subagent_id, message),
    }
}

fn parse_args(call: &ToolCall) -> Result<DelegateTaskArgs, ToolObservation> {
    let args =
        serde_json::from_value::<DelegateTaskArgs>(call.arguments.clone()).map_err(|err| {
            ToolObservation::failure(
                call.call_id.clone(),
                call.tool_name.clone(),
                format!("delegate_task 参数无效:{err}"),
            )
        })?;
    if args.task.trim().is_empty() {
        return Err(ToolObservation::failure(
            call.call_id.clone(),
            call.tool_name.clone(),
            "delegate_task.task 不能为空",
        ));
    }
    Ok(args)
}

async fn run_subagent_model(
    services: &RuntimeServices,
    name: &str,
    task: &str,
    cancellation: &CancellationToken,
) -> Result<String, String> {
    let request = ModelRequest::new(vec![
        Message::system(subagent_system_prompt(name)),
        Message::user(task.to_string()),
    ]);
    let mut stream = services
        .model
        .complete_stream(request)
        .await
        .map_err(|err| format!("子代理模型调用失败:{err}"))?;
    collect_subagent_stream(&mut stream, cancellation).await
}

async fn collect_subagent_stream(
    stream: &mut crate::model::ModelStream,
    cancellation: &CancellationToken,
) -> Result<String, String> {
    let mut text = String::new();
    let mut completed: Option<ModelResponse> = None;
    while let Some(event) = stream.next().await {
        if cancellation.is_cancelled() {
            return Err("子代理任务已取消".to_string());
        }
        match event.map_err(|err| format!("子代理流式输出失败:{err}"))? {
            ModelStreamEvent::TextDelta(delta) => text.push_str(&delta),
            ModelStreamEvent::ReasoningDelta(_) => {}
            ModelStreamEvent::ToolCall(_) => return Err("子代理不支持工具调用".to_string()),
            ModelStreamEvent::Completed(response) => completed = Some(response),
        }
    }
    let fallback = completed
        .and_then(|response| response.text)
        .unwrap_or_default();
    let summary = if text.is_empty() { fallback } else { text };
    Ok(summary.trim().to_string())
}

fn finish_success(
    session: &Session,
    turn_id: &TurnId,
    call: &ToolCall,
    subagent_id: SubAgentId,
    name: &str,
    summary: String,
) -> ToolObservation {
    let summary = if summary.is_empty() {
        "子代理完成,但没有返回文本。".to_string()
    } else {
        summary
    };
    session.finish_subagent(turn_id, subagent_id, true, summary.clone());
    ToolObservation::success(
        call.call_id.clone(),
        ToolName::new(DELEGATE_TASK_TOOL),
        format!("子代理 {name} 完成"),
        ObservationData::Text(summary),
    )
}

fn finish_failure(
    session: &Session,
    turn_id: &TurnId,
    call: &ToolCall,
    subagent_id: SubAgentId,
    message: String,
) -> ToolObservation {
    session.finish_subagent(turn_id, subagent_id, false, message.clone());
    ToolObservation::failure(
        call.call_id.clone(),
        ToolName::new(DELEGATE_TASK_TOOL),
        message,
    )
}

fn subagent_system_prompt(name: &str) -> String {
    format!(
        "你是 onetcli agent runtime 派发的隔离子代理 `{name}`。\
只完成用户给你的子任务,不要调用工具,不要修改外部状态。\
输出简体中文结论摘要,包含关键发现、证据和下一步建议。"
    )
}
