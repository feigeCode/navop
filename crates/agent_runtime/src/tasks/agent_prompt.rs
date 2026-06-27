use crate::ResourceContext;
use crate::runtime::TaskKind;
use crate::tools::ToolSpec;

const AGENT_SYSTEM: &str = "你是 onetcli 的 AI 运维助手。请根据用户目标自主决定如何行动:\
简单问题直接、简洁地用简体中文回答;需要查询或操作资源时调用相应工具;\
面对多步任务时,先调用 `update_plan` 列出步骤并随进展更新状态(每完成一步就更新)。\
不要为简单问题强行制定计划。完成后直接给出最终回答。";

const ASK_SYSTEM: &str = "你是 onetcli 的 AI 助手。当前处于 Ask 模式:\
优先直接、简洁地回答用户问题;不要主动创建计划;不要调用工具,除非用户明确要求查询、操作或需要上下文资源。\
回答使用简体中文。";

const PLAN_SYSTEM: &str = "你是 onetcli 的 AI 助手。当前处于 Plan 模式:\
面对用户目标时先调用 `update_plan` 给出清晰步骤,再按步骤执行;每完成一步都更新计划状态。\
如果目标缺少必要信息,先提出需要补充的问题。回答使用简体中文。";

pub(super) fn build_system_prompt(
    kind: TaskKind,
    tools: &[ToolSpec],
    resources: &ResourceContext,
) -> String {
    let mut prompt = system_prompt(kind).to_string();
    append_resource_context(&mut prompt, resources);
    if !tools.is_empty() {
        let names = tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        prompt.push_str("\n\n可用 function calling 工具名: ");
        prompt.push_str(&names);
        prompt.push_str(
            "。只能调用这里列出的工具名;不要调用名为 `tool` 的通用伪工具。工具 arguments 必须是合法 JSON object。",
        );
    }
    prompt
}

fn append_resource_context(prompt: &mut String, resources: &ResourceContext) {
    if resources.is_empty() {
        return;
    }
    prompt.push_str("\n\n当前可操作资源:\n");
    prompt.push_str(&resources.describe());
    prompt.push_str(
        "调用工具时优先使用上面列出的当前资源 id 作为 connection、connection_id、session_id 等参数;\
若工具需要 database、schema、db 或 cwd 等作用域参数,优先使用资源作用域里的值。\
不要猜测未列出的资源或连接标识。",
    );
}

fn system_prompt(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::Agent => AGENT_SYSTEM,
        TaskKind::Ask => ASK_SYSTEM,
        TaskKind::Plan => PLAN_SYSTEM,
    }
}
