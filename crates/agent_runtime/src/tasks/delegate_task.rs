//! 内建子代理派发工具。
//!
//! `delegate_task` 不是业务工具,也不是某个具体 CLI 后端。它表示当前 agent
//! 把一个清晰子任务交给隔离的子代理采样执行,结果再作为 observation 回到主会话。

use crate::ids::{SubAgentId, TurnId};
use crate::runtime::{RuntimeServices, Session};
use crate::tasks::subagent_task::run_subagent_model;
use crate::tools::{ObservationData, ToolCall, ToolName, ToolObservation, ToolSpec};
use serde::Deserialize;
use serde_json::json;
use tokio_util::sync::CancellationToken;

pub const DELEGATE_TASK_TOOL: &str = "delegate_task";

#[derive(Deserialize)]
struct DelegateTaskArgs {
    name: String,
    task: String,
}

pub fn delegate_task_spec() -> ToolSpec {
    ToolSpec::new(
        DELEGATE_TASK_TOOL,
        "将一个边界清晰的子任务派发给隔离子代理执行。子代理只能调用只读业务工具,不能继续委派或更新计划,最终返回结论摘要。",
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "根据子任务用途分配的子代理名称,例如 reviewer、researcher、planner"
                },
                "task": {
                    "type": "string",
                    "description": "说明这个子代理被用来做什么,并给出需要完成的明确任务"
                }
            },
            "required": ["name", "task"],
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
    let name = args.name;
    let subagent_id = SubAgentId::from_string(call.call_id.as_str().to_string());
    session.start_subagent(
        turn_id,
        subagent_id.clone(),
        name.clone(),
        args.task.clone(),
    );

    match run_subagent_model(
        session,
        services,
        turn_id,
        &subagent_id,
        &name,
        &args.task,
        cancellation,
    )
    .await
    {
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
    if args.name.trim().is_empty() {
        return Err(ToolObservation::failure(
            call.call_id.clone(),
            call.tool_name.clone(),
            "delegate_task.name 不能为空",
        ));
    }
    Ok(args)
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
